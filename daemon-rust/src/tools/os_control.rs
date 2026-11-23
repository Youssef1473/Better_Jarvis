use anyhow::Result;
use futures::future::BoxFuture;
use std::ffi::c_void;
use tokio::sync::{mpsc, oneshot};
use tracing::info;

#[repr(C)]
struct KeybdInput {
    w_vk: u16,
    w_scan: u16,
    dw_flags: u32,
    time: u32,
    dw_extra_info: usize,
}

#[repr(C)]
union InputUnion {
    ki: std::mem::ManuallyDrop<KeybdInput>,
    padding: [u8; 32],
}

#[repr(C)]
struct Input {
    r#type: u32,
    u: InputUnion,
}

const INPUT_KEYBOARD: u32 = 1;
const KEYEVENTF_KEYUP: u32 = 0x0002;

// Virtual Keycodes for Media & Audio Transport
pub const VK_VOLUME_MUTE: u16 = 0xAD;
pub const VK_VOLUME_DOWN: u16 = 0xAE;
pub const VK_VOLUME_UP: u16 = 0xAF;
pub const VK_MEDIA_NEXT_TRACK: u16 = 0xB0;
pub const VK_MEDIA_PREV_TRACK: u16 = 0xB1;
pub const VK_MEDIA_PLAY_PAUSE: u16 = 0xB3;

const COINIT_APARTMENTTHREADED: u32 = 0x2;
const RPC_E_CHANGED_MODE: i32 = -2147417850; // 0x80010106

#[link(name = "ole32")]
#[link(name = "user32")]
extern "system" {
    fn SendInput(c_inputs: u32, p_inputs: *const Input, cb_size: i32) -> u32;
    fn CoInitializeEx(pv_reserved: *mut c_void, dw_co_init: u32) -> i32;
    fn CoUninitialize();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsMediaAction {
    VolumeUp,
    VolumeDown,
    VolumeMuteToggle,
    MediaPlayPause,
    MediaNextTrack,
    MediaPrevTrack,
}

/// Abstract OS control interface decoupled from execution threading
pub trait OsController: Send + Sync {
    fn control_media(&self, action: OsMediaAction) -> BoxFuture<'static, Result<()>>;
    fn launch_target(&self, target: &str) -> BoxFuture<'static, Result<()>>;
    fn shutdown(&self) -> BoxFuture<'static, Result<()>>;
}

enum OsStaCommand {
    Media(OsMediaAction, oneshot::Sender<Result<()>>),
    Launch(String, oneshot::Sender<Result<()>>),
    Shutdown(oneshot::Sender<Result<()>>),
}

/// Production Win32 Single-Threaded Apartment (STA) OS Controller.
/// All Win32 COM, SendInput, and explorer process invocations run on a dedicated OS thread.
#[derive(Clone)]
pub struct Win32StaOsController {
    sender: mpsc::Sender<OsStaCommand>,
}

pub const ALLOWED_DESKTOP_APPS: &[&str] = &[
    "notepad", "notepad.exe",
    "calculator", "calc", "calc.exe",
    "terminal", "wt", "wt.exe",
    "spotify", "spotify:",
    "settings", "windows settings", "ms-settings:",
    "explorer",
    "code",
    "chrome",
];

/// Validates that an app launch target matches the strict allowlist of benign desktop applications.
pub fn validate_launch_target(target: &str) -> Result<()> {
    let trimmed = target.trim();
    if trimmed.is_empty() {
        anyhow::bail!("Cannot launch empty target");
    }

    let lower = trimmed.to_lowercase();

    // Reject arbitrary paths, shell scripts, and unapproved URI schemes
    if lower.contains('\\')
        || lower.contains('/')
        || lower.ends_with(".bat")
        || lower.ends_with(".ps1")
        || lower.ends_with(".cmd")
        || lower.ends_with(".vbs")
        || lower.ends_with(".js")
        || lower.starts_with(r"\\")
        || lower.starts_with("file:")
        || lower.starts_with("http:")
        || lower.starts_with("https:")
    {
        anyhow::bail!("Arbitrary paths, scripts, and URI schemes are forbidden: {}", target);
    }

    // Zero-argument guarantee: Reject any CLI flags, arguments, or switches
    if lower.contains(" -")
        || lower.contains(" /")
        || lower.contains("--")
        || lower.contains('&')
        || lower.contains('|')
        || lower.contains(';')
    {
        anyhow::bail!("Command-line flags or arguments are forbidden in launch_target: {}", target);
    }

    if !ALLOWED_DESKTOP_APPS.contains(&lower.as_str()) {
        anyhow::bail!(
            "Target '{}' is not on the allowed desktop application launch list. Permitted apps: {:?}",
            target,
            ALLOWED_DESKTOP_APPS
        );
    }

    Ok(())
}

impl Win32StaOsController {
    pub fn spawn() -> Result<Self> {
        let (tx, mut rx) = mpsc::channel::<OsStaCommand>(32);
        let (init_tx, init_rx) = std::sync::mpsc::channel();

        std::thread::Builder::new()
            .name("jarvis-win32-sta".into())
            .spawn(move || {
                let hr = unsafe { CoInitializeEx(std::ptr::null_mut(), COINIT_APARTMENTTHREADED) };
                if hr < 0 && hr != RPC_E_CHANGED_MODE {
                    let _ = init_tx.send(Err(anyhow::anyhow!(
                        "CoInitializeEx(APARTMENTTHREADED) failed: hr=0x{:08X}",
                        hr
                    )));
                    return;
                }
                info!("Dedicated Win32 STA OS worker thread initialized");
                let _ = init_tx.send(Ok(()));

                while let Some(cmd) = rx.blocking_recv() {
                    match cmd {
                        OsStaCommand::Media(action, reply) => {
                            let res = Self::sync_execute_media_action(action);
                            let _ = reply.send(res);
                        }
                        OsStaCommand::Launch(target, reply) => {
                            let res = Self::sync_execute_launch_target(&target);
                            let _ = reply.send(res);
                        }
                        OsStaCommand::Shutdown(reply) => {
                            unsafe {
                                CoUninitialize();
                            }
                            let _ = reply.send(Ok(()));
                            info!("Win32 STA thread cleanly uninitialized and exited");
                            return;
                        }
                    }
                }

                unsafe {
                    CoUninitialize();
                }
            })?;

        init_rx.recv().map_err(|e| anyhow::anyhow!("STA init channel error: {e}"))??;

        Ok(Self { sender: tx })
    }

    fn sync_send_virtual_key_stroke(vk: u16) -> Result<()> {
        let mut inputs: [Input; 2] = [
            Input {
                r#type: INPUT_KEYBOARD,
                u: InputUnion {
                    ki: std::mem::ManuallyDrop::new(KeybdInput {
                        w_vk: vk,
                        w_scan: 0,
                        dw_flags: 0,
                        time: 0,
                        dw_extra_info: 0,
                    }),
                },
            },
            Input {
                r#type: INPUT_KEYBOARD,
                u: InputUnion {
                    ki: std::mem::ManuallyDrop::new(KeybdInput {
                        w_vk: vk,
                        w_scan: 0,
                        dw_flags: KEYEVENTF_KEYUP,
                        time: 0,
                        dw_extra_info: 0,
                    }),
                },
            },
        ];

        let sent = unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_mut_ptr(),
                std::mem::size_of::<Input>() as i32,
            )
        };

        if sent != inputs.len() as u32 {
            anyhow::bail!("SendInput failed to send keycode 0x{:02X}", vk);
        }
        Ok(())
    }

    fn sync_execute_media_action(action: OsMediaAction) -> Result<()> {
        match action {
            OsMediaAction::VolumeUp => {
                Self::sync_send_virtual_key_stroke(VK_VOLUME_UP)?;
                Self::sync_send_virtual_key_stroke(VK_VOLUME_UP)
            }
            OsMediaAction::VolumeDown => {
                Self::sync_send_virtual_key_stroke(VK_VOLUME_DOWN)?;
                Self::sync_send_virtual_key_stroke(VK_VOLUME_DOWN)
            }
            OsMediaAction::VolumeMuteToggle => Self::sync_send_virtual_key_stroke(VK_VOLUME_MUTE),
            OsMediaAction::MediaPlayPause => Self::sync_send_virtual_key_stroke(VK_MEDIA_PLAY_PAUSE),
            OsMediaAction::MediaNextTrack => Self::sync_send_virtual_key_stroke(VK_MEDIA_NEXT_TRACK),
            OsMediaAction::MediaPrevTrack => Self::sync_send_virtual_key_stroke(VK_MEDIA_PREV_TRACK),
        }
    }

    fn sync_execute_launch_target(target: &str) -> Result<()> {
        let trimmed = target.trim();
        validate_launch_target(trimmed)?;

        let lower = trimmed.to_lowercase();
        match lower.as_str() {
            "notepad" | "notepad.exe" => {
                std::process::Command::new("notepad.exe").spawn()?;
            }
            "calculator" | "calc" | "calc.exe" => {
                std::process::Command::new("calc.exe").spawn()?;
            }
            "terminal" | "wt" | "wt.exe" => {
                std::process::Command::new("wt.exe").spawn()?;
            }
            "spotify" | "spotify:" => {
                std::process::Command::new("explorer.exe")
                    .arg("spotify:")
                    .spawn()?;
            }
            "settings" | "windows settings" | "ms-settings:" => {
                std::process::Command::new("explorer.exe")
                    .arg("ms-settings:")
                    .spawn()?;
            }
            "explorer" => {
                std::process::Command::new("explorer.exe").spawn()?;
            }
            "code" => {
                std::process::Command::new("code.cmd").spawn()?;
            }
            "chrome" => {
                std::process::Command::new("chrome.exe").spawn()?;
            }
            other => {
                anyhow::bail!("Target '{}' is not an allowed application", other);
            }
        }

        Ok(())
    }
}

impl OsController for Win32StaOsController {
    fn control_media(&self, action: OsMediaAction) -> BoxFuture<'static, Result<()>> {
        let sender = self.sender.clone();
        Box::pin(async move {
            let (reply, rx) = oneshot::channel();
            sender
                .send(OsStaCommand::Media(action, reply))
                .await
                .map_err(|_| anyhow::anyhow!("STA worker thread disconnected"))?;
            rx.await.map_err(|_| anyhow::anyhow!("STA worker dropped reply"))?
        })
    }

    fn launch_target(&self, target: &str) -> BoxFuture<'static, Result<()>> {
        let sender = self.sender.clone();
        let target_str = target.to_string();
        Box::pin(async move {
            let (reply, rx) = oneshot::channel();
            sender
                .send(OsStaCommand::Launch(target_str, reply))
                .await
                .map_err(|_| anyhow::anyhow!("STA worker thread disconnected"))?;
            rx.await.map_err(|_| anyhow::anyhow!("STA worker dropped reply"))?
        })
    }

    fn shutdown(&self) -> BoxFuture<'static, Result<()>> {
        let sender = self.sender.clone();
        Box::pin(async move {
            let (reply, rx) = oneshot::channel();
            if sender.send(OsStaCommand::Shutdown(reply)).await.is_err() {
                return Ok(());
            }
            rx.await.map_err(|_| anyhow::anyhow!("STA worker dropped reply"))?
        })
    }
}

/// Mock OS Controller for unit tests and headless environments
#[cfg(test)]
#[derive(Default, Clone)]
pub struct MockOsController {
    pub last_action: std::sync::Arc<std::sync::Mutex<Option<OsMediaAction>>>,
    pub last_target: std::sync::Arc<std::sync::Mutex<Option<String>>>,
}

#[cfg(test)]
impl OsController for MockOsController {
    fn control_media(&self, action: OsMediaAction) -> BoxFuture<'static, Result<()>> {
        let last = self.last_action.clone();
        Box::pin(async move {
            *last.lock().unwrap() = Some(action);
            Ok(())
        })
    }

    fn launch_target(&self, target: &str) -> BoxFuture<'static, Result<()>> {
        let last = self.last_target.clone();
        let t = target.to_string();
        Box::pin(async move {
            validate_launch_target(&t)?;
            *last.lock().unwrap() = Some(t);
            Ok(())
        })
    }

    fn shutdown(&self) -> BoxFuture<'static, Result<()>> {
        Box::pin(async move { Ok(()) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_controller_records_actions() {
        let mock = MockOsController::default();
        mock.control_media(OsMediaAction::VolumeUp).await.unwrap();
        assert_eq!(*mock.last_action.lock().unwrap(), Some(OsMediaAction::VolumeUp));

        mock.launch_target("spotify:").await.unwrap();
        assert_eq!(*mock.last_target.lock().unwrap(), Some("spotify:".into()));
    }

    #[tokio::test]
    async fn test_sta_controller_lifecycle() {
        let controller = Win32StaOsController::spawn().expect("STA thread spawned");
        controller.shutdown().await.expect("STA thread clean shutdown");
    }

    #[test]
    fn test_arbitrary_launch_target_rejected() {
        // Raw executables and scripts must be rejected
        assert!(validate_launch_target("cmd.exe").is_err());
        assert!(validate_launch_target("powershell.exe").is_err());
        assert!(validate_launch_target("C:\\Windows\\System32\\cmd.exe").is_err());
        assert!(validate_launch_target("evil.bat").is_err());
        assert!(validate_launch_target("script.ps1").is_err());
        assert!(validate_launch_target("macro.vbs").is_err());
        assert!(validate_launch_target(r"\\remote\share\tool.exe").is_err());
        assert!(validate_launch_target("file:///C:/secret.txt").is_err());
        assert!(validate_launch_target("http://malicious.site").is_err());

        // Zero-argument guarantee: Reject flags, switches, and argument injection
        assert!(validate_launch_target("wt -- cmd /c calc").is_err());
        assert!(validate_launch_target("explorer.exe /root,C:\\").is_err());
        assert!(validate_launch_target("chrome.exe --no-sandbox").is_err());
        assert!(validate_launch_target("notepad.exe C:\\secret.txt").is_err());

        // Allowed named desktop apps must pass
        assert!(validate_launch_target("notepad").is_ok());
        assert!(validate_launch_target("calc").is_ok());
        assert!(validate_launch_target("spotify").is_ok());
        assert!(validate_launch_target("terminal").is_ok());
        assert!(validate_launch_target("settings").is_ok());
    }
}
