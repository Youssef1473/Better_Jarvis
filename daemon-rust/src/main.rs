pub mod agent;
pub mod config;
mod ipc;
pub mod memory;
pub mod security;
pub mod telemetry;
pub mod tools;

use agent::GeminiClient;
use anyhow::Result;
use bytes::Bytes;
use config::AppConfig;
use futures::{SinkExt, StreamExt};
use ipc::codec::{IpcFrame, JarvisFrameCodec, MSG_TYPE_AUDIO_PCM, MSG_TYPE_CONTROL_JSON};
use ipc::pipe_security::{
    create_hardened_pipe_server, create_pipe_security_descriptor, get_parent_pid,
    get_pipe_client_pid,
};
use ipc::reassembler::PcmReassembler;
use memory::MemoryEngine;
use security::policy::PolicyEngine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::net::windows::named_pipe::NamedPipeServer;
use tokio_util::codec::Framed;
use tokio_util::sync::CancellationToken;
use tools::confirmation::ConfirmationManager;
use tools::os_control::{OsController, Win32StaOsController};
use tools::router::{execute_intent, parse_intent, RouteExecutionResult};
use tracing::{error, info, warn};

const PIPE_NAME: &str = r"\\.\pipe\jarvis-ipc";

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: String,
    method: String,
    #[serde(default)]
    params: Value,
    id: Option<Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    result: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Value>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "jarvis_daemon=info".into()),
        )
        .init();

    info!("Starting jarvis-daemon IPC server on {}", PIPE_NAME);

    // Parse CLI arguments for PID pinning and dev bypass
    let args: Vec<String> = std::env::args().collect();
    let mut passed_parent_pid: Option<u32> = None;
    let mut allow_same_user_ipc = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--parent-pid" => {
                if i + 1 < args.len() {
                    passed_parent_pid = args[i + 1].parse::<u32>().ok();
                    i += 1;
                }
            }
            "--unsafe-allow-same-user-ipc" => {
                allow_same_user_ipc = true;
                warn!("DEV OVERRIDE ACTIVE: --unsafe-allow-same-user-ipc relaxes PID pinning check");
            }
            _ => {}
        }
        i += 1;
    }

    // Determine and enforce expected parent PID pinning
    let expected_parent_pid = match get_parent_pid() {
        Ok(real_parent) => {
            if let Some(passed) = passed_parent_pid {
                if passed != real_parent {
                    error!(
                        "FATAL PID PINNING MISMATCH: Passed --parent-pid {} does not match real OS parent PID {}",
                        passed, real_parent
                    );
                    anyhow::bail!(
                        "Parent PID mismatch: passed {} != real OS parent {}",
                        passed,
                        real_parent
                    );
                }
                info!("Verified parent PID chain: {}", real_parent);
                Some(real_parent)
            } else if !allow_same_user_ipc {
                info!("Daemon operating under parent PID {}", real_parent);
                Some(real_parent)
            } else {
                None
            }
        }
        Err(e) => {
            warn!("Failed to query parent process ID via Toolhelp32: {e}");
            passed_parent_pid
        }
    };

    let config = AppConfig::find_and_load();
    info!(
        "Loaded config: fast_model='{}', heavy_model='{}', allowed_dirs={:?}",
        config.fast_model, config.heavy_model, config.allowed_directories
    );

    let policy = Arc::new(PolicyEngine::new(&config.allowed_directories));
    let confirmation_mgr = ConfirmationManager::new();

    // Initialize Memory Engine (SQLite with WAL mode + profile.md synchronization)
    let local_app_data = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into());
    let memory_db_path = PathBuf::from(local_app_data)
        .join("Better_Jarvis")
        .join("memory")
        .join("jarvis_memory.db");

    let memory = Arc::new(match MemoryEngine::open(&memory_db_path, None).await {
        Ok(m) => m,
        Err(e) => {
            warn!("Falling back to in-memory SQLite database: {}", e);
            MemoryEngine::in_memory().await?
        }
    });

    let os_controller = Arc::new(Win32StaOsController::spawn()?);
    let telemetry = Arc::new(telemetry::DaemonTelemetry::new());
    let shutdown_token = CancellationToken::new();

    let api_key = std::env::var(&config.api_key_env).ok();
    if api_key.is_none() {
        warn!(
            "Environment variable '{}' is not set; Gemini LLM calls will fail if invoked",
            config.api_key_env
        );
    }

    let gemini = Arc::new(GeminiClient::new(
        api_key,
        Some(config.fast_model),
        Some(config.heavy_model),
    )?);

    // Create kernel SDDL DACL guard granting Generic All strictly to Logon SID
    let sec_desc = Arc::new(create_pipe_security_descriptor()?);

    // Pre-create first pipe instance before entering accept loop so clients never race
    let mut server = create_hardened_pipe_server(PIPE_NAME, true, &sec_desc)?;

    loop {
        tokio::select! {
            _ = shutdown_token.cancelled() => {
                info!("Shutdown signal received in main loop");
                break;
            }
            conn_res = server.connect() => {
                if let Err(e) = conn_res {
                    if shutdown_token.is_cancelled() {
                        break;
                    }
                    return Err(e.into());
                }
                let connected_pipe = server;

                // Allocate next listening instance immediately before handling client
                server = create_hardened_pipe_server(PIPE_NAME, false, &sec_desc)?;

                // Verify connected client PID against expected parent PID pinning
                if !allow_same_user_ipc {
                    if let Some(expected_pid) = expected_parent_pid {
                        match get_pipe_client_pid(&connected_pipe) {
                            Ok(client_pid) => {
                                if client_pid != expected_pid {
                                    warn!(
                                        "SECURITY ALERT: Rejected unauthorized Named Pipe client PID {} (expected parent PID {})",
                                        client_pid, expected_pid
                                    );
                                    // Drop connected_pipe to disconnect unauthorized rogue client immediately
                                    continue;
                                }
                                info!("Verified client PID {} matches expected parent PID", client_pid);
                            }
                            Err(err) => {
                                warn!("Failed to query connected client PID: {err}; dropping connection");
                                continue;
                            }
                        }
                    }
                }

                let policy_clone = policy.clone();
                let confirmation_clone = confirmation_mgr.clone();
                let gemini_clone = gemini.clone();
                let memory_clone = memory.clone();
                let os_controller_clone = os_controller.clone();
                let shutdown_token_clone = shutdown_token.clone();
                let telemetry_clone = telemetry.clone();

                tokio::spawn(async move {
                    if let Err(err) = handle_client(
                        connected_pipe,
                        policy_clone,
                        confirmation_clone,
                        gemini_clone,
                        memory_clone,
                        os_controller_clone,
                        shutdown_token_clone,
                        telemetry_clone,
                    )
                    .await
                    {
                        warn!("IPC client disconnected: {err}");
                    }
                });
            }
        }
    }

    info!("Initiating clean shutdown sequence...");
    if let Err(e) = memory.db().shutdown().await {
        warn!("Error shutting down db actor: {e}");
    }
    if let Err(e) = os_controller.shutdown().await {
        warn!("Error shutting down os_controller actor: {e}");
    }
    crate::tools::browser_cdp::shutdown_managed_browser().await;
    info!("Clean shutdown complete. Exiting process.");
    std::process::exit(0);
}

fn pcm_to_wav(pcm_bytes: &[u8], sample_rate: u32) -> Vec<u8> {
    let mut wav = Vec::with_capacity(44 + pcm_bytes.len());
    let data_len = pcm_bytes.len() as u32;
    let file_len = 36 + data_len;
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&file_len.to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(pcm_bytes);
    wav
}

#[allow(clippy::too_many_arguments)]
async fn process_utterance(
    utterance: String,
    turn_id: u32,
    policy: Arc<PolicyEngine>,
    confirmation_mgr: ConfirmationManager,
    gemini: Arc<GeminiClient>,
    memory: Arc<MemoryEngine>,
    os_controller: Arc<Win32StaOsController>,
    turn_token: CancellationToken,
    out_tx: tokio::sync::mpsc::Sender<IpcFrame>,
    telemetry: Arc<telemetry::DaemonTelemetry>,
    t_ipc_rx: std::time::Instant,
    ack_req_id: Option<Value>,
) -> Result<()> {
    let mut active_timing = telemetry::ActiveTurnTiming::new(turn_id);
    active_timing.t_ipc_rx = t_ipc_rx;
    active_timing.t_router_start = Some(std::time::Instant::now());

    let intent = parse_intent(&utterance);
    info!("Fast router parsed intent: {:?}", intent);

    let cur_dir = std::env::current_dir().ok();
    let workspace_ref = cur_dir.as_deref().or(Some(Path::new(".")));

    let exec_res = execute_intent(
        &intent,
        &policy,
        &confirmation_mgr,
        workspace_ref,
        os_controller.as_ref(),
    )
    .await;

    match exec_res {
        Ok(RouteExecutionResult::Executed { output, .. }) => {
            active_timing.t_router_done = Some(std::time::Instant::now());
            active_timing.path = telemetry::ExecutionPath::FastPath;
            active_timing.routing_reason = format!("{:?}", intent);

            if let Some(id) = ack_req_id {
                let ack = JsonRpcResponse {
                    jsonrpc: "2.0",
                    result: json!({ "status": "PROCESSED", "recognized_text": utterance, "response": output }),
                    id: Some(id),
                };
                if let Ok(payload) = serde_json::to_vec(&ack) {
                    let _ = out_tx.send(IpcFrame { msg_type: MSG_TYPE_CONTROL_JSON, turn_id, payload: Bytes::from(payload) }).await;
                }
            }

            if !output.is_empty() {
                active_timing.t_first_tts_dispatch = Some(std::time::Instant::now());
                let tts_notify = json!({
                    "jsonrpc": "2.0",
                    "method": "tts.speak",
                    "params": { "text": output, "turn_id": turn_id }
                });
                if let Ok(payload) = serde_json::to_vec(&tts_notify) {
                    let _ = out_tx.send(IpcFrame { msg_type: MSG_TYPE_CONTROL_JSON, turn_id, payload: Bytes::from(payload) }).await;
                }
            }

            telemetry.record_completed_turn(active_timing);
        }
        Ok(RouteExecutionResult::Denied { reason, .. }) => {
            active_timing.t_router_done = Some(std::time::Instant::now());
            active_timing.path = telemetry::ExecutionPath::FastPath;
            active_timing.routing_reason = format!("{:?}", intent);

            let denied_msg = format!("Action cancelled: {}", reason);
            if let Some(id) = ack_req_id {
                let ack = JsonRpcResponse {
                    jsonrpc: "2.0",
                    result: json!({ "status": "DENIED", "recognized_text": utterance, "response": denied_msg }),
                    id: Some(id),
                };
                if let Ok(payload) = serde_json::to_vec(&ack) {
                    let _ = out_tx.send(IpcFrame { msg_type: MSG_TYPE_CONTROL_JSON, turn_id, payload: Bytes::from(payload) }).await;
                }
            }
            active_timing.t_first_tts_dispatch = Some(std::time::Instant::now());
            let tts_notify = json!({
                "jsonrpc": "2.0",
                "method": "tts.speak",
                "params": { "text": denied_msg, "turn_id": turn_id }
            });
            if let Ok(payload) = serde_json::to_vec(&tts_notify) {
                let _ = out_tx.send(IpcFrame { msg_type: MSG_TYPE_CONTROL_JSON, turn_id, payload: Bytes::from(payload) }).await;
            }

            telemetry.record_completed_turn(active_timing);
        }
        Ok(RouteExecutionResult::FallbackToLlm { prompt }) => {
            active_timing.t_router_done = Some(std::time::Instant::now());
            active_timing.t_llm_start = Some(std::time::Instant::now());

            info!("Forwarding prompt to Gemini LLM planner (streaming): \"{}\"", prompt);

            let (tts_chunk_tx, mut tts_chunk_rx) = tokio::sync::mpsc::channel::<crate::agent::gemini_client::TtsChunk>(32);
            let out_tx_clone = out_tx.clone();

            let forwarder = tokio::spawn(async move {
                let mut first_dispatch_time = None;
                while let Some(chunk) = tts_chunk_rx.recv().await {
                    if first_dispatch_time.is_none() && !chunk.text.is_empty() {
                        first_dispatch_time = Some(std::time::Instant::now());
                    }
                    let notify = json!({
                        "jsonrpc": "2.0",
                        "method": "tts.speak_chunk",
                        "params": {
                            "turn_id": chunk.turn_id,
                            "seq": chunk.seq,
                            "text": chunk.text,
                            "is_final": chunk.is_final
                        }
                    });
                    if let Ok(payload) = serde_json::to_vec(&notify) {
                        let _ = out_tx_clone.send(IpcFrame {
                            msg_type: MSG_TYPE_CONTROL_JSON,
                            turn_id: chunk.turn_id,
                            payload: Bytes::from(payload),
                        }).await;
                    }
                }
                first_dispatch_time
            });

            let t_first_token = Arc::new(std::sync::Mutex::new(None));
            let t_first_token_clone = t_first_token.clone();

            let res = gemini
                .chat_with_tools_streaming(
                    &prompt,
                    turn_id,
                    None,
                    &policy,
                    &confirmation_mgr,
                    workspace_ref,
                    Some(&memory),
                    os_controller.as_ref(),
                    &turn_token,
                    Some(tts_chunk_tx),
                    move || {
                        let mut g = t_first_token_clone.lock().unwrap();
                        if g.is_none() {
                            *g = Some(std::time::Instant::now());
                        }
                    },
                )
                .await;

            let first_tts = forwarder.await.ok().flatten();
            active_timing.t_first_tts_dispatch = first_tts;
            active_timing.t_llm_first_token = *t_first_token.lock().unwrap();
            active_timing.t_llm_done = Some(std::time::Instant::now());

            match res {
                Ok((reply, model_sel)) => {
                    active_timing.path = match model_sel.tier {
                        crate::agent::model_router::ModelTier::Fast => telemetry::ExecutionPath::CloudFast,
                        crate::agent::model_router::ModelTier::Heavy => telemetry::ExecutionPath::CloudHeavy,
                    };
                    active_timing.routing_reason = model_sel.reason.to_string();

                    if let Some(id) = ack_req_id {
                        let ack = JsonRpcResponse {
                            jsonrpc: "2.0",
                            result: json!({ "status": "PROCESSED", "recognized_text": utterance, "response": reply }),
                            id: Some(id),
                        };
                        if let Ok(payload) = serde_json::to_vec(&ack) {
                            let _ = out_tx.send(IpcFrame { msg_type: MSG_TYPE_CONTROL_JSON, turn_id, payload: Bytes::from(payload) }).await;
                        }
                    }

                    telemetry.record_completed_turn(active_timing);
                }
                Err(err) => {
                    warn!("Gemini LLM streaming failed: {}", err);
                    if let Some(id) = ack_req_id {
                        let ack = JsonRpcResponse {
                            jsonrpc: "2.0",
                            result: json!({ "status": "ERROR", "error": format!("AI Planner unavailable: {}", err) }),
                            id: Some(id),
                        };
                        if let Ok(payload) = serde_json::to_vec(&ack) {
                            let _ = out_tx.send(IpcFrame { msg_type: MSG_TYPE_CONTROL_JSON, turn_id, payload: Bytes::from(payload) }).await;
                        }
                    }
                }
            }
        }
        Err(err) => {
            error!("Intent execution failed: {}", err);
            if let Some(id) = ack_req_id {
                let ack = JsonRpcResponse {
                    jsonrpc: "2.0",
                    result: json!({ "status": "ERROR", "error": format!("Execution failed: {}", err) }),
                    id: Some(id),
                };
                if let Ok(payload) = serde_json::to_vec(&ack) {
                    let _ = out_tx.send(IpcFrame { msg_type: MSG_TYPE_CONTROL_JSON, turn_id, payload: Bytes::from(payload) }).await;
                }
            }
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn handle_client(
    pipe: NamedPipeServer,
    policy: Arc<PolicyEngine>,
    confirmation_mgr: ConfirmationManager,
    gemini: Arc<GeminiClient>,
    memory: Arc<MemoryEngine>,
    os_controller: Arc<Win32StaOsController>,
    shutdown_token: CancellationToken,
    telemetry: Arc<telemetry::DaemonTelemetry>,
) -> Result<()> {
    info!("C++ Host connected to Named Pipe");
    let mut framed = Framed::new(pipe, JarvisFrameCodec);
    let mut conf_rx = confirmation_mgr.subscribe();
    let mut pcm_reassembler = PcmReassembler::new();
    let active_turn_token: Arc<std::sync::Mutex<Option<CancellationToken>>> =
        Arc::new(std::sync::Mutex::new(None));
    let (out_tx, mut out_rx) = tokio::sync::mpsc::channel::<IpcFrame>(128);

    loop {
        tokio::select! {
            _ = shutdown_token.cancelled() => {
                break;
            }
            Some(out_frame) = out_rx.recv() => {
                framed.send(out_frame).await?;
            }
            frame_res = framed.next() => {
                let frame = match frame_res {
                    Some(Ok(f)) => f,
                    Some(Err(e)) => return Err(e.into()),
                    None => break,
                };

                let turn_id = frame.turn_id;

                match frame.msg_type {
                    MSG_TYPE_AUDIO_PCM => {
                        info!("Received {} bytes audio PCM chunk from host (turn_id={})", frame.payload.len(), turn_id);
                        if let Some(assembled_pcm) = pcm_reassembler.handle_frame(frame.turn_id, &frame.payload) {
                            if assembled_pcm.len() >= 3200 {
                                if let Some(prev) = active_turn_token.lock().unwrap().take() {
                                    info!("Cancelling previous in-flight turn for incoming audio utterance");
                                    prev.cancel();
                                }
                                let turn_token = shutdown_token.child_token();
                                *active_turn_token.lock().unwrap() = Some(turn_token.clone());

                                let wav = pcm_to_wav(&assembled_pcm, 16000);
                                let transcribed = match gemini.transcribe_audio(&wav).await {
                                    Ok(t) if !t.is_empty() => t,
                                    Ok(_) => {
                                        info!("Gemini transcribed silence/empty");
                                        *active_turn_token.lock().unwrap() = None;
                                        continue;
                                    }
                                    Err(e) => {
                                        warn!("Gemini audio transcription failed: {e}");
                                        *active_turn_token.lock().unwrap() = None;
                                        continue;
                                    }
                                };
                                info!("Transcribed voice command: \"{}\"", transcribed);

                                let t_rx = std::time::Instant::now();
                                let policy_c = policy.clone();
                                let conf_c = confirmation_mgr.clone();
                                let gemini_c = gemini.clone();
                                let mem_c = memory.clone();
                                let os_c = os_controller.clone();
                                let out_c = out_tx.clone();
                                let telem_c = telemetry.clone();
                                let token_c = turn_token.clone();

                                tokio::spawn(async move {
                                    let _ = process_utterance(
                                        transcribed,
                                        turn_id,
                                        policy_c,
                                        conf_c,
                                        gemini_c,
                                        mem_c,
                                        os_c,
                                        token_c,
                                        out_c,
                                        telem_c,
                                        t_rx,
                                        None,
                                    ).await;
                                });
                            }
                        }
                    }
                    MSG_TYPE_CONTROL_JSON => {
                        let req: JsonRpcRequest = serde_json::from_slice(&frame.payload)?;
                        info!("RPC <- method={} id={:?}", req.method, req.id);

                        if req.method == "system.ping" {
                            let now_ms = SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis() as u64;

                            let pong = JsonRpcResponse {
                                jsonrpc: "2.0",
                                result: json!({
                                    "status": "READY",
                                    "daemon_version": env!("CARGO_PKG_VERSION"),
                                    "timestamp_ms": now_ms,
                                    "echo": req.params
                                }),
                                id: req.id,
                            };

                            let payload = Bytes::from(serde_json::to_vec(&pong)?);
                            let _ = out_tx
                                .send(IpcFrame {
                                    msg_type: MSG_TYPE_CONTROL_JSON,
                                    turn_id,
                                    payload,
                                })
                                .await;
                        } else if req.method == "system.barge_in" {
                            info!("Received system.barge_in RPC; cancelling active turn");
                            let was_active = if let Some(token) = active_turn_token.lock().unwrap().take() {
                                token.cancel();
                                true
                            } else {
                                false
                            };
                            let resp = JsonRpcResponse {
                                jsonrpc: "2.0",
                                result: json!({ "status": "BARGED_IN", "interrupted_turn": was_active }),
                                id: req.id.clone(),
                            };
                            let payload = Bytes::from(serde_json::to_vec(&resp)?);
                            let _ = out_tx
                                .send(IpcFrame {
                                    msg_type: MSG_TYPE_CONTROL_JSON,
                                    turn_id,
                                    payload,
                                })
                                .await;
                        } else if req.method == "system.shutdown" {
                            info!("Received system.shutdown RPC; replying ACK and triggering shutdown");
                            let resp = JsonRpcResponse {
                                jsonrpc: "2.0",
                                result: json!({ "status": "SHUTTING_DOWN" }),
                                id: req.id,
                            };
                            let payload = Bytes::from(serde_json::to_vec(&resp)?);
                            let _ = out_tx
                                .send(IpcFrame {
                                    msg_type: MSG_TYPE_CONTROL_JSON,
                                    turn_id,
                                    payload,
                                })
                                .await;
                            shutdown_token.cancel();
                            break;
                        } else if req.method == "speech.transcribed" {
                            let text = req
                                .params
                                .get("text")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default();
                            let duration_ms = req
                                .params
                                .get("duration_ms")
                                .and_then(|v| v.as_u64())
                                .unwrap_or_default();
                            info!("Speech recognized: \"{}\" ({} ms)", text, duration_ms);

                            if let Some(prev) = active_turn_token.lock().unwrap().take() {
                                info!("Cancelling previous in-flight turn for incoming transcribed speech");
                                prev.cancel();
                            }
                            let turn_token = shutdown_token.child_token();
                            *active_turn_token.lock().unwrap() = Some(turn_token.clone());

                            let t_rx = std::time::Instant::now();
                            let policy_c = policy.clone();
                            let conf_c = confirmation_mgr.clone();
                            let gemini_c = gemini.clone();
                            let mem_c = memory.clone();
                            let os_c = os_controller.clone();
                            let out_c = out_tx.clone();
                            let telem_c = telemetry.clone();
                            let token_c = turn_token.clone();
                            let req_id = req.id.clone();
                            let text_str = text.to_string();

                            tokio::spawn(async move {
                                let _ = process_utterance(
                                    text_str,
                                    turn_id,
                                    policy_c,
                                    conf_c,
                                    gemini_c,
                                    mem_c,
                                    os_c,
                                    token_c,
                                    out_c,
                                    telem_c,
                                    t_rx,
                                    req_id,
                                ).await;
                            });
                        } else if req.method == "system.confirm" {
                            let confirm_id = req.params.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
                            let approved = req.params.get("approved").and_then(|v| v.as_bool()).unwrap_or(false);
                            let resolved = confirmation_mgr.resolve_confirmation(confirm_id, approved).await;

                            let resp = JsonRpcResponse {
                                jsonrpc: "2.0",
                                result: json!({
                                    "resolved": resolved,
                                    "id": confirm_id,
                                    "approved": approved
                                }),
                                id: req.id,
                            };
                            let payload = Bytes::from(serde_json::to_vec(&resp)?);
                            let _ = out_tx
                                .send(IpcFrame {
                                    msg_type: MSG_TYPE_CONTROL_JSON,
                                    turn_id,
                                    payload,
                                })
                                .await;
                        } else if req.method == "memory.list_facts" {
                            let facts = memory.db().list_facts().await.unwrap_or_default();
                            let resp = JsonRpcResponse {
                                jsonrpc: "2.0",
                                result: json!({ "facts": facts }),
                                id: req.id,
                            };
                            let payload = Bytes::from(serde_json::to_vec(&resp)?);
                            let _ = out_tx
                                .send(IpcFrame {
                                    msg_type: MSG_TYPE_CONTROL_JSON,
                                    turn_id,
                                    payload,
                                })
                                .await;
                        } else if req.method == "memory.upsert_fact" {
                            let key = req.params.get("key").and_then(|v| v.as_str()).unwrap_or_default();
                            let val = req.params.get("value").and_then(|v| v.as_str()).unwrap_or_default();
                            let tier = req.params.get("tier").and_then(|v| v.as_str()).unwrap_or("preference");

                            let res = if !key.is_empty() {
                                let _ = memory.db().upsert_fact(key, val, tier).await;
                                let _ = memory.sync().export_profile_atomic().await;
                                true
                            } else {
                                false
                            };

                            let resp = JsonRpcResponse {
                                jsonrpc: "2.0",
                                result: json!({ "success": res, "key": key, "value": val }),
                                id: req.id,
                            };
                            let payload = Bytes::from(serde_json::to_vec(&resp)?);
                            let _ = out_tx
                                .send(IpcFrame {
                                    msg_type: MSG_TYPE_CONTROL_JSON,
                                    turn_id,
                                    payload,
                                })
                                .await;
                        }
                    }
                    other => {
                        error!("Unknown IPC frame type: 0x{:04x}", other);
                    }
                }
            }
            Ok(conf_notice) = conf_rx.recv() => {
                info!("Sending HUD confirmation request down Named Pipe: {:?}", conf_notice);
                let payload = Bytes::from(serde_json::to_vec(&conf_notice)?);
                let _ = out_tx.send(IpcFrame {
                    msg_type: MSG_TYPE_CONTROL_JSON,
                    turn_id: 0,
                    payload,
                }).await;
            }
        }
    }

    Ok(())
}
