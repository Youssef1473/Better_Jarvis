use crate::security::policy::{PolicyEngine, SecurityTier};
use crate::security::sandbox;
use crate::tools::os_control::{OsController, OsMediaAction};
use crate::tools::{browser_cdp, confirmation::ConfirmationManager};
use anyhow::Result;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedIntent {
    VolumeUp,
    VolumeDown,
    VolumeMute,
    MediaPlayPause,
    MediaNextTrack,
    MediaPrevTrack,
    OpenYouTube { query: Option<String> },
    OpenUrl { url: String },
    LaunchApp { name: String, target: String },
    TerminalCommand { command: String },
    NeedsLlm { prompt: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteExecutionResult {
    Executed {
        intent: String,
        output: String,
        tier: SecurityTier,
    },
    Denied {
        intent: String,
        reason: String,
    },
    FallbackToLlm {
        prompt: String,
    },
}

/// Normalizes voice/text input by trimming, lowercasing, and stripping punctuation.
fn clean_input(raw: &str) -> String {
    let mut s = raw.trim().to_lowercase();
    while s.ends_with('.') || s.ends_with('?') || s.ends_with('!') || s.ends_with(',') {
        s.pop();
    }
    s.trim().to_string()
}

/// Deterministic, zero-LLM fast intent parser (< 5 ms execution).
pub fn parse_intent(raw: &str) -> ParsedIntent {
    let cleaned = clean_input(raw);
    if cleaned.is_empty() {
        return ParsedIntent::NeedsLlm {
            prompt: raw.to_string(),
        };
    }

    // 1. Volume Controls
    if cleaned == "volume up"
        || cleaned == "turn up volume"
        || cleaned == "turn up the volume"
        || cleaned == "increase volume"
        || cleaned == "louder"
        || cleaned == "turn it up"
        || cleaned == "volume higher"
    {
        return ParsedIntent::VolumeUp;
    }

    if cleaned == "volume down"
        || cleaned == "turn down volume"
        || cleaned == "turn down the volume"
        || cleaned == "decrease volume"
        || cleaned == "lower volume"
        || cleaned == "quieter"
        || cleaned == "turn it down"
    {
        return ParsedIntent::VolumeDown;
    }

    if cleaned == "mute"
        || cleaned == "unmute"
        || cleaned == "mute volume"
        || cleaned == "mute audio"
        || cleaned == "silence audio"
        || cleaned == "silence"
    {
        return ParsedIntent::VolumeMute;
    }

    // 2. Media Transport
    if cleaned == "play"
        || cleaned == "pause"
        || cleaned == "resume"
        || cleaned == "play music"
        || cleaned == "pause music"
        || cleaned == "stop music"
        || cleaned == "play audio"
        || cleaned == "pause audio"
    {
        return ParsedIntent::MediaPlayPause;
    }

    if cleaned == "next song"
        || cleaned == "next track"
        || cleaned == "skip"
        || cleaned == "skip song"
        || cleaned == "skip track"
        || cleaned == "play next song"
    {
        return ParsedIntent::MediaNextTrack;
    }

    if cleaned == "previous song"
        || cleaned == "previous track"
        || cleaned == "prev song"
        || cleaned == "prev track"
        || cleaned == "last song"
        || cleaned == "go back"
    {
        return ParsedIntent::MediaPrevTrack;
    }

    // 3. YouTube Queries & Navigation
    if cleaned == "open youtube"
        || cleaned == "go to youtube"
        || cleaned == "launch youtube"
        || cleaned == "start youtube"
    {
        return ParsedIntent::OpenYouTube { query: None };
    }

    if let Some(query) = cleaned.strip_prefix("search youtube for ") {
        return ParsedIntent::OpenYouTube {
            query: Some(query.trim().to_string()),
        };
    }
    if let Some(query) = cleaned.strip_prefix("search on youtube for ") {
        return ParsedIntent::OpenYouTube {
            query: Some(query.trim().to_string()),
        };
    }
    if let Some(query) = cleaned.strip_prefix("youtube search ") {
        return ParsedIntent::OpenYouTube {
            query: Some(query.trim().to_string()),
        };
    }
    if let Some(query) = cleaned.strip_prefix("youtube ") {
        return ParsedIntent::OpenYouTube {
            query: Some(query.trim().to_string()),
        };
    }
    if cleaned.starts_with("play ") && cleaned.ends_with(" on youtube") {
        let middle = &cleaned[5..cleaned.len() - 11];
        return ParsedIntent::OpenYouTube {
            query: Some(middle.trim().to_string()),
        };
    }

    // 4. URL Navigation
    if cleaned.starts_with("http://") || cleaned.starts_with("https://") {
        return ParsedIntent::OpenUrl { url: cleaned };
    }
    if let Some(target) = cleaned.strip_prefix("open ") {
        let t = target.trim();
        if t.contains(".com") || t.contains(".org") || t.contains(".net") || t.contains(".io") || t.contains(".dev") {
            let url = if t.starts_with("http://") || t.starts_with("https://") {
                t.to_string()
            } else {
                format!("https://{}", t)
            };
            return ParsedIntent::OpenUrl { url };
        }
    }

    // 5. Common Applications
    if cleaned == "open spotify" || cleaned == "launch spotify" {
        return ParsedIntent::LaunchApp {
            name: "Spotify".into(),
            target: "spotify:".into(),
        };
    }
    if cleaned == "open discord" || cleaned == "launch discord" {
        return ParsedIntent::LaunchApp {
            name: "Discord".into(),
            target: "discord:".into(),
        };
    }
    if cleaned == "open calculator" || cleaned == "launch calculator" || cleaned == "open calc" || cleaned == "calc" {
        return ParsedIntent::LaunchApp {
            name: "Calculator".into(),
            target: "calc.exe".into(),
        };
    }
    if cleaned == "open notepad" || cleaned == "launch notepad" {
        return ParsedIntent::LaunchApp {
            name: "Notepad".into(),
            target: "notepad.exe".into(),
        };
    }
    if cleaned == "open terminal" || cleaned == "launch terminal" || cleaned == "open powershell" {
        return ParsedIntent::LaunchApp {
            name: "Windows Terminal".into(),
            target: "wt.exe".into(),
        };
    }
    if cleaned == "open settings" || cleaned == "launch settings" || cleaned == "windows settings" {
        return ParsedIntent::LaunchApp {
            name: "Windows Settings".into(),
            target: "ms-settings:".into(),
        };
    }

    // 6. Fast Safe Terminal Queries (Read-Only)
    if cleaned == "git status" || cleaned == "check git" || cleaned == "check git status" || cleaned == "show git status" {
        return ParsedIntent::TerminalCommand {
            command: "git status".into(),
        };
    }
    if cleaned == "git diff" || cleaned == "show git diff" {
        return ParsedIntent::TerminalCommand {
            command: "git diff".into(),
        };
    }
    if cleaned == "git log" || cleaned == "show git log" {
        return ParsedIntent::TerminalCommand {
            command: "git log -n 5 --oneline".into(),
        };
    }
    if cleaned == "ipconfig" || cleaned == "check ip" || cleaned == "what is my ip" {
        return ParsedIntent::TerminalCommand {
            command: "ipconfig".into(),
        };
    }

    // Default: Forward ambiguous or natural language requests to LLM
    ParsedIntent::NeedsLlm {
        prompt: raw.to_string(),
    }
}

/// Executes a parsed intent, respecting 3-tier security policies and confirmation.
pub async fn execute_intent(
    intent: &ParsedIntent,
    policy: &PolicyEngine,
    confirmation_mgr: &ConfirmationManager,
    workspace_dir: Option<&Path>,
    os_controller: &dyn OsController,
) -> Result<RouteExecutionResult> {
    match intent {
        ParsedIntent::VolumeUp => {
            os_controller.control_media(OsMediaAction::VolumeUp).await?;
            Ok(RouteExecutionResult::Executed {
                intent: "VolumeUp".into(),
                output: "Volume increased".into(),
                tier: SecurityTier::Green,
            })
        }
        ParsedIntent::VolumeDown => {
            os_controller.control_media(OsMediaAction::VolumeDown).await?;
            Ok(RouteExecutionResult::Executed {
                intent: "VolumeDown".into(),
                output: "Volume decreased".into(),
                tier: SecurityTier::Green,
            })
        }
        ParsedIntent::VolumeMute => {
            os_controller.control_media(OsMediaAction::VolumeMuteToggle).await?;
            Ok(RouteExecutionResult::Executed {
                intent: "VolumeMute".into(),
                output: "Volume mute toggled".into(),
                tier: SecurityTier::Green,
            })
        }
        ParsedIntent::MediaPlayPause => {
            os_controller.control_media(OsMediaAction::MediaPlayPause).await?;
            Ok(RouteExecutionResult::Executed {
                intent: "MediaPlayPause".into(),
                output: "Media playback toggled".into(),
                tier: SecurityTier::Green,
            })
        }
        ParsedIntent::MediaNextTrack => {
            os_controller.control_media(OsMediaAction::MediaNextTrack).await?;
            Ok(RouteExecutionResult::Executed {
                intent: "MediaNextTrack".into(),
                output: "Skipped to next track".into(),
                tier: SecurityTier::Green,
            })
        }
        ParsedIntent::MediaPrevTrack => {
            os_controller.control_media(OsMediaAction::MediaPrevTrack).await?;
            Ok(RouteExecutionResult::Executed {
                intent: "MediaPrevTrack".into(),
                output: "Returned to previous track".into(),
                tier: SecurityTier::Green,
            })
        }
        ParsedIntent::OpenYouTube { query } => {
            browser_cdp::open_youtube(query.as_deref()).await?;
            let output = match query {
                Some(q) => format!("Opened YouTube search for \"{}\"", q),
                None => "Opened YouTube".into(),
            };
            Ok(RouteExecutionResult::Executed {
                intent: "OpenYouTube".into(),
                output,
                tier: SecurityTier::Green,
            })
        }
        ParsedIntent::OpenUrl { url } => {
            browser_cdp::open_url(url).await?;
            Ok(RouteExecutionResult::Executed {
                intent: "OpenUrl".into(),
                output: format!("Opened URL: {}", url),
                tier: SecurityTier::Green,
            })
        }
        ParsedIntent::LaunchApp { name, target } => {
            os_controller.launch_target(target).await?;
            Ok(RouteExecutionResult::Executed {
                intent: "LaunchApp".into(),
                output: format!("Launched {}", name),
                tier: SecurityTier::Green,
            })
        }
        ParsedIntent::TerminalCommand { command } => {
            let action_policy = policy.evaluate_command(command);
            match action_policy.tier {
                SecurityTier::Red => Ok(RouteExecutionResult::Denied {
                    intent: format!("Command: {}", command),
                    reason: action_policy.reason,
                }),
                SecurityTier::Amber => {
                    let approved = confirmation_mgr
                        .request_confirmation(&format!("Execute: {}", command), 8000)
                        .await;
                    if !approved {
                        return Ok(RouteExecutionResult::Denied {
                            intent: format!("Command: {}", command),
                            reason: "Action denied or confirmation timed out".into(),
                        });
                    }
                    let out = sandbox::run_guarded_command(command, workspace_dir, 10).await?;
                    let text = if !out.stdout.is_empty() {
                        out.stdout
                    } else {
                        out.stderr
                    };
                    Ok(RouteExecutionResult::Executed {
                        intent: format!("Command: {}", command),
                        output: text,
                        tier: SecurityTier::Amber,
                    })
                }
                SecurityTier::Green => {
                    let out = sandbox::run_guarded_command(command, workspace_dir, 10).await?;
                    let text = if !out.stdout.is_empty() {
                        out.stdout
                    } else {
                        out.stderr
                    };
                    Ok(RouteExecutionResult::Executed {
                        intent: format!("Command: {}", command),
                        output: text,
                        tier: SecurityTier::Green,
                    })
                }
            }
        }
        ParsedIntent::NeedsLlm { prompt } => Ok(RouteExecutionResult::FallbackToLlm {
            prompt: prompt.clone(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn test_intent_parsing_latency_under_5ms() {
        let queries = [
            "volume up",
            "search youtube for lofi chill beats",
            "open spotify",
            "git status",
            "turn down the volume",
            "play synthwave on youtube",
            "what is the meaning of life?",
        ];

        for query in queries {
            let start = Instant::now();
            let _ = parse_intent(query);
            let elapsed = start.elapsed();
            assert!(
                elapsed.as_millis() < 5,
                "Intent parsing for '{}' took {:?}, exceeding 5ms target",
                query,
                elapsed
            );
        }
    }

    #[test]
    fn test_volume_and_media_routing() {
        assert_eq!(parse_intent("Volume up!"), ParsedIntent::VolumeUp);
        assert_eq!(parse_intent("turn down volume"), ParsedIntent::VolumeDown);
        assert_eq!(parse_intent("mute"), ParsedIntent::VolumeMute);
        assert_eq!(parse_intent("play"), ParsedIntent::MediaPlayPause);
        assert_eq!(parse_intent("next track"), ParsedIntent::MediaNextTrack);
        assert_eq!(parse_intent("previous song"), ParsedIntent::MediaPrevTrack);
    }

    #[test]
    fn test_youtube_routing() {
        assert_eq!(
            parse_intent("open youtube"),
            ParsedIntent::OpenYouTube { query: None }
        );
        assert_eq!(
            parse_intent("search youtube for ambient focus"),
            ParsedIntent::OpenYouTube {
                query: Some("ambient focus".into())
            }
        );
        assert_eq!(
            parse_intent("play jazz on youtube"),
            ParsedIntent::OpenYouTube {
                query: Some("jazz".into())
            }
        );
    }

    #[test]
    fn test_app_launch_routing() {
        assert_eq!(
            parse_intent("open spotify"),
            ParsedIntent::LaunchApp {
                name: "Spotify".into(),
                target: "spotify:".into(),
            }
        );
        assert_eq!(
            parse_intent("launch discord"),
            ParsedIntent::LaunchApp {
                name: "Discord".into(),
                target: "discord:".into(),
            }
        );
        assert_eq!(
            parse_intent("calc"),
            ParsedIntent::LaunchApp {
                name: "Calculator".into(),
                target: "calc.exe".into(),
            }
        );
    }

    #[test]
    fn test_git_status_fast_terminal() {
        assert_eq!(
            parse_intent("git status"),
            ParsedIntent::TerminalCommand {
                command: "git status".into()
            }
        );
        assert_eq!(
            parse_intent("check git"),
            ParsedIntent::TerminalCommand {
                command: "git status".into()
            }
        );
    }

    #[test]
    fn test_fallback_to_llm() {
        let complex = "Draft an email to Alex explaining why our deployment was delayed";
        assert_eq!(
            parse_intent(complex),
            ParsedIntent::NeedsLlm {
                prompt: complex.into()
            }
        );
    }
}
