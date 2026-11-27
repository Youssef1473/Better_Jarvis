use crate::agent::sse_parser::{FunctionCall, ParsedModelResponse};
use crate::agent::tool_schemas::{classify_tool_execution, ToolExecutionClass};
use crate::security::policy::{self, PolicyEngine, SecurityTier, TurnTaintState};
use crate::security::sandbox;
use crate::tools::{browser_cdp, confirmation::ConfirmationManager};
use anyhow::Result;
use serde_json::{json, Value};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Abstraction over terminal command execution for production sandboxing and test instrumentation.
pub trait TerminalExecutor: Send + Sync {
    fn execute_command(
        &self,
        command: &str,
        workspace_dir: Option<&Path>,
        timeout_secs: u64,
    ) -> futures::future::BoxFuture<'static, Result<sandbox::GuardedCommandOutput>>;
}

/// Production terminal executor routing strictly through the Win32 Job Object sandbox.
#[derive(Debug, Default, Clone, Copy)]
pub struct SandboxTerminalExecutor;

impl TerminalExecutor for SandboxTerminalExecutor {
    fn execute_command(
        &self,
        command: &str,
        workspace_dir: Option<&Path>,
        timeout_secs: u64,
    ) -> futures::future::BoxFuture<'static, Result<sandbox::GuardedCommandOutput>> {
        let cmd = command.to_string();
        let dir = workspace_dir.map(Path::to_path_buf);
        Box::pin(
            async move { sandbox::run_guarded_command(&cmd, dir.as_deref(), timeout_secs).await },
        )
    }
}

/// Abstraction over proven-independent read-only/taint-source tool execution.
pub(crate) trait ReadOnlyToolExecutor: Send + Sync {
    fn inspect_active_window(
        &self,
    ) -> futures::future::BoxFuture<
        'static,
        std::result::Result<
            crate::tools::screen_vision::WindowMetadata,
            crate::tools::screen_vision::VisionError,
        >,
    >;

    fn read_active_window_ocr(
        &self,
        notifier: Option<tokio::sync::broadcast::Sender<Value>>,
    ) -> futures::future::BoxFuture<
        'static,
        std::result::Result<
            (crate::tools::screen_vision::WindowMetadata, String),
            crate::tools::screen_vision::VisionError,
        >,
    >;

    fn read_active_tab(&self) -> futures::future::BoxFuture<'static, Result<Option<String>>>;

    fn read_browser_ax_tree(&self) -> futures::future::BoxFuture<'static, Result<Option<String>>> {
        Box::pin(async move { Ok(None) })
    }
}

/// Production read-only executor delegating directly to Win32 vision privacy guards and Chrome CDP.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct DefaultReadOnlyToolExecutor;

impl ReadOnlyToolExecutor for DefaultReadOnlyToolExecutor {
    fn inspect_active_window(
        &self,
    ) -> futures::future::BoxFuture<
        'static,
        std::result::Result<
            crate::tools::screen_vision::WindowMetadata,
            crate::tools::screen_vision::VisionError,
        >,
    > {
        Box::pin(async move { crate::tools::screen_vision::get_active_window_metadata() })
    }

    fn read_active_window_ocr(
        &self,
        notifier: Option<tokio::sync::broadcast::Sender<Value>>,
    ) -> futures::future::BoxFuture<
        'static,
        std::result::Result<
            (crate::tools::screen_vision::WindowMetadata, String),
            crate::tools::screen_vision::VisionError,
        >,
    > {
        Box::pin(async move {
            crate::tools::screen_vision::capture_and_ocr_active_window(notifier).await
        })
    }

    fn read_active_tab(&self) -> futures::future::BoxFuture<'static, Result<Option<String>>> {
        Box::pin(async move { browser_cdp::read_active_tab().await })
    }

    fn read_browser_ax_tree(&self) -> futures::future::BoxFuture<'static, Result<Option<String>>> {
        Box::pin(async move { browser_cdp::read_active_tab_ax_tree().await })
    }
}

/// Abstraction over browser navigation for production CDP isolation and test instrumentation.
pub(crate) trait BrowserNavigator: Send + Sync {
    fn open_url(&self, url: &str) -> futures::future::BoxFuture<'static, Result<()>>;
    fn open_youtube(&self, query: Option<&str>) -> futures::future::BoxFuture<'static, Result<()>>;
}

/// Production browser navigator delegating directly to isolated Chromium CDP pipes.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct DefaultBrowserNavigator;

impl BrowserNavigator for DefaultBrowserNavigator {
    fn open_url(&self, url: &str) -> futures::future::BoxFuture<'static, Result<()>> {
        let target = url.to_string();
        Box::pin(async move { browser_cdp::open_url(&target).await })
    }

    fn open_youtube(&self, query: Option<&str>) -> futures::future::BoxFuture<'static, Result<()>> {
        let q = query.map(ToString::to_string);
        Box::pin(async move { browser_cdp::open_youtube(q.as_deref()).await })
    }
}

fn resolve_workspace_path(raw: &str, workspace_dir: Option<&Path>) -> PathBuf {
    let trimmed = raw.trim();
    if let Some(ws) = workspace_dir {
        if trimmed == "." {
            return ws.to_path_buf();
        }
        let stripped = trimmed
            .strip_prefix("./")
            .or_else(|| trimmed.strip_prefix(".\\"))
            .unwrap_or(trimmed);
        let p = PathBuf::from(stripped);
        if p.is_relative() {
            return ws.join(p);
        }
        return p;
    }
    PathBuf::from(trimmed)
}

fn read_sandboxed_file_sync(
    policy: &PolicyEngine,
    target_path: &Path,
    max_bytes: usize,
) -> std::result::Result<(String, bool), (bool, String)> {
    let mut file = policy.open_validated_file(target_path, false).map_err(|e| {
        let blocked = e.kind() == std::io::ErrorKind::PermissionDenied;
        (blocked, e.to_string())
    })?;

    let meta = file.metadata().map_err(|e| (false, e.to_string()))?;
    if meta.is_dir() {
        return Err((
            false,
            "Target path is a directory; use list_directory instead".to_string(),
        ));
    }

    let mut buf = Vec::with_capacity(max_bytes.min(4096) + 1);
    (&mut file)
        .take((max_bytes + 1) as u64)
        .read_to_end(&mut buf)
        .map_err(|e| (false, format!("Failed to read file: {e}")))?;

    let truncated = buf.len() > max_bytes;
    if truncated {
        buf.truncate(max_bytes);
    }

    let content = String::from_utf8_lossy(&buf).into_owned();
    Ok((content, truncated))
}

fn list_sandboxed_directory_sync(
    policy: &PolicyEngine,
    target_path: &Path,
    max_entries: usize,
) -> std::result::Result<(Vec<String>, bool), (bool, String)> {
    let resolved = policy
        .validate_path(target_path)
        .map_err(|pol| (true, pol.reason))?;

    let meta = std::fs::metadata(&resolved)
        .map_err(|e| (false, format!("Failed to stat directory: {e}")))?;
    if !meta.is_dir() {
        return Err((false, "Target path is not a directory".to_string()));
    }

    let read_dir = std::fs::read_dir(&resolved)
        .map_err(|e| (false, format!("Failed to read directory: {e}")))?;

    let mut formatted = Vec::new();
    for entry_res in read_dir {
        let Ok(entry) = entry_res else {
            continue;
        };
        let entry_path = target_path.join(entry.file_name());
        if policy.validate_path(&entry_path).is_err() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Ok(emeta) = entry.metadata() {
            if emeta.is_dir() {
                formatted.push(format!("[dir] {name}"));
            } else {
                formatted.push(format!("[file] {name} ({} B)", emeta.len()));
            }
        } else {
            formatted.push(format!("[entry] {name}"));
        }
    }

    formatted.sort();
    let truncated = formatted.len() > max_entries;
    if truncated {
        formatted.truncate(max_entries);
    }
    Ok((formatted, truncated))
}

/// Shared duplicate/cancellation/timeout guardrail wrapper for both single and batched tool execution.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_guarded_tool_invocation(
    idx: usize,
    call: &FunctionCall,
    is_cancelled: bool,
    is_dup: bool,
    policy: &PolicyEngine,
    confirmation_mgr: &ConfirmationManager,
    workspace_dir: Option<&Path>,
    memory: Option<&crate::memory::MemoryEngine>,
    os_controller: &dyn crate::tools::os_control::OsController,
    terminal_executor: &dyn TerminalExecutor,
    readonly_executor: &dyn ReadOnlyToolExecutor,
    browser_navigator: &dyn BrowserNavigator,
    taint_snapshot: &TurnTaintState,
    turn_token: &CancellationToken,
) -> (usize, Value, Vec<&'static str>) {
    if is_cancelled {
        return (
            idx,
            json!({
                "status": "CANCELLED",
                "error": "Turn was cancelled by barge-in or shutdown"
            }),
            Vec::new(),
        );
    }
    if is_dup {
        warn!(
            "Duplicate tool call intercepted in turn: {} {}",
            call.name, call.args
        );
        return (
            idx,
            json!({
                "status": "DUPLICATE_CALL_BLOCKED",
                "error": format!("Tool call '{}' with identical arguments was already executed in this turn", call.name)
            }),
            Vec::new(),
        );
    }

    let dispatch_fut = dispatch_tool_call_with_executor(
        call,
        policy,
        confirmation_mgr,
        workspace_dir,
        memory,
        os_controller,
        terminal_executor,
        readonly_executor,
        browser_navigator,
        taint_snapshot,
        turn_token,
    );

    let (val, emitted_taint) = tokio::select! {
        _ = turn_token.cancelled() => {
            (
                json!({
                    "status": "CANCELLED",
                    "error": "Turn was cancelled by barge-in or shutdown"
                }),
                Vec::new(),
            )
        }
        res = tokio::time::timeout(Duration::from_secs(15), dispatch_fut) => {
            match res {
                Ok(pair) => pair,
                Err(_) => {
                    warn!("Tool call {} timed out after 15s", call.name);
                    (
                        json!({
                            "status": "TIMEOUT",
                            "error": format!("Tool '{}' execution exceeded 15s timeout limit", call.name)
                        }),
                        Vec::new(),
                    )
                }
            }
        }
    };
    (idx, val, emitted_taint)
}

/// Executes a tool call with duplicate loop detection, timeout guard, and cancellation abort.
#[allow(clippy::too_many_arguments)]
pub async fn execute_tool_with_guardrails(
    executed_tools: &mut std::collections::HashSet<(String, String)>,
    call: &FunctionCall,
    policy: &PolicyEngine,
    confirmation_mgr: &ConfirmationManager,
    workspace_dir: Option<&Path>,
    memory: Option<&crate::memory::MemoryEngine>,
    os_controller: &dyn crate::tools::os_control::OsController,
    taint: &mut TurnTaintState,
    turn_token: &CancellationToken,
) -> Value {
    let is_cancelled = turn_token.is_cancelled();
    let fc_key = (call.name.clone(), call.args.to_string());
    let is_dup = !is_cancelled && !executed_tools.insert(fc_key);

    let (_, val, new_taint_sources) = run_guarded_tool_invocation(
        0,
        call,
        is_cancelled,
        is_dup,
        policy,
        confirmation_mgr,
        workspace_dir,
        memory,
        os_controller,
        &SandboxTerminalExecutor,
        &DefaultReadOnlyToolExecutor,
        &DefaultBrowserNavigator,
        taint,
        turn_token,
    )
    .await;

    for src in new_taint_sources {
        taint.mark_tainted(src);
    }
    val
}

/// Dispatches a functionCall strictly through the 3-Tier Security Policy and Confirmation Manager.
#[allow(clippy::too_many_arguments)]
pub async fn dispatch_tool_call(
    call: &FunctionCall,
    policy: &PolicyEngine,
    confirmation_mgr: &ConfirmationManager,
    workspace_dir: Option<&Path>,
    memory: Option<&crate::memory::MemoryEngine>,
    os_controller: &dyn crate::tools::os_control::OsController,
    taint: &mut TurnTaintState,
    turn_token: &CancellationToken,
) -> Value {
    let (val, new_taint_sources) = dispatch_tool_call_with_executor(
        call,
        policy,
        confirmation_mgr,
        workspace_dir,
        memory,
        os_controller,
        &SandboxTerminalExecutor,
        &DefaultReadOnlyToolExecutor,
        &DefaultBrowserNavigator,
        taint,
        turn_token,
    )
    .await;
    for src in new_taint_sources {
        taint.mark_tainted(src);
    }
    val
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn dispatch_tool_call_with_executor(
    call: &FunctionCall,
    policy: &PolicyEngine,
    confirmation_mgr: &ConfirmationManager,
    workspace_dir: Option<&Path>,
    memory: Option<&crate::memory::MemoryEngine>,
    os_controller: &dyn crate::tools::os_control::OsController,
    terminal_executor: &dyn TerminalExecutor,
    readonly_executor: &dyn ReadOnlyToolExecutor,
    browser_navigator: &dyn BrowserNavigator,
    taint: &TurnTaintState,
    turn_token: &CancellationToken,
) -> (Value, Vec<&'static str>) {
    if turn_token.is_cancelled() {
        return (
            json!({
                "status": "CANCELLED",
                "error": "Turn was cancelled by barge-in or shutdown"
            }),
            Vec::new(),
        );
    }

    info!("Executing Gemini functionCall: {}", call.name);
    let mut emitted_taint: Vec<&'static str> = Vec::new();

    let val = match call.name.as_str() {
        "run_terminal_command" => {
            let command = match call.args.get("command").and_then(|v| v.as_str()) {
                Some(c) => c,
                None => {
                    return (
                        json!({ "error": "Missing required 'command' argument" }),
                        emitted_taint,
                    );
                }
            };

            let mut action_policy = policy.evaluate_command(command);
            if action_policy.tier == SecurityTier::Green && taint.is_tainted() {
                action_policy =
                    policy.evaluate_action_with_taint("run_terminal_command", command, taint);
            }

            match action_policy.tier {
                SecurityTier::Red => {
                    warn!("Gemini tool call blocked by RED tier: {}", action_policy.reason);
                    return (
                        json!({
                            "status": "BLOCKED",
                            "reason": format!("Policy Blocked: {}", action_policy.reason)
                        }),
                        emitted_taint,
                    );
                }
                SecurityTier::Amber => {
                    let prompt_text = if taint.is_tainted() {
                        format!("Execute (Tainted Context): {}", command)
                    } else {
                        format!("Execute: {}", command)
                    };
                    info!("Gemini tool call requires AMBER confirmation: {}", prompt_text);
                    if !confirmation_mgr.request_confirmation(&prompt_text, 8000).await {
                        return (
                            json!({
                                "status": "DENIED",
                                "reason": "User denied action confirmation or request timed out"
                            }),
                            emitted_taint,
                        );
                    }
                }
                SecurityTier::Green => {}
            }

            match terminal_executor
                .execute_command(command, workspace_dir, 15)
                .await
            {
                Ok(out) => {
                    emitted_taint.push("terminal_output");
                    json!({
                        "status": "SUCCESS",
                        "exit_code": out.exit_code,
                        "stdout": out.stdout,
                        "stderr": out.stderr,
                        "duration_ms": out.duration_ms
                    })
                }
                Err(e) => json!({ "status": "ERROR", "error": e.to_string() }),
            }
        }
        "open_browser_url" => {
            let url = match call.args.get("url").and_then(|v| v.as_str()) {
                Some(u) => u,
                None => {
                    return (
                        json!({ "error": "Missing required 'url' argument" }),
                        emitted_taint,
                    )
                }
            };

            let action_policy = policy.evaluate_action_with_taint("open_browser_url", url, taint);
            if action_policy.requires_confirmation {
                let approved = confirmation_mgr
                    .request_confirmation(&format!("Open URL (Tainted Context): {}", url), 8000)
                    .await;
                if !approved {
                    return (
                        json!({
                            "status": "DENIED",
                            "reason": "User denied action confirmation or request timed out"
                        }),
                        emitted_taint,
                    );
                }
            }

            match browser_navigator.open_url(url).await {
                Ok(_) => {
                    json!({ "status": "SUCCESS", "message": format!("Navigated to {}", url) })
                }
                Err(e) => json!({ "status": "ERROR", "error": e.to_string() }),
            }
        }
        "search_youtube" => {
            let query = call.args.get("query").and_then(|v| v.as_str());
            let target_str = query.unwrap_or("homepage");
            let action_policy =
                policy.evaluate_action_with_taint("search_youtube", target_str, taint);
            if action_policy.requires_confirmation {
                let prompt_str = match query {
                    Some(q) => format!("Open YouTube search: {}", q),
                    None => "Open YouTube homepage".to_string(),
                };
                let approved = confirmation_mgr.request_confirmation(&prompt_str, 8000).await;
                if !approved {
                    return (
                        json!({
                            "status": "DENIED",
                            "reason": "User denied action confirmation or request timed out"
                        }),
                        emitted_taint,
                    );
                }
            }

            match browser_navigator.open_youtube(query).await {
                Ok(_) => json!({
                    "status": "SUCCESS",
                    "message": match query {
                        Some(q) => format!("Opened YouTube search for '{}'", q),
                        None => "Opened YouTube homepage".to_string(),
                    }
                }),
                Err(e) => json!({ "status": "ERROR", "error": e.to_string() }),
            }
        }
        "read_active_tab" => match readonly_executor.read_active_tab().await {
            Ok(Some(tab_info)) => {
                emitted_taint.push("browser_cdp:read");
                json!({
                    "status": "SUCCESS",
                    "tab": tab_info
                })
            }
            Ok(None) => json!({
                "status": "SUCCESS",
                "tab": "No active managed Chromium browser session available"
            }),
            Err(e) => json!({ "status": "ERROR", "error": e.to_string() }),
        },
        "read_browser_ax_tree" => match readonly_executor.read_browser_ax_tree().await {
            Ok(Some(ax_tree)) => {
                emitted_taint.push("browser_cdp:ax_tree");
                json!({
                    "status": "SUCCESS",
                    "ax_tree": ax_tree
                })
            }
            Ok(None) => json!({
                "status": "SUCCESS",
                "ax_tree": "No active managed Chromium browser session available"
            }),
            Err(e) => json!({ "status": "ERROR", "error": e.to_string() }),
        },
        "read_file" => {
            let path_arg = match call
                .args
                .get("path")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                Some(p) => p,
                None => {
                    return (
                        json!({ "error": "Missing required 'path' argument" }),
                        emitted_taint,
                    )
                }
            };
            let max_bytes = call
                .args
                .get("max_bytes")
                .and_then(|v| v.as_u64())
                .unwrap_or(16384)
                .clamp(1, 16384) as usize;
            let resolved_target = resolve_workspace_path(path_arg, workspace_dir);
            let policy_clone = policy.clone();

            match tokio::task::spawn_blocking(move || {
                read_sandboxed_file_sync(&policy_clone, &resolved_target, max_bytes)
            })
            .await
            {
                Ok(Ok((content, truncated))) => {
                    emitted_taint.push("file:read");
                    json!({
                        "status": "SUCCESS",
                        "path": path_arg,
                        "content": content,
                        "truncated": truncated
                    })
                }
                Ok(Err((true, reason))) => json!({
                    "status": "BLOCKED",
                    "reason": format!("Security Policy Violation: {}", reason)
                }),
                Ok(Err((false, err))) => json!({ "status": "ERROR", "error": err }),
                Err(e) => json!({ "status": "ERROR", "error": e.to_string() }),
            }
        }
        "list_directory" => {
            let path_arg = match call
                .args
                .get("path")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                Some(p) => p,
                None => {
                    return (
                        json!({ "error": "Missing required 'path' argument" }),
                        emitted_taint,
                    )
                }
            };
            let resolved_target = resolve_workspace_path(path_arg, workspace_dir);
            let policy_clone = policy.clone();

            match tokio::task::spawn_blocking(move || {
                list_sandboxed_directory_sync(&policy_clone, &resolved_target, 200)
            })
            .await
            {
                Ok(Ok((entries, truncated))) => {
                    emitted_taint.push("file:list");
                    json!({
                        "status": "SUCCESS",
                        "path": path_arg,
                        "entries": entries,
                        "truncated": truncated
                    })
                }
                Ok(Err((true, reason))) => json!({
                    "status": "BLOCKED",
                    "reason": format!("Security Policy Violation: {}", reason)
                }),
                Ok(Err((false, err))) => json!({ "status": "ERROR", "error": err }),
                Err(e) => json!({ "status": "ERROR", "error": e.to_string() }),
            }
        }
        "control_os_media" => {
            let action = match call.args.get("action").and_then(|v| v.as_str()) {
                Some(a) => a,
                None => {
                    return (
                        json!({ "error": "Missing required 'action' argument" }),
                        emitted_taint,
                    )
                }
            };

            let res = match action {
                "volume_up" => os_controller
                    .control_media(crate::tools::os_control::OsMediaAction::VolumeUp)
                    .await
                    .map(|_| "Volume increased"),
                "volume_down" => os_controller
                    .control_media(crate::tools::os_control::OsMediaAction::VolumeDown)
                    .await
                    .map(|_| "Volume decreased"),
                "mute" => os_controller
                    .control_media(crate::tools::os_control::OsMediaAction::VolumeMuteToggle)
                    .await
                    .map(|_| "Volume mute toggled"),
                "play_pause" => os_controller
                    .control_media(crate::tools::os_control::OsMediaAction::MediaPlayPause)
                    .await
                    .map(|_| "Media playback toggled"),
                "next" => os_controller
                    .control_media(crate::tools::os_control::OsMediaAction::MediaNextTrack)
                    .await
                    .map(|_| "Skipped to next track"),
                "prev" => os_controller
                    .control_media(crate::tools::os_control::OsMediaAction::MediaPrevTrack)
                    .await
                    .map(|_| "Returned to previous track"),
                other => Err(anyhow::anyhow!("Unknown media action: {}", other)),
            };

            match res {
                Ok(msg) => json!({ "status": "SUCCESS", "message": msg }),
                Err(e) => json!({ "status": "ERROR", "error": e.to_string() }),
            }
        }
        "launch_application" => {
            let target = match call.args.get("target").and_then(|v| v.as_str()) {
                Some(t) => t,
                None => {
                    return (
                        json!({ "error": "Missing required 'target' argument" }),
                        emitted_taint,
                    )
                }
            };

            if let Err(e) = crate::tools::os_control::validate_launch_target(target) {
                return (
                    json!({ "status": "BLOCKED", "reason": format!("Launch blocked: {}", e) }),
                    emitted_taint,
                );
            }

            let action_policy = policy.evaluate_action_with_taint("launch_application", target, taint);
            if action_policy.requires_confirmation {
                let approved = confirmation_mgr
                    .request_confirmation(&format!("Launch application: {}", target), 8000)
                    .await;
                if !approved {
                    return (
                        json!({
                            "status": "DENIED",
                            "reason": "User denied action confirmation or request timed out"
                        }),
                        emitted_taint,
                    );
                }
            }

            match os_controller.launch_target(target).await {
                Ok(_) => json!({ "status": "SUCCESS", "message": format!("Launched {}", target) }),
                Err(e) => json!({ "status": "ERROR", "error": e.to_string() }),
            }
        }
        "remember_fact" => {
            let key = match call.args.get("key").and_then(|v| v.as_str()) {
                Some(k) => k,
                None => {
                    return (
                        json!({ "error": "Missing required 'key' argument" }),
                        emitted_taint,
                    )
                }
            };
            let value = match call.args.get("value").and_then(|v| v.as_str()) {
                Some(v) => v,
                None => {
                    return (
                        json!({ "error": "Missing required 'value' argument" }),
                        emitted_taint,
                    )
                }
            };
            let tier = call
                .args
                .get("tier")
                .and_then(|v| v.as_str())
                .unwrap_or("preference");

            let action_policy = policy.evaluate_action_with_taint("remember_fact", key, taint);
            if action_policy.requires_confirmation {
                let approved = confirmation_mgr
                    .request_confirmation(
                        &format!("Remember fact (Tainted Context): {} = {}", key, value),
                        8000,
                    )
                    .await;
                if !approved {
                    return (
                        json!({
                            "status": "DENIED",
                            "reason": "User denied action confirmation or request timed out"
                        }),
                        emitted_taint,
                    );
                }
            }

            if let Some(mem) = memory {
                let source_trusted = !taint.is_tainted();
                match mem.upsert_fact(key, value, tier, source_trusted).await {
                    Ok(_) => {
                        let _ = mem.sync().export_profile_atomic().await;
                        json!({
                            "status": "SUCCESS",
                            "message": format!("Stored memory fact: {} = {}", key, value)
                        })
                    }
                    Err(e) => json!({ "status": "ERROR", "error": e.to_string() }),
                }
            } else {
                json!({ "status": "ERROR", "error": "Memory engine not available" })
            }
        }
        "search_memory" => {
            let query = match call.args.get("query").and_then(|v| v.as_str()) {
                Some(q) => q,
                None => {
                    return (
                        json!({ "error": "Missing required 'query' argument" }),
                        emitted_taint,
                    )
                }
            };
            if let Some(mem) = memory {
                match mem.search_hybrid(query, 5).await {
                    Ok(hits) => {
                        let mut results = Vec::new();
                        for hit in hits {
                            let chunk = &hit.chunk;
                            if !chunk.source_trusted {
                                emitted_taint.push("memory:untrusted_note");
                            }
                            results.push(json!({
                                "heading": chunk.heading,
                                "path": chunk.path,
                                "content": chunk.content,
                                "source_trusted": chunk.source_trusted,
                                "rrf_score": hit.rrf_score,
                            }));
                        }

                        let mut fact_results = Vec::new();
                        if let Ok(facts) = mem.search_facts(query, 5).await {
                            for (fact, score) in facts {
                                if !fact.source_trusted {
                                    emitted_taint.push("memory:untrusted_note");
                                }
                                fact_results.push(json!({
                                    "key": fact.key,
                                    "value": fact.value,
                                    "tier": fact.tier,
                                    "source_trusted": fact.source_trusted,
                                    "score": score,
                                }));
                            }
                        }

                        json!({
                            "status": "SUCCESS",
                            "query": query,
                            "results": results,
                            "facts": fact_results
                        })
                    }
                    Err(e) => json!({ "status": "ERROR", "error": e.to_string() }),
                }
            } else {
                json!({ "status": "ERROR", "error": "Memory engine not available" })
            }
        }
        "inspect_active_window" => match readonly_executor.inspect_active_window().await {
            Ok(meta) => {
                emitted_taint.push("vision:window_title");
                json!({
                    "status": "SUCCESS",
                    "process_name": meta.process_name,
                    "window_title": meta.title,
                    "width": meta.width,
                    "height": meta.height,
                })
            }
            Err(e) => {
                warn!("inspect_active_window blocked or failed: {e}");
                json!({
                    "status": "BLOCKED_OR_FAILED",
                    "error": e.to_string()
                })
            }
        },
        "read_active_window_ocr" => {
            let notifier = Some(confirmation_mgr.notifier());
            match readonly_executor.read_active_window_ocr(notifier).await {
                Ok((meta, ocr_text)) => {
                    emitted_taint.push("vision:ocr");
                    json!({
                        "status": "SUCCESS",
                        "process_name": meta.process_name,
                        "window_title": meta.title,
                        "ocr_text": ocr_text,
                    })
                }
                Err(e) => {
                    warn!("read_active_window_ocr blocked or failed: {e}");
                    json!({
                        "status": "BLOCKED_OR_FAILED",
                        "error": e.to_string()
                    })
                }
            }
        }
        unknown => {
            warn!("Unknown Gemini functionCall attempted: {}", unknown);
            json!({ "status": "ERROR", "error": format!("Tool '{}' does not exist", unknown) })
        }
    };

    (val, emitted_taint)
}

#[allow(clippy::too_many_arguments)]
async fn flush_readonly_batch(
    readonly_indices: &[usize],
    parsed: &ParsedModelResponse,
    ordered_outputs: &mut [Option<Value>],
    executed_tools_in_turn: &mut std::collections::HashSet<(String, String)>,
    policy: &PolicyEngine,
    confirmation_mgr: &ConfirmationManager,
    workspace_dir: Option<&Path>,
    memory: Option<&crate::memory::MemoryEngine>,
    os_controller: &dyn crate::tools::os_control::OsController,
    terminal_executor: &dyn TerminalExecutor,
    readonly_executor: &dyn ReadOnlyToolExecutor,
    browser_navigator: &dyn BrowserNavigator,
    taint: &mut TurnTaintState,
    turn_token: &CancellationToken,
) -> Result<()> {
    if readonly_indices.is_empty() {
        return Ok(());
    }

    let taint_snapshot = taint.clone();
    let mut futs = Vec::with_capacity(readonly_indices.len());
    for &idx in readonly_indices {
        let call = parsed.function_calls[idx].call.clone();
        let is_cancelled = turn_token.is_cancelled();
        let fc_key = (call.name.clone(), call.args.to_string());
        let is_dup = !is_cancelled && !executed_tools_in_turn.insert(fc_key);
        let taint_for_task = taint_snapshot.clone();

        futs.push(async move {
            run_guarded_tool_invocation(
                idx,
                &call,
                is_cancelled,
                is_dup,
                policy,
                confirmation_mgr,
                workspace_dir,
                memory,
                os_controller,
                terminal_executor,
                readonly_executor,
                browser_navigator,
                &taint_for_task,
                turn_token,
            )
            .await
        });
    }

    let batch_results = futures::future::join_all(futs).await;
    for (idx, val, emitted_taint) in batch_results {
        if val.get("status").and_then(|s| s.as_str()) == Some("CANCELLED") {
            anyhow::bail!("Turn cancelled by barge-in or shutdown");
        }
        for src in emitted_taint {
            taint.mark_tainted(src);
        }
        ordered_outputs[idx] = Some(val);
    }

    Ok(())
}

/// Executes a parsed turn's tool calls via the ordered batch scheduler and returns
/// `(tool_name, optional_id, nonced_wrapped_output)` in exact model-emitted order.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_tool_batch_outputs(
    parsed: &ParsedModelResponse,
    executed_tools_in_turn: &mut std::collections::HashSet<(String, String)>,
    policy: &PolicyEngine,
    confirmation_mgr: &ConfirmationManager,
    workspace_dir: Option<&Path>,
    memory: Option<&crate::memory::MemoryEngine>,
    os_controller: &dyn crate::tools::os_control::OsController,
    terminal_executor: &dyn TerminalExecutor,
    readonly_executor: &dyn ReadOnlyToolExecutor,
    browser_navigator: &dyn BrowserNavigator,
    taint: &mut TurnTaintState,
    turn_nonce: &str,
    turn_token: &CancellationToken,
) -> Result<Vec<(String, Option<String>, String)>> {
    let total = parsed.function_calls.len();
    let mut ordered_outputs: Vec<Option<Value>> = vec![None; total];
    let mut pending_readonly_indices: Vec<usize> = Vec::new();

    for (idx, mtc) in parsed.function_calls.iter().enumerate() {
        match classify_tool_execution(&mtc.call.name) {
            ToolExecutionClass::ReadOnlyConcurrent => {
                pending_readonly_indices.push(idx);
            }
            ToolExecutionClass::SerializedPrivileged => {
                if !pending_readonly_indices.is_empty() {
                    flush_readonly_batch(
                        &pending_readonly_indices,
                        parsed,
                        &mut ordered_outputs,
                        executed_tools_in_turn,
                        policy,
                        confirmation_mgr,
                        workspace_dir,
                        memory,
                        os_controller,
                        terminal_executor,
                        readonly_executor,
                        browser_navigator,
                        taint,
                        turn_token,
                    )
                    .await?;
                    pending_readonly_indices.clear();
                }

                let call = &mtc.call;
                let is_cancelled = turn_token.is_cancelled();
                let fc_key = (call.name.clone(), call.args.to_string());
                let is_dup = !is_cancelled && !executed_tools_in_turn.insert(fc_key);
                let taint_snapshot = taint.clone();

                let (_, val, emitted_taint) = run_guarded_tool_invocation(
                    idx,
                    call,
                    is_cancelled,
                    is_dup,
                    policy,
                    confirmation_mgr,
                    workspace_dir,
                    memory,
                    os_controller,
                    terminal_executor,
                    readonly_executor,
                    browser_navigator,
                    &taint_snapshot,
                    turn_token,
                )
                .await;

                if val.get("status").and_then(|s| s.as_str()) == Some("CANCELLED") {
                    anyhow::bail!("Turn cancelled by barge-in or shutdown");
                }
                for src in emitted_taint {
                    taint.mark_tainted(src);
                }
                ordered_outputs[idx] = Some(val);
            }
        }
    }

    if !pending_readonly_indices.is_empty() {
        flush_readonly_batch(
            &pending_readonly_indices,
            parsed,
            &mut ordered_outputs,
            executed_tools_in_turn,
            policy,
            confirmation_mgr,
            workspace_dir,
            memory,
            os_controller,
            terminal_executor,
            readonly_executor,
            browser_navigator,
            taint,
            turn_token,
        )
        .await?;
    }

    let mut wrapped_results = Vec::with_capacity(total);
    for (idx, mtc) in parsed.function_calls.iter().enumerate() {
        let tool_output = ordered_outputs[idx]
            .take()
            .unwrap_or_else(|| json!({ "status": "ERROR" }));
        let output_str = serde_json::to_string(&tool_output).unwrap_or_default();
        let wrapped_response =
            policy::wrap_untrusted_content_with_nonce(&mtc.call.name, &output_str, turn_nonce);
        wrapped_results.push((mtc.call.name.clone(), mtc.call.id.clone(), wrapped_response));
    }

    Ok(wrapped_results)
}

/// Executes model tool calls using a conservative ordered batch scheduler.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_parallel_tool_calls(
    parsed: &ParsedModelResponse,
    contents: &mut Vec<Value>,
    executed_tools_in_turn: &mut std::collections::HashSet<(String, String)>,
    policy: &PolicyEngine,
    confirmation_mgr: &ConfirmationManager,
    workspace_dir: Option<&Path>,
    memory: Option<&crate::memory::MemoryEngine>,
    os_controller: &dyn crate::tools::os_control::OsController,
    terminal_executor: &dyn TerminalExecutor,
    readonly_executor: &dyn ReadOnlyToolExecutor,
    browser_navigator: &dyn BrowserNavigator,
    taint: &mut TurnTaintState,
    turn_nonce: &str,
    turn_token: &CancellationToken,
) -> Result<()> {
    contents.push(json!({
        "role": "model",
        "parts": parsed.model_parts
    }));

    let wrapped_outputs = execute_tool_batch_outputs(
        parsed,
        executed_tools_in_turn,
        policy,
        confirmation_mgr,
        workspace_dir,
        memory,
        os_controller,
        terminal_executor,
        readonly_executor,
        browser_navigator,
        taint,
        turn_nonce,
        turn_token,
    )
    .await?;

    let mut response_parts = Vec::with_capacity(wrapped_outputs.len());
    for (name, id_opt, wrapped_response) in wrapped_outputs {
        let mut resp_obj = json!({
            "name": name,
            "response": {
                "output": wrapped_response
            }
        });
        if let Some(id) = id_opt {
            resp_obj["id"] = json!(id);
        }
        response_parts.push(json!({ "functionResponse": resp_obj }));
    }

    contents.push(json!({
        "role": "user",
        "parts": response_parts
    }));

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockAxReadOnlyExecutor;

    impl ReadOnlyToolExecutor for MockAxReadOnlyExecutor {
        fn inspect_active_window(
            &self,
        ) -> futures::future::BoxFuture<
            'static,
            std::result::Result<
                crate::tools::screen_vision::WindowMetadata,
                crate::tools::screen_vision::VisionError,
            >,
        > {
            Box::pin(async move {
                Ok(crate::tools::screen_vision::WindowMetadata {
                    hwnd: 1,
                    title: "Mock".to_string(),
                    process_name: "mock.exe".to_string(),
                    process_id: 1,
                    width: 800,
                    height: 600,
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
            Box::pin(async move {
                Err(crate::tools::screen_vision::VisionError::CaptureFailed(
                    "not used".to_string(),
                ))
            })
        }

        fn read_active_tab(&self) -> futures::future::BoxFuture<'static, Result<Option<String>>> {
            Box::pin(async move { Ok(None) })
        }

        fn read_browser_ax_tree(
            &self,
        ) -> futures::future::BoxFuture<'static, Result<Option<String>>> {
            Box::pin(async move { Ok(Some("[heading] Mock Accessibility Heading".to_string())) })
        }
    }

    #[tokio::test]
    async fn test_tier_3_amber_confirmation_workflow_real_pipeline() {
        let policy = PolicyEngine::new(&["C:\\Projects".into()]);
        let mock_os = crate::tools::os_control::MockOsController::default();
        let mut taint = TurnTaintState::new();
        let token = CancellationToken::new();

        let conf_mgr_approved = ConfirmationManager::new();
        let mut sub_approved = conf_mgr_approved.subscribe();
        let conf_mgr_approved_clone = conf_mgr_approved.clone();

        let call_approved = FunctionCall {
            id: Some("call_cmd_1".into()),
            name: "run_terminal_command".into(),
            args: json!({ "command": "cargo build" }),
            thought_signature: None,
        };

        let conf_task = tokio::spawn(async move {
            if let Ok(notice) = sub_approved.recv().await {
                if let Some(req_id) = notice["params"]["id"].as_u64() {
                    conf_mgr_approved_clone.resolve_confirmation(req_id, true).await;
                }
            }
        });

        let res_approved = dispatch_tool_call(
            &call_approved,
            &policy,
            &conf_mgr_approved,
            None,
            None,
            &mock_os,
            &mut taint,
            &token,
        )
        .await;

        let _ = conf_task.await;
        assert_eq!(res_approved["status"], "SUCCESS");

        let conf_mgr_denied = ConfirmationManager::new();
        let mut sub_denied = conf_mgr_denied.subscribe();
        let conf_mgr_denied_clone = conf_mgr_denied.clone();

        let conf_task_denied = tokio::spawn(async move {
            if let Ok(notice) = sub_denied.recv().await {
                if let Some(req_id) = notice["params"]["id"].as_u64() {
                    conf_mgr_denied_clone.resolve_confirmation(req_id, false).await;
                }
            }
        });

        let call_denied = FunctionCall {
            id: Some("call_cmd_2".into()),
            name: "run_terminal_command".into(),
            args: json!({ "command": "cargo clean" }),
            thought_signature: None,
        };

        let res_denied = dispatch_tool_call(
            &call_denied,
            &policy,
            &conf_mgr_denied,
            None,
            None,
            &mock_os,
            &mut taint,
            &token,
        )
        .await;

        let _ = conf_task_denied.await;
        assert_eq!(res_denied["status"], "DENIED");
    }

    #[tokio::test]
    async fn test_tool_dispatch_red_tier_blocks_dangerous_command() {
        let policy = PolicyEngine::new(&["C:\\Projects".into()]);
        let confirmation_mgr = ConfirmationManager::new();
        let mock_os = crate::tools::os_control::MockOsController::default();
        let mut taint = TurnTaintState::new();
        let token = CancellationToken::new();

        let call = FunctionCall {
            id: None,
            name: "run_terminal_command".into(),
            args: json!({ "command": "rm -rf /" }),
            thought_signature: None,
        };

        let result = dispatch_tool_call(
            &call,
            &policy,
            &confirmation_mgr,
            None,
            None,
            &mock_os,
            &mut taint,
            &token,
        )
        .await;

        assert_eq!(result["status"], "BLOCKED");
        assert!(result["reason"].as_str().unwrap().contains("Policy Blocked"));
    }

    #[tokio::test]
    async fn test_tool_dispatch_green_tier_runs_safely() {
        let policy = PolicyEngine::new(&["C:\\Projects".into()]);
        let confirmation_mgr = ConfirmationManager::new();
        let mock_os = crate::tools::os_control::MockOsController::default();
        let mut taint = TurnTaintState::new();
        let token = CancellationToken::new();

        let call = FunctionCall {
            id: None,
            name: "run_terminal_command".into(),
            args: json!({ "command": "git status" }),
            thought_signature: None,
        };

        let result = dispatch_tool_call(
            &call,
            &policy,
            &confirmation_mgr,
            None,
            None,
            &mock_os,
            &mut taint,
            &token,
        )
        .await;

        assert_eq!(result["status"], "SUCCESS");
        assert_eq!(result["exit_code"], 0);
        assert!(taint.is_tainted());
    }

    #[tokio::test]
    async fn test_tool_dispatch_media_control() {
        let policy = PolicyEngine::new(&["C:\\Projects".into()]);
        let confirmation_mgr = ConfirmationManager::new();
        let mock_os = crate::tools::os_control::MockOsController::default();
        let mut taint = TurnTaintState::new();
        let token = CancellationToken::new();

        let call = FunctionCall {
            id: None,
            name: "control_os_media".into(),
            args: json!({ "action": "mute" }),
            thought_signature: None,
        };

        let result = dispatch_tool_call(
            &call,
            &policy,
            &confirmation_mgr,
            None,
            None,
            &mock_os,
            &mut taint,
            &token,
        )
        .await;

        assert_eq!(result["status"], "SUCCESS");
        assert_eq!(
            *mock_os.last_action.lock().unwrap(),
            Some(crate::tools::os_control::OsMediaAction::VolumeMuteToggle)
        );
    }

    #[tokio::test]
    async fn test_tool_dispatch_remember_fact() {
        let policy = PolicyEngine::new(&["C:\\Projects".into()]);
        let confirmation_mgr = ConfirmationManager::new();
        let memory = crate::memory::MemoryEngine::in_memory().await.unwrap();
        let mock_os = crate::tools::os_control::MockOsController::default();
        let mut taint = TurnTaintState::new();
        let token = CancellationToken::new();

        let call = FunctionCall {
            id: None,
            name: "remember_fact".into(),
            args: json!({
                "key": "favorite_editor",
                "value": "neovim",
                "tier": "preference"
            }),
            thought_signature: None,
        };

        let result = dispatch_tool_call(
            &call,
            &policy,
            &confirmation_mgr,
            None,
            Some(&memory),
            &mock_os,
            &mut taint,
            &token,
        )
        .await;

        assert_eq!(result["status"], "SUCCESS");
        assert_eq!(
            memory.db().get_fact("favorite_editor").await.unwrap().unwrap(),
            "neovim"
        );
    }

    #[tokio::test]
    async fn test_tool_dispatch_taint_escalation_blocks_unconfirmed_navigation() {
        let policy = PolicyEngine::new(&["C:\\Projects".into()]);
        let confirmation_mgr = ConfirmationManager::new();
        let mock_os = crate::tools::os_control::MockOsController::default();
        let mut taint = TurnTaintState::new();
        let token = CancellationToken::new();

        taint.mark_tainted("browser_cdp:read_active_tab");

        let call = FunctionCall {
            id: None,
            name: "open_browser_url".into(),
            args: json!({ "url": "https://attacker.invalid/exfil" }),
            thought_signature: None,
        };

        let result = dispatch_tool_call(
            &call,
            &policy,
            &confirmation_mgr,
            None,
            None,
            &mock_os,
            &mut taint,
            &token,
        )
        .await;

        assert_eq!(result["status"], "DENIED");
        assert!(result["reason"].as_str().unwrap().contains("User denied"));
    }

    #[tokio::test]
    async fn test_tool_dispatch_taint_escalation_blocks_unconfirmed_launch() {
        let policy = PolicyEngine::new(&["C:\\Projects".into()]);
        let confirmation_mgr = ConfirmationManager::new();
        let mock_os = crate::tools::os_control::MockOsController::default();
        let mut taint = TurnTaintState::new();
        let token = CancellationToken::new();

        taint.mark_tainted("external_document");

        let call = FunctionCall {
            id: None,
            name: "launch_application".into(),
            args: json!({ "target": "calc" }),
            thought_signature: None,
        };

        let result = dispatch_tool_call(
            &call,
            &policy,
            &confirmation_mgr,
            None,
            None,
            &mock_os,
            &mut taint,
            &token,
        )
        .await;

        assert_eq!(result["status"], "DENIED");
    }

    #[tokio::test]
    async fn test_tool_dispatch_fixed_media_safe_under_taint() {
        let policy = PolicyEngine::new(&["C:\\Projects".into()]);
        let confirmation_mgr = ConfirmationManager::new();
        let mock_os = crate::tools::os_control::MockOsController::default();
        let mut taint = TurnTaintState::new();
        let token = CancellationToken::new();

        taint.mark_tainted("malicious_page");

        let call = FunctionCall {
            id: None,
            name: "control_os_media".into(),
            args: json!({ "action": "volume_up" }),
            thought_signature: None,
        };

        let result = dispatch_tool_call(
            &call,
            &policy,
            &confirmation_mgr,
            None,
            None,
            &mock_os,
            &mut taint,
            &token,
        )
        .await;

        assert_eq!(result["status"], "SUCCESS");
    }

    #[tokio::test]
    async fn test_tool_dispatch_blocks_invalid_launch_target() {
        let policy = PolicyEngine::new(&["C:\\Projects".into()]);
        let confirmation_mgr = ConfirmationManager::new();
        let mock_os = crate::tools::os_control::MockOsController::default();
        let mut taint = TurnTaintState::new();
        let token = CancellationToken::new();

        let call = FunctionCall {
            id: None,
            name: "launch_application".into(),
            args: json!({ "target": "cmd.exe" }),
            thought_signature: None,
        };

        let result = dispatch_tool_call(
            &call,
            &policy,
            &confirmation_mgr,
            None,
            None,
            &mock_os,
            &mut taint,
            &token,
        )
        .await;

        assert_eq!(result["status"], "BLOCKED");
        assert!(result["reason"].as_str().unwrap().contains("Launch blocked"));
    }

    #[tokio::test]
    async fn test_duplicate_tool_call_intercepted() {
        let policy = PolicyEngine::new(&["C:\\Projects".into()]);
        let confirmation_mgr = ConfirmationManager::new();
        let mock_os = crate::tools::os_control::MockOsController::default();
        let mut taint = TurnTaintState::new();
        let token = CancellationToken::new();
        let mut executed_tools = std::collections::HashSet::new();

        let call = FunctionCall {
            id: None,
            name: "control_os_media".into(),
            args: json!({ "action": "mute" }),
            thought_signature: None,
        };

        let res1 = execute_tool_with_guardrails(
            &mut executed_tools,
            &call,
            &policy,
            &confirmation_mgr,
            None,
            None,
            &mock_os,
            &mut taint,
            &token,
        )
        .await;
        assert_eq!(res1["status"], "SUCCESS");
        assert_eq!(
            *mock_os.last_action.lock().unwrap(),
            Some(crate::tools::os_control::OsMediaAction::VolumeMuteToggle)
        );

        *mock_os.last_action.lock().unwrap() = None;

        let res2 = execute_tool_with_guardrails(
            &mut executed_tools,
            &call,
            &policy,
            &confirmation_mgr,
            None,
            None,
            &mock_os,
            &mut taint,
            &token,
        )
        .await;
        assert_eq!(res2["status"], "DUPLICATE_CALL_BLOCKED");
        assert!(res2["error"].as_str().unwrap().contains("already executed"));
        assert_eq!(*mock_os.last_action.lock().unwrap(), None);
    }

    #[tokio::test]
    async fn test_turn_cancellation_aborts_tool_dispatch() {
        let policy = PolicyEngine::new(&["C:\\Projects".into()]);
        let confirmation_mgr = ConfirmationManager::new();
        let mock_os = crate::tools::os_control::MockOsController::default();
        let mut taint = TurnTaintState::new();
        let token = CancellationToken::new();
        let mut executed_tools = std::collections::HashSet::new();

        token.cancel();

        let call = FunctionCall {
            id: None,
            name: "control_os_media".into(),
            args: json!({ "action": "volume_up" }),
            thought_signature: None,
        };

        let result = execute_tool_with_guardrails(
            &mut executed_tools,
            &call,
            &policy,
            &confirmation_mgr,
            None,
            None,
            &mock_os,
            &mut taint,
            &token,
        )
        .await;

        assert_eq!(result["status"], "CANCELLED");
        assert!(result["error"].as_str().unwrap().contains("cancelled"));
        assert_eq!(*mock_os.last_action.lock().unwrap(), None);
    }

    #[tokio::test]
    async fn test_tool_dispatch_read_file_and_list_directory_enforce_sandbox() {
        let root = std::env::temp_dir().join(format!(
            "jarvis_fs_tool_{}",
            policy::generate_crypto_nonce()
        ));
        let outside = std::env::temp_dir().join(format!(
            "jarvis_fs_outside_{}",
            policy::generate_crypto_nonce()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        std::fs::write(root.join("notes.txt"), "hello sandboxed world").unwrap();
        std::fs::write(root.join(".env"), "API_KEY=supersecret").unwrap();
        std::fs::write(root.join("secret.key"), "PRIVATE").unwrap();
        std::fs::write(outside.join("leak.txt"), "outside").unwrap();

        let policy = PolicyEngine::new(&[root.to_string_lossy().into_owned()]);
        let confirmation_mgr = ConfirmationManager::new();
        let mock_os = crate::tools::os_control::MockOsController::default();
        let token = CancellationToken::new();

        // 1. Read allowed file succeeds & sets file:read taint
        let mut taint = TurnTaintState::new();
        let read_ok = dispatch_tool_call(
            &FunctionCall {
                id: None,
                name: "read_file".into(),
                args: json!({ "path": "notes.txt", "max_bytes": 12 }),
                thought_signature: None,
            },
            &policy,
            &confirmation_mgr,
            Some(&root),
            None,
            &mock_os,
            &mut taint,
            &token,
        )
        .await;
        assert_eq!(read_ok["status"], "SUCCESS");
        assert_eq!(read_ok["content"], "hello sandbo");
        assert_eq!(read_ok["truncated"], true);
        assert!(taint.taint_sources.contains(&"file:read".to_string()));

        // 2. Read .env & out-of-root file blocked fail-closed
        for blocked_path in [
            root.join(".env").to_string_lossy().into_owned(),
            outside.join("leak.txt").to_string_lossy().into_owned(),
        ] {
            let blocked = dispatch_tool_call(
                &FunctionCall {
                    id: None,
                    name: "read_file".into(),
                    args: json!({ "path": blocked_path }),
                    thought_signature: None,
                },
                &policy,
                &confirmation_mgr,
                Some(&root),
                None,
                &mock_os,
                &mut taint,
                &token,
            )
            .await;
            assert_eq!(blocked["status"], "BLOCKED", "expected block for {blocked:?}");
        }

        // 3. List allowed directory succeeds & filters out .env and .key
        let mut taint_list = TurnTaintState::new();
        let list_ok = dispatch_tool_call(
            &FunctionCall {
                id: None,
                name: "list_directory".into(),
                args: json!({ "path": "." }),
                thought_signature: None,
            },
            &policy,
            &confirmation_mgr,
            Some(&root),
            None,
            &mock_os,
            &mut taint_list,
            &token,
        )
        .await;
        assert_eq!(list_ok["status"], "SUCCESS");
        let entries_str = list_ok["entries"].to_string();
        assert!(entries_str.contains("notes.txt"));
        assert!(!entries_str.contains(".env"));
        assert!(!entries_str.contains("secret.key"));
        assert!(taint_list.taint_sources.contains(&"file:list".to_string()));

        // 4. List out-of-root directory blocked
        let list_blocked = dispatch_tool_call(
            &FunctionCall {
                id: None,
                name: "list_directory".into(),
                args: json!({ "path": outside.to_string_lossy() }),
                thought_signature: None,
            },
            &policy,
            &confirmation_mgr,
            Some(&root),
            None,
            &mock_os,
            &mut taint_list,
            &token,
        )
        .await;
        assert_eq!(list_blocked["status"], "BLOCKED");

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[tokio::test]
    async fn test_tool_dispatch_search_memory_returns_facts_and_axtree_marks_taint() {
        let policy = PolicyEngine::new(&["C:\\Projects".into()]);
        let confirmation_mgr = ConfirmationManager::new();
        let memory = crate::memory::MemoryEngine::in_memory().await.unwrap();
        let mock_os = crate::tools::os_control::MockOsController::default();
        let token = CancellationToken::new();

        memory
            .upsert_fact("coffee_order", "Double espresso oat milk", "preference", true)
            .await
            .unwrap();

        let mut taint = TurnTaintState::new();
        let search_res = dispatch_tool_call(
            &FunctionCall {
                id: None,
                name: "search_memory".into(),
                args: json!({ "query": "espresso" }),
                thought_signature: None,
            },
            &policy,
            &confirmation_mgr,
            None,
            Some(&memory),
            &mock_os,
            &mut taint,
            &token,
        )
        .await;
        assert_eq!(search_res["status"], "SUCCESS");
        assert_eq!(search_res["facts"][0]["key"], "coffee_order");

        let (ax_res, emitted) = dispatch_tool_call_with_executor(
            &FunctionCall {
                id: None,
                name: "read_browser_ax_tree".into(),
                args: json!({}),
                thought_signature: None,
            },
            &policy,
            &confirmation_mgr,
            None,
            None,
            &mock_os,
            &SandboxTerminalExecutor,
            &MockAxReadOnlyExecutor,
            &DefaultBrowserNavigator,
            &taint,
            &token,
        )
        .await;
        assert_eq!(ax_res["status"], "SUCCESS");
        assert!(ax_res["ax_tree"]
            .as_str()
            .unwrap()
            .contains("[heading] Mock Accessibility Heading"));
        assert!(emitted.contains(&"browser_cdp:ax_tree"));
    }
}
