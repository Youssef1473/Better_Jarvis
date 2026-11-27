use crate::agent::gemini_client::{
    classify_http_failure, is_fallback_eligible_gemini_error, GeminiClient, GeminiRequestError,
    LlmTransport, ProviderFallbackTransport, MAX_TOOL_TURNS,
};
use crate::agent::model_router::{ModelSelection, ModelTier};
use crate::agent::ollama_client::{chat_with_ollama_streaming, OllamaTurnContext};
use crate::agent::sse_parser::{
    parse_sse_bytes_stream_full, push_candidate_parts, validate_response_envelope,
    ParsedModelResponse, TtsChunk,
};
use crate::agent::tool_schemas::build_payload_with_tokens;
use crate::agent::tool_scheduler::{
    execute_parallel_tool_calls, BrowserNavigator, DefaultBrowserNavigator,
    DefaultReadOnlyToolExecutor, ReadOnlyToolExecutor, SandboxTerminalExecutor, TerminalExecutor,
};
use crate::security::policy::{self, PolicyEngine, TurnTaintState};
use crate::tools::confirmation::ConfirmationManager;
use anyhow::{Context, Result};
use futures::StreamExt;
use reqwest::header::HeaderMap;
use serde_json::{json, Value};
use std::path::Path;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

struct PrepareTurnArgs<'a> {
    prompt: &'a str,
    contents: &'a [Value],
    taint: &'a TurnTaintState,
    turn: usize,
    turn_nonce: &'a str,
    model_override: Option<&'a str>,
    streaming: bool,
}

impl GeminiClient {
    async fn prepare_turn_request(
        &self,
        args: PrepareTurnArgs<'_>,
    ) -> Result<(ModelSelection, String, HeaderMap, Value)> {
        let mut model_sel = self
            .router()
            .select_tier(args.prompt, args.taint.is_tainted(), args.turn - 1);
        let requested_model = args
            .model_override
            .unwrap_or(&model_sel.model_name)
            .to_string();

        let (endpoint, headers) = self
            .prepare_request_for_model(&requested_model, args.streaming)
            .await?;
        if requested_model != self.fallback_model && endpoint.contains(&self.fallback_model) {
            model_sel.model_name = self.fallback_model.clone();
            model_sel.tier = ModelTier::Fallback;
        }

        let max_tokens = self.max_output_tokens_for_model(&model_sel.model_name);
        let payload = build_payload_with_tokens(
            args.contents,
            true,
            Some(args.turn_nonce),
            Some(max_tokens),
        );

        Ok((model_sel, endpoint, headers, payload))
    }

    /// Executes a bounded (max 4 turns) function-calling chat session.
    #[allow(clippy::too_many_arguments)]
    pub async fn chat_with_tools(
        &self,
        prompt: &str,
        model_override: Option<&str>,
        policy: &PolicyEngine,
        confirmation_mgr: &ConfirmationManager,
        workspace_dir: Option<&Path>,
        memory: Option<&crate::memory::MemoryEngine>,
        os_controller: &dyn crate::tools::os_control::OsController,
        turn_token: &CancellationToken,
    ) -> Result<String> {
        let transport = crate::agent::gemini_client::ReqwestLlmTransport {
            http: self.http.clone(),
        };
        let (reply, _) = self
            .chat_with_tools_with_executor(
                prompt,
                model_override,
                policy,
                confirmation_mgr,
                workspace_dir,
                memory,
                os_controller,
                &SandboxTerminalExecutor,
                &transport,
                turn_token,
            )
            .await?;
        Ok(reply)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn chat_with_tools_with_executor(
        &self,
        prompt: &str,
        model_override: Option<&str>,
        policy: &PolicyEngine,
        confirmation_mgr: &ConfirmationManager,
        workspace_dir: Option<&Path>,
        memory: Option<&crate::memory::MemoryEngine>,
        os_controller: &dyn crate::tools::os_control::OsController,
        terminal_executor: &dyn TerminalExecutor,
        transport: &dyn LlmTransport,
        turn_token: &CancellationToken,
    ) -> Result<(String, TurnTaintState)> {
        let turn_nonce = policy::generate_crypto_nonce();
        let mut taint = TurnTaintState::new();

        let effective_prompt = if let Some(mem) = memory {
            let context = mem
                .retrieve_context_for_turn(prompt, 2048, &mut taint, &turn_nonce)
                .await;
            if !context.is_empty() {
                format!("{}\n\nUser Request: {}", context, prompt)
            } else {
                prompt.to_string()
            }
        } else {
            prompt.to_string()
        };

        let mut contents = vec![json!({
            "role": "user",
            "parts": [{ "text": effective_prompt }]
        })];

        let mut executed_tools_in_turn = std::collections::HashSet::<(String, String)>::new();

        for turn in 1..=MAX_TOOL_TURNS {
            if turn_token.is_cancelled() {
                anyhow::bail!("Turn cancelled by barge-in or shutdown");
            }

            let (model_sel, endpoint, headers, payload) = self
                .prepare_turn_request(PrepareTurnArgs {
                    prompt,
                    contents: &contents,
                    taint: &taint,
                    turn,
                    turn_nonce: &turn_nonce,
                    model_override,
                    streaming: false,
                })
                .await?;

            info!(
                "Gemini API Turn {}/{} (Model: {})",
                turn, MAX_TOOL_TURNS, model_sel.model_name
            );

            let mut response = tokio::select! {
                _ = turn_token.cancelled() => {
                    anyhow::bail!("Turn cancelled by barge-in or shutdown");
                }
                res = transport.post_json(endpoint, headers, payload) => {
                    res?
                }
            };

            if !response.status.is_success() {
                let err_text = response
                    .error_body
                    .unwrap_or_else(|| "Unknown HTTP error".to_string());
                anyhow::bail!("Gemini API returned HTTP {}: {}", response.status, err_text);
            }

            let mut body_bytes = Vec::new();
            while let Some(chunk_res) = response.body_stream.next().await {
                let chunk = chunk_res.context("Failed to read Gemini API response body")?;
                body_bytes.extend_from_slice(&chunk);
            }

            let resp_json: Value = serde_json::from_slice(&body_bytes)
                .context("Failed to parse Gemini API JSON response")?;

            let (parts, saw_finish_stop) = validate_response_envelope(&resp_json)?
                .ok_or_else(|| {
                    anyhow::anyhow!("Gemini API returned missing candidates: {:?}", resp_json)
                })?;

            if parts.is_empty() {
                anyhow::bail!("Gemini API returned empty candidate parts: {:?}", resp_json);
            }

            let mut parsed = ParsedModelResponse::default();
            let mut seen_first_fc = false;
            push_candidate_parts(&parts, &mut parsed, &mut seen_first_fc, |_, _| {});

            if !parsed.function_calls.is_empty() {
                if !saw_finish_stop {
                    anyhow::bail!(
                        "Gemini API response ended before terminal finishReason STOP for tool call"
                    );
                }
                execute_parallel_tool_calls(
                    &parsed,
                    &mut contents,
                    &mut executed_tools_in_turn,
                    policy,
                    confirmation_mgr,
                    workspace_dir,
                    memory,
                    os_controller,
                    terminal_executor,
                    &DefaultReadOnlyToolExecutor,
                    &DefaultBrowserNavigator,
                    &mut taint,
                    &turn_nonce,
                    turn_token,
                )
                .await?;
            } else {
                return Ok((parsed.text.trim().to_string(), taint));
            }
        }

        warn!(
            "Gemini tool-calling loop reached max turns ({})",
            MAX_TOOL_TURNS
        );
        Ok((
            "Operation completed (reached maximum tool iterations).".to_string(),
            taint,
        ))
    }

    /// Executes a bounded function-calling chat session with Gemini SSE streaming.
    #[allow(clippy::too_many_arguments)]
    pub async fn chat_with_tools_streaming(
        &self,
        prompt: &str,
        turn_id: u32,
        model_override: Option<&str>,
        policy: &PolicyEngine,
        confirmation_mgr: &ConfirmationManager,
        workspace_dir: Option<&Path>,
        memory: Option<&crate::memory::MemoryEngine>,
        os_controller: &dyn crate::tools::os_control::OsController,
        turn_token: &CancellationToken,
        chunk_tx: Option<tokio::sync::mpsc::Sender<TtsChunk>>,
        on_ttft: impl FnMut() + Send,
    ) -> Result<(String, ModelSelection, Option<f64>)> {
        let local_http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(
                self.local_fallback.timeout_secs.max(5),
            ))
            .build()
            .unwrap_or_else(|_| self.http.clone());
        let transport = ProviderFallbackTransport {
            cloud_http: self.http.clone(),
            local_http,
        };
        let (reply, model_sel, memory_recall_ms, _) = self
            .chat_with_tools_streaming_with_executor(
                prompt,
                turn_id,
                model_override,
                policy,
                confirmation_mgr,
                workspace_dir,
                memory,
                os_controller,
                &SandboxTerminalExecutor,
                &transport,
                turn_token,
                chunk_tx,
                on_ttft,
            )
            .await?;
        Ok((reply, model_sel, memory_recall_ms))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn chat_with_tools_streaming_with_executor(
        &self,
        prompt: &str,
        turn_id: u32,
        model_override: Option<&str>,
        policy: &PolicyEngine,
        confirmation_mgr: &ConfirmationManager,
        workspace_dir: Option<&Path>,
        memory: Option<&crate::memory::MemoryEngine>,
        os_controller: &dyn crate::tools::os_control::OsController,
        terminal_executor: &dyn TerminalExecutor,
        transport: &dyn LlmTransport,
        turn_token: &CancellationToken,
        chunk_tx: Option<tokio::sync::mpsc::Sender<TtsChunk>>,
        on_ttft: impl FnMut() + Send,
    ) -> Result<(String, ModelSelection, Option<f64>, TurnTaintState)> {
        self.chat_with_tools_streaming_with_options(
            prompt,
            turn_id,
            model_override,
            policy,
            confirmation_mgr,
            workspace_dir,
            memory,
            os_controller,
            terminal_executor,
            &DefaultReadOnlyToolExecutor,
            transport,
            None,
            turn_token,
            chunk_tx,
            on_ttft,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn chat_with_tools_streaming_with_options(
        &self,
        prompt: &str,
        turn_id: u32,
        model_override: Option<&str>,
        policy: &PolicyEngine,
        confirmation_mgr: &ConfirmationManager,
        workspace_dir: Option<&Path>,
        memory: Option<&crate::memory::MemoryEngine>,
        os_controller: &dyn crate::tools::os_control::OsController,
        terminal_executor: &dyn TerminalExecutor,
        readonly_executor: &dyn ReadOnlyToolExecutor,
        transport: &dyn LlmTransport,
        initial_taint: Option<TurnTaintState>,
        turn_token: &CancellationToken,
        chunk_tx: Option<tokio::sync::mpsc::Sender<TtsChunk>>,
        on_ttft: impl FnMut() + Send,
    ) -> Result<(String, ModelSelection, Option<f64>, TurnTaintState)> {
        self.chat_with_tools_streaming_with_all_executors(
            prompt,
            turn_id,
            model_override,
            policy,
            confirmation_mgr,
            workspace_dir,
            memory,
            os_controller,
            terminal_executor,
            readonly_executor,
            &DefaultBrowserNavigator,
            transport,
            initial_taint,
            turn_token,
            chunk_tx,
            on_ttft,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn chat_with_tools_streaming_with_all_executors(
        &self,
        prompt: &str,
        turn_id: u32,
        model_override: Option<&str>,
        policy: &PolicyEngine,
        confirmation_mgr: &ConfirmationManager,
        workspace_dir: Option<&Path>,
        memory: Option<&crate::memory::MemoryEngine>,
        os_controller: &dyn crate::tools::os_control::OsController,
        terminal_executor: &dyn TerminalExecutor,
        readonly_executor: &dyn ReadOnlyToolExecutor,
        browser_navigator: &dyn BrowserNavigator,
        transport: &dyn LlmTransport,
        initial_taint: Option<TurnTaintState>,
        turn_token: &CancellationToken,
        chunk_tx: Option<tokio::sync::mpsc::Sender<TtsChunk>>,
        mut on_ttft: impl FnMut() + Send,
    ) -> Result<(String, ModelSelection, Option<f64>, TurnTaintState)> {
        let turn_nonce = policy::generate_crypto_nonce();
        let mut taint = initial_taint.unwrap_or_default();

        let t_mem_start = std::time::Instant::now();
        let (effective_prompt, memory_recall_ms) = if let Some(mem) = memory {
            let context = mem
                .retrieve_context_for_turn(prompt, 2048, &mut taint, &turn_nonce)
                .await;
            let elapsed_ms = t_mem_start.elapsed().as_secs_f64() * 1000.0;
            if !context.is_empty() {
                (
                    format!("{}\n\nUser Request: {}", context, prompt),
                    Some(elapsed_ms),
                )
            } else {
                (prompt.to_string(), Some(elapsed_ms))
            }
        } else {
            (prompt.to_string(), None)
        };

        let mut contents = vec![json!({
            "role": "user",
            "parts": [{ "text": effective_prompt }]
        })];

        let mut executed_tools_in_turn = std::collections::HashSet::<(String, String)>::new();

        let fallback_enabled = self.local_fallback.enabled
            && self.local_fallback.provider.trim().eq_ignore_ascii_case("ollama");

        macro_rules! run_ollama {
            ($reason:expr) => {
                chat_with_ollama_streaming(
                    OllamaTurnContext {
                        local_cfg: &self.local_fallback,
                        effective_prompt: &effective_prompt,
                        turn_id,
                        turn_nonce: &turn_nonce,
                        memory_recall_ms,
                        fallback_reason: $reason,
                        policy,
                        confirmation_mgr,
                        workspace_dir,
                        memory,
                        os_controller,
                        terminal_executor,
                        readonly_executor,
                        browser_navigator,
                        transport,
                        taint: &mut taint,
                        turn_token,
                        chunk_tx,
                    },
                    on_ttft,
                )
                .await
            };
        }

        if self.provider.trim().eq_ignore_ascii_case("ollama") {
            if !fallback_enabled {
                anyhow::bail!(
                    "Provider 'ollama' configured but [ai.local_fallback] is disabled or unsupported"
                );
            }
            return run_ollama!("configured_local_provider");
        }

        for turn in 1..=MAX_TOOL_TURNS {
            if turn_token.is_cancelled() {
                anyhow::bail!("Turn cancelled by barge-in or shutdown");
            }

            let turn_res: Result<(ModelSelection, crate::agent::gemini_client::LlmHttpResponse)> =
                async {
                    let (model_sel, endpoint, headers, payload) = self
                        .prepare_turn_request(PrepareTurnArgs {
                            prompt,
                            contents: &contents,
                            taint: &taint,
                            turn,
                            turn_nonce: &turn_nonce,
                            model_override,
                            streaming: true,
                        })
                        .await?;

                    info!(
                        "Gemini Streaming API Turn {}/{} (Model: {})",
                        turn, MAX_TOOL_TURNS, model_sel.model_name
                    );

                    let response = tokio::select! {
                        _ = turn_token.cancelled() => {
                            anyhow::bail!("Turn cancelled by barge-in or shutdown");
                        }
                        res = transport.post_json(endpoint, headers, payload) => {
                            res.map_err(|e| anyhow::Error::new(GeminiRequestError::TransportUnavailable(e.to_string())))?
                        }
                    };

                    if !response.status.is_success() {
                        let err_text = response
                            .error_body
                            .unwrap_or_else(|| "Unknown HTTP error".to_string());
                        return Err(classify_http_failure(response.status, err_text));
                    }

                    Ok((model_sel, response))
                }
                .await;

            let (model_sel, response) = match turn_res {
                Ok(pair) => pair,
                Err(err) if turn == 1 && fallback_enabled && is_fallback_eligible_gemini_error(&err) => {
                    warn!(
                        "Gemini unavailable or quota-limited on turn 1 ({}); falling back to Ollama '{}'",
                        err, self.local_fallback.model
                    );
                    return run_ollama!("gemini_quota_or_unavailable_fallback_ollama");
                }
                Err(err) => return Err(err),
            };

            let parsed = parse_sse_bytes_stream_full(
                response.body_stream,
                turn_id,
                turn_token,
                chunk_tx.as_ref(),
                &mut on_ttft,
            )
            .await?;

            if !parsed.function_calls.is_empty() {
                execute_parallel_tool_calls(
                    &parsed,
                    &mut contents,
                    &mut executed_tools_in_turn,
                    policy,
                    confirmation_mgr,
                    workspace_dir,
                    memory,
                    os_controller,
                    terminal_executor,
                    readonly_executor,
                    browser_navigator,
                    &mut taint,
                    &turn_nonce,
                    turn_token,
                )
                .await?;
            } else {
                return Ok((parsed.text, model_sel, memory_recall_ms, taint));
            }
        }

        anyhow::bail!("Exceeded maximum ReAct tool turns ({})", MAX_TOOL_TURNS);
    }
}

#[cfg(test)]
#[path = "react_loop_tests.rs"]
mod tests;
