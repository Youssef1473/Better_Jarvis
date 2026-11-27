use crate::agent::model_router::{ModelSelection, ModelTier};
use crate::agent::sentence_chunker::SentenceChunker;
use crate::agent::sse_parser::{FunctionCall, ModelToolCall, ParsedModelResponse, TtsChunk};
use crate::agent::tool_schemas::{build_system_instruction_text, get_ollama_tool_declarations};
use crate::agent::tool_scheduler::{
    execute_tool_batch_outputs, BrowserNavigator, ReadOnlyToolExecutor, TerminalExecutor,
};
use crate::config::{validate_loopback_base_url, LocalFallbackConfig};
use crate::security::policy::{PolicyEngine, TurnTaintState};
use crate::tools::confirmation::ConfirmationManager;
use anyhow::{Context, Result};
use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde_json::{json, Value};
use std::path::Path;
use tokio_util::sync::CancellationToken;
use tracing::info;

const MAX_OLLAMA_TOOL_TURNS: usize = 4;

/// Stateful `<think>...</think>` stripper so local Qwen reasoning traces never leak into TTS audio.
#[derive(Debug, Default)]
pub(crate) struct ThinkTagStripper {
    in_think: bool,
    pending: String,
}

impl ThinkTagStripper {
    pub(crate) fn push(&mut self, delta: &str) -> String {
        self.pending.push_str(delta);
        let mut out = String::new();

        loop {
            if self.in_think {
                if let Some(end_idx) = self.pending.find("</think>") {
                    self.pending.drain(..end_idx + "</think>".len());
                    self.in_think = false;
                } else {
                    let keep = Self::trailing_partial_tag_len(&self.pending, "</think>");
                    let drain_len = self.pending.len().saturating_sub(keep);
                    if drain_len > 0 {
                        // Advance only at valid UTF-8 boundary
                        let safe_drain = Self::floor_char_boundary(&self.pending, drain_len);
                        self.pending.drain(..safe_drain);
                    }
                    break;
                }
            } else if let Some(start_idx) = self.pending.find("<think>") {
                out.push_str(&self.pending[..start_idx]);
                self.pending.drain(..start_idx + "<think>".len());
                self.in_think = true;
            } else {
                let keep = Self::trailing_partial_tag_len(&self.pending, "<think>");
                let emit_len = self.pending.len().saturating_sub(keep);
                if emit_len > 0 {
                    let safe_emit = Self::floor_char_boundary(&self.pending, emit_len);
                    out.push_str(&self.pending[..safe_emit]);
                    self.pending.drain(..safe_emit);
                }
                break;
            }
        }

        out
    }

    pub(crate) fn flush(&mut self) -> String {
        if self.in_think {
            self.pending.clear();
            return String::new();
        }
        std::mem::take(&mut self.pending)
    }

    fn floor_char_boundary(s: &str, max_idx: usize) -> usize {
        let mut idx = max_idx.min(s.len());
        while idx > 0 && !s.is_char_boundary(idx) {
            idx -= 1;
        }
        idx
    }

    fn trailing_partial_tag_len(buf: &str, tag: &str) -> usize {
        let max_check = (tag.len() - 1).min(buf.len());
        for len in (1..=max_check).rev() {
            let start = buf.len() - len;
            if buf.is_char_boundary(start) && tag.starts_with(&buf[start..]) {
                return len;
            }
        }
        0
    }
}

pub(crate) fn build_ollama_chat_payload(
    model: &str,
    messages: &[Value],
    max_output_tokens: u32,
) -> Value {
    json!({
        "model": model,
        "messages": messages,
        "tools": get_ollama_tool_declarations(),
        "stream": true,
        "think": false,
        "options": {
            "temperature": 0.2,
            "num_predict": max_output_tokens
        }
    })
}

fn parse_tool_arguments(raw_args: &Value) -> Value {
    match raw_args {
        Value::String(s) => serde_json::from_str::<Value>(s).unwrap_or(Value::Null),
        other => other.clone(),
    }
}

fn extract_ollama_tool_calls(
    tool_calls_val: &[Value],
    parsed: &mut ParsedModelResponse,
    raw_tool_calls_out: &mut Vec<Value>,
) {
    for tc in tool_calls_val {
        let func = tc.get("function").unwrap_or(&Value::Null);
        let name = func
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .trim()
            .to_string();
        if name.is_empty() {
            continue;
        }
        let args = parse_tool_arguments(func.get("arguments").unwrap_or(&Value::Null));
        let id = tc
            .get("id")
            .and_then(|v| v.as_str())
            .map(ToString::to_string);

        let part_index = parsed.model_parts.len();
        parsed.model_parts.push(tc.clone());
        raw_tool_calls_out.push(tc.clone());
        parsed.function_calls.push(ModelToolCall {
            part_index,
            call: FunctionCall {
                id,
                name,
                args,
                thought_signature: None,
            },
            raw_part: tc.clone(),
        });
    }
}

fn process_ndjson_line(
    line: &str,
    parsed: &mut ParsedModelResponse,
    raw_tool_calls_out: &mut Vec<Value>,
    stripper: &mut ThinkTagStripper,
    chunker: &mut SentenceChunker,
    first_token_recorded: &mut bool,
    on_first_token: &mut impl FnMut(),
) -> Result<(bool, Vec<String>)> {
    let val: Value = serde_json::from_str(line)
        .map_err(|e| anyhow::anyhow!("Malformed JSON in Ollama NDJSON stream: {e}"))?;

    if let Some(err_msg) = val.get("error").and_then(|v| v.as_str()) {
        anyhow::bail!("Ollama API stream error: {}", err_msg);
    }

    let mut pending_clauses = Vec::new();
    if let Some(msg) = val.get("message") {
        if let Some(content_delta) = msg.get("content").and_then(|v| v.as_str()) {
            let visible = stripper.push(content_delta);
            if !visible.is_empty() {
                if !*first_token_recorded {
                    on_first_token();
                    *first_token_recorded = true;
                }
                parsed.text.push_str(&visible);
                if parsed.function_calls.is_empty() {
                    pending_clauses.extend(chunker.push_delta(&visible));
                }
            }
        }
        if let Some(tc_arr) = msg.get("tool_calls").and_then(|v| v.as_array()) {
            extract_ollama_tool_calls(tc_arr, parsed, raw_tool_calls_out);
        }
    }

    let is_done = val.get("done").and_then(|v| v.as_bool()).unwrap_or(false);
    Ok((is_done, pending_clauses))
}

async fn emit_clauses(
    clauses: Vec<String>,
    parsed_has_tools: bool,
    turn_id: u32,
    seq: &mut u32,
    text_chunks_emitted: &mut usize,
    chunk_tx: Option<&tokio::sync::mpsc::Sender<TtsChunk>>,
) {
    if parsed_has_tools {
        return;
    }
    if let Some(tx) = chunk_tx {
        for clause in clauses {
            let _ = tx
                .send(TtsChunk {
                    turn_id,
                    seq: *seq,
                    text: clause,
                    is_final: false,
                })
                .await;
            *seq += 1;
            *text_chunks_emitted += 1;
        }
    }
}

/// Parses an Ollama `/api/chat` NDJSON byte stream incrementally, stripping `<think>...</think>`
/// tags and feeding clean text into `SentenceChunker`. Fails closed if stream stops before `done: true`.
pub(crate) async fn parse_ollama_ndjson_stream<S, E>(
    mut stream: S,
    turn_id: u32,
    turn_token: &CancellationToken,
    chunk_tx: Option<&tokio::sync::mpsc::Sender<TtsChunk>>,
    mut on_first_token: impl FnMut(),
) -> Result<(ParsedModelResponse, Vec<Value>)>
where
    S: futures::Stream<Item = std::result::Result<bytes::Bytes, E>> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    let mut chunker = SentenceChunker::new();
    let mut stripper = ThinkTagStripper::default();
    let mut seq = 0u32;
    let mut line_buf = String::new();
    let mut parsed = ParsedModelResponse::default();
    let mut raw_tool_calls = Vec::new();
    let mut saw_done = false;
    let mut first_token_recorded = false;
    let mut text_chunks_emitted = 0usize;

    if turn_token.is_cancelled() {
        anyhow::bail!("Turn cancelled by barge-in or shutdown");
    }

    while let Some(item_res) = tokio::select! {
        biased;
        _ = turn_token.cancelled() => {
            anyhow::bail!("Turn cancelled by barge-in or shutdown");
        }
        res = stream.next() => res,
    } {
        let bytes =
            item_res.map_err(|e| anyhow::anyhow!("Error reading Ollama response stream: {e}"))?;
        line_buf.push_str(&String::from_utf8_lossy(&bytes));

        while let Some(newline_pos) = line_buf.find('\n') {
            let line = line_buf[..newline_pos].trim().to_string();
            line_buf = line_buf[newline_pos + 1..].to_string();
            if line.is_empty() {
                continue;
            }

            let (is_done, clauses) = process_ndjson_line(
                &line,
                &mut parsed,
                &mut raw_tool_calls,
                &mut stripper,
                &mut chunker,
                &mut first_token_recorded,
                &mut on_first_token,
            )?;
            if is_done {
                saw_done = true;
            }
            emit_clauses(
                clauses,
                !parsed.function_calls.is_empty(),
                turn_id,
                &mut seq,
                &mut text_chunks_emitted,
                chunk_tx,
            )
            .await;
        }
    }

    let trailing = line_buf.trim().to_string();
    if !trailing.is_empty() {
        let (is_done, clauses) = process_ndjson_line(
            &trailing,
            &mut parsed,
            &mut raw_tool_calls,
            &mut stripper,
            &mut chunker,
            &mut first_token_recorded,
            &mut on_first_token,
        )?;
        if is_done {
            saw_done = true;
        }
        emit_clauses(
            clauses,
            !parsed.function_calls.is_empty(),
            turn_id,
            &mut seq,
            &mut text_chunks_emitted,
            chunk_tx,
        )
        .await;
    }

    if !saw_done {
        anyhow::bail!("Ollama stream ended before terminal done=true frame");
    }

    let flushed_text = stripper.flush();
    if !flushed_text.is_empty() {
        if !first_token_recorded {
            on_first_token();
        }
        parsed.text.push_str(&flushed_text);
        if parsed.function_calls.is_empty() {
            for clause in chunker.push_delta(&flushed_text) {
                if let Some(tx) = chunk_tx {
                    let _ = tx
                        .send(TtsChunk {
                            turn_id,
                            seq,
                            text: clause,
                            is_final: false,
                        })
                        .await;
                    seq += 1;
                    text_chunks_emitted += 1;
                }
            }
        }
    }

    if parsed.function_calls.is_empty() {
        if let Some(final_clause) = chunker.flush() {
            if let Some(tx) = chunk_tx {
                let _ = tx
                    .send(TtsChunk {
                        turn_id,
                        seq,
                        text: final_clause,
                        is_final: true,
                    })
                    .await;
            }
        } else if text_chunks_emitted > 0 {
            if let Some(tx) = chunk_tx {
                let _ = tx
                    .send(TtsChunk {
                        turn_id,
                        seq,
                        text: String::new(),
                        is_final: true,
                    })
                    .await;
            }
        }
    } else if text_chunks_emitted > 0 {
        if let Some(tx) = chunk_tx {
            let _ = tx
                .send(TtsChunk {
                    turn_id,
                    seq,
                    text: String::new(),
                    is_final: true,
                })
                .await;
        }
    }

    Ok((parsed, raw_tool_calls))
}

pub(crate) struct OllamaTurnContext<'a> {
    pub local_cfg: &'a LocalFallbackConfig,
    pub effective_prompt: &'a str,
    pub turn_id: u32,
    pub turn_nonce: &'a str,
    pub memory_recall_ms: Option<f64>,
    pub fallback_reason: &'static str,
    pub policy: &'a PolicyEngine,
    pub confirmation_mgr: &'a ConfirmationManager,
    pub workspace_dir: Option<&'a Path>,
    pub memory: Option<&'a crate::memory::MemoryEngine>,
    pub os_controller: &'a dyn crate::tools::os_control::OsController,
    pub terminal_executor: &'a dyn TerminalExecutor,
    pub readonly_executor: &'a dyn ReadOnlyToolExecutor,
    pub browser_navigator: &'a dyn BrowserNavigator,
    pub transport: &'a dyn crate::agent::gemini_client::LlmTransport,
    pub taint: &'a mut TurnTaintState,
    pub turn_token: &'a CancellationToken,
    pub chunk_tx: Option<tokio::sync::mpsc::Sender<TtsChunk>>,
}

/// Runs the local Ollama (`/api/chat`) ReAct tool-calling loop reusing the full sandbox & taint scheduler.
pub(crate) async fn chat_with_ollama_streaming(
    ctx: OllamaTurnContext<'_>,
    mut on_ttft: impl FnMut() + Send,
) -> Result<(String, ModelSelection, Option<f64>, TurnTaintState)> {
    let base_url = validate_loopback_base_url(&ctx.local_cfg.base_url)?;
    let endpoint = format!("{}/api/chat", base_url);

    let sys_prompt = build_system_instruction_text(Some(ctx.turn_nonce));
    let mut messages = vec![
        json!({ "role": "system", "content": sys_prompt }),
        json!({ "role": "user", "content": ctx.effective_prompt }),
    ];

    let mut executed_tools_in_turn = std::collections::HashSet::<(String, String)>::new();
    let model_sel = ModelSelection {
        tier: ModelTier::LocalFallback,
        model_name: ctx.local_cfg.model.clone(),
        reason: ctx.fallback_reason,
    };

    for turn in 1..=MAX_OLLAMA_TOOL_TURNS {
        if ctx.turn_token.is_cancelled() {
            anyhow::bail!("Turn cancelled by barge-in or shutdown");
        }

        info!(
            "Ollama Local Fallback Turn {}/{} (Model: {}, Endpoint: {})",
            turn, MAX_OLLAMA_TOOL_TURNS, ctx.local_cfg.model, endpoint
        );

        let payload = build_ollama_chat_payload(
            &ctx.local_cfg.model,
            &messages,
            ctx.local_cfg.max_output_tokens,
        );
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let response = tokio::select! {
            _ = ctx.turn_token.cancelled() => {
                anyhow::bail!("Turn cancelled by barge-in or shutdown");
            }
            res = ctx.transport.post_json(endpoint.clone(), headers, payload) => {
                res.with_context(|| format!("Failed to reach local Ollama at {}", endpoint))?
            }
        };

        if !response.status.is_success() {
            let err_text = response
                .error_body
                .unwrap_or_else(|| "Unknown HTTP error".to_string());
            anyhow::bail!("Ollama API returned HTTP {}: {}", response.status, err_text);
        }

        let (parsed, raw_tool_calls) = parse_ollama_ndjson_stream(
            response.body_stream,
            ctx.turn_id,
            ctx.turn_token,
            ctx.chunk_tx.as_ref(),
            &mut on_ttft,
        )
        .await?;

        if !parsed.function_calls.is_empty() {
            messages.push(json!({
                "role": "assistant",
                "content": parsed.text,
                "tool_calls": raw_tool_calls
            }));

            let wrapped_outputs = execute_tool_batch_outputs(
                &parsed,
                &mut executed_tools_in_turn,
                ctx.policy,
                ctx.confirmation_mgr,
                ctx.workspace_dir,
                ctx.memory,
                ctx.os_controller,
                ctx.terminal_executor,
                ctx.readonly_executor,
                ctx.browser_navigator,
                ctx.taint,
                ctx.turn_nonce,
                ctx.turn_token,
            )
            .await?;

            for (name, _id, wrapped_response) in wrapped_outputs {
                messages.push(json!({
                    "role": "tool",
                    "tool_name": name,
                    "content": wrapped_response
                }));
            }
        } else {
            return Ok((
                parsed.text,
                model_sel,
                ctx.memory_recall_ms,
                ctx.taint.clone(),
            ));
        }
    }

    anyhow::bail!(
        "Exceeded maximum Ollama ReAct tool turns ({})",
        MAX_OLLAMA_TOOL_TURNS
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_think_tag_stripper_handles_fragmented_chunks() {
        let mut s = ThinkTagStripper::default();
        let mut out = String::new();
        for piece in ["<thi", "nk>internal ", "step 1</th", "ink>Hello ", "world."] {
            out.push_str(&s.push(piece));
        }
        out.push_str(&s.flush());
        assert_eq!(out, "Hello world.");
    }

    #[test]
    fn test_think_tag_stripper_discards_unclosed_reasoning_at_eof() {
        let mut s = ThinkTagStripper::default();
        let p1 = s.push("Answer: <think>dangling");
        let p2 = s.flush();
        assert_eq!(format!("{}{}", p1, p2), "Answer: ");
    }

    #[tokio::test]
    async fn test_ollama_ndjson_rejects_incomplete_stream_before_tool_dispatch() {
        let token = CancellationToken::new();
        let line = b"{\"message\":{\"role\":\"assistant\",\"content\":\"\",\"tool_calls\":[{\"function\":{\"name\":\"read_file\",\"arguments\":{\"path\":\"a.txt\"}}}]},\"done\":false}\n";
        let stream = futures::stream::iter(vec![Ok::<bytes::Bytes, std::io::Error>(
            bytes::Bytes::from_static(line),
        )]);

        let res = parse_ollama_ndjson_stream(stream, 1, &token, None, || {}).await;
        assert!(res.is_err());
        assert!(res
            .unwrap_err()
            .to_string()
            .contains("before terminal done=true"));
    }

    #[tokio::test]
    async fn test_ollama_ndjson_parses_tool_calls_and_strips_think_tags() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<TtsChunk>(10);
        let token = CancellationToken::new();

        let chunks = vec![
            Ok::<bytes::Bytes, std::io::Error>(bytes::Bytes::from_static(
                b"{\"message\":{\"role\":\"assistant\",\"content\":\"<think>quiet\"},\"done\":false}\n",
            )),
            Ok::<bytes::Bytes, std::io::Error>(bytes::Bytes::from_static(
                b"{\"message\":{\"role\":\"assistant\",\"content\":\"</think>Done checking files.\"},\"done\":false}\n",
            )),
            Ok::<bytes::Bytes, std::io::Error>(bytes::Bytes::from_static(
                b"{\"message\":{\"role\":\"assistant\",\"content\":\"\"},\"done\":true}\n",
            )),
        ];

        let stream = futures::stream::iter(chunks);
        let (parsed, _raw_tc) =
            parse_ollama_ndjson_stream(stream, 9, &token, Some(&tx), || {})
                .await
                .unwrap();

        assert_eq!(parsed.text, "Done checking files.");
        let c0 = rx.recv().await.unwrap();
        assert_eq!(c0.text, "Done checking files.");
        let c1 = rx.recv().await.unwrap();
        assert!(c1.is_final);
    }
}
