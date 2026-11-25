use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub fast_model: String,
    pub heavy_model: String,
    pub api_key_env: String,
    pub allowed_directories: Vec<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            fast_model: "gemini-2.5-flash".to_string(),
            heavy_model: "gemini-2.5-pro".to_string(),
            api_key_env: "GEMINI_API_KEY".to_string(),
            allowed_directories: vec![
                "C:\\Users\\Youssef\\Projects".to_string(),
                "C:\\Users\\Youssef\\Documents".to_string(),
            ],
        }
    }
}

impl AppConfig {
    /// Loads configuration from a TOML file path or falls back to sane defaults.
    pub fn load_from_file_or_default(path: &Path) -> Self {
        let mut config = Self::default();

        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return config,
        };

        let mut current_section = "";
        let mut allowed_dirs = Vec::new();
        let mut in_allowed_dirs_array = false;

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                current_section = &trimmed[1..trimmed.len() - 1];
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

                if current_section == "ai" {
                    let clean_val = val.trim_matches('"').trim_matches('\'').trim();
                    match key {
                        "fast_model" => config.fast_model = clean_val.to_string(),
                        "heavy_model" => config.heavy_model = clean_val.to_string(),
                        "api_key_env" => config.api_key_env = clean_val.to_string(),
                        _ => {}
                    }
                } else if current_section == "security" && key == "allowed_directories" && val.starts_with('[') {
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
        assert_eq!(cfg.fast_model, "gemini-2.5-flash");
        assert_eq!(cfg.heavy_model, "gemini-2.5-pro");
        assert_eq!(cfg.api_key_env, "GEMINI_API_KEY");
        assert!(!cfg.allowed_directories.is_empty());
    }

    #[test]
    fn test_load_from_file() {
        let toml_sample = r#"
[ai]
provider = "google_ai_studio"
api_key_env = "MY_CUSTOM_KEY"
fast_model = "gemini-2.5-flash"
heavy_model = "gemini-2.5-pro"

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

        assert_eq!(cfg.fast_model, "gemini-2.5-flash");
        assert_eq!(cfg.heavy_model, "gemini-2.5-pro");
        assert_eq!(cfg.api_key_env, "MY_CUSTOM_KEY");
        assert_eq!(cfg.allowed_directories.len(), 2);
        assert_eq!(cfg.allowed_directories[0], "D:\\Code");
    }
}
