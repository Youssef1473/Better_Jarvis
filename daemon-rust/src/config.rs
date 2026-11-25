use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub fast_model: String,
    pub heavy_model: String,
    pub fallback_model: String,
    pub api_key_env: String,
    pub allowed_directories: Vec<String>,
    pub max_output_tokens_fast: u32,
    pub max_output_tokens_fallback: u32,
    pub max_output_tokens_heavy: u32,
    pub enterprise: EnterpriseConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            fast_model: "gemini-3.5-flash-lite".to_string(),
            heavy_model: "gemini-4-hydrogen".to_string(),
            fallback_model: "gemini-3.8-flash".to_string(),
            api_key_env: "GEMINI_API_KEY".to_string(),
            allowed_directories: vec![
                "C:\\Users\\Youssef\\Projects".to_string(),
                "C:\\Users\\Youssef\\Documents".to_string(),
            ],
            max_output_tokens_fast: 8192,
            max_output_tokens_fallback: 4096,
            max_output_tokens_heavy: 65536,
            enterprise: EnterpriseConfig::default(),
        }
    }
}

impl AppConfig {
    pub fn max_output_tokens_for_model(&self, model: &str) -> u32 {
        if model == self.fast_model || model.contains("flash-lite") {
            self.max_output_tokens_fast
        } else if model == self.fallback_model || model.contains("flash") {
            self.max_output_tokens_fallback
        } else if model == self.heavy_model || model.contains("hydrogen") || model.contains("pro") {
            self.max_output_tokens_heavy
        } else {
            4096
        }
    }

    /// Loads configuration from a TOML file path or falls back to sane defaults.
    pub fn load_from_file_or_default(path: &Path) -> Self {
        let mut config = Self::default();

        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return config,
        };

        let mut current_section = String::new();
        let mut allowed_dirs = Vec::new();
        let mut in_allowed_dirs_array = false;

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                current_section = trimmed[1..trimmed.len() - 1].trim().to_string();
                in_allowed_dirs_array = false;
                continue;
            }

            if in_allowed_dirs_array {
                if trimmed.contains(']') {
                    in_allowed_dirs_array = false;
                }
                let item = trimmed
                    .trim_matches(|c| c == ']' || c == ',' || c == '"' || c == '\'')
                    .trim()
                    .replace("\\\\", "\\");
                if !item.is_empty() {
                    allowed_dirs.push(item);
                }
                continue;
            }

            if let Some((key, val)) = trimmed.split_once('=') {
                let key = key.trim();
                let val = val.trim();
                let clean_val = val.trim_matches('"').trim_matches('\'').trim();

                match current_section.as_str() {
                    "ai" => match key {
                        "fast_model" => config.fast_model = clean_val.to_string(),
                        "heavy_model" => config.heavy_model = clean_val.to_string(),
                        "fallback_model" => config.fallback_model = clean_val.to_string(),
                        "api_key_env" => config.api_key_env = clean_val.to_string(),
                        _ => {}
                    },
                    "ai.gemini_3_5_flash_lite" => {
                        if key == "max_output_tokens" {
                            if let Ok(v) = clean_val.parse::<u32>() {
                                config.max_output_tokens_fast = v;
                            }
                        }
                    }
                    "ai.gemini_3_8_flash" => {
                        if key == "max_output_tokens" {
                            if let Ok(v) = clean_val.parse::<u32>() {
                                config.max_output_tokens_fallback = v;
                            }
                        }
                    }
                    "ai.gemini_4_hydrogen" => {
                        if key == "max_output_tokens" {
                            if let Ok(v) = clean_val.parse::<u32>() {
                                config.max_output_tokens_heavy = v;
                            }
                        }
                    }
                    "ai.enterprise" => match key {
                        "project_id" => config.enterprise.project_id = clean_val.to_string(),
                        "location" => config.enterprise.location = clean_val.to_string(),
                        "allow_metadata_service" => {
                            if let Ok(v) = clean_val.parse::<bool>() {
                                config.enterprise.allow_metadata_service = v;
                            }
                        }
                        _ => {}
                    },
                    "security" if key == "allowed_directories" && val.starts_with('[') => {
                        if !val.ends_with(']') {
                            in_allowed_dirs_array = true;
                        }
                        let inner = val.trim_start_matches('[').trim_end_matches(']').trim();
                        for item in inner.split(',') {
                            let item_clean = item
                                .trim()
                                .trim_matches('"')
                                .trim_matches('\'')
                                .trim()
                                .replace("\\\\", "\\");
                            if !item_clean.is_empty() {
                                allowed_dirs.push(item_clean);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        if !allowed_dirs.is_empty() {
            config.allowed_directories = allowed_dirs;
        }

        config
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
}
