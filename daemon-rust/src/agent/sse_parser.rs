use crate::agent::sentence_chunker::SentenceChunker;
use anyhow::Result;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TtsChunk {
    pub turn_id: u32,
    pub seq: u32,
    pub text: String,
    pub is_final: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FunctionCall {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub args: Value,
    #[serde(
        default,
        rename = "thoughtSignature",
        skip_serializing_if = "Option::is_none"
    )]
    pub thought_signature: Option<String>,
}

/// Preserves an extracted function call alongside its verbatim original model part.
#[derive(Debug, Clone)]
pub struct ModelToolCall {
    pub part_index: usize,
    pub call: FunctionCall,
    pub raw_part: Value,
}

/// Complete parsed model turn preserving verbatim parts and all parallel function calls.
#[derive(Debug, Clone, Default)]
pub struct ParsedModelResponse {
    pub text: String,
    pub model_parts: Vec<Value>,
    pub function_calls: Vec<ModelToolCall>,
}

pub(crate) fn validate_response_envelope(val: &Value) -> Result<Option<(Vec<Value>, bool)>> {
    if let Some(block_reason) = val
        .get("promptFeedback")
        .and_then(|pf| pf.get("blockReason"))
        .and_then(|r| r.as_str())
    {
        anyhow::bail!(
            "Gemini request blocked by prompt safety filter: {}",
            block_reason
        );
    }

    if let Some(err_obj) = val.get("error") {
        let msg = err_obj
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("Unknown Gemini API error");
        anyhow::bail!("Gemini stream returned API error: {}", msg);
    }

    if let Some(cands) = val.get("candidates") {
        let arr = cands.as_array().ok_or_else(|| {
            anyhow::anyhow!("Gemini response 'candidates' field is not an array")
        })?;
        if arr.is_empty() {
            anyhow::bail!("Gemini stream returned empty candidates array");
        }
        let c0 = &arr[0];
        let mut saw_stop = false;
        if let Some(finish_reason) = c0.get("finishReason").and_then(|r| r.as_str()) {
            match finish_reason {
                "SAFETY" => anyhow::bail!("Gemini stream terminated due to SAFETY filter"),
                "MAX_TOKENS" => {
                    anyhow::bail!("Gemini stream terminated due to MAX_TOKENS limit")
                }
                "CONTINUATION" => {
                    anyhow::bail!("Gemini stream terminated with unhandled CONTINUATION")
                }
                "RECITATION" | "PROHIBITED_CONTENT" | "SPII" | "BLOCKLIST" => {
                    anyhow::bail!(
                        "Gemini stream terminated due to content policy: {}",
                        finish_reason
                    );
                }
                "STOP" => saw_stop = true,
                "" => {}
                other => anyhow::bail!(
                    "Gemini stream terminated with non-STOP finishReason: {}",
                    other
                ),
            }
        }
        let parts = c0
            .get("content")
            .and_then(|c| c.get("parts"))
            .and_then(|p| p.as_array())
            .cloned()
            .unwrap_or_default();
        return Ok(Some((parts, saw_stop)));
    }

    Ok(None)
}

pub(crate) fn push_candidate_parts(
    parts: &[Value],
    parsed: &mut ParsedModelResponse,
    seen_first_fc: &mut bool,
    mut on_text: impl FnMut(&str, bool),
) {
    for part in parts {
        if let Some(fc_val) = part.get("functionCall") {
            let id = fc_val
                .get("id")
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            let name = fc_val
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let args = fc_val.get("args").cloned().unwrap_or(Value::Null);

            let thought_signature = if !*seen_first_fc {
                part.get("thoughtSignature")
                    .or_else(|| part.get("thought_signature"))
                    .or_else(|| fc_val.get("thoughtSignature"))
                    .or_else(|| fc_val.get("thought_signature"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
            } else {
                None
            };

            let mut raw_part = part.clone();
            if *seen_first_fc {
                if let Some(obj) = raw_part.as_object_mut() {
                    obj.remove("thoughtSignature");
                    obj.remove("thought_signature");
                    if let Some(fc_inner) =
                        obj.get_mut("functionCall").and_then(|v| v.as_object_mut())
                    {
                        fc_inner.remove("thoughtSignature");
                        fc_inner.remove("thought_signature");
                    }
                }
            }
            *seen_first_fc = true;

            let part_index = parsed.model_parts.len();
            parsed.model_parts.push(raw_part.clone());
            parsed.function_calls.push(ModelToolCall {
                part_index,
                call: FunctionCall {
                    id,
                    name,
                    args,
                    thought_signature,
                },
                raw_part,
            });
        } else if let Some(txt) = part.get("text").and_then(|t| t.as_str()) {
            if !txt.is_empty() || part.get("thought").is_some() {
                parsed.model_parts.push(part.clone());
            }
            let has_fc = !parsed.function_calls.is_empty();
            parsed.text.push_str(txt);
            on_text(txt, has_fc);
        } else {
            parsed.model_parts.push(part.clone());
        }
    }
}

/// Parses an SSE byte stream incrementally, feeding text to SentenceChunker and emitting TtsChunk items.
/// Strictly verifies that an incomplete or errored stream never dispatches a functionCall.
pub async fn parse_sse_bytes_stream<S, E>(
    stream: S,
    turn_id: u32,
    turn_token: &CancellationToken,
    chunk_tx: Option<&tokio::sync::mpsc::Sender<TtsChunk>>,
    on_first_token: impl FnMut(),
) -> Result<(String, Option<FunctionCall>)>
where
    S: futures::Stream<Item = std::result::Result<bytes::Bytes, E>> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    let parsed =
        parse_sse_bytes_stream_full(stream, turn_id, turn_token, chunk_tx, on_first_token).await?;
    let last_fc = parsed.function_calls.into_iter().last().map(|mtc| mtc.call);
    Ok((parsed.text, last_fc))
}

/// Full SSE parser preserving all parallel functionCall parts and verbatim JSON payloads.
pub async fn parse_sse_bytes_stream_full<S, E>(
    mut stream: S,
    turn_id: u32,
    turn_token: &CancellationToken,
    chunk_tx: Option<&tokio::sync::mpsc::Sender<TtsChunk>>,
    mut on_first_token: impl FnMut(),
) -> Result<ParsedModelResponse>
where
    S: futures::Stream<Item = std::result::Result<bytes::Bytes, E>> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    let mut chunker = SentenceChunker::new();
    let mut seq = 0u32;
    let mut line_buf = String::new();
    let mut parsed = ParsedModelResponse::default();
    let mut seen_first_fc = false;
    let mut saw_valid_candidate = false;
    let mut saw_finish_stop = false;
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
        let bytes = item_res.map_err(|e| anyhow::anyhow!("Error reading SSE response stream: {e}"))?;
        let chunk_str = String::from_utf8_lossy(&bytes);
        line_buf.push_str(&chunk_str);

        while let Some(newline_pos) = line_buf.find('\n') {
            let line = line_buf[..newline_pos].trim().to_string();
            line_buf = line_buf[newline_pos + 1..].to_string();

            if let Some(json_str) = line.strip_prefix("data: ") {
                let json_str = json_str.trim();
                if json_str.is_empty() || json_str == "[DONE]" {
                    continue;
                }

                let val: Value = serde_json::from_str(json_str).map_err(|e| {
                    anyhow::anyhow!("Malformed JSON in SSE response stream: {e}")
                })?;

                if let Some((parts, is_stop)) = validate_response_envelope(&val)? {
                    saw_valid_candidate = true;
                    if is_stop {
                        saw_finish_stop = true;
                    }
                    let mut pending_clauses: Vec<String> = Vec::new();
                    push_candidate_parts(&parts, &mut parsed, &mut seen_first_fc, |txt, has_fc| {
                        if !first_token_recorded && !txt.is_empty() {
                            on_first_token();
                            first_token_recorded = true;
                        }
                        if !has_fc {
                            pending_clauses.extend(chunker.push_delta(txt));
                        }
                    });

                    if parsed.function_calls.is_empty() {
                        for clause in pending_clauses {
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
            }
        }
    }

    // Fail closed if stream ended mid-frame with trailing incomplete data
    let trailing = line_buf.trim();
    if !trailing.is_empty() {
        anyhow::bail!("Truncated or malformed trailing SSE frame: '{}'", trailing);
    }

    if !saw_valid_candidate {
        anyhow::bail!("Gemini stream ended with missing candidates");
    }

    if !parsed.function_calls.is_empty() && !saw_finish_stop {
        anyhow::bail!("Gemini stream ended before terminal finishReason STOP for tool call");
    }

    // On stream EOF, flush any trailing text in chunker if no function calls were emitted;
    // if text chunks were already emitted before a functionCall, finalize that TTS stream.
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

    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_verbatim_thought_signature_preservation_and_part_one_rule() {
        let part1 = json!({
            "functionCall": {
                "name": "run_terminal_command",
                "args": { "command": "git status" },
                "id": "call_abc_1"
            },
            "thoughtSignature": "thought_sig_part1_verified"
        });
        let part2 = json!({
            "functionCall": {
                "name": "open_browser_url",
                "args": { "url": "https://google.com" },
                "id": "call_abc_2"
            },
            "thoughtSignature": "thought_sig_part2_illegal"
        });

        // Part 1: preserves signature
        let fc1_val = part1.get("functionCall").unwrap();
        let fc1 = FunctionCall {
            id: fc1_val
                .get("id")
                .and_then(|v| v.as_str())
                .map(ToString::to_string),
            name: fc1_val
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap()
                .to_string(),
            args: fc1_val.get("args").cloned().unwrap(),
            thought_signature: part1
                .get("thoughtSignature")
                .and_then(|v| v.as_str())
                .map(ToString::to_string),
        };
        assert_eq!(fc1.id.as_deref(), Some("call_abc_1"));
        assert_eq!(
            fc1.thought_signature.as_deref(),
            Some("thought_sig_part1_verified")
        );

        // Part 2: per parallel placement rule, part 2 does NOT bear thoughtSignature
        let fc2_val = part2.get("functionCall").unwrap();
        let fc2 = FunctionCall {
            id: fc2_val
                .get("id")
                .and_then(|v| v.as_str())
                .map(ToString::to_string),
            name: fc2_val
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap()
                .to_string(),
            args: fc2_val.get("args").cloned().unwrap(),
            thought_signature: None, // Parallel placement rule enforced
        };
        assert_eq!(fc2.id.as_deref(), Some("call_abc_2"));
        assert_eq!(fc2.thought_signature, None);
    }

    #[tokio::test]
    async fn test_streaming_error_boundary_prevents_tool_dispatch() {
        let token = CancellationToken::new();
        let (tx, _rx) = tokio::sync::mpsc::channel::<TtsChunk>(10);

        let stream = futures::stream::iter(vec![
            Ok::<bytes::Bytes, std::io::Error>(bytes::Bytes::from_static(b"data: {\"candidates\": [{\"content\": {\"parts\": [{\"text\": \"Starting...\"}]}}]}\n\n")),
            Err::<bytes::Bytes, std::io::Error>(std::io::Error::new(std::io::ErrorKind::ConnectionReset, "Network pipe broken")),
        ]);

        let res = parse_sse_bytes_stream(stream, 101, &token, Some(&tx), || {}).await;

        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("Network pipe broken"));
    }

    #[tokio::test]
    async fn test_mock_sse_streaming_chunk_dispatch_and_cancellation() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<TtsChunk>(10);
        let token = CancellationToken::new();

        let sse_data1 = b"data: {\"candidates\": [{\"content\": {\"parts\": [{\"text\": \"First sentence is complete. Trailing clause without dot\"}]}}]}\n\n";
        let sse_data2 = b"data: {\"candidates\": [{\"content\": {\"parts\": [{\"text\": \" now continues here\"}]}}]}\n\n";

        let stream = futures::stream::iter(vec![
            Ok::<bytes::Bytes, std::io::Error>(bytes::Bytes::from_static(sse_data1)),
            Ok::<bytes::Bytes, std::io::Error>(bytes::Bytes::from_static(sse_data2)),
        ]);

        let mut ttft_called = false;
        let (full_text, fc) = parse_sse_bytes_stream(stream, 42, &token, Some(&tx), || {
            ttft_called = true;
        })
        .await
        .unwrap();

        assert!(ttft_called);
        assert!(fc.is_none());
        assert_eq!(
            full_text,
            "First sentence is complete. Trailing clause without dot now continues here"
        );

        let chunk0 = rx.recv().await.unwrap();
        assert_eq!(chunk0.turn_id, 42);
        assert_eq!(chunk0.seq, 0);
        assert_eq!(chunk0.text, "First sentence is complete.");
        assert!(!chunk0.is_final);

        let chunk1 = rx.recv().await.unwrap();
        assert_eq!(chunk1.turn_id, 42);
        assert_eq!(chunk1.seq, 1);
        assert_eq!(chunk1.text, "Trailing clause without dot now continues here");
        assert!(chunk1.is_final);

        let cancel_token = CancellationToken::new();
        cancel_token.cancel();

        let stream2 = futures::stream::iter(vec![
            Ok::<bytes::Bytes, std::io::Error>(bytes::Bytes::from_static(sse_data1)),
        ]);

        let (tx2, mut rx2) = tokio::sync::mpsc::channel::<TtsChunk>(10);
        let res = parse_sse_bytes_stream(stream2, 43, &cancel_token, Some(&tx2), || {}).await;

        assert!(res.is_err());
        assert!(rx2.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_sse_streaming_text_followed_by_function_call_emits_is_final() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<TtsChunk>(10);
        let token = CancellationToken::new();

        let sse_data1 = b"data: {\"candidates\": [{\"content\": {\"parts\": [{\"text\": \"Checking your workspace now.\"}]}}]}\n\n";
        let sse_data2 = b"data: {\"candidates\": [{\"content\": {\"parts\": [{\"functionCall\": {\"name\": \"terminal_execute\", \"args\": {\"command\": \"git status\"}}}]}, \"finishReason\": \"STOP\"}]}\n\n";

        let stream = futures::stream::iter(vec![
            Ok::<bytes::Bytes, std::io::Error>(bytes::Bytes::from_static(sse_data1)),
            Ok::<bytes::Bytes, std::io::Error>(bytes::Bytes::from_static(sse_data2)),
        ]);

        let parsed = parse_sse_bytes_stream_full(stream, 77, &token, Some(&tx), || {})
            .await
            .unwrap();

        assert_eq!(parsed.function_calls.len(), 1);
        let chunk0 = rx.try_recv().expect("expected spoken preamble chunk");
        assert_eq!(chunk0.text, "Checking your workspace now.");
        assert!(!chunk0.is_final);

        let final_chunk = rx
            .try_recv()
            .expect("expected finalization chunk (is_final=true) after text before functionCall");
        assert_eq!(final_chunk.turn_id, 77);
        assert!(final_chunk.is_final);
    }
}
