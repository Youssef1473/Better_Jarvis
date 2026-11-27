use crate::security::policy;
use google_cloud_auth::credentials::{CacheableResource, Credentials as GAuthCredentials};
use http::Extensions as AuthExtensions;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProbeOutcome {
    Passed,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnterpriseAuthError {
    CredentialsFileNotFound {
        path: String,
        correlation_id: String,
    },
    MalformedCredentials {
        path: String,
        correlation_id: String,
        details: String,
    },
    NetworkConnectivityError {
        correlation_id: String,
        category: &'static str,
    },
    OAuthConsentOrGrantRevoked {
        correlation_id: String,
        category: &'static str,
    },
    TokenExchangeFailure {
        correlation_id: String,
        category: &'static str,
        details: String,
    },
    CryptoProviderUninitialized {
        correlation_id: String,
    },
}

impl std::fmt::Display for EnterpriseAuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CredentialsFileNotFound {
                path,
                correlation_id,
            } => {
                write!(
                    f,
                    "Enterprise credentials file not found at '{}' [correlation_id: {}]",
                    path, correlation_id
                )
            }
            Self::MalformedCredentials {
                path,
                correlation_id,
                details,
            } => {
                write!(
                    f,
                    "Enterprise credentials file at '{}' is malformed: {} [correlation_id: {}]",
                    path, details, correlation_id
                )
            }
            Self::NetworkConnectivityError {
                correlation_id,
                category,
            } => {
                write!(
                    f,
                    "Enterprise authentication network error (category: {}) [correlation_id: {}]",
                    category, correlation_id
                )
            }
            Self::OAuthConsentOrGrantRevoked {
                correlation_id,
                category,
            } => {
                write!(
                    f,
                    "Enterprise OAuth consent or grant revoked (category: {}) [correlation_id: {}]",
                    category, correlation_id
                )
            }
            Self::TokenExchangeFailure {
                correlation_id,
                category,
                details,
            } => {
                write!(
                    f,
                    "Enterprise token exchange failure (category: {}): {} [correlation_id: {}]",
                    category, details, correlation_id
                )
            }
            Self::CryptoProviderUninitialized { correlation_id } => {
                write!(
                    f,
                    "Rustls CryptoProvider is not initialized [correlation_id: {}]",
                    correlation_id
                )
            }
        }
    }
}

impl std::error::Error for EnterpriseAuthError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdcDiscoveryResult {
    Found(PathBuf),
    NotFoundFallBackToDeveloper { reason: String },
}

fn validate_adc_json_file(
    path: PathBuf,
    display_path: &str,
    correlation_id: &str,
) -> Result<AdcDiscoveryResult, EnterpriseAuthError> {
    let content = std::fs::read_to_string(&path).map_err(|e| {
        EnterpriseAuthError::MalformedCredentials {
            path: display_path.to_string(),
            correlation_id: correlation_id.to_string(),
            details: format!("Failed to read file: {e}"),
        }
    })?;

    let parsed: Value = serde_json::from_str(&content).map_err(|e| {
        EnterpriseAuthError::MalformedCredentials {
            path: display_path.to_string(),
            correlation_id: correlation_id.to_string(),
            details: format!("Invalid JSON: {e}"),
        }
    })?;

    if !parsed.is_object() {
        return Err(EnterpriseAuthError::MalformedCredentials {
            path: display_path.to_string(),
            correlation_id: correlation_id.to_string(),
            details: "ADC JSON root must be an object".to_string(),
        });
    }

    Ok(AdcDiscoveryResult::Found(path))
}

/// Discovers Application Default Credentials (ADC) on Windows desktop.
/// Fails closed on missing explicit credentials or malformed JSON.
pub fn discover_windows_adc(
    allow_metadata_service: bool,
) -> Result<AdcDiscoveryResult, EnterpriseAuthError> {
    let correlation_id = policy::generate_crypto_nonce();

    // 1. Check GOOGLE_APPLICATION_CREDENTIALS env var
    if let Ok(env_path_str) = std::env::var("GOOGLE_APPLICATION_CREDENTIALS") {
        let env_path_str = env_path_str.trim().to_string();
        if !env_path_str.is_empty() {
            let path = PathBuf::from(&env_path_str);
            if !path.exists() {
                return Err(EnterpriseAuthError::CredentialsFileNotFound {
                    path: env_path_str,
                    correlation_id,
                });
            }
            return validate_adc_json_file(path, &env_path_str, &correlation_id);
        }
    }

    // 2. Check standard Windows desktop path: %APPDATA%\gcloud\application_default_credentials.json
    if let Ok(appdata_dir) = std::env::var("APPDATA") {
        let default_path = PathBuf::from(appdata_dir)
            .join("gcloud")
            .join("application_default_credentials.json");
        if default_path.exists() {
            let display = default_path.to_string_lossy().to_string();
            return validate_adc_json_file(default_path, &display, &correlation_id);
        }
    }

    // 3. Fallback check
    if !allow_metadata_service {
        Ok(AdcDiscoveryResult::NotFoundFallBackToDeveloper {
            reason: "EnterpriseCredentialsUnavailable".to_string(),
        })
    } else {
        Ok(AdcDiscoveryResult::NotFoundFallBackToDeveloper {
            reason: "MetadataServiceAllowedAttemptGce".to_string(),
        })
    }
}

/// Safely extracts enterprise authorization headers using google-cloud-auth without secret leakage.
pub async fn extract_enterprise_headers(
    credentials: &GAuthCredentials,
    correlation_id: &str,
) -> Result<HeaderMap, EnterpriseAuthError> {
    let cacheable = credentials
        .headers(AuthExtensions::new())
        .await
        .map_err(|e| {
            let err_str = e.to_string().to_lowercase();
            if err_str.contains("connect") || err_str.contains("network") || err_str.contains("dns")
            {
                EnterpriseAuthError::NetworkConnectivityError {
                    correlation_id: correlation_id.to_string(),
                    category: "network_connectivity_error",
                }
            } else if err_str.contains("consent")
                || err_str.contains("revoked")
                || err_str.contains("invalid_grant")
            {
                EnterpriseAuthError::OAuthConsentOrGrantRevoked {
                    correlation_id: correlation_id.to_string(),
                    category: "oauth_consent_or_grant_revoked",
                }
            } else {
                EnterpriseAuthError::TokenExchangeFailure {
                    correlation_id: correlation_id.to_string(),
                    category: "token_exchange_failure",
                    details: "OAuth token exchange or refresh failed".to_string(),
                }
            }
        })?;

    let http_headers = match cacheable {
        CacheableResource::New { data, .. } => data,
        CacheableResource::NotModified => {
            return Err(EnterpriseAuthError::TokenExchangeFailure {
                correlation_id: correlation_id.to_string(),
                category: "token_exchange_failure",
                details: "Unexpected NotModified response without entity tag".to_string(),
            });
        }
    };

    let mut out_headers = HeaderMap::new();
    for (name, val) in http_headers.iter() {
        if let Ok(hname) = HeaderName::from_bytes(name.as_str().as_bytes()) {
            if let Ok(hval) = HeaderValue::from_bytes(val.as_bytes()) {
                out_headers.insert(hname, hval);
            }
        }
    }

    Ok(out_headers)
}

#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_probe_outcome_variants() {
        assert_eq!(ProbeOutcome::Passed, ProbeOutcome::Passed);
        assert_ne!(ProbeOutcome::Passed, ProbeOutcome::Skipped);
        assert_ne!(ProbeOutcome::Skipped, ProbeOutcome::Failed);
    }

    #[test]
    fn test_windows_adc_discovery_missing_env_file_fails_closed() {
        let _lock = TEST_ENV_LOCK.blocking_lock();
        let non_existent = std::env::temp_dir().join("non_existent_adc_creds_9999.json");
        std::env::set_var("GOOGLE_APPLICATION_CREDENTIALS", &non_existent);

        let res = discover_windows_adc(false);
        std::env::remove_var("GOOGLE_APPLICATION_CREDENTIALS");

        match res {
            Err(EnterpriseAuthError::CredentialsFileNotFound {
                path,
                correlation_id,
            }) => {
                assert!(path.contains("non_existent_adc_creds_9999.json"));
                assert!(!correlation_id.is_empty());
            }
            other => panic!("Expected CredentialsFileNotFound, got {:?}", other),
        }
    }

    #[test]
    fn test_windows_adc_discovery_malformed_json_fails_closed() {
        let _lock = TEST_ENV_LOCK.blocking_lock();
        let temp_file = std::env::temp_dir().join("malformed_adc_creds.json");
        std::fs::write(&temp_file, "{ not valid json").unwrap();
        std::env::set_var("GOOGLE_APPLICATION_CREDENTIALS", &temp_file);

        let res = discover_windows_adc(false);
        std::env::remove_var("GOOGLE_APPLICATION_CREDENTIALS");
        let _ = std::fs::remove_file(temp_file);

        match res {
            Err(EnterpriseAuthError::MalformedCredentials {
                path,
                correlation_id,
                details,
            }) => {
                assert!(path.contains("malformed_adc_creds.json"));
                assert!(!correlation_id.is_empty());
                assert!(details.contains("Invalid JSON"));
            }
            other => panic!("Expected MalformedCredentials, got {:?}", other),
        }
    }

    #[test]
    fn test_windows_adc_discovery_valid_file() {
        let _lock = TEST_ENV_LOCK.blocking_lock();
        let temp_file = std::env::temp_dir().join("valid_test_adc.json");
        std::fs::write(
            &temp_file,
            r#"{"type": "authorized_user", "client_id": "test"}"#,
        )
        .unwrap();
        std::env::set_var("GOOGLE_APPLICATION_CREDENTIALS", &temp_file);

        let res = discover_windows_adc(false);
        std::env::remove_var("GOOGLE_APPLICATION_CREDENTIALS");
        let _ = std::fs::remove_file(temp_file);

        match res {
            Ok(AdcDiscoveryResult::Found(path)) => {
                assert!(path.to_string_lossy().contains("valid_test_adc.json"));
            }
            other => panic!("Expected AdcDiscoveryResult::Found, got {:?}", other),
        }
    }

    #[test]
    fn test_windows_adc_discovery_neither_exists_clean_fallback() {
        let _lock = TEST_ENV_LOCK.blocking_lock();
        std::env::remove_var("GOOGLE_APPLICATION_CREDENTIALS");
        let appdata_backup = std::env::var("APPDATA").ok();
        let empty_temp = std::env::temp_dir().join("empty_appdata_test_dir");
        let _ = std::fs::create_dir_all(&empty_temp);
        std::env::set_var("APPDATA", &empty_temp);

        let res = discover_windows_adc(false);
        if let Some(ref orig) = appdata_backup {
            std::env::set_var("APPDATA", orig);
        } else {
            std::env::remove_var("APPDATA");
        }
        let _ = std::fs::remove_dir_all(&empty_temp);

        assert_eq!(
            res,
            Ok(AdcDiscoveryResult::NotFoundFallBackToDeveloper {
                reason: "EnterpriseCredentialsUnavailable".to_string()
            })
        );
    }

    #[test]
    fn test_sanitized_enterprise_auth_error_no_secret_leakage() {
        let err = EnterpriseAuthError::TokenExchangeFailure {
            correlation_id: "test_corr_123".to_string(),
            category: "token_exchange_failure",
            details: "OAuth token exchange or refresh failed".to_string(),
        };
        let err_str = err.to_string();
        assert!(err_str.contains("test_corr_123"));
        assert!(err_str.contains("token_exchange_failure"));
        // Guarantee no actual bearer token pattern appears
        assert!(!err_str.contains("ya29."));
    }
}
