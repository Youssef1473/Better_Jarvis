use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::warn;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct EnterpriseConfig {
    pub project_id: String,
    pub location: String,
    pub allow_metadata_service: bool,
}

impl Default for EnterpriseConfig {
    fn default() -> Self {
        Self {
            project_id: String::new(),
            location: "global".to_string(),
            allow_metadata_service: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct LocalFallbackConfig {
    pub enabled: bool,
    pub provider: String,
    pub base_url: String,
    pub model: String,
    pub max_output_tokens: u32,
    pub timeout_secs: u64,
}

impl Default for LocalFallbackConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            provider: "ollama".to_string(),
            base_url: "http://127.0.0.1:11434".to_string(),
            model: "qwen3.5:2b".to_string(),
            max_output_tokens: 4096,
            timeout_secs: 60,
        }
    }
}

/// Validates that a configured local LLM fallback endpoint strictly targets loopback HTTP without credentials.
pub fn validate_loopback_base_url(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    let parsed = reqwest::Url::parse(trimmed)
        .with_context(|| format!("Invalid local fallback base_url '{}'", raw))?;
    if parsed.scheme() != "http" {
        anyhow::bail!("Local fallback base_url must use http:// scheme");
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        anyhow::bail!("Local fallback base_url must not contain userinfo");
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        anyhow::bail!("Local fallback base_url must not contain query or fragment");
    }
    let host = parsed.host_str().unwrap_or("");
    if !matches!(host, "127.0.0.1" | "localhost" | "[::1]") {
        anyhow::bail!(
            "Local fallback base_url host '{}' must be loopback (127.0.0.1, localhost, or [::1])",
            host
        );
    }
    if !parsed.path().is_empty() && parsed.path() != "/" {
        anyhow::bail!("Local fallback base_url must not include non-root path segments");
    }
    Ok(parsed.as_str().trim_end_matches('/').to_string())
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub provider: String,
    pub fast_model: String,
    pub heavy_model: String,
    pub fallback_model: String,
    pub api_key_env: String,
    pub allowed_directories: Vec<String>,
    pub max_output_tokens_fast: u32,
    pub max_output_tokens_fallback: u32,
    pub max_output_tokens_heavy: u32,
    pub enterprise: EnterpriseConfig,
    pub local_fallback: LocalFallbackConfig,
}

fn resolve_user_profile_dir() -> Option<PathBuf> {
    if let Ok(profile) = std::env::var("USERPROFILE") {
        let trimmed = profile.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    if let (Ok(drive), Ok(path)) = (std::env::var("HOMEDRIVE"), std::env::var("HOMEPATH")) {
        let combined = format!("{}{}", drive.trim(), path.trim());
        if !combined.is_empty() {
            return Some(PathBuf::from(combined));
        }
    }
    None
}

pub fn expand_user_env_vars(raw: &str) -> String {
    let mut out = raw.to_string();
    if out.contains("%USERPROFILE%") {
        if let Some(profile) = resolve_user_profile_dir() {
            out = out.replace("%USERPROFILE%", &profile.to_string_lossy());
        }
    }
    if out.contains("%LOCALAPPDATA%") {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            out = out.replace("%LOCALAPPDATA%", &local_app_data);
        }
    }
    out
}

pub fn default_allowed_directories() -> Vec<String> {
    if let Some(profile) = resolve_user_profile_dir() {
        vec![
            profile.join("Projects").to_string_lossy().into_owned(),
            profile.join("Documents").to_string_lossy().into_owned(),
        ]
    } else {
        vec![".".to_string()]
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            provider: "auto".to_string(),
            fast_model: "gemini-3.5-flash-lite".to_string(),
            heavy_model: "gemini-4-hydrogen".to_string(),
            fallback_model: "gemini-3.8-flash".to_string(),
            api_key_env: "GEMINI_API_KEY".to_string(),
            allowed_directories: default_allowed_directories(),
            max_output_tokens_fast: 8192,
            max_output_tokens_fallback: 4096,
            max_output_tokens_heavy: 65536,
            enterprise: EnterpriseConfig::default(),
            local_fallback: LocalFallbackConfig::default(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawModelLimits {
    max_output_tokens: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawAiSection {
    provider: Option<String>,
    api_key_env: Option<String>,
    fast_model: Option<String>,
    heavy_model: Option<String>,
    fallback_model: Option<String>,
    gemini_3_5_flash_lite: RawModelLimits,
    gemini_3_8_flash: RawModelLimits,
    gemini_4_hydrogen: RawModelLimits,
    enterprise: Option<EnterpriseConfig>,
    local_fallback: Option<LocalFallbackConfig>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawSecuritySection {
    allowed_directories: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawTomlConfig {
    ai: RawAiSection,
    security: RawSecuritySection,
}

impl AppConfig {
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

    /// Parses configuration strictly from a TOML string, failing on syntax or type errors.
    pub fn load_from_str(content: &str) -> Result<Self> {
        let parsed: RawTomlConfig =
            toml::from_str(content).context("Failed to parse configuration TOML")?;
        let mut config = Self::default();

        if let Some(p) = parsed.ai.provider.filter(|s| !s.trim().is_empty()) {
            config.provider = p.trim().to_string();
        }
        if let Some(k) = parsed.ai.api_key_env.filter(|s| !s.trim().is_empty()) {
            config.api_key_env = k.trim().to_string();
        }
        if let Some(m) = parsed.ai.fast_model.filter(|s| !s.trim().is_empty()) {
            config.fast_model = m.trim().to_string();
        }
        if let Some(m) = parsed.ai.heavy_model.filter(|s| !s.trim().is_empty()) {
            config.heavy_model = m.trim().to_string();
        }
        if let Some(m) = parsed.ai.fallback_model.filter(|s| !s.trim().is_empty()) {
            config.fallback_model = m.trim().to_string();
        }
        if let Some(t) = parsed.ai.gemini_3_5_flash_lite.max_output_tokens {
            config.max_output_tokens_fast = t;
        }
        if let Some(t) = parsed.ai.gemini_3_8_flash.max_output_tokens {
            config.max_output_tokens_fallback = t;
        }
        if let Some(t) = parsed.ai.gemini_4_hydrogen.max_output_tokens {
            config.max_output_tokens_heavy = t;
        }
        if let Some(ent) = parsed.ai.enterprise {
            config.enterprise = ent;
        }
        if let Some(mut lf) = parsed.ai.local_fallback {
            lf.provider = if lf.provider.trim().is_empty() {
                "ollama".to_string()
            } else {
                lf.provider.trim().to_lowercase()
            };
            lf.base_url = validate_loopback_base_url(&lf.base_url)?;
            lf.model = if lf.model.trim().is_empty() {
                "qwen3.5:2b".to_string()
            } else {
                lf.model.trim().to_string()
            };
            if lf.timeout_secs < 5 {
                lf.timeout_secs = 5;
            }
            config.local_fallback = lf;
        }
        if let Some(dirs) = parsed.security.allowed_directories {
            let expanded: Vec<String> = dirs
                .into_iter()
                .map(|d| expand_user_env_vars(d.trim()))
                .filter(|d| !d.is_empty())
                .collect();
            if !expanded.is_empty() {
                config.allowed_directories = expanded;
            }
        }

        Ok(config)
    }

    /// Loads configuration strictly from a TOML file path.
    pub fn load_from_file(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file at {}", path.display()))?;
        Self::load_from_str(&content)
    }

    /// Loads configuration from a TOML file path or logs a warning and falls back to defaults.
    pub fn load_from_file_or_default(path: &Path) -> Self {
        match Self::load_from_file(path) {
            Ok(cfg) => cfg,
            Err(err) => {
                if path.exists() {
                    warn!("Invalid configuration at {}: {}; falling back to defaults", path.display(), err);
                }
                Self::default()
            }
        }
    }

    /// Finds config/config.toml relative to the binary or current working directory.
    pub fn find_and_load() -> Self {
        let candidates = [
            PathBuf::from("config/config.toml"),
            PathBuf::from("../config/config.toml"),
            PathBuf::from("../../config/config.toml"),
        ];

        for candidate in &candidates {
            if candidate.exists() {
                return Self::load_from_file_or_default(candidate);
            }
        }

        Self::default()
    }
}

/// Shared resolver for tier-based max output token budgets across `AppConfig` and `GeminiClient`.
pub fn resolve_max_output_tokens(
    model: &str,
    fast_model: &str,
    fallback_model: &str,
    heavy_model: &str,
    fast_limit: u32,
    fallback_limit: u32,
    heavy_limit: u32,
) -> u32 {
    if model == fast_model || model.contains("flash-lite") {
        fast_limit
    } else if model == fallback_model || model.contains("flash") {
        fallback_limit
    } else if model == heavy_model || model.contains("hydrogen") || model.contains("pro") {
        heavy_limit
    } else {
        4096
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.fast_model, "gemini-3.5-flash-lite");
        assert_eq!(cfg.heavy_model, "gemini-4-hydrogen");
        assert_eq!(cfg.fallback_model, "gemini-3.8-flash");
        assert_eq!(cfg.api_key_env, "GEMINI_API_KEY");
        assert_eq!(cfg.max_output_tokens_fast, 8192);
        assert_eq!(cfg.max_output_tokens_fallback, 4096);
        assert_eq!(cfg.max_output_tokens_heavy, 65536);
        assert_eq!(cfg.enterprise.project_id, "");
        assert_eq!(cfg.enterprise.location, "global");
        assert!(!cfg.enterprise.allow_metadata_service);
        assert!(!cfg.allowed_directories.is_empty());
    }

    #[test]
    fn test_load_from_file() {
        let toml_sample = r#"
[ai]
provider = "google_ai_studio"
api_key_env = "MY_CUSTOM_KEY"
fast_model = "gemini-3.5-flash-lite"
heavy_model = "gemini-4-hydrogen"
fallback_model = "gemini-3.8-flash"

[ai.gemini_3_5_flash_lite]
max_output_tokens = 8192

[ai.gemini_3_8_flash]
max_output_tokens = 4096

[ai.gemini_4_hydrogen]
max_output_tokens = 65536

[ai.enterprise]
project_id = "test-project-123"
location = "us-central1"
allow_metadata_service = true

[security]
allowed_directories = [
    "D:\\Code",
    "C:\\Users\\Test\\Projects"
]
"#;
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join("test_config.toml");
        std::fs::write(&test_file, toml_sample).unwrap();

        let cfg = AppConfig::load_from_file_or_default(&test_file);
        let _ = std::fs::remove_file(test_file);

        assert_eq!(cfg.fast_model, "gemini-3.5-flash-lite");
        assert_eq!(cfg.heavy_model, "gemini-4-hydrogen");
        assert_eq!(cfg.fallback_model, "gemini-3.8-flash");
        assert_eq!(cfg.provider, "google_ai_studio");
        assert_eq!(cfg.api_key_env, "MY_CUSTOM_KEY");
        assert_eq!(cfg.max_output_tokens_fast, 8192);
        assert_eq!(cfg.max_output_tokens_fallback, 4096);
        assert_eq!(cfg.max_output_tokens_heavy, 65536);
        assert_eq!(cfg.enterprise.project_id, "test-project-123");
        assert_eq!(cfg.enterprise.location, "us-central1");
        assert!(cfg.enterprise.allow_metadata_service);
        assert_eq!(cfg.allowed_directories.len(), 2);
        assert_eq!(cfg.allowed_directories[0], "D:\\Code");
    }

    #[test]
    fn test_max_output_tokens_for_model() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.max_output_tokens_for_model("gemini-3.5-flash-lite"), 8192);
        assert_eq!(cfg.max_output_tokens_for_model("gemini-3.8-flash"), 4096);
        assert_eq!(cfg.max_output_tokens_for_model("gemini-4-hydrogen"), 65536);
        assert_eq!(cfg.max_output_tokens_for_model("unknown-model"), 4096);
    }

    #[test]
    fn test_default_config_has_no_hardcoded_username() {
        let repo_cfg = include_str!("../../config/config.toml");
        assert!(
            !repo_cfg.contains("Youssef"),
            "config/config.toml must not hardcode developer username paths"
        );
        let expanded = expand_user_env_vars("%USERPROFILE%\\Projects");
        assert!(!expanded.contains("%USERPROFILE%"));
    }

    #[test]
    fn test_malformed_toml_rejected_by_strict_loader() {
        assert!(AppConfig::load_from_str("[ai\nbroken = true").is_err());
        assert!(AppConfig::load_from_str("[ai.gemini_3_8_flash]\nmax_output_tokens = \"bad\"").is_err());
    }

    #[test]
    fn test_local_fallback_config_parsing_and_loopback_validation() {
        let cfg = AppConfig::default();
        assert!(cfg.local_fallback.enabled);
        assert_eq!(cfg.local_fallback.provider, "ollama");
        assert_eq!(cfg.local_fallback.base_url, "http://127.0.0.1:11434");
        assert_eq!(cfg.local_fallback.model, "qwen3.5:2b");
        assert_eq!(cfg.local_fallback.max_output_tokens, 4096);
        assert_eq!(cfg.local_fallback.timeout_secs, 60);

        let valid = AppConfig::load_from_str(
            r#"
[ai.local_fallback]
enabled = true
provider = "ollama"
base_url = "http://localhost:11434/"
model = "qwen3.5:2b"
max_output_tokens = 2048
timeout_secs = 45
"#,
        )
        .expect("valid loopback localhost config");
        assert_eq!(valid.local_fallback.base_url, "http://localhost:11434");
        assert_eq!(valid.local_fallback.max_output_tokens, 2048);
        assert_eq!(valid.local_fallback.timeout_secs, 45);

        // SSRF / remote exfiltration protection: non-loopback or userinfo URLs must be rejected
        for bad_url in [
            "http://10.0.0.5:11434",
            "https://evil.example:11434",
            "http://user@127.0.0.1:11434",
            "file:///etc/passwd",
        ] {
            let toml = format!("[ai.local_fallback]\nbase_url = \"{}\"\n", bad_url);
            assert!(
                AppConfig::load_from_str(&toml).is_err(),
                "non-loopback URL must be rejected: {}",
                bad_url
            );
        }
    }
}
