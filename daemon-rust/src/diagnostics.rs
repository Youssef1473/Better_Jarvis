use std::backtrace::Backtrace;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;
use tracing::error;

static CRASH_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Scrubs sensitive API keys, Bearer tokens, and secrets from panic messages or stack traces.
pub fn scrub_secrets(input: &str) -> String {
    let mut result = input.to_string();

    // 1. Scrub Google Gemini / AI Studio API Keys (AIzaSy...)
    while let Some(pos) = result.find("AIza") {
        let key_len = result[pos..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
            .count();
        if key_len >= 30 {
            result.replace_range(pos..pos + key_len, "[SCRUBBED_API_KEY]");
        } else {
            break;
        }
    }

    // 2. Scrub Bearer tokens
    let mut search_start = 0;
    while search_start < result.len() {
        let lower = result[search_start..].to_lowercase();
        if let Some(idx) = lower.find("bearer ") {
            let abs_idx = search_start + idx + 7;
            let token_len = result[abs_idx..]
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != '"' && *c != '\'')
                .count();
            if token_len > 0 {
                result.replace_range(abs_idx..abs_idx + token_len, "[SCRUBBED_TOKEN]");
                search_start = abs_idx + 16;
            } else {
                search_start = abs_idx + 1;
            }
        } else {
            break;
        }
    }

    // 3. Scrub sensitive assignments (api_key=, secret=, token=, password=)
    for sensitive_key in &["api_key", "apikey", "secret", "token", "password"] {
        let mut offset = 0;
        while offset < result.len() {
            let lower_curr = result[offset..].to_lowercase();
            let Some(k_pos) = lower_curr.find(sensitive_key) else {
                break;
            };
            let abs_pos = offset + k_pos + sensitive_key.len();
            let remaining = &result[abs_pos..];
            let trimmed = remaining.trim_start();
            let leading_spaces = remaining.len() - trimmed.len();
            if trimmed.starts_with('=') || trimmed.starts_with(':') {
                let sep_len = 1;
                let val_part = trimmed[sep_len..].trim_start();
                let sep_spaces = trimmed.len() - sep_len - val_part.len();
                let val_start = abs_pos + leading_spaces + sep_len + sep_spaces;

                let quote_char = if val_part.starts_with('"') {
                    Some('"')
                } else if val_part.starts_with('\'') {
                    Some('\'')
                } else {
                    None
                };

                let (v_start, v_len) = if let Some(q) = quote_char {
                    let inside = &val_part[1..];
                    let inside_len = inside.chars().take_while(|&c| c != q).count();
                    (val_start + 1, inside_len)
                } else {
                    let raw_len = val_part
                        .chars()
                        .take_while(|c| !c.is_whitespace() && *c != ',' && *c != '}')
                        .count();
                    (val_start, raw_len)
                };

                if v_len > 0 {
                    result.replace_range(v_start..v_start + v_len, "[SCRUBBED_SECRET]");
                    offset = v_start + 17;
                } else {
                    offset = abs_pos + 1;
                }
            } else {
                offset = abs_pos + 1;
            }
        }
    }

    result
}

/// Prunes older panic log files keeping strictly the max_keep newest files.
pub fn prune_panic_logs(crash_dir: &Path, max_keep: usize) {
    if !crash_dir.exists() || !crash_dir.is_dir() {
        return;
    }

    let Ok(entries) = fs::read_dir(crash_dir) else {
        return;
    };

    struct LogEntry {
        path: PathBuf,
        mtime: SystemTime,
    }

    let mut logs: Vec<LogEntry> = entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            let p = e.path();
            p.is_file()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("jarvis-daemon-panic-") && n.ends_with(".log"))
                    .unwrap_or(false)
        })
        .map(|e| {
            let mtime = e.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
            LogEntry {
                path: e.path(),
                mtime,
            }
        })
        .collect();

    if logs.len() <= max_keep {
        return;
    }

    // Sort newest first; break ties by path descending
    logs.sort_by(|a, b| {
        if a.mtime != b.mtime {
            b.mtime.cmp(&a.mtime)
        } else {
            b.path.cmp(&a.path)
        }
    });

    for entry in &logs[max_keep..] {
        let _ = fs::remove_file(&entry.path);
    }
}

/// Initializes the daemon's local-only panic hook with secret scrubbing and retention pruning.
pub fn init_panic_hook(crash_dir: &Path) {
    let _ = fs::create_dir_all(crash_dir);
    {
        let mut guard = CRASH_DIR.lock().unwrap();
        *guard = Some(crash_dir.to_path_buf());
    }

    std::panic::set_hook(Box::new(|info| {
        let crash_dir = {
            let guard = CRASH_DIR.lock().unwrap();
            guard.clone().unwrap_or_else(std::env::temp_dir)
        };

        let timestamp_secs = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let pid = std::process::id();
        let thread_name = std::thread::current()
            .name()
            .unwrap_or("<unnamed>")
            .to_string();

        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "Unknown panic payload".to_string()
        };

        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown location".to_string());

        let backtrace = Backtrace::force_capture();

        let raw_report = format!(
            "JARVIS DAEMON CRASH REPORT\n\
             ==========================\n\
             Epoch Seconds: {}\n\
             Process ID: {}\n\
             Thread: {}\n\
             Location: {}\n\
             Payload: {}\n\n\
             Stack Backtrace:\n\
             {}\n",
            timestamp_secs,
            pid,
            thread_name,
            location,
            payload,
            backtrace
        );

        let scrubbed_report = scrub_secrets(&raw_report);

        let log_filename = format!("jarvis-daemon-panic-{}-pid{}.log", timestamp_secs, pid);
        let log_path = crash_dir.join(log_filename);

        if let Ok(()) = fs::write(&log_path, scrubbed_report) {
            prune_panic_logs(&crash_dir, 5);
        }

        error!("Daemon panicked: {} at {}", payload, location);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_secret_scrubbing() {
        let raw = "Crash while calling Gemini with AIzaSyD3f4uLtK3yN0tRe4lF0rTeSt1ng12345678 \
                   and Authorization: Bearer secret_jwt_token_12345 \
                   and GEMINI_API_KEY=\"AIzaSyCustomKeyForTestingPurposes12345\" \
                   and api_key: 'super_secret_api_key_value_98765'";

        let scrubbed = scrub_secrets(raw);

        assert!(!scrubbed.contains("AIzaSyD3f4uLtK3yN0tRe4lF0rTeSt1ng12345678"));
        assert!(!scrubbed.contains("AIzaSyCustomKeyForTestingPurposes12345"));
        assert!(!scrubbed.contains("secret_jwt_token_12345"));
        assert!(!scrubbed.contains("super_secret_api_key_value_98765"));

        assert!(scrubbed.contains("[SCRUBBED_API_KEY]"));
        assert!(scrubbed.contains("Bearer [SCRUBBED_TOKEN]"));
        assert!(scrubbed.contains("[SCRUBBED_SECRET]"));
    }

    #[test]
    fn test_panic_retention_pruning() {
        let temp_dir = std::env::temp_dir().join(format!(
            "jarvis_test_panic_retention_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        let _ = fs::create_dir_all(&temp_dir);

        let base_time = SystemTime::now();
        let mut created = Vec::new();

        // Create 8 dummy panic logs
        for i in 1..=8 {
            let path = temp_dir.join(format!("jarvis-daemon-panic-20261124-00000{}-pid100{}.log", i, i));
            fs::write(&path, format!("CRASH REPORT {}", i)).unwrap();

            // Set file modified time (1 is oldest, 8 is newest)
            let mtime = base_time + std::time::Duration::from_secs(10 * i);
            let file = fs::File::open(&path).unwrap();
            let _ = file.set_modified(mtime);

            created.push(path);
        }

        prune_panic_logs(&temp_dir, 5);

        // Verify strictly 5 newest remain
        let remaining: Vec<_> = fs::read_dir(&temp_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(remaining.len(), 5);

        // Files 1, 2, 3 must be deleted; Files 6, 7, 8 must exist
        assert!(!created[0].exists());
        assert!(!created[1].exists());
        assert!(!created[2].exists());
        assert!(created[7].exists());
        assert!(created[6].exists());

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
