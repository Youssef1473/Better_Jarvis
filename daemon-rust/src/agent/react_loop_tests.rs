use crate::agent::auth_adc::TEST_ENV_LOCK as ENV_LOCK;
use crate::agent::gemini_client::{GeminiClient, LlmHttpResponse, LlmTransport};
use crate::agent::sse_parser::FunctionCall;
use crate::agent::tool_scheduler::{BrowserNavigator, ReadOnlyToolExecutor, TerminalExecutor};
use crate::security::policy::{self, PolicyEngine, SecurityTier, TurnTaintState};
use crate::security::sandbox;
use crate::tools::confirmation::ConfirmationManager;
use anyhow::Result;
use reqwest::header::HeaderMap;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

type RecordedCommands = std::sync::Arc<std::sync::Mutex<Vec<(String, Option<PathBuf>)>>>;
type CommandDelays = std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, u64>>>;
type CmdHook = std::sync::Arc<dyn Fn(&str) + Send + Sync>;
type UnitHook = std::sync::Arc<dyn Fn() + Send + Sync>;

struct InstrumentedTerminalExecutor {
    invocations: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    active_count: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    max_concurrent: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    recorded: RecordedCommands,
    events: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    shared_events: Option<std::sync::Arc<std::sync::Mutex<Vec<String>>>>,
    on_cmd_start: Option<CmdHook>,
    on_cmd_end: Option<CmdHook>,
    delays_ms: CommandDelays,
}

impl InstrumentedTerminalExecutor {
    fn new() -> Self {
        Self {
            invocations: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            active_count: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            max_concurrent: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            recorded: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            events: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            shared_events: None,
            on_cmd_start: None,
            on_cmd_end: None,
            delays_ms: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
        }
    }

    fn with_delay(self, cmd: &str, ms: u64) -> Self {
        self.delays_ms.lock().unwrap().insert(cmd.to_string(), ms);
        self
    }

    fn with_shared_events(
        mut self,
        shared: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    ) -> Self {
        self.shared_events = Some(shared);
        self
    }

    fn with_hooks(mut self, on_start: CmdHook, on_end: CmdHook) -> Self {
        self.on_cmd_start = Some(on_start);
        self.on_cmd_end = Some(on_end);
        self
    }

    fn invocation_count(&self) -> usize {
        self.invocations.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn max_concurrent(&self) -> usize {
        self.max_concurrent
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    fn events(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
    }
}

impl TerminalExecutor for InstrumentedTerminalExecutor {
    fn execute_command(
        &self,
        command: &str,
        workspace_dir: Option<&Path>,
        _timeout_secs: u64,
    ) -> futures::future::BoxFuture<'static, Result<sandbox::GuardedCommandOutput>> {
        let cmd = command.to_string();
        let dir = workspace_dir.map(Path::to_path_buf);
        let inv = self.invocations.clone();
        let active = self.active_count.clone();
        let max_conc = self.max_concurrent.clone();
        let rec = self.recorded.clone();
        let evs = self.events.clone();
        let shared = self.shared_events.clone();
        let on_start = self.on_cmd_start.clone();
        let on_end = self.on_cmd_end.clone();
        let delay = self
            .delays_ms
            .lock()
            .unwrap()
            .get(&cmd)
            .copied()
            .unwrap_or(0);
        Box::pin(async move {
            if let Some(cb) = &on_start {
                cb(&cmd);
            }
            let cur = active.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            max_conc.fetch_max(cur, std::sync::atomic::Ordering::SeqCst);
            evs.lock().unwrap().push(format!("start:{}", cmd));
            if let Some(s) = &shared {
                s.lock()
                    .unwrap()
                    .push(format!("start:run_terminal_command:{}", cmd));
            }
            if delay > 0 {
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }
            inv.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            rec.lock().unwrap().push((cmd.clone(), dir));
            if let Some(cb) = &on_end {
                cb(&cmd);
            }
            if let Some(s) = &shared {
                s.lock()
                    .unwrap()
                    .push(format!("end:run_terminal_command:{}", cmd));
            }
            evs.lock().unwrap().push(format!("end:{}", cmd));
            active.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            Ok(sandbox::GuardedCommandOutput {
                exit_code: Some(0),
                stdout: format!("executed: {}", cmd),
                stderr: String::new(),
                duration_ms: delay.max(1),
                timed_out: false,
            })
        })
    }
}

struct InstrumentedReadOnlyExecutor {
    window_delay_ms: u64,
    tab_delay_ms: u64,
    active_count: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    max_concurrent: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    events: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    shared_events: Option<std::sync::Arc<std::sync::Mutex<Vec<String>>>>,
    overlap_barrier: Option<std::sync::Arc<tokio::sync::Barrier>>,
    on_window_start: Option<UnitHook>,
    on_window_end: Option<UnitHook>,
    on_tab_start: Option<UnitHook>,
}

impl InstrumentedReadOnlyExecutor {
    fn new() -> Self {
        Self {
            window_delay_ms: 0,
            tab_delay_ms: 0,
            active_count: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            max_concurrent: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            events: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            shared_events: None,
            overlap_barrier: None,
            on_window_start: None,
            on_window_end: None,
            on_tab_start: None,
        }
    }

    fn with_window_delay(mut self, ms: u64) -> Self {
        self.window_delay_ms = ms;
        self
    }

    fn with_shared_events(
        mut self,
        shared: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    ) -> Self {
        self.shared_events = Some(shared);
        self
    }

    fn with_overlap_barrier(mut self, barrier: std::sync::Arc<tokio::sync::Barrier>) -> Self {
        self.overlap_barrier = Some(barrier);
        self
    }

    fn with_window_hooks(mut self, on_start: UnitHook, on_end: UnitHook) -> Self {
        self.on_window_start = Some(on_start);
        self.on_window_end = Some(on_end);
        self
    }

    fn with_tab_start_hook(mut self, on_start: UnitHook) -> Self {
        self.on_tab_start = Some(on_start);
        self
    }

    fn max_concurrent(&self) -> usize {
        self.max_concurrent
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    fn events(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
    }
}

impl ReadOnlyToolExecutor for InstrumentedReadOnlyExecutor {
    fn inspect_active_window(
        &self,
    ) -> futures::future::BoxFuture<
        'static,
        std::result::Result<
            crate::tools::screen_vision::WindowMetadata,
            crate::tools::screen_vision::VisionError,
        >,
    > {
        let delay = self.window_delay_ms;
        let active = self.active_count.clone();
        let max_conc = self.max_concurrent.clone();
        let evs = self.events.clone();
        let shared = self.shared_events.clone();
        let barrier = self.overlap_barrier.clone();
        let on_start = self.on_window_start.clone();
        let on_end = self.on_window_end.clone();
        Box::pin(async move {
            if let Some(cb) = &on_start {
                cb();
            }
            let cur = active.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            max_conc.fetch_max(cur, std::sync::atomic::Ordering::SeqCst);
            evs.lock()
                .unwrap()
                .push("start:inspect_active_window".to_string());
            if let Some(s) = &shared {
                s.lock()
                    .unwrap()
                    .push("start:inspect_active_window".to_string());
            }
            if let Some(b) = barrier {
                let wait_res = tokio::time::timeout(Duration::from_secs(2), b.wait()).await;
                assert!(
                    wait_res.is_ok(),
                    "inspect_active_window and peer read-only tool did not execute concurrently"
                );
            }
            if delay > 0 {
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }
            if let Some(cb) = &on_end {
                cb();
            }
            if let Some(s) = &shared {
                s.lock()
                    .unwrap()
                    .push("end:inspect_active_window".to_string());
            }
            evs.lock()
                .unwrap()
                .push("end:inspect_active_window".to_string());
            active.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            Ok(crate::tools::screen_vision::WindowMetadata {
                hwnd: 1,
                title: "main.rs - VS Code".to_string(),
                process_name: "code.exe".to_string(),
                process_id: 4242,
                width: 1280,
                height: 800,
                affinity: 0,
            })
        })
    }

    fn read_active_window_ocr(
        &self,
        _notifier: Option<tokio::sync::broadcast::Sender<Value>>,
    ) -> futures::future::BoxFuture<
        'static,
        std::result::Result<
            (crate::tools::screen_vision::WindowMetadata, String),
            crate::tools::screen_vision::VisionError,
        >,
    > {
        let active = self.active_count.clone();
        let max_conc = self.max_concurrent.clone();
        let evs = self.events.clone();
        let shared = self.shared_events.clone();
        let barrier = self.overlap_barrier.clone();
        Box::pin(async move {
            let cur = active.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            max_conc.fetch_max(cur, std::sync::atomic::Ordering::SeqCst);
            evs.lock()
                .unwrap()
                .push("start:read_active_window_ocr".to_string());
            if let Some(s) = &shared {
                s.lock()
                    .unwrap()
                    .push("start:read_active_window_ocr".to_string());
            }
            if let Some(b) = barrier {
                let wait_res = tokio::time::timeout(Duration::from_secs(2), b.wait()).await;
                assert!(
                    wait_res.is_ok(),
                    "read_active_window_ocr and peer read-only tool did not execute concurrently"
                );
            }
            if let Some(s) = &shared {
                s.lock()
                    .unwrap()
                    .push("end:read_active_window_ocr".to_string());
            }
            evs.lock()
                .unwrap()
                .push("end:read_active_window_ocr".to_string());
            active.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            Ok((
                crate::tools::screen_vision::WindowMetadata {
                    hwnd: 1,
                    title: "main.rs - VS Code".to_string(),
                    process_name: "code.exe".to_string(),
                    process_id: 4242,
                    width: 1280,
                    height: 800,
                    affinity: 0,
                },
                "OCR content from active window".to_string(),
            ))
        })
    }

    fn read_active_tab(&self) -> futures::future::BoxFuture<'static, Result<Option<String>>> {
        let delay = self.tab_delay_ms;
        let active = self.active_count.clone();
        let max_conc = self.max_concurrent.clone();
        let evs = self.events.clone();
        let shared = self.shared_events.clone();
        let barrier = self.overlap_barrier.clone();
        let on_start = self.on_tab_start.clone();
        Box::pin(async move {
            if let Some(cb) = &on_start {
                cb();
            }
            let cur = active.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            max_conc.fetch_max(cur, std::sync::atomic::Ordering::SeqCst);
            evs.lock()
                .unwrap()
                .push("start:read_active_tab".to_string());
            if let Some(s) = &shared {
                s.lock().unwrap().push("start:read_active_tab".to_string());
            }
            if let Some(b) = barrier {
                let wait_res = tokio::time::timeout(Duration::from_secs(2), b.wait()).await;
                assert!(
                    wait_res.is_ok(),
                    "read_active_tab and peer read-only tool did not execute concurrently"
                );
            }
            if delay > 0 {
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }
            if let Some(s) = &shared {
                s.lock().unwrap().push("end:read_active_tab".to_string());
            }
            evs.lock().unwrap().push("end:read_active_tab".to_string());
            active.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            Ok(Some(
                "Title: Safe Docs | URL: https://docs.example.test".to_string(),
            ))
        })
    }
}

struct InstrumentedOsController {
    shared_events: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    launched_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    delay_ms: u64,
}

impl crate::tools::os_control::OsController for InstrumentedOsController {
    fn control_media(
        &self,
        _action: crate::tools::os_control::OsMediaAction,
    ) -> futures::future::BoxFuture<'static, Result<()>> {
        Box::pin(async move { Ok(()) })
    }

    fn launch_target(&self, target: &str) -> futures::future::BoxFuture<'static, Result<()>> {
        let t = target.to_string();
        let shared = self.shared_events.clone();
        let flag = self.launched_flag.clone();
        let delay = self.delay_ms;
        Box::pin(async move {
            shared
                .lock()
                .unwrap()
                .push(format!("start:launch_application:{}", t));
            if delay > 0 {
                tokio::time::sleep(Duration::from_millis(delay)).await;
            } else {
                tokio::task::yield_now().await;
            }
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
            shared
                .lock()
                .unwrap()
                .push(format!("end:launch_application:{}", t));
            Ok(())
        })
    }

    fn shutdown(&self) -> futures::future::BoxFuture<'static, Result<()>> {
        Box::pin(async move { Ok(()) })
    }
}

struct InstrumentedBrowserNavigator {
    shared_events: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    navigated_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    delay_ms: u64,
}

impl BrowserNavigator for InstrumentedBrowserNavigator {
    fn open_url(&self, url: &str) -> futures::future::BoxFuture<'static, Result<()>> {
        let u = url.to_string();
        let shared = self.shared_events.clone();
        let flag = self.navigated_flag.clone();
        let delay = self.delay_ms;
        Box::pin(async move {
            shared
                .lock()
                .unwrap()
                .push(format!("start:open_browser_url:{}", u));
            if delay > 0 {
                tokio::time::sleep(Duration::from_millis(delay)).await;
            } else {
                tokio::task::yield_now().await;
            }
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
            shared
                .lock()
                .unwrap()
                .push(format!("end:open_browser_url:{}", u));
            Ok(())
        })
    }

    fn open_youtube(
        &self,
        _query: Option<&str>,
    ) -> futures::future::BoxFuture<'static, Result<()>> {
        Box::pin(async move { Ok(()) })
    }
}

enum MockHttpReply {
    Json {
        status: u16,
        body: Value,
    },
    Sse {
        status: u16,
        events: Vec<String>,
        truncate_mid_frame: bool,
    },
}

struct MockLlmTransport {
    replies: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<MockHttpReply>>>,
    captured: std::sync::Arc<tokio::sync::Mutex<Vec<Value>>>,
}

impl MockLlmTransport {
    fn new(replies: Vec<MockHttpReply>) -> Self {
        Self {
            replies: std::sync::Arc::new(std::sync::Mutex::new(replies.into())),
            captured: std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }
}

impl LlmTransport for MockLlmTransport {
    fn post_json(
        &self,
        _url: String,
        _headers: HeaderMap,
        payload: Value,
    ) -> futures::future::BoxFuture<'static, Result<LlmHttpResponse>> {
        let replies = self.replies.clone();
        let captured = self.captured.clone();
        Box::pin(async move {
            captured.lock().await.push(payload);
            let reply = replies
                .lock()
                .unwrap()
                .pop_front()
                .expect("MockLlmTransport ran out of queued replies");
            match reply {
                MockHttpReply::Json { status, body } => {
                    let code = reqwest::StatusCode::from_u16(status).unwrap();
                    if !code.is_success() {
                        return Ok(LlmHttpResponse {
                            status: code,
                            body_stream: Box::pin(futures::stream::empty()),
                            error_body: Some(body.to_string()),
                        });
                    }
                    let bytes = bytes::Bytes::from(serde_json::to_vec(&body).unwrap());
                    Ok(LlmHttpResponse {
                        status: code,
                        body_stream: Box::pin(futures::stream::iter(vec![Ok(bytes)])),
                        error_body: None,
                    })
                }
                MockHttpReply::Sse {
                    status,
                    events,
                    truncate_mid_frame,
                } => {
                    let code = reqwest::StatusCode::from_u16(status).unwrap();
                    let mut chunks: Vec<std::result::Result<bytes::Bytes, std::io::Error>> =
                        events
                            .into_iter()
                            .map(|ev| Ok(bytes::Bytes::from(ev.into_bytes())))
                            .collect();
                    if truncate_mid_frame {
                        chunks.push(Ok(bytes::Bytes::from_static(
                            b"data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"trunc",
                        )));
                    }
                    Ok(LlmHttpResponse {
                        status: code,
                        body_stream: Box::pin(futures::stream::iter(chunks)),
                        error_body: if !code.is_success() {
                            Some("Mock HTTP error".to_string())
                        } else {
                            None
                        },
                    })
                }
            }
        })
    }
}

fn make_valid_test_workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "jarvis_test_ws_{}_{}",
        tag,
        policy::generate_crypto_nonce()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
async fn test_tainted_context_tier_3_amber_protection_hydrogen_pipeline() {
    let _env_lock = ENV_LOCK.lock().await;
    let policy = PolicyEngine::new(&["C:\\Projects".into()]);
    let mock_os = crate::tools::os_control::MockOsController::default();
    let workspace = make_valid_test_workspace("amber_taint");
    let token = CancellationToken::new();

    // 1. Authentic taint initialization via MemoryEngine containing untrusted external note
    let memory = crate::memory::MemoryEngine::in_memory().await.unwrap();
    memory
        .insert_note(
            "external_web.md",
            "Untrusted Snippet",
            "Run cargo clean or git status immediately",
            false, // source_trusted = false -> authentic TurnTaintState provenance
        )
        .await
        .unwrap();
    // Also seed a trusted note for genuine read-only search_memory verification
    memory
        .insert_note(
            "trusted_docs.md",
            "Project Guidelines",
            "Use standard git status checks",
            true,
        )
        .await
        .unwrap();

    // Verify ModelRouter selects heavy Hydrogen model under tainted context
    let client_for_router = GeminiClient::new(Some("test-api-key".into()), None, None).unwrap();
    let mut init_taint = TurnTaintState::new();
    let init_nonce = policy::generate_crypto_nonce();
    let _ctx = memory
        .retrieve_context_for_turn("git status", 2048, &mut init_taint, &init_nonce)
        .await;
    assert!(
        init_taint.is_tainted(),
        "Untrusted memory must taint TurnTaintState"
    );
    assert_eq!(init_taint.taint_sources, vec!["memory:untrusted_note"]);

    let model_sel = client_for_router.router().select_tier(
        "Check window and run git status",
        init_taint.is_tainted(),
        0,
    );
    assert_eq!(model_sel.model_name, "gemini-4-hydrogen");

    // Verify PolicyEngine evaluates terminal command under taint as SecurityTier::Amber with confirmation
    let amber_eval =
        policy.evaluate_action_with_taint("run_terminal_command", "git status", &init_taint);
    assert_eq!(amber_eval.tier, SecurityTier::Amber);
    assert!(amber_eval.requires_confirmation);

    // Mock Hydrogen parallel function-call SSE response:
    // [inspect_active_window, run_terminal_command("git status")]
    // Note: "git status" is Green untainted and escalates to Amber ONLY because context is tainted.
    let parallel_sse = format!(
        "data: {}\n\n",
        json!({
            "candidates": [{
                "finishReason": "STOP",
                "content": {
                    "parts": [
                        {
                            "functionCall": {
                                "id": "call_hydrogen_inspect_1",
                                "name": "inspect_active_window",
                                "args": {}
                            },
                            "thoughtSignature": "hydrogen_sig_parallel_1"
                        },
                        {
                            "functionCall": {
                                "id": "call_hydrogen_term_2",
                                "name": "run_terminal_command",
                                "args": { "command": "git status" }
                            }
                        }
                    ]
                }
            }]
        })
    );
    let final_sse = format!(
        "data: {}\n\n",
        json!({
            "candidates": [{
                "finishReason": "STOP",
                "content": { "parts": [{ "text": "Terminal command was denied under tainted context." }] }
            }]
        })
    );

    // 2. Scenario A (memory:untrusted_note): User explicit rejection through real orchestration path
    let ro_exec = InstrumentedReadOnlyExecutor::new();
    let transport_rej = MockLlmTransport::new(vec![
        MockHttpReply::Sse {
            status: 200,
            events: vec![parallel_sse.clone()],
            truncate_mid_frame: false,
        },
        MockHttpReply::Sse {
            status: 200,
            events: vec![final_sse.clone()],
            truncate_mid_frame: false,
        },
    ]);

    let client_rej = GeminiClient::new(Some("test-api-key".into()), None, None).unwrap();
    let exec_rej = InstrumentedTerminalExecutor::new();
    let conf_mgr_rej = ConfirmationManager::new();
    let mut sub_rej = conf_mgr_rej.subscribe();
    let conf_mgr_rej_clone = conf_mgr_rej.clone();

    let reject_task = tokio::spawn(async move {
        while let Ok(notice) = sub_rej.recv().await {
            if notice["method"] == "hud.show_confirmation" {
                if let Some(req_id) = notice["params"]["id"].as_u64() {
                    assert!(
                        notice["params"]["command"]
                            .as_str()
                            .unwrap()
                            .contains("Tainted Context"),
                        "Confirmation prompt must clearly identify Tainted Context"
                    );
                    conf_mgr_rej_clone.resolve_confirmation(req_id, false).await;
                    break;
                }
            }
        }
    });

    let (reply_rej, _, _, final_taint_rej) = client_rej
        .chat_with_tools_streaming_with_options(
            "Check active window and run git status",
            10,
            None,
            &policy,
            &conf_mgr_rej,
            Some(&workspace),
            Some(&memory),
            &mock_os,
            &exec_rej,
            &ro_exec,
            &transport_rej,
            None,
            &token,
            None,
            || {},
        )
        .await
        .unwrap();
    let _ = reject_task.await;

    let expected_mem_provenance = vec![
        "memory:untrusted_note".to_string(),
        "vision:window_title".to_string(),
    ];
    assert!(reply_rej.contains("denied"));
    assert_eq!(
        exec_rej.invocation_count(),
        0,
        "Terminal executor MUST have 0 invocations on user rejection even with valid workspace"
    );
    assert!(final_taint_rej.is_tainted());
    assert_eq!(
        final_taint_rej.taint_sources, expected_mem_provenance,
        "Exact memory taint provenance must survive user rejection unchanged"
    );

    let reqs_rej = transport_rej.captured.lock().await;
    assert_eq!(reqs_rej.len(), 2);
    let user_tool_resp_parts = reqs_rej[1]["contents"][2]["parts"].as_array().unwrap();
    assert_eq!(
        user_tool_resp_parts.len(),
        2,
        "Both parallel responses must be returned"
    );
    assert_eq!(
        user_tool_resp_parts[0]["functionResponse"]["id"],
        "call_hydrogen_inspect_1"
    );
    assert_eq!(
        user_tool_resp_parts[1]["functionResponse"]["id"],
        "call_hydrogen_term_2"
    );
    assert!(
        user_tool_resp_parts[1]["functionResponse"]["response"]["output"]
            .as_str()
            .unwrap()
            .contains("\"status\":\"DENIED\""),
        "Turn 2 functionResponse must report DENIED for rejected terminal command"
    );

    // 3. Scenario B (memory:untrusted_note): Confirmation timeout through real orchestration path (real 8000ms confirmation timeout)
    let transport_to = MockLlmTransport::new(vec![
        MockHttpReply::Sse {
            status: 200,
            events: vec![parallel_sse.clone()],
            truncate_mid_frame: false,
        },
        MockHttpReply::Sse {
            status: 200,
            events: vec![final_sse.clone()],
            truncate_mid_frame: false,
        },
    ]);

    let client_to = GeminiClient::new(Some("test-api-key".into()), None, None).unwrap();
    let exec_to = InstrumentedTerminalExecutor::new();
    let conf_mgr_to = ConfirmationManager::new();

    let (_reply_to, _, _, final_taint_to) = client_to
        .chat_with_tools_streaming_with_options(
            "Check active window and run git status",
            11,
            None,
            &policy,
            &conf_mgr_to,
            Some(&workspace),
            Some(&memory),
            &mock_os,
            &exec_to,
            &ro_exec,
            &transport_to,
            None,
            &token,
            None,
            || {},
        )
        .await
        .unwrap();

    assert_eq!(
        exec_to.invocation_count(),
        0,
        "Terminal executor MUST have 0 invocations on confirmation timeout even with valid workspace"
    );
    assert_eq!(
        final_taint_to.taint_sources, expected_mem_provenance,
        "Exact memory taint provenance must survive confirmation timeout unchanged"
    );

    let reqs_to = transport_to.captured.lock().await;
    let to_resp_parts = reqs_to[1]["contents"][2]["parts"].as_array().unwrap();
    assert!(
        to_resp_parts[1]["functionResponse"]["response"]["output"]
            .as_str()
            .unwrap()
            .contains("\"status\":\"DENIED\""),
        "Turn 2 functionResponse must report DENIED on confirmation timeout"
    );

    // 4. Scenario C & D: Frozen specification browser-OCR taint fixture (`mark_tainted("untrusted_browser_ocr")`)
    let mut browser_ocr_init = TurnTaintState::new();
    browser_ocr_init.mark_tainted("untrusted_browser_ocr");
    let browser_amber_eval = policy.evaluate_action_with_taint(
        "run_terminal_command",
        "git status",
        &browser_ocr_init,
    );
    assert_eq!(browser_amber_eval.tier, SecurityTier::Amber);
    assert!(browser_amber_eval.requires_confirmation);
    let browser_router_sel = client_for_router.router().select_tier(
        "Check window and run git status",
        browser_ocr_init.is_tainted(),
        0,
    );
    assert_eq!(browser_router_sel.model_name, "gemini-4-hydrogen");

    let expected_browser_provenance = vec![
        "untrusted_browser_ocr".to_string(),
        "vision:window_title".to_string(),
    ];

    // 4a. Browser-OCR taint: user explicit rejection -> DENIED, 0 invocations, exact provenance survival
    let transport_ocr_rej = MockLlmTransport::new(vec![
        MockHttpReply::Sse {
            status: 200,
            events: vec![parallel_sse.clone()],
            truncate_mid_frame: false,
        },
        MockHttpReply::Sse {
            status: 200,
            events: vec![final_sse.clone()],
            truncate_mid_frame: false,
        },
    ]);
    let client_ocr_rej = GeminiClient::new(Some("test-api-key".into()), None, None).unwrap();
    let exec_ocr_rej = InstrumentedTerminalExecutor::new();
    let conf_ocr_rej = ConfirmationManager::new();
    let mut sub_ocr_rej = conf_ocr_rej.subscribe();
    let conf_ocr_rej_clone = conf_ocr_rej.clone();
    let ocr_reject_task = tokio::spawn(async move {
        while let Ok(notice) = sub_ocr_rej.recv().await {
            if notice["method"] == "hud.show_confirmation" {
                if let Some(req_id) = notice["params"]["id"].as_u64() {
                    conf_ocr_rej_clone.resolve_confirmation(req_id, false).await;
                    break;
                }
            }
        }
    });
    let (_reply_ocr_rej, _, _, final_taint_ocr_rej) = client_ocr_rej
        .chat_with_tools_streaming_with_options(
            "Check active window and run git status",
            12,
            None,
            &policy,
            &conf_ocr_rej,
            Some(&workspace),
            None,
            &mock_os,
            &exec_ocr_rej,
            &ro_exec,
            &transport_ocr_rej,
            Some(browser_ocr_init.clone()),
            &token,
            None,
            || {},
        )
        .await
        .unwrap();
    let _ = ocr_reject_task.await;
    assert_eq!(exec_ocr_rej.invocation_count(), 0);
    assert_eq!(
        final_taint_ocr_rej.taint_sources, expected_browser_provenance,
        "Exact untrusted_browser_ocr provenance must survive rejection unchanged"
    );
    let reqs_ocr_rej = transport_ocr_rej.captured.lock().await;
    assert!(
        reqs_ocr_rej[1]["contents"][2]["parts"][1]["functionResponse"]["response"]["output"]
            .as_str()
            .unwrap()
            .contains("\"status\":\"DENIED\"")
    );

    // 4b. Browser-OCR taint: confirmation timeout -> DENIED, 0 invocations, exact provenance survival
    let transport_ocr_to = MockLlmTransport::new(vec![
        MockHttpReply::Sse {
            status: 200,
            events: vec![parallel_sse],
            truncate_mid_frame: false,
        },
        MockHttpReply::Sse {
            status: 200,
            events: vec![final_sse],
            truncate_mid_frame: false,
        },
    ]);
    let client_ocr_to = GeminiClient::new(Some("test-api-key".into()), None, None).unwrap();
    let exec_ocr_to = InstrumentedTerminalExecutor::new();
    let conf_ocr_to = ConfirmationManager::new();
    let (_reply_ocr_to, _, _, final_taint_ocr_to) = client_ocr_to
        .chat_with_tools_streaming_with_options(
            "Check active window and run git status",
            13,
            None,
            &policy,
            &conf_ocr_to,
            Some(&workspace),
            None,
            &mock_os,
            &exec_ocr_to,
            &ro_exec,
            &transport_ocr_to,
            Some(browser_ocr_init),
            &token,
            None,
            || {},
        )
        .await
        .unwrap();
    assert_eq!(exec_ocr_to.invocation_count(), 0);
    assert_eq!(
        final_taint_ocr_to.taint_sources, expected_browser_provenance,
        "Exact untrusted_browser_ocr provenance must survive confirmation timeout unchanged"
    );
    let reqs_ocr_to = transport_ocr_to.captured.lock().await;
    assert!(
        reqs_ocr_to[1]["contents"][2]["parts"][1]["functionResponse"]["response"]["output"]
            .as_str()
            .unwrap()
            .contains("\"status\":\"DENIED\"")
    );

    // 5. Verify genuine read-only execution (`search_memory` on trusted note) leaves taint provenance unchanged
    let mut provenance_check_taint = final_taint_ocr_to.clone();
    let trusted_search_call = FunctionCall {
        id: Some("call_ro_mem".into()),
        name: "search_memory".into(),
        args: json!({ "query": "Project Guidelines" }),
        thought_signature: None,
    };
    let ro_res = GeminiClient::dispatch_tool_call(
        &trusted_search_call,
        &policy,
        &conf_mgr_to,
        Some(&workspace),
        Some(&memory),
        &mock_os,
        &mut provenance_check_taint,
        &token,
    )
    .await;
    assert_eq!(ro_res["status"], "SUCCESS");
    assert_eq!(
        provenance_check_taint.taint_sources, expected_browser_provenance,
        "Read-only trusted search_memory must not alter exact taint provenance"
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn test_thought_signature_colocated_preservation() {
    let policy = PolicyEngine::new(&["C:\\Projects".into()]);
    let mock_os = crate::tools::os_control::MockOsController::default();
    let conf_mgr = ConfirmationManager::new();
    let token = CancellationToken::new();
    let workspace = make_valid_test_workspace("sig_colocated");
    let exec = InstrumentedTerminalExecutor::new();

    let turn1_sse = format!(
        "data: {}\n\n",
        json!({
            "candidates": [{
                "finishReason": "STOP",
                "content": {
                    "parts": [{
                        "functionCall": {
                            "id": "call_coloc_1",
                            "name": "run_terminal_command",
                            "args": { "command": "git status" }
                        },
                        "thoughtSignature": "sig_verified_colocated_xyz",
                        "extraMetadata": "preserved_field"
                    }]
                }
            }]
        })
    );
    let turn2_sse = format!(
        "data: {}\n\n",
        json!({
            "candidates": [{
                "finishReason": "STOP",
                "content": { "parts": [{ "text": "Done." }] }
            }]
        })
    );

    let transport = MockLlmTransport::new(vec![
        MockHttpReply::Sse {
            status: 200,
            events: vec![turn1_sse],
            truncate_mid_frame: false,
        },
        MockHttpReply::Sse {
            status: 200,
            events: vec![turn2_sse],
            truncate_mid_frame: false,
        },
    ]);

    let client = GeminiClient::new(Some("test-api-key".into()), None, None).unwrap();

    let (reply, _, _, _) = client
        .chat_with_tools_streaming_with_executor(
            "Run git status",
            1,
            None,
            &policy,
            &conf_mgr,
            Some(&workspace),
            None,
            &mock_os,
            &exec,
            &transport,
            &token,
            None,
            || {},
        )
        .await
        .unwrap();

    assert_eq!(reply, "Done.");
    assert_eq!(exec.invocation_count(), 1);

    let reqs = transport.captured.lock().await;
    assert_eq!(reqs.len(), 2);
    let model_part = &reqs[1]["contents"][1]["parts"][0];
    assert_eq!(model_part["thoughtSignature"], "sig_verified_colocated_xyz");
    assert_eq!(model_part["extraMetadata"], "preserved_field");
    assert_eq!(model_part["functionCall"]["id"], "call_coloc_1");
    assert_eq!(model_part["functionCall"]["name"], "run_terminal_command");
    assert_eq!(model_part["functionCall"]["args"]["command"], "git status");

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn test_parallel_calls_part_one_only_signature() {
    let policy = PolicyEngine::new(&["C:\\Projects".into()]);
    let mock_os = crate::tools::os_control::MockOsController::default();
    let conf_mgr = ConfirmationManager::new();
    let mut sub = conf_mgr.subscribe();
    let conf_mgr_c = conf_mgr.clone();
    let approve_task = tokio::spawn(async move {
        while let Ok(n) = sub.recv().await {
            if n["method"] == "hud.show_confirmation" {
                if let Some(id) = n["params"]["id"].as_u64() {
                    conf_mgr_c.resolve_confirmation(id, true).await;
                    break;
                }
            }
        }
    });
    let token = CancellationToken::new();
    let workspace = make_valid_test_workspace("sig_parallel");
    let exec = InstrumentedTerminalExecutor::new();

    // Part 0 is leading thought text; Part 1 is first functionCall (carries signature);
    // Part 2 is second functionCall (must NOT carry signature).
    let turn1_sse = format!(
        "data: {}\n\n",
        json!({
            "candidates": [{
                "finishReason": "STOP",
                "content": {
                    "parts": [
                        { "text": "Checking workspace...", "thought": true },
                        {
                            "functionCall": { "id": "call_fc_1", "name": "run_terminal_command", "args": { "command": "git status" } },
                            "thoughtSignature": "sig_first_fc_only"
                        },
                        {
                            "functionCall": { "id": "call_fc_2", "name": "run_terminal_command", "args": { "command": "git --version" } },
                            "thoughtSignature": "sig_second_fc_must_be_stripped"
                        }
                    ]
                }
            }]
        })
    );
    let turn2_sse = format!(
        "data: {}\n\n",
        json!({
            "candidates": [{
                "finishReason": "STOP",
                "content": { "parts": [{ "text": "Checked." }] }
            }]
        })
    );

    let transport = MockLlmTransport::new(vec![
        MockHttpReply::Sse {
            status: 200,
            events: vec![turn1_sse],
            truncate_mid_frame: false,
        },
        MockHttpReply::Sse {
            status: 200,
            events: vec![turn2_sse],
            truncate_mid_frame: false,
        },
    ]);

    let client = GeminiClient::new(Some("test-api-key".into()), None, None).unwrap();

    let _ = client
        .chat_with_tools_streaming_with_executor(
            "Run parallel checks",
            2,
            None,
            &policy,
            &conf_mgr,
            Some(&workspace),
            None,
            &mock_os,
            &exec,
            &transport,
            &token,
            None,
            || {},
        )
        .await
        .unwrap();
    let _ = approve_task.await;

    let reqs = transport.captured.lock().await;
    let model_parts = reqs[1]["contents"][1]["parts"].as_array().unwrap();
    assert_eq!(model_parts.len(), 3);
    assert_eq!(model_parts[1]["thoughtSignature"], "sig_first_fc_only");
    assert!(
        model_parts[2].get("thoughtSignature").is_none(),
        "Only first functionCall part may bear thoughtSignature"
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn test_parallel_readonly_tools_finish_out_of_order_preserve_model_emitted_order() {
    let policy = PolicyEngine::new(&["C:\\Projects".into()]);
    let mock_os = crate::tools::os_control::MockOsController::default();
    let conf_mgr = ConfirmationManager::new();
    let token = CancellationToken::new();
    let workspace = make_valid_test_workspace("ro_order_pres");
    let exec = InstrumentedTerminalExecutor::new();
    // Force first read-only call ("inspect_active_window") to take 70ms while "read_active_window_ocr" finishes immediately
    let ro_exec = InstrumentedReadOnlyExecutor::new().with_window_delay(70);

    let turn1_sse = format!(
        "data: {}\n\n",
        json!({
            "candidates": [{
                "finishReason": "STOP",
                "content": {
                    "parts": [
                        { "functionCall": { "id": "ro_first_slow", "name": "inspect_active_window", "args": {} }, "thoughtSignature": "sig_ro" },
                        { "functionCall": { "id": "ro_second_fast", "name": "read_active_window_ocr", "args": {} } }
                    ]
                }
            }]
        })
    );
    let turn2_sse = format!(
        "data: {}\n\n",
        json!({
            "candidates": [{
                "finishReason": "STOP",
                "content": { "parts": [{ "text": "Read-only parallel complete." }] }
            }]
        })
    );

    let transport = MockLlmTransport::new(vec![
        MockHttpReply::Sse {
            status: 200,
            events: vec![turn1_sse],
            truncate_mid_frame: false,
        },
        MockHttpReply::Sse {
            status: 200,
            events: vec![turn2_sse],
            truncate_mid_frame: false,
        },
    ]);

    let client = GeminiClient::new(Some("test-api-key".into()), None, None).unwrap();

    let (reply, _, _, final_taint) = client
        .chat_with_tools_streaming_with_options(
            "Inspect window and read OCR",
            30,
            None,
            &policy,
            &conf_mgr,
            Some(&workspace),
            None,
            &mock_os,
            &exec,
            &ro_exec,
            &transport,
            None,
            &token,
            None,
            || {},
        )
        .await
        .unwrap();

    assert_eq!(reply, "Read-only parallel complete.");
    assert_eq!(exec.invocation_count(), 0);
    assert_eq!(
        ro_exec.max_concurrent(),
        2,
        "Proven-independent read-only tools must execute concurrently"
    );

    let events = ro_exec.events();
    let end_ocr_idx = events
        .iter()
        .position(|e| e == "end:read_active_window_ocr")
        .unwrap();
    let end_win_idx = events
        .iter()
        .position(|e| e == "end:inspect_active_window")
        .unwrap();
    assert!(
        end_ocr_idx < end_win_idx,
        "Fast second read-only call must complete before delayed first call: {:?}",
        events
    );

    let reqs = transport.captured.lock().await;
    let resp_parts = reqs[1]["contents"][2]["parts"].as_array().unwrap();
    assert_eq!(resp_parts.len(), 2);
    assert_eq!(resp_parts[0]["functionResponse"]["id"], "ro_first_slow");
    assert_eq!(
        resp_parts[0]["functionResponse"]["name"],
        "inspect_active_window"
    );
    assert_eq!(resp_parts[1]["functionResponse"]["id"], "ro_second_fast");
    assert_eq!(
        resp_parts[1]["functionResponse"]["name"],
        "read_active_window_ocr"
    );
    assert!(final_taint
        .taint_sources
        .contains(&"vision:window_title".to_string()));
    assert!(final_taint
        .taint_sources
        .contains(&"vision:ocr".to_string()));

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn test_tool_history_preserves_model_emitted_order() {
    let policy = PolicyEngine::new(&["C:\\Projects".into()]);
    let mock_os = crate::tools::os_control::MockOsController::default();
    let conf_mgr = ConfirmationManager::new();
    let mut sub = conf_mgr.subscribe();
    let conf_mgr_c = conf_mgr.clone();
    // Call 1 ("git status") finishes first cleanly and marks `terminal_output` taint;
    // Call 2 ("git --version") runs serially AFTER Call 1 and must escalate to Amber confirmation.
    let approve_task = tokio::spawn(async move {
        while let Ok(n) = sub.recv().await {
            if n["method"] == "hud.show_confirmation" {
                if let Some(id) = n["params"]["id"].as_u64() {
                    assert!(
                        n["params"]["command"].as_str().unwrap().contains("Tainted Context"),
                        "Second serialized terminal call must observe live taint from first call"
                    );
                    conf_mgr_c.resolve_confirmation(id, true).await;
                    break;
                }
            }
        }
    });
    let token = CancellationToken::new();
    let workspace = make_valid_test_workspace("order_pres");
    // Even with a 70ms delay on call 1, privileged run_terminal_command calls MUST execute serially:
    // start(git status) -> end(git status) -> start(git --version) -> end(git --version).
    let exec = InstrumentedTerminalExecutor::new().with_delay("git status", 70);

    let turn1_sse = format!(
        "data: {}\n\n",
        json!({
            "candidates": [{
                "finishReason": "STOP",
                "content": {
                    "parts": [
                        { "functionCall": { "id": "call_emitted_first_slow", "name": "run_terminal_command", "args": { "command": "git status" } }, "thoughtSignature": "sig_ord" },
                        { "functionCall": { "id": "call_emitted_second_fast", "name": "run_terminal_command", "args": { "command": "git --version" } } }
                    ]
                }
            }]
        })
    );
    let turn2_sse = format!(
        "data: {}\n\n",
        json!({
            "candidates": [{
                "finishReason": "STOP",
                "content": { "parts": [{ "text": "Ordered." }] }
            }]
        })
    );

    let transport = MockLlmTransport::new(vec![
        MockHttpReply::Sse {
            status: 200,
            events: vec![turn1_sse],
            truncate_mid_frame: false,
        },
        MockHttpReply::Sse {
            status: 200,
            events: vec![turn2_sse],
            truncate_mid_frame: false,
        },
    ]);

    let client = GeminiClient::new(Some("test-api-key".into()), None, None).unwrap();

    let _ = client
        .chat_with_tools_streaming_with_executor(
            "Run ordered tools",
            3,
            None,
            &policy,
            &conf_mgr,
            Some(&workspace),
            None,
            &mock_os,
            &exec,
            &transport,
            &token,
            None,
            || {},
        )
        .await
        .unwrap();
    let _ = approve_task.await;

    assert_eq!(exec.invocation_count(), 2);
    assert_eq!(
        exec.max_concurrent(),
        1,
        "Privileged run_terminal_command operations MUST NEVER run concurrently"
    );
    assert_eq!(
        exec.events(),
        vec![
            "start:git status",
            "end:git status",
            "start:git --version",
            "end:git --version"
        ],
        "Second privileged operation must not begin before first finishes"
    );

    // Verify history sent to Turn 2 preserves MODEL-EMITTED order [call_1, call_2]
    let reqs = transport.captured.lock().await;
    let resp_parts = reqs[1]["contents"][2]["parts"].as_array().unwrap();
    assert_eq!(resp_parts.len(), 2);
    assert_eq!(
        resp_parts[0]["functionResponse"]["id"],
        "call_emitted_first_slow"
    );
    assert!(resp_parts[0]["functionResponse"]["response"]["output"]
        .as_str()
        .unwrap()
        .contains("executed: git status"));
    assert_eq!(
        resp_parts[1]["functionResponse"]["id"],
        "call_emitted_second_fast"
    );
    assert!(resp_parts[1]["functionResponse"]["response"]["output"]
        .as_str()
        .unwrap()
        .contains("executed: git --version"));

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn test_calls_to_same_function_different_ids_correlated() {
    let policy = PolicyEngine::new(&["C:\\Projects".into()]);
    let mock_os = crate::tools::os_control::MockOsController::default();
    let conf_mgr = ConfirmationManager::new();
    let mut sub = conf_mgr.subscribe();
    let conf_mgr_c = conf_mgr.clone();
    let approve_task = tokio::spawn(async move {
        while let Ok(n) = sub.recv().await {
            if n["method"] == "hud.show_confirmation" {
                if let Some(id) = n["params"]["id"].as_u64() {
                    conf_mgr_c.resolve_confirmation(id, true).await;
                    break;
                }
            }
        }
    });
    let token = CancellationToken::new();
    let workspace = make_valid_test_workspace("same_fn_diff_id");
    let exec = InstrumentedTerminalExecutor::new().with_delay("git status", 40);

    let turn1_sse = format!(
        "data: {}\n\n",
        json!({
            "candidates": [{
                "finishReason": "STOP",
                "content": {
                    "parts": [
                        { "functionCall": { "id": "id_alpha_1", "name": "run_terminal_command", "args": { "command": "git status" } }, "thoughtSignature": "sig_same" },
                        { "functionCall": { "id": "id_beta_2", "name": "run_terminal_command", "args": { "command": "git --version" } } }
                    ]
                }
            }]
        })
    );
    let turn2_sse = format!(
        "data: {}\n\n",
        json!({
            "candidates": [{
                "finishReason": "STOP",
                "content": { "parts": [{ "text": "Correlated." }] }
            }]
        })
    );

    let transport = MockLlmTransport::new(vec![
        MockHttpReply::Sse {
            status: 200,
            events: vec![turn1_sse],
            truncate_mid_frame: false,
        },
        MockHttpReply::Sse {
            status: 200,
            events: vec![turn2_sse],
            truncate_mid_frame: false,
        },
    ]);

    let client = GeminiClient::new(Some("test-api-key".into()), None, None).unwrap();

    let _ = client
        .chat_with_tools_streaming_with_executor(
            "Correlate calls",
            4,
            None,
            &policy,
            &conf_mgr,
            Some(&workspace),
            None,
            &mock_os,
            &exec,
            &transport,
            &token,
            None,
            || {},
        )
        .await
        .unwrap();
    let _ = approve_task.await;

    assert_eq!(exec.max_concurrent(), 1);
    assert_eq!(
        exec.events(),
        vec![
            "start:git status",
            "end:git status",
            "start:git --version",
            "end:git --version"
        ]
    );

    let reqs = transport.captured.lock().await;
    let resp_parts = reqs[1]["contents"][2]["parts"].as_array().unwrap();
    assert_eq!(resp_parts[0]["functionResponse"]["id"], "id_alpha_1");
    assert_eq!(
        resp_parts[0]["functionResponse"]["name"],
        "run_terminal_command"
    );
    assert!(resp_parts[0]["functionResponse"]["response"]["output"]
        .as_str()
        .unwrap()
        .contains("git status"));

    assert_eq!(resp_parts[1]["functionResponse"]["id"], "id_beta_2");
    assert_eq!(
        resp_parts[1]["functionResponse"]["name"],
        "run_terminal_command"
    );
    assert!(resp_parts[1]["functionResponse"]["response"]["output"]
        .as_str()
        .unwrap()
        .contains("git --version"));

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn test_scheduler_launch_application_completes_before_inspect_active_window() {
    let policy = PolicyEngine::new(&["C:\\Projects".into()]);
    let conf_mgr = ConfirmationManager::new();
    let token = CancellationToken::new();
    let workspace = make_valid_test_workspace("sched_4a");

    let shared_events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let launched_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let launched_check = launched_flag.clone();

    let os_ctrl = InstrumentedOsController {
        shared_events: shared_events.clone(),
        launched_flag,
        delay_ms: 25,
    };
    let ro_exec = InstrumentedReadOnlyExecutor::new()
        .with_shared_events(shared_events.clone())
        .with_window_hooks(
            std::sync::Arc::new(move || {
                assert!(
                    launched_check.load(std::sync::atomic::Ordering::SeqCst),
                    "inspect_active_window must not start until launch_application completes"
                );
            }),
            std::sync::Arc::new(|| {}),
        );
    let term_exec = InstrumentedTerminalExecutor::new();
    let browser_nav = InstrumentedBrowserNavigator {
        shared_events: shared_events.clone(),
        navigated_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        delay_ms: 0,
    };

    let turn1_sse = format!(
        "data: {}\n\n",
        json!({
            "candidates": [{
                "finishReason": "STOP",
                "content": {
                    "parts": [
                        { "functionCall": { "id": "call_launch_1", "name": "launch_application", "args": { "target": "notepad.exe" } }, "thoughtSignature": "sig_4a" },
                        { "functionCall": { "id": "call_inspect_2", "name": "inspect_active_window", "args": {} } }
                    ]
                }
            }]
        })
    );
    let turn2_sse = format!(
        "data: {}\n\n",
        json!({
            "candidates": [{
                "finishReason": "STOP",
                "content": { "parts": [{ "text": "Launched then inspected." }] }
            }]
        })
    );
    let transport = MockLlmTransport::new(vec![
        MockHttpReply::Sse {
            status: 200,
            events: vec![turn1_sse],
            truncate_mid_frame: false,
        },
        MockHttpReply::Sse {
            status: 200,
            events: vec![turn2_sse],
            truncate_mid_frame: false,
        },
    ]);

    let client = GeminiClient::new(Some("test-api-key".into()), None, None).unwrap();
    let (reply, _, _, final_taint) = client
        .chat_with_tools_streaming_with_all_executors(
            "Launch notepad and inspect window",
            401,
            None,
            &policy,
            &conf_mgr,
            Some(&workspace),
            None,
            &os_ctrl,
            &term_exec,
            &ro_exec,
            &browser_nav,
            &transport,
            None,
            &token,
            None,
            || {},
        )
        .await
        .unwrap();

    assert_eq!(reply, "Launched then inspected.");
    let trace = shared_events.lock().unwrap().clone();
    println!("TRACE_4A: {:?}", trace);
    assert_eq!(
        trace,
        vec![
            "start:launch_application:notepad.exe",
            "end:launch_application:notepad.exe",
            "start:inspect_active_window",
            "end:inspect_active_window",
        ]
    );
    assert_eq!(final_taint.taint_sources, vec!["vision:window_title"]);

    let reqs = transport.captured.lock().await;
    let resp_parts = reqs[1]["contents"][2]["parts"].as_array().unwrap();
    assert_eq!(resp_parts.len(), 2);
    assert_eq!(resp_parts[0]["functionResponse"]["id"], "call_launch_1");
    assert_eq!(
        resp_parts[0]["functionResponse"]["name"],
        "launch_application"
    );
    assert_eq!(resp_parts[1]["functionResponse"]["id"], "call_inspect_2");
    assert_eq!(
        resp_parts[1]["functionResponse"]["name"],
        "inspect_active_window"
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn test_scheduler_open_browser_url_completes_before_read_active_tab() {
    let policy = PolicyEngine::new(&["C:\\Projects".into()]);
    let conf_mgr = ConfirmationManager::new();
    let token = CancellationToken::new();
    let workspace = make_valid_test_workspace("sched_4b");

    let shared_events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let navigated_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let navigated_check = navigated_flag.clone();

    let os_ctrl = InstrumentedOsController {
        shared_events: shared_events.clone(),
        launched_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        delay_ms: 0,
    };
    let ro_exec = InstrumentedReadOnlyExecutor::new()
        .with_shared_events(shared_events.clone())
        .with_tab_start_hook(std::sync::Arc::new(move || {
            assert!(
                navigated_check.load(std::sync::atomic::Ordering::SeqCst),
                "read_active_tab must not start until open_browser_url completes"
            );
        }));
    let term_exec = InstrumentedTerminalExecutor::new();
    let browser_nav = InstrumentedBrowserNavigator {
        shared_events: shared_events.clone(),
        navigated_flag,
        delay_ms: 25,
    };

    let turn1_sse = format!(
        "data: {}\n\n",
        json!({
            "candidates": [{
                "finishReason": "STOP",
                "content": {
                    "parts": [
                        { "functionCall": { "id": "call_nav_1", "name": "open_browser_url", "args": { "url": "https://example.com" } }, "thoughtSignature": "sig_4b" },
                        { "functionCall": { "id": "call_tab_2", "name": "read_active_tab", "args": {} } }
                    ]
                }
            }]
        })
    );
    let turn2_sse = format!(
        "data: {}\n\n",
        json!({
            "candidates": [{
                "finishReason": "STOP",
                "content": { "parts": [{ "text": "Navigated then read tab." }] }
            }]
        })
    );
    let transport = MockLlmTransport::new(vec![
        MockHttpReply::Sse {
            status: 200,
            events: vec![turn1_sse],
            truncate_mid_frame: false,
        },
        MockHttpReply::Sse {
            status: 200,
            events: vec![turn2_sse],
            truncate_mid_frame: false,
        },
    ]);

    let client = GeminiClient::new(Some("test-api-key".into()), None, None).unwrap();
    let (reply, _, _, final_taint) = client
        .chat_with_tools_streaming_with_all_executors(
            "Open example.com and read tab",
            402,
            None,
            &policy,
            &conf_mgr,
            Some(&workspace),
            None,
            &os_ctrl,
            &term_exec,
            &ro_exec,
            &browser_nav,
            &transport,
            None,
            &token,
            None,
            || {},
        )
        .await
        .unwrap();

    assert_eq!(reply, "Navigated then read tab.");
    let trace = shared_events.lock().unwrap().clone();
    println!("TRACE_4B: {:?}", trace);
    assert_eq!(
        trace,
        vec![
            "start:open_browser_url:https://example.com",
            "end:open_browser_url:https://example.com",
            "start:read_active_tab",
            "end:read_active_tab",
        ]
    );
    assert_eq!(final_taint.taint_sources, vec!["browser_cdp:read"]);

    let reqs = transport.captured.lock().await;
    let resp_parts = reqs[1]["contents"][2]["parts"].as_array().unwrap();
    assert_eq!(resp_parts.len(), 2);
    assert_eq!(resp_parts[0]["functionResponse"]["id"], "call_nav_1");
    assert_eq!(
        resp_parts[0]["functionResponse"]["name"],
        "open_browser_url"
    );
    assert_eq!(resp_parts[1]["functionResponse"]["id"], "call_tab_2");
    assert_eq!(resp_parts[1]["functionResponse"]["name"], "read_active_tab");

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn test_scheduler_readonly_pair_overlaps_and_commits_taint_before_terminal() {
    let policy = PolicyEngine::new(&["C:\\Projects".into()]);
    let conf_mgr = ConfirmationManager::new();
    let mut sub = conf_mgr.subscribe();
    let conf_mgr_c = conf_mgr.clone();
    let saw_tainted_prompt = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let saw_tainted_c = saw_tainted_prompt.clone();
    let approve_task = tokio::spawn(async move {
        while let Ok(n) = sub.recv().await {
            if n["method"] == "hud.show_confirmation" {
                if let Some(id) = n["params"]["id"].as_u64() {
                    assert!(
                        n["params"]["command"].as_str().unwrap().contains("Tainted Context"),
                        "Terminal authorization must see taint committed from flushed read-only batch"
                    );
                    saw_tainted_c.store(true, std::sync::atomic::Ordering::SeqCst);
                    conf_mgr_c.resolve_confirmation(id, true).await;
                    break;
                }
            }
        }
    });

    let token = CancellationToken::new();
    let workspace = make_valid_test_workspace("sched_4c");
    let shared_events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));

    let os_ctrl = InstrumentedOsController {
        shared_events: shared_events.clone(),
        launched_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        delay_ms: 0,
    };
    let ro_exec = InstrumentedReadOnlyExecutor::new()
        .with_shared_events(shared_events.clone())
        .with_overlap_barrier(barrier);
    let term_exec =
        InstrumentedTerminalExecutor::new().with_shared_events(shared_events.clone());
    let browser_nav = InstrumentedBrowserNavigator {
        shared_events: shared_events.clone(),
        navigated_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        delay_ms: 0,
    };

    let turn1_sse = format!(
        "data: {}\n\n",
        json!({
            "candidates": [{
                "finishReason": "STOP",
                "content": {
                    "parts": [
                        { "functionCall": { "id": "call_ro_win", "name": "inspect_active_window", "args": {} }, "thoughtSignature": "sig_4c" },
                        { "functionCall": { "id": "call_ro_ocr", "name": "read_active_window_ocr", "args": {} } },
                        { "functionCall": { "id": "call_priv_term", "name": "run_terminal_command", "args": { "command": "git status" } } }
                    ]
                }
            }]
        })
    );
    let turn2_sse = format!(
        "data: {}\n\n",
        json!({
            "candidates": [{
                "finishReason": "STOP",
                "content": { "parts": [{ "text": "Read-only flushed then terminal ran." }] }
            }]
        })
    );
    let transport = MockLlmTransport::new(vec![
        MockHttpReply::Sse {
            status: 200,
            events: vec![turn1_sse],
            truncate_mid_frame: false,
        },
        MockHttpReply::Sse {
            status: 200,
            events: vec![turn2_sse],
            truncate_mid_frame: false,
        },
    ]);

    let client = GeminiClient::new(Some("test-api-key".into()), None, None).unwrap();
    let (reply, _, _, final_taint) = client
        .chat_with_tools_streaming_with_all_executors(
            "Inspect window and OCR then check git status",
            403,
            None,
            &policy,
            &conf_mgr,
            Some(&workspace),
            None,
            &os_ctrl,
            &term_exec,
            &ro_exec,
            &browser_nav,
            &transport,
            None,
            &token,
            None,
            || {},
        )
        .await
        .unwrap();
    let _ = approve_task.await;

    assert_eq!(reply, "Read-only flushed then terminal ran.");
    assert!(saw_tainted_prompt.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(
        ro_exec.max_concurrent(),
        2,
        "Read-only pair must overlap concurrently"
    );
    assert_eq!(term_exec.invocation_count(), 1);

    let trace = shared_events.lock().unwrap().clone();
    println!("TRACE_4C: {:?}", trace);
    let end_win = trace
        .iter()
        .position(|e| e == "end:inspect_active_window")
        .unwrap();
    let end_ocr = trace
        .iter()
        .position(|e| e == "end:read_active_window_ocr")
        .unwrap();
    let start_term = trace
        .iter()
        .position(|e| e == "start:run_terminal_command:git status")
        .unwrap();
    let end_term = trace
        .iter()
        .position(|e| e == "end:run_terminal_command:git status")
        .unwrap();
    assert!(
        end_win < start_term && end_ocr < start_term,
        "Both read-only tools must finish before terminal begins: {:?}",
        trace
    );
    assert!(start_term < end_term);
    assert!(final_taint
        .taint_sources
        .contains(&"vision:window_title".to_string()));
    assert!(final_taint
        .taint_sources
        .contains(&"vision:ocr".to_string()));
    assert!(final_taint
        .taint_sources
        .contains(&"terminal_output".to_string()));

    let reqs = transport.captured.lock().await;
    let resp_parts = reqs[1]["contents"][2]["parts"].as_array().unwrap();
    assert_eq!(resp_parts.len(), 3);
    assert_eq!(resp_parts[0]["functionResponse"]["id"], "call_ro_win");
    assert_eq!(resp_parts[1]["functionResponse"]["id"], "call_ro_ocr");
    assert_eq!(resp_parts[2]["functionResponse"]["id"], "call_priv_term");

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn test_scheduler_terminal_inspect_terminal_preserves_cross_category_barriers() {
    let policy = PolicyEngine::new(&["C:\\Projects".into()]);
    let conf_mgr = ConfirmationManager::new();
    let mut sub = conf_mgr.subscribe();
    let conf_mgr_c = conf_mgr.clone();
    let approve_task = tokio::spawn(async move {
        while let Ok(n) = sub.recv().await {
            if n["method"] == "hud.show_confirmation" {
                if let Some(id) = n["params"]["id"].as_u64() {
                    assert!(
                        n["params"]["command"].as_str().unwrap().contains("Tainted Context"),
                        "Second terminal call must see taint from first terminal call and window inspection"
                    );
                    conf_mgr_c.resolve_confirmation(id, true).await;
                    break;
                }
            }
        }
    });

    let token = CancellationToken::new();
    let workspace = make_valid_test_workspace("sched_4d");
    let shared_events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let t1_done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let ro2_done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    let t1_done_end = t1_done.clone();
    let ro2_done_check = ro2_done.clone();
    let t1_done_check = t1_done.clone();
    let ro2_done_end = ro2_done.clone();

    let os_ctrl = InstrumentedOsController {
        shared_events: shared_events.clone(),
        launched_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        delay_ms: 0,
    };
    let ro_exec = InstrumentedReadOnlyExecutor::new()
        .with_shared_events(shared_events.clone())
        .with_window_hooks(
            std::sync::Arc::new(move || {
                assert!(
                    t1_done_check.load(std::sync::atomic::Ordering::SeqCst),
                    "inspect_active_window must not start until first run_terminal_command finishes"
                );
            }),
            std::sync::Arc::new(move || {
                ro2_done_end.store(true, std::sync::atomic::Ordering::SeqCst);
            }),
        );
    let term_exec = InstrumentedTerminalExecutor::new()
        .with_delay("git status", 25)
        .with_shared_events(shared_events.clone())
        .with_hooks(
            std::sync::Arc::new(move |cmd| {
                if cmd == "git --version" {
                    assert!(
                        ro2_done_check.load(std::sync::atomic::Ordering::SeqCst),
                        "Second run_terminal_command must not start until inspect_active_window finishes"
                    );
                }
            }),
            std::sync::Arc::new(move |cmd| {
                if cmd == "git status" {
                    t1_done_end.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            }),
        );
    let browser_nav = InstrumentedBrowserNavigator {
        shared_events: shared_events.clone(),
        navigated_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        delay_ms: 0,
    };

    let turn1_sse = format!(
        "data: {}\n\n",
        json!({
            "candidates": [{
                "finishReason": "STOP",
                "content": {
                    "parts": [
                        { "functionCall": { "id": "call_t1", "name": "run_terminal_command", "args": { "command": "git status" } }, "thoughtSignature": "sig_4d" },
                        { "functionCall": { "id": "call_ro_2", "name": "inspect_active_window", "args": {} } },
                        { "functionCall": { "id": "call_t3", "name": "run_terminal_command", "args": { "command": "git --version" } } }
                    ]
                }
            }]
        })
    );
    let turn2_sse = format!(
        "data: {}\n\n",
        json!({
            "candidates": [{
                "finishReason": "STOP",
                "content": { "parts": [{ "text": "Serial cross-category sequence finished." }] }
            }]
        })
    );
    let transport = MockLlmTransport::new(vec![
        MockHttpReply::Sse {
            status: 200,
            events: vec![turn1_sse],
            truncate_mid_frame: false,
        },
        MockHttpReply::Sse {
            status: 200,
            events: vec![turn2_sse],
            truncate_mid_frame: false,
        },
    ]);

    let client = GeminiClient::new(Some("test-api-key".into()), None, None).unwrap();
    let (reply, _, _, final_taint) = client
        .chat_with_tools_streaming_with_all_executors(
            "Check status, inspect window, check version",
            404,
            None,
            &policy,
            &conf_mgr,
            Some(&workspace),
            None,
            &os_ctrl,
            &term_exec,
            &ro_exec,
            &browser_nav,
            &transport,
            None,
            &token,
            None,
            || {},
        )
        .await
        .unwrap();
    let _ = approve_task.await;

    assert_eq!(reply, "Serial cross-category sequence finished.");
    assert_eq!(term_exec.max_concurrent(), 1);
    assert_eq!(term_exec.invocation_count(), 2);

    let trace = shared_events.lock().unwrap().clone();
    println!("TRACE_4D: {:?}", trace);
    assert_eq!(
        trace,
        vec![
            "start:run_terminal_command:git status",
            "end:run_terminal_command:git status",
            "start:inspect_active_window",
            "end:inspect_active_window",
            "start:run_terminal_command:git --version",
            "end:run_terminal_command:git --version",
        ]
    );
    assert_eq!(
        final_taint.taint_sources,
        vec!["terminal_output", "vision:window_title", "terminal_output"]
    );

    let reqs = transport.captured.lock().await;
    let resp_parts = reqs[1]["contents"][2]["parts"].as_array().unwrap();
    assert_eq!(resp_parts.len(), 3);
    assert_eq!(resp_parts[0]["functionResponse"]["id"], "call_t1");
    assert_eq!(resp_parts[1]["functionResponse"]["id"], "call_ro_2");
    assert_eq!(resp_parts[2]["functionResponse"]["id"], "call_t3");

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn test_scheduler_read_active_tab_serializes_with_inspect_active_window() {
    let policy = PolicyEngine::new(&["C:\\Projects".into()]);
    let conf_mgr = ConfirmationManager::new();
    let token = CancellationToken::new();
    let workspace = make_valid_test_workspace("sched_4e");

    let shared_events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let win_done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let win_done_set = win_done.clone();
    let win_done_chk = win_done.clone();

    let os_ctrl = InstrumentedOsController {
        shared_events: shared_events.clone(),
        launched_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        delay_ms: 0,
    };
    let ro_exec = InstrumentedReadOnlyExecutor::new()
        .with_shared_events(shared_events.clone())
        .with_window_delay(25)
        .with_window_hooks(
            std::sync::Arc::new(|| {}),
            std::sync::Arc::new(move || {
                win_done_set.store(true, std::sync::atomic::Ordering::SeqCst);
            }),
        )
        .with_tab_start_hook(std::sync::Arc::new(move || {
            assert!(
                win_done_chk.load(std::sync::atomic::Ordering::SeqCst),
                "read_active_tab must execute serially after inspect_active_window completes"
            );
        }));
    let term_exec = InstrumentedTerminalExecutor::new();
    let browser_nav = InstrumentedBrowserNavigator {
        shared_events: shared_events.clone(),
        navigated_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        delay_ms: 0,
    };

    let turn1_sse = format!(
        "data: {}\n\n",
        json!({
            "candidates": [{
                "finishReason": "STOP",
                "content": {
                    "parts": [
                        { "functionCall": { "id": "call_win_1", "name": "inspect_active_window", "args": {} }, "thoughtSignature": "sig_4e" },
                        { "functionCall": { "id": "call_tab_2", "name": "read_active_tab", "args": {} } }
                    ]
                }
            }]
        })
    );
    let turn2_sse = format!(
        "data: {}\n\n",
        json!({
            "candidates": [{
                "finishReason": "STOP",
                "content": { "parts": [{ "text": "Window inspected then tab read serially." }] }
            }]
        })
    );
    let transport = MockLlmTransport::new(vec![
        MockHttpReply::Sse {
            status: 200,
            events: vec![turn1_sse],
            truncate_mid_frame: false,
        },
        MockHttpReply::Sse {
            status: 200,
            events: vec![turn2_sse],
            truncate_mid_frame: false,
        },
    ]);

    let client = GeminiClient::new(Some("test-api-key".into()), None, None).unwrap();
    let (reply, _, _, final_taint) = client
        .chat_with_tools_streaming_with_all_executors(
            "Inspect window and read active tab",
            405,
            None,
            &policy,
            &conf_mgr,
            Some(&workspace),
            None,
            &os_ctrl,
            &term_exec,
            &ro_exec,
            &browser_nav,
            &transport,
            None,
            &token,
            None,
            || {},
        )
        .await
        .unwrap();

    assert_eq!(reply, "Window inspected then tab read serially.");
    assert_eq!(
        ro_exec.max_concurrent(),
        1,
        "read_active_tab must not execute concurrently with inspect_active_window"
    );
    let trace = shared_events.lock().unwrap().clone();
    println!("TRACE_4E: {:?}", trace);
    assert_eq!(
        trace,
        vec![
            "start:inspect_active_window",
            "end:inspect_active_window",
            "start:read_active_tab",
            "end:read_active_tab",
        ]
    );
    assert_eq!(
        final_taint.taint_sources,
        vec!["vision:window_title", "browser_cdp:read"]
    );

    let reqs = transport.captured.lock().await;
    let resp_parts = reqs[1]["contents"][2]["parts"].as_array().unwrap();
    assert_eq!(resp_parts.len(), 2);
    assert_eq!(resp_parts[0]["functionResponse"]["id"], "call_win_1");
    assert_eq!(
        resp_parts[0]["functionResponse"]["name"],
        "inspect_active_window"
    );
    assert_eq!(resp_parts[1]["functionResponse"]["id"], "call_tab_2");
    assert_eq!(resp_parts[1]["functionResponse"]["name"], "read_active_tab");

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn test_missing_function_call_id_no_synthetic_id() {
    let policy = PolicyEngine::new(&["C:\\Projects".into()]);
    let mock_os = crate::tools::os_control::MockOsController::default();
    let conf_mgr = ConfirmationManager::new();
    let token = CancellationToken::new();
    let workspace = make_valid_test_workspace("no_synth_id");
    let exec = InstrumentedTerminalExecutor::new();

    let turn1_sse = format!(
        "data: {}\n\n",
        json!({
            "candidates": [{
                "finishReason": "STOP",
                "content": {
                    "parts": [{
                        "functionCall": { "name": "run_terminal_command", "args": { "command": "git status" } },
                        "thoughtSignature": "sig_no_id"
                    }]
                }
            }]
        })
    );
    let turn2_sse = format!(
        "data: {}\n\n",
        json!({
            "candidates": [{
                "finishReason": "STOP",
                "content": { "parts": [{ "text": "No synthetic ID." }] }
            }]
        })
    );

    let transport = MockLlmTransport::new(vec![
        MockHttpReply::Sse {
            status: 200,
            events: vec![turn1_sse],
            truncate_mid_frame: false,
        },
        MockHttpReply::Sse {
            status: 200,
            events: vec![turn2_sse],
            truncate_mid_frame: false,
        },
    ]);

    let client = GeminiClient::new(Some("test-api-key".into()), None, None).unwrap();

    let _ = client
        .chat_with_tools_streaming_with_executor(
            "Run without id",
            5,
            None,
            &policy,
            &conf_mgr,
            Some(&workspace),
            None,
            &mock_os,
            &exec,
            &transport,
            &token,
            None,
            || {},
        )
        .await
        .unwrap();

    let reqs = transport.captured.lock().await;
    let model_fc = &reqs[1]["contents"][1]["parts"][0]["functionCall"];
    let user_fr = &reqs[1]["contents"][2]["parts"][0]["functionResponse"];
    assert!(
        model_fc.get("id").is_none(),
        "Must not synthesize ID on model functionCall"
    );
    assert!(
        user_fr.get("id").is_none(),
        "Must not synthesize ID on user functionResponse"
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn test_three_turn_conversation_history_structure() {
    let policy = PolicyEngine::new(&["C:\\Projects".into()]);
    let mock_os = crate::tools::os_control::MockOsController::default();
    let conf_mgr = ConfirmationManager::new();
    let mut sub = conf_mgr.subscribe();
    let conf_mgr_clone = conf_mgr.clone();
    // Turn 1's terminal execution marks `terminal_output` taint, so Turn 2 requires Amber confirmation
    let conf_task = tokio::spawn(async move {
        while let Ok(n) = sub.recv().await {
            if n["method"] == "hud.show_confirmation" {
                if let Some(id) = n["params"]["id"].as_u64() {
                    conf_mgr_clone.resolve_confirmation(id, true).await;
                    break;
                }
            }
        }
    });
    let token = CancellationToken::new();
    let workspace = make_valid_test_workspace("three_turn");
    let exec = InstrumentedTerminalExecutor::new();

    let t1_sse = format!(
        "data: {}\n\n",
        json!({
            "candidates": [{
                "finishReason": "STOP",
                "content": { "parts": [{ "functionCall": { "id": "t1_id", "name": "run_terminal_command", "args": { "command": "git status" } }, "thoughtSignature": "sig_t1" }] }
            }]
        })
    );
    let t2_sse = format!(
        "data: {}\n\n",
        json!({
            "candidates": [{
                "finishReason": "STOP",
                "content": { "parts": [{ "functionCall": { "id": "t2_id", "name": "run_terminal_command", "args": { "command": "git --version" } }, "thoughtSignature": "sig_t2" }] }
            }]
        })
    );
    let t3_sse = format!(
        "data: {}\n\n",
        json!({
            "candidates": [{
                "finishReason": "STOP",
                "content": { "parts": [{ "text": "Three turns done." }] }
            }]
        })
    );

    let transport = MockLlmTransport::new(vec![
        MockHttpReply::Sse {
            status: 200,
            events: vec![t1_sse],
            truncate_mid_frame: false,
        },
        MockHttpReply::Sse {
            status: 200,
            events: vec![t2_sse],
            truncate_mid_frame: false,
        },
        MockHttpReply::Sse {
            status: 200,
            events: vec![t3_sse],
            truncate_mid_frame: false,
        },
    ]);

    let client = GeminiClient::new(Some("test-api-key".into()), None, None).unwrap();

    let (reply, _, _, _) = client
        .chat_with_tools_streaming_with_executor(
            "Execute multi-turn workflow",
            6,
            None,
            &policy,
            &conf_mgr,
            Some(&workspace),
            None,
            &mock_os,
            &exec,
            &transport,
            &token,
            None,
            || {},
        )
        .await
        .unwrap();
    let _ = conf_task.await;

    assert_eq!(reply, "Three turns done.");
    assert_eq!(exec.invocation_count(), 2);

    let reqs = transport.captured.lock().await;
    assert_eq!(reqs.len(), 3);
    let t3_contents = reqs[2]["contents"].as_array().unwrap();
    assert_eq!(t3_contents.len(), 5);
    assert_eq!(t3_contents[0]["role"], "user");
    assert_eq!(t3_contents[1]["role"], "model");
    assert_eq!(t3_contents[1]["parts"][0]["functionCall"]["id"], "t1_id");
    assert_eq!(t3_contents[1]["parts"][0]["thoughtSignature"], "sig_t1");
    assert_eq!(t3_contents[2]["role"], "user");
    assert_eq!(
        t3_contents[2]["parts"][0]["functionResponse"]["id"],
        "t1_id"
    );
    assert_eq!(t3_contents[3]["role"], "model");
    assert_eq!(t3_contents[3]["parts"][0]["functionCall"]["id"], "t2_id");
    assert_eq!(t3_contents[3]["parts"][0]["thoughtSignature"], "sig_t2");
    assert_eq!(t3_contents[4]["role"], "user");
    assert_eq!(
        t3_contents[4]["parts"][0]["functionResponse"]["id"],
        "t2_id"
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

async fn assert_streaming_failure_dispatches_zero_tools(
    reply: MockHttpReply,
    expected_err_substr: &str,
) {
    let policy = PolicyEngine::new(&["C:\\Projects".into()]);
    let mock_os = crate::tools::os_control::MockOsController::default();
    let conf_mgr = ConfirmationManager::new();
    let token = CancellationToken::new();
    let workspace = make_valid_test_workspace("stream_fail");
    let exec = InstrumentedTerminalExecutor::new();

    let transport = MockLlmTransport::new(vec![reply]);
    let client = GeminiClient::new(Some("test-api-key".into()), None, None).unwrap();

    let res = client
        .chat_with_tools_streaming_with_executor(
            "Run git status",
            99,
            None,
            &policy,
            &conf_mgr,
            Some(&workspace),
            None,
            &mock_os,
            &exec,
            &transport,
            &token,
            None,
            || {},
        )
        .await;

    assert!(res.is_err(), "Stream boundary failure must return Err");
    let err_msg = res.unwrap_err().to_string();
    assert!(
        err_msg.contains(expected_err_substr),
        "Expected error containing '{}', got '{}'",
        expected_err_substr,
        err_msg
    );
    assert_eq!(
        exec.invocation_count(),
        0,
        "Zero terminal tools may be dispatched"
    );
    assert_eq!(
        *mock_os.last_action.lock().unwrap(),
        None,
        "Zero OS tools may be dispatched"
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn test_streaming_zero_dispatch_on_non_200_http() {
    assert_streaming_failure_dispatches_zero_tools(
        MockHttpReply::Json {
            status: 500,
            body: json!({ "error": { "message": "Internal failure" } }),
        },
        "HTTP 500",
    )
    .await;
}

#[tokio::test]
async fn test_streaming_zero_dispatch_on_prompt_safety_block() {
    let event = format!(
        "data: {}\n\n",
        json!({ "promptFeedback": { "blockReason": "SAFETY" } })
    );
    assert_streaming_failure_dispatches_zero_tools(
        MockHttpReply::Sse {
            status: 200,
            events: vec![event],
            truncate_mid_frame: false,
        },
        "blocked by prompt safety filter: SAFETY",
    )
    .await;
}

#[tokio::test]
async fn test_streaming_zero_dispatch_on_empty_or_missing_candidates() {
    let empty_cands = format!("data: {}\n\n", json!({ "candidates": [] }));
    assert_streaming_failure_dispatches_zero_tools(
        MockHttpReply::Sse {
            status: 200,
            events: vec![empty_cands],
            truncate_mid_frame: false,
        },
        "empty candidates array",
    )
    .await;

    let missing_cands = format!(
        "data: {}\n\n",
        json!({ "usageMetadata": { "totalTokenCount": 5 } })
    );
    assert_streaming_failure_dispatches_zero_tools(
        MockHttpReply::Sse {
            status: 200,
            events: vec![missing_cands],
            truncate_mid_frame: false,
        },
        "missing candidates",
    )
    .await;
}

#[tokio::test]
async fn test_streaming_zero_dispatch_on_finish_reason_safety() {
    // Chunk 1 emits an executable tool call; Chunk 2 terminates with SAFETY -> MUST discard Chunk 1!
    let chunk1 = format!(
        "data: {}\n\n",
        json!({
            "candidates": [{
                "content": { "parts": [{ "functionCall": { "id": "c1", "name": "run_terminal_command", "args": { "command": "git status" } } }] }
            }]
        })
    );
    let chunk2 = format!(
        "data: {}\n\n",
        json!({ "candidates": [{ "finishReason": "SAFETY" }] })
    );
    assert_streaming_failure_dispatches_zero_tools(
        MockHttpReply::Sse {
            status: 200,
            events: vec![chunk1, chunk2],
            truncate_mid_frame: false,
        },
        "SAFETY filter",
    )
    .await;
}

#[tokio::test]
async fn test_streaming_zero_dispatch_on_finish_reason_max_tokens() {
    let chunk1 = format!(
        "data: {}\n\n",
        json!({
            "candidates": [{
                "content": { "parts": [{ "functionCall": { "id": "c1", "name": "run_terminal_command", "args": { "command": "git status" } } }] }
            }]
        })
    );
    let chunk2 = format!(
        "data: {}\n\n",
        json!({ "candidates": [{ "finishReason": "MAX_TOKENS" }] })
    );
    assert_streaming_failure_dispatches_zero_tools(
        MockHttpReply::Sse {
            status: 200,
            events: vec![chunk1, chunk2],
            truncate_mid_frame: false,
        },
        "MAX_TOKENS limit",
    )
    .await;
}

#[tokio::test]
async fn test_streaming_zero_dispatch_on_finish_reason_continuation() {
    let chunk1 = format!(
        "data: {}\n\n",
        json!({
            "candidates": [{
                "content": { "parts": [{ "functionCall": { "id": "c1", "name": "run_terminal_command", "args": { "command": "git status" } } }] }
            }]
        })
    );
    let chunk2 = format!(
        "data: {}\n\n",
        json!({ "candidates": [{ "finishReason": "CONTINUATION" }] })
    );
    assert_streaming_failure_dispatches_zero_tools(
        MockHttpReply::Sse {
            status: 200,
            events: vec![chunk1, chunk2],
            truncate_mid_frame: false,
        },
        "unhandled CONTINUATION",
    )
    .await;
}

#[tokio::test]
async fn test_streaming_zero_dispatch_on_malformed_stream() {
    // Subcase 1: Chunk 1 has functionCall, Chunk 2 has malformed JSON
    let chunk1 = format!(
        "data: {}\n\n",
        json!({
            "candidates": [{
                "content": { "parts": [{ "functionCall": { "id": "c1", "name": "run_terminal_command", "args": { "command": "git status" } } }] }
            }]
        })
    );
    assert_streaming_failure_dispatches_zero_tools(
        MockHttpReply::Sse {
            status: 200,
            events: vec![chunk1.clone(), "data: { malformed json\n\n".to_string()],
            truncate_mid_frame: false,
        },
        "Malformed JSON",
    )
    .await;

    // Subcase 2: Chunk 1 has functionCall, stream truncates mid-frame at EOF
    assert_streaming_failure_dispatches_zero_tools(
        MockHttpReply::Sse {
            status: 200,
            events: vec![chunk1.clone()],
            truncate_mid_frame: true,
        },
        "Truncated or malformed trailing SSE frame",
    )
    .await;

    // Subcase 3: Chunk 1 has complete SSE functionCall event without finishReason STOP,
    // followed by premature clean EOF at event boundary -> zero dispatch
    assert_streaming_failure_dispatches_zero_tools(
        MockHttpReply::Sse {
            status: 200,
            events: vec![chunk1],
            truncate_mid_frame: false,
        },
        "terminal finishReason STOP",
    )
    .await;
}

#[tokio::test]
async fn test_streaming_zero_dispatch_on_unexpected_finish_reason() {
    let chunk1 = format!(
        "data: {}\n\n",
        json!({
            "candidates": [{
                "content": { "parts": [{ "functionCall": { "id": "c1", "name": "run_terminal_command", "args": { "command": "git status" } } }] }
            }]
        })
    );
    let chunk2 = format!(
        "data: {}\n\n",
        json!({ "candidates": [{ "finishReason": "MALFORMED_FUNCTION_CALL" }] })
    );
    assert_streaming_failure_dispatches_zero_tools(
        MockHttpReply::Sse {
            status: 200,
            events: vec![chunk1, chunk2],
            truncate_mid_frame: false,
        },
        "non-STOP finishReason: MALFORMED_FUNCTION_CALL",
    )
    .await;
}

#[tokio::test]
async fn test_streaming_completed_stop_passes_individual_authorization() {
    let policy = PolicyEngine::new(&["C:\\Projects".into()]);
    let mock_os = crate::tools::os_control::MockOsController::default();
    let token = CancellationToken::new();
    let workspace = make_valid_test_workspace("stop_auth");

    let amber_sse = format!(
        "data: {}\n\n",
        json!({
            "candidates": [{
                "finishReason": "STOP",
                "content": {
                    "parts": [{
                        "functionCall": { "name": "run_terminal_command", "args": { "command": "cargo build" }, "id": "call_stop_amber" },
                        "thoughtSignature": "sig_stop"
                    }]
                }
            }]
        })
    );
    let done_sse = format!(
        "data: {}\n\n",
        json!({
            "candidates": [{
                "finishReason": "STOP",
                "content": { "parts": [{ "text": "Authorization enforced." }] }
            }]
        })
    );

    // 1. Denied Amber command on STOP response -> 0 invocations
    let transport1 = MockLlmTransport::new(vec![
        MockHttpReply::Sse {
            status: 200,
            events: vec![amber_sse.clone()],
            truncate_mid_frame: false,
        },
        MockHttpReply::Sse {
            status: 200,
            events: vec![done_sse.clone()],
            truncate_mid_frame: false,
        },
    ]);
    let client1 = GeminiClient::new(Some("test-api-key".into()), None, None).unwrap();
    let exec1 = InstrumentedTerminalExecutor::new();
    let conf1 = ConfirmationManager::new();
    let mut sub1 = conf1.subscribe();
    let conf1_c = conf1.clone();
    let t1 = tokio::spawn(async move {
        while let Ok(n) = sub1.recv().await {
            if let Some(id) = n["params"]["id"].as_u64() {
                conf1_c.resolve_confirmation(id, false).await;
                break;
            }
        }
    });
    let _ = client1
        .chat_with_tools_streaming_with_executor(
            "Build project",
            70,
            None,
            &policy,
            &conf1,
            Some(&workspace),
            None,
            &mock_os,
            &exec1,
            &transport1,
            &token,
            None,
            || {},
        )
        .await
        .unwrap();
    let _ = t1.await;
    assert_eq!(
        exec1.invocation_count(),
        0,
        "Denied Amber command must not execute"
    );

    // 2. Approved Amber command on STOP response -> 1 invocation
    let transport2 = MockLlmTransport::new(vec![
        MockHttpReply::Sse {
            status: 200,
            events: vec![amber_sse],
            truncate_mid_frame: false,
        },
        MockHttpReply::Sse {
            status: 200,
            events: vec![done_sse],
            truncate_mid_frame: false,
        },
    ]);
    let client2 = GeminiClient::new(Some("test-api-key".into()), None, None).unwrap();
    let exec2 = InstrumentedTerminalExecutor::new();
    let conf2 = ConfirmationManager::new();
    let mut sub2 = conf2.subscribe();
    let conf2_c = conf2.clone();
    let t2 = tokio::spawn(async move {
        while let Ok(n) = sub2.recv().await {
            if let Some(id) = n["params"]["id"].as_u64() {
                conf2_c.resolve_confirmation(id, true).await;
                break;
            }
        }
    });
    let _ = client2
        .chat_with_tools_streaming_with_executor(
            "Build project",
            71,
            None,
            &policy,
            &conf2,
            Some(&workspace),
            None,
            &mock_os,
            &exec2,
            &transport2,
            &token,
            None,
            || {},
        )
        .await
        .unwrap();
    let _ = t2.await;
    assert_eq!(
        exec2.invocation_count(),
        1,
        "Approved Amber command must execute once"
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn test_gemini_429_quota_falls_back_to_ollama_qwen_with_tools_and_tts() {
    let _env_lock = ENV_LOCK.lock().await;
    let workspace = make_valid_test_workspace("ollama_429_fallback");
    std::fs::write(workspace.join("status.txt"), "All subsystems green.").unwrap();

    let policy = PolicyEngine::new(&[workspace.to_string_lossy().into_owned()]);
    let conf_mgr = ConfirmationManager::new();
    let mock_os = crate::tools::os_control::MockOsController::default();
    let exec = InstrumentedTerminalExecutor::new();
    let token = CancellationToken::new();
    let (tts_tx, mut tts_rx) = tokio::sync::mpsc::channel(16);

    let transport = MockLlmTransport::new(vec![
        // Turn 1 attempt on Gemini: 429 Quota Exhausted
        MockHttpReply::Json {
            status: 429,
            body: json!({
                "error": {
                    "code": 429,
                    "status": "RESOURCE_EXHAUSTED",
                    "message": "Free tier quota exceeded"
                }
            }),
        },
        // Automatic fallback to Ollama /api/chat: Turn 1 requests sandboxed read_file
        MockHttpReply::Sse {
            status: 200,
            events: vec![
                "{\"model\":\"qwen3.5:2b\",\"message\":{\"role\":\"assistant\",\"content\":\"\",\"tool_calls\":[{\"function\":{\"name\":\"read_file\",\"arguments\":{\"path\":\"status.txt\"}}}]},\"done\":true}\n".to_string(),
            ],
            truncate_mid_frame: false,
        },
        // Ollama Turn 2: strips <think>...</think> and streams final TTS sentence
        MockHttpReply::Sse {
            status: 200,
            events: vec![
                "{\"model\":\"qwen3.5:2b\",\"message\":{\"role\":\"assistant\",\"content\":\"<think>file says green</think>\"},\"done\":false}\n".to_string(),
                "{\"model\":\"qwen3.5:2b\",\"message\":{\"role\":\"assistant\",\"content\":\"All subsystems report green.\"},\"done\":true}\n".to_string(),
            ],
            truncate_mid_frame: false,
        },
    ]);

    let mut ttft_fired = false;
    let client = GeminiClient::new(Some("free-tier-key".into()), None, None).unwrap();
    let (reply, model_sel, _, taint) = client
        .chat_with_tools_streaming_with_executor(
            "Check status.txt",
            88,
            None,
            &policy,
            &conf_mgr,
            Some(&workspace),
            None,
            &mock_os,
            &exec,
            &transport,
            &token,
            Some(tts_tx),
            || {
                ttft_fired = true;
            },
        )
        .await
        .expect("429 quota exhaustion must transparently fall back to Ollama qwen3.5:2b");

    assert!(ttft_fired);
    assert_eq!(
        model_sel.tier,
        crate::agent::model_router::ModelTier::LocalFallback
    );
    assert_eq!(model_sel.model_name, "qwen3.5:2b");
    assert_eq!(reply, "All subsystems report green.");
    assert!(
        taint.taint_sources.contains(&"file:read".to_string()),
        "Ollama fallback tool loop must enforce taint propagation"
    );

    let tts0 = tts_rx.recv().await.expect("expected TTS chunk from Ollama fallback");
    assert_eq!(tts0.turn_id, 88);
    assert_eq!(tts0.text, "All subsystems report green.");
    let tts_final = tts_rx.recv().await.expect("expected is_final TTS termination chunk");
    assert_eq!(tts_final.turn_id, 88);
    assert!(tts_final.is_final);

    let captured = transport.captured.lock().await;
    assert_eq!(captured.len(), 3, "1 Gemini attempt + 2 Ollama turns");
    assert_eq!(captured[1]["model"], "qwen3.5:2b");
    assert!(
        captured[2]["messages"][3]["content"]
            .as_str()
            .unwrap()
            .contains("All subsystems green."),
        "Wrapped nonced tool output must be sent back in Ollama tool message"
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn test_gemini_safety_block_does_not_trigger_ollama_fallback() {
    let transport = MockLlmTransport::new(vec![MockHttpReply::Sse {
        status: 200,
        events: vec![
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"blocked\"}]},\"finishReason\":\"SAFETY\"}]}\n\n".to_string(),
        ],
        truncate_mid_frame: false,
    }]);

    let policy = PolicyEngine::new(&["C:\\Projects".into()]);
    let conf_mgr = ConfirmationManager::new();
    let mock_os = crate::tools::os_control::MockOsController::default();
    let exec = InstrumentedTerminalExecutor::new();
    let token = CancellationToken::new();

    let client = GeminiClient::new(Some("free-tier-key".into()), None, None).unwrap();
    let res = client
        .chat_with_tools_streaming_with_executor(
            "Unsafe request",
            89,
            None,
            &policy,
            &conf_mgr,
            None,
            None,
            &mock_os,
            &exec,
            &transport,
            &token,
            None,
            || {},
        )
        .await;

    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("SAFETY"));
    let captured = transport.captured.lock().await;
    assert_eq!(
        captured.len(),
        1,
        "SAFETY termination must fail closed without calling Ollama fallback"
    );
}


