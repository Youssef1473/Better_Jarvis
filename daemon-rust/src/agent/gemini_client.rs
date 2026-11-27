use crate::agent::model_router::ModelRouter;
use crate::config::{resolve_max_output_tokens, AppConfig, EnterpriseConfig, LocalFallbackConfig};
use crate::security::policy::{self, PolicyEngine, TurnTaintState};
use crate::tools::confirmation::ConfirmationManager;
use anyhow::{Context, Result};
use futures::StreamExt;
use google_cloud_auth::credentials::Builder as AuthBuilder;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

pub use crate::agent::auth_adc::{
    discover_windows_adc, extract_enterprise_headers, AdcDiscoveryResult, EnterpriseAuthError,
    ProbeOutcome,
};
pub use crate::agent::sse_parser::{
    parse_sse_bytes_stream, parse_sse_bytes_stream_full, FunctionCall, ModelToolCall,
    ParsedModelResponse, TtsChunk,
};
pub use crate::agent::tool_schemas::{
    build_payload, build_payload_with_tokens, get_tool_declarations,
};
pub use crate::agent::tool_scheduler::{
    dispatch_tool_call, execute_tool_with_guardrails, SandboxTerminalExecutor, TerminalExecutor,
};

pub const DEFAULT_FAST_MODEL: &str = "gemini-3.5-flash-lite";
pub const DEFAULT_HEAVY_MODEL: &str = "gemini-4-hydrogen";
pub const DEFAULT_FALLBACK_MODEL: &str = "gemini-3.8-flash";
pub const MAX_TOOL_TURNS: usize = 4;
pub const HTTP_TIMEOUT_SECS: u64 = 15;

pub(crate) type LlmByteStream =
    futures::stream::BoxStream<'static, std::result::Result<bytes::Bytes, std::io::Error>>;

pub(crate) struct LlmHttpResponse {
    pub status: reqwest::StatusCode,
    pub body_stream: LlmByteStream,
    pub error_body: Option<String>,
}

pub(crate) trait LlmTransport: Send + Sync {
    fn post_json(
        &self,
        url: String,
        headers: HeaderMap,
        payload: Value,
    ) -> futures::future::BoxFuture<'static, Result<LlmHttpResponse>>;
}

pub(crate) struct ReqwestLlmTransport {
    pub(crate) http: reqwest::Client,
}

impl LlmTransport for ReqwestLlmTransport {
    fn post_json(
        &self,
        url: String,
        headers: HeaderMap,
        payload: Value,
    ) -> futures::future::BoxFuture<'static, Result<LlmHttpResponse>> {
        let http = self.http.clone();
        Box::pin(async move {
            let response = http
                .post(&url)
                .headers(headers)
                .json(&payload)
                .send()
                .await
                .context("Failed to communicate with LLM API")?;
            let status = response.status();
            if !status.is_success() {
                let err_text = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "Unknown HTTP error".to_string());
                return Ok(LlmHttpResponse {
                    status,
                    body_stream: Box::pin(futures::stream::empty()),
                    error_body: Some(err_text),
                });
            }
            let stream = response
                .bytes_stream()
                .map(|r| r.map_err(std::io::Error::other));
            Ok(LlmHttpResponse {
                status,
                body_stream: Box::pin(stream),
                error_body: None,
            })
        })
    }
}

#[derive(Debug, Clone)]
pub enum GeminiRequestError {
    MissingCredentials(String),
    QuotaOrRateLimit { status: u16, body: String },
    TransportUnavailable(String),
}

impl std::fmt::Display for GeminiRequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingCredentials(msg) => write!(f, "{}", msg),
            Self::QuotaOrRateLimit { status, body } => {
                write!(f, "Gemini API returned HTTP {}: {}", status, body)
            }
            Self::TransportUnavailable(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for GeminiRequestError {}

pub(crate) fn is_fallback_eligible_gemini_error(err: &anyhow::Error) -> bool {
    err.downcast_ref::<GeminiRequestError>().is_some()
}

pub(crate) fn classify_http_failure(status: reqwest::StatusCode, body: String) -> anyhow::Error {
    let code = status.as_u16();
    let lower = body.to_lowercase();
    let is_quota_forbidden = code == 403
        && (lower.contains("quota")
            || lower.contains("rate")
            || lower.contains("resource_exhausted"));
    if code == 429 || code == 402 || is_quota_forbidden {
        anyhow::Error::new(GeminiRequestError::QuotaOrRateLimit { status: code, body })
    } else {
        anyhow::anyhow!("Gemini API returned HTTP {}: {}", status, body)
    }
}

pub(crate) struct ProviderFallbackTransport {
    pub(crate) cloud_http: reqwest::Client,
    pub(crate) local_http: reqwest::Client,
}

impl LlmTransport for ProviderFallbackTransport {
    fn post_json(
        &self,
        url: String,
        headers: HeaderMap,
        payload: Value,
    ) -> futures::future::BoxFuture<'static, Result<LlmHttpResponse>> {
        let http = if url.ends_with("/api/chat") {
            self.local_http.clone()
        } else {
            self.cloud_http.clone()
        };
        ReqwestLlmTransport { http }.post_json(url, headers, payload)
    }
}

#[derive(Debug, Clone)]
pub struct GeminiClient {
    pub(crate) http: reqwest::Client,
    api_key: Option<String>,
    pub provider: String,
    pub fast_model: String,
    pub heavy_model: String,
    pub fallback_model: String,
    pub enterprise: Option<EnterpriseConfig>,
    pub local_fallback: LocalFallbackConfig,
    pub max_output_tokens_fast: u32,
    pub max_output_tokens_fallback: u32,
    pub max_output_tokens_heavy: u32,
}

impl GeminiClient {
    pub fn new(
        api_key: Option<String>,
        fast_model: Option<String>,
        heavy_model: Option<String>,
    ) -> Result<Self> {
        Self::with_config(api_key, fast_model, heavy_model, None, None)
    }

    pub fn with_config(
        api_key: Option<String>,
        fast_model: Option<String>,
        heavy_model: Option<String>,
        fallback_model: Option<String>,
        enterprise: Option<EnterpriseConfig>,
    ) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
            .build()
            .context("Failed to construct reqwest HTTP client for Gemini API")?;

        Ok(Self {
            http,
            api_key,
            provider: "auto".to_string(),
            fast_model: fast_model.unwrap_or_else(|| DEFAULT_FAST_MODEL.to_string()),
            heavy_model: heavy_model.unwrap_or_else(|| DEFAULT_HEAVY_MODEL.to_string()),
            fallback_model: fallback_model.unwrap_or_else(|| DEFAULT_FALLBACK_MODEL.to_string()),
            enterprise,
            local_fallback: LocalFallbackConfig::default(),
            max_output_tokens_fast: 8192,
            max_output_tokens_fallback: 4096,
            max_output_tokens_heavy: 65536,
        })
    }

    /// Creates a GeminiClient from full AppConfig.
    pub fn from_config(config: &AppConfig) -> Result<Self> {
        let api_key = std::env::var(&config.api_key_env).ok();
        let mut client = Self::with_config(
            api_key,
            Some(config.fast_model.clone()),
            Some(config.heavy_model.clone()),
            Some(config.fallback_model.clone()),
            Some(config.enterprise.clone()),
        )?;
        client.provider = config.provider.clone();
        client.local_fallback = config.local_fallback.clone();
        client.max_output_tokens_fast = config.max_output_tokens_fast;
        client.max_output_tokens_fallback = config.max_output_tokens_fallback;
        client.max_output_tokens_heavy = config.max_output_tokens_heavy;
        Ok(client)
    }

    /// Creates a GeminiClient reading the API key from the environment.
    pub fn from_env(fast_model: Option<String>, heavy_model: Option<String>) -> Result<Self> {
        let api_key = std::env::var("GEMINI_API_KEY").ok();
        Self::new(api_key, fast_model, heavy_model)
    }

    pub fn has_api_key(&self) -> bool {
        self.api_key.as_ref().is_some_and(|k| !k.is_empty())
    }

    pub fn router(&self) -> ModelRouter {
        ModelRouter::with_fallback(
            self.fast_model.clone(),
            self.heavy_model.clone(),
            self.fallback_model.clone(),
        )
    }

    pub fn is_enterprise_model(model: &str) -> bool {
        let lower = model.to_lowercase();
        lower.starts_with("gemini-4") || lower.contains("hydrogen") || lower.contains("enterprise")
    }

    pub fn max_output_tokens_for_model(&self, model: &str) -> u32 {
        resolve_max_output_tokens(
            model,
            &self.fast_model,
            &self.fallback_model,
            &self.heavy_model,
            self.max_output_tokens_fast,
            self.max_output_tokens_fallback,
            self.max_output_tokens_heavy,
        )
    }

    pub(crate) fn build_vertex_endpoint(
        project_id: &str,
        location: &str,
        model: &str,
        streaming: bool,
    ) -> Result<String> {
        let project = project_id.trim();
        if project.is_empty() {
            anyhow::bail!(
                "Vertex AI enterprise routing requires a non-empty [ai.enterprise].project_id"
            );
        }
        let loc = if location.trim().is_empty() {
            "global"
        } else {
            location.trim()
        };
        Ok(if streaming {
            format!(
                "https://aiplatform.googleapis.com/v1/projects/{}/locations/{}/publishers/google/models/{}:streamGenerateContent?alt=sse",
                project, loc, model
            )
        } else {
            format!(
                "https://aiplatform.googleapis.com/v1/projects/{}/locations/{}/publishers/google/models/{}:generateContent",
                project, loc, model
            )
        })
    }

    fn build_ai_studio_request(&self, model: &str, streaming: bool) -> Result<(String, HeaderMap)> {
        let api_key = self
            .api_key
            .as_deref()
            .filter(|k| !k.trim().is_empty())
            .ok_or_else(|| {
                anyhow::Error::new(GeminiRequestError::MissingCredentials(
                    "GEMINI_API_KEY is not configured in environment or settings".to_string(),
                ))
            })?;

        let mut headers = HeaderMap::new();
        headers.insert(
            "x-goog-api-key",
            HeaderValue::from_str(api_key).context("Invalid characters in GEMINI_API_KEY")?,
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let url = if streaming {
            format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?alt=sse",
                model
            )
        } else {
            format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
                model
            )
        };

        Ok((url, headers))
    }

    /// Prepares endpoint and authorization headers for the given model.
    /// Respects `[ai].provider`, rejects empty `project_id` on Vertex AI, and performs clean fallback.
    pub async fn prepare_request_for_model(
        &self,
        model: &str,
        streaming: bool,
    ) -> Result<(String, HeaderMap)> {
        let provider = self.provider.trim().to_lowercase();

        if provider == "google_ai_studio" || provider == "ai_studio" {
            let target_model = if Self::is_enterprise_model(model) {
                info!(
                    "Provider is '{}' — routing enterprise model '{}' via fallback '{}' on Google AI Studio",
                    self.provider, model, self.fallback_model
                );
                self.fallback_model.as_str()
            } else {
                model
            };
            return self.build_ai_studio_request(target_model, streaming);
        }

        let explicit_vertex = provider == "vertex_ai" || provider == "enterprise";

        if explicit_vertex || Self::is_enterprise_model(model) {
            let project_id = self
                .enterprise
                .as_ref()
                .map(|e| e.project_id.trim())
                .unwrap_or("");
            let location = self
                .enterprise
                .as_ref()
                .map(|e| e.location.trim())
                .filter(|s| !s.is_empty())
                .unwrap_or("global");

            if project_id.is_empty() {
                if explicit_vertex {
                    anyhow::bail!(
                        "Provider '{}' requires a non-empty [ai.enterprise].project_id",
                        self.provider
                    );
                }
                warn!(
                    "Enterprise model '{}' requested with empty [ai.enterprise].project_id; falling back cleanly to '{}' on Google AI Studio",
                    model, self.fallback_model
                );
                return self.build_ai_studio_request(&self.fallback_model, streaming);
            }

            let allow_metadata = self
                .enterprise
                .as_ref()
                .map(|e| e.allow_metadata_service)
                .unwrap_or(false);

            let discovery = discover_windows_adc(allow_metadata)?;

            match discovery {
                AdcDiscoveryResult::Found(_path) => {
                    if rustls::crypto::CryptoProvider::get_default().is_none() {
                        return Err(EnterpriseAuthError::CryptoProviderUninitialized {
                            correlation_id: policy::generate_crypto_nonce(),
                        }
                        .into());
                    }

                    let url = Self::build_vertex_endpoint(project_id, location, model, streaming)?;
                    let correlation_id = policy::generate_crypto_nonce();
                    let builder = AuthBuilder::default()
                        .with_scopes(["https://www.googleapis.com/auth/cloud-platform"]);
                    let credentials =
                        builder
                            .build()
                            .map_err(|e| EnterpriseAuthError::TokenExchangeFailure {
                                correlation_id: correlation_id.clone(),
                                category: "token_exchange_failure",
                                details: format!("Failed to build enterprise credentials: {e}"),
                            })?;

                    let mut headers =
                        extract_enterprise_headers(&credentials, &correlation_id).await?;
                    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

                    Ok((url, headers))
                }
                AdcDiscoveryResult::NotFoundFallBackToDeveloper { reason } => {
                    if explicit_vertex {
                        anyhow::bail!(
                            "Vertex AI provider '{}' configured but ADC credentials unavailable ({})",
                            self.provider, reason
                        );
                    }
                    info!(
                        "Enterprise model '{}' requested but ADC unavailable (reason: '{}'); performing clean pre-dispatch fallback to '{}'",
                        model, reason, self.fallback_model
                    );
                    self.build_ai_studio_request(&self.fallback_model, streaming)
                }
            }
        } else {
            self.build_ai_studio_request(model, streaming)
        }
    }

    /// Probes availability of a model, returning typed ProbeOutcome.
    pub async fn probe_model(&self, model: &str) -> ProbeOutcome {
        if (self.provider.eq_ignore_ascii_case("google_ai_studio")
            || self.provider.eq_ignore_ascii_case("ai_studio"))
            && Self::is_enterprise_model(model)
        {
            return ProbeOutcome::Skipped;
        }
        if Self::is_enterprise_model(model) {
            let allow_metadata = self
                .enterprise
                .as_ref()
                .map(|e| e.allow_metadata_service)
                .unwrap_or(false);
            match discover_windows_adc(allow_metadata) {
                Ok(AdcDiscoveryResult::NotFoundFallBackToDeveloper { .. }) => {
                    return ProbeOutcome::Skipped;
                }
                Err(_) => {
                    return ProbeOutcome::Failed;
                }
                Ok(AdcDiscoveryResult::Found(_)) => {}
            }
        } else if !self.has_api_key() {
            return ProbeOutcome::Skipped;
        }

        let contents = vec![json!({
            "role": "user",
            "parts": [{ "text": "ping" }]
        })];
        let payload = Self::build_payload_with_tokens(&contents, false, None, Some(1));

        let (endpoint, headers) = match self.prepare_request_for_model(model, false).await {
            Ok(res) => res,
            Err(_) => return ProbeOutcome::Failed,
        };

        match self
            .http
            .post(&endpoint)
            .headers(headers)
            .json(&payload)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => ProbeOutcome::Passed,
            Ok(_) => ProbeOutcome::Failed,
            Err(_) => ProbeOutcome::Failed,
        }
    }

    /// Transcribes audio bytes (WAV formatted) via Gemini API multimodal input.
    pub async fn transcribe_audio(&self, wav_bytes: &[u8]) -> Result<String> {
        let api_key = match &self.api_key {
            Some(k) if !k.is_empty() => k,
            _ => anyhow::bail!("No GEMINI_API_KEY available for transcription"),
        };

        let wav_b64 = Self::base64_encode(wav_bytes);

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
            self.fast_model
        );

        let request_body = json!({
            "contents": [
                {
                    "parts": [
                        {
                            "inline_data": {
                                "mime_type": "audio/wav",
                                "data": wav_b64
                            }
                        },
                        {
                            "text": "Transcribe the user's spoken voice command verbatim. Output ONLY the transcribed words with no other commentary or markdown. If no speech is present, return nothing."
                        }
                    ]
                }
            ],
            "generationConfig": {
                "temperature": 0.0,
                "maxOutputTokens": 64
            }
        });

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert("x-goog-api-key", HeaderValue::from_str(api_key)?);

        let resp = self
            .http
            .post(&url)
            .headers(headers)
            .json(&request_body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Gemini audio transcription error {status}: {body}");
        }

        let resp_json: Value = resp.json().await?;
        let text = resp_json
            .get("candidates")
            .and_then(|c| c.get(0))
            .and_then(|c0| c0.get("content"))
            .and_then(|cnt| cnt.get("parts"))
            .and_then(|parts| parts.get(0))
            .and_then(|p0| p0.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .trim();

        Ok(text.to_string())
    }

    fn base64_encode(data: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut result = String::with_capacity(data.len().div_ceil(3) * 4);
        for chunk in data.chunks(3) {
            let b0 = chunk[0];
            let b1 = if chunk.len() > 1 { chunk[1] } else { 0 };
            let b2 = if chunk.len() > 2 { chunk[2] } else { 0 };

            result.push(ALPHABET[(b0 >> 2) as usize] as char);
            result.push(ALPHABET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);

            if chunk.len() > 1 {
                result.push(ALPHABET[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
            } else {
                result.push('=');
            }

            if chunk.len() > 2 {
                result.push(ALPHABET[(b2 & 0x3f) as usize] as char);
            } else {
                result.push('=');
            }
        }
        result
    }

    pub fn get_tool_declarations() -> Value {
        get_tool_declarations()
    }

    pub fn build_payload(contents: &[Value], include_tools: bool, nonce: Option<&str>) -> Value {
        build_payload(contents, include_tools, nonce)
    }

    pub fn build_payload_with_tokens(
        contents: &[Value],
        include_tools: bool,
        nonce: Option<&str>,
        max_output_tokens: Option<u32>,
    ) -> Value {
        build_payload_with_tokens(contents, include_tools, nonce, max_output_tokens)
    }

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
        execute_tool_with_guardrails(
            executed_tools,
            call,
            policy,
            confirmation_mgr,
            workspace_dir,
            memory,
            os_controller,
            taint,
            turn_token,
        )
        .await
    }

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
        dispatch_tool_call(
            call,
            policy,
            confirmation_mgr,
            workspace_dir,
            memory,
            os_controller,
            taint,
            turn_token,
        )
        .await
    }

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
        parse_sse_bytes_stream(stream, turn_id, turn_token, chunk_tx, on_first_token).await
    }

    pub async fn parse_sse_bytes_stream_full<S, E>(
        stream: S,
        turn_id: u32,
        turn_token: &CancellationToken,
        chunk_tx: Option<&tokio::sync::mpsc::Sender<TtsChunk>>,
        on_first_token: impl FnMut(),
    ) -> Result<ParsedModelResponse>
    where
        S: futures::Stream<Item = std::result::Result<bytes::Bytes, E>> + Unpin,
        E: std::error::Error + Send + Sync + 'static,
    {
        parse_sse_bytes_stream_full(stream, turn_id, turn_token, chunk_tx, on_first_token).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::auth_adc::TEST_ENV_LOCK as ENV_LOCK;

    #[test]
    fn test_client_model_defaults() {
        let client = GeminiClient::new(None, None, None).unwrap();
        assert_eq!(client.fast_model, "gemini-3.5-flash-lite");
        assert_eq!(client.heavy_model, "gemini-4-hydrogen");
        assert_eq!(client.fallback_model, "gemini-3.8-flash");
        assert_eq!(
            client.max_output_tokens_for_model("gemini-3.5-flash-lite"),
            8192
        );
        assert_eq!(client.max_output_tokens_for_model("gemini-3.8-flash"), 4096);
        assert_eq!(
            client.max_output_tokens_for_model("gemini-4-hydrogen"),
            65536
        );
    }

    #[test]
    fn test_is_enterprise_model() {
        assert!(GeminiClient::is_enterprise_model("gemini-4-hydrogen"));
        assert!(GeminiClient::is_enterprise_model("gemini-4-preview"));
        assert!(GeminiClient::is_enterprise_model("enterprise-agent"));
        assert!(!GeminiClient::is_enterprise_model("gemini-3.5-flash-lite"));
        assert!(!GeminiClient::is_enterprise_model("gemini-3.8-flash"));
    }

    #[tokio::test]
    async fn test_live_probe_outcomes_for_all_tiers() {
        let _env_lock = ENV_LOCK.lock().await;
        let config = AppConfig::default();
        let client = GeminiClient::from_config(&config).unwrap();

        let outcome_hydrogen = client.probe_model("gemini-4-hydrogen").await;
        assert_eq!(
            outcome_hydrogen,
            ProbeOutcome::Skipped,
            "Enterprise model without ADC must report SKIPPED"
        );

        let outcome_flash_lite = client.probe_model("gemini-3.5-flash-lite").await;
        let outcome_flash = client.probe_model("gemini-3.8-flash").await;

        println!("LIVE_PROBE: gemini-4-hydrogen = {:?}", outcome_hydrogen);
        println!(
            "LIVE_PROBE: gemini-3.5-flash-lite = {:?}",
            outcome_flash_lite
        );
        println!("LIVE_PROBE: gemini-3.8-flash = {:?}", outcome_flash);
    }

    #[tokio::test]
    async fn test_provider_routing_and_empty_project_id_handling() {
        assert!(GeminiClient::build_vertex_endpoint("", "global", "gemini-4-hydrogen", false).is_err());
        assert!(GeminiClient::build_vertex_endpoint("   ", "global", "gemini-4-hydrogen", true).is_err());
        let ok_ep = GeminiClient::build_vertex_endpoint("my-proj", "us-central1", "gemini-4-hydrogen", false)
            .expect("valid project_id builds endpoint");
        assert!(ok_ep.contains("/projects/my-proj/locations/us-central1/"));

        let mut client_studio = GeminiClient::new(Some("test-key".into()), None, None).unwrap();
        client_studio.provider = "google_ai_studio".to_string();
        let (ep, _) = client_studio
            .prepare_request_for_model("gemini-4-hydrogen", false)
            .await
            .expect("google_ai_studio routes via fallback_model");
        assert!(ep.contains("generativelanguage.googleapis.com/v1beta/models/gemini-3.8-flash"));

        let mut client_vertex = GeminiClient::new(Some("test-key".into()), None, None).unwrap();
        client_vertex.provider = "vertex_ai".to_string();
        client_vertex.enterprise = Some(EnterpriseConfig {
            project_id: "".to_string(),
            location: "global".to_string(),
            allow_metadata_service: false,
        });
        assert!(
            client_vertex
                .prepare_request_for_model("gemini-4-hydrogen", false)
                .await
                .is_err(),
            "vertex_ai with empty project_id must fail explicitly"
        );
    }
}
