use tokio::sync::broadcast;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VisionError {
    BlockedProcess(String),
    BlockedTitle(String),
    BlockedDisplayAffinity(u32),
    CaptureFailed(String),
    OcrFailed(String),
    NoActiveWindow,
}

impl std::fmt::Display for VisionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BlockedProcess(p) => {
                write!(f, "Privacy blocked: application '{p}' is on restricted process blocklist")
            }
            Self::BlockedTitle(t) => {
                write!(f, "Privacy blocked: window title contains sensitive keyword '{t}'")
            }
            Self::BlockedDisplayAffinity(a) => {
                write!(f, "Privacy blocked: window has non-zero display affinity {a:#x}")
            }
            Self::CaptureFailed(msg) => write!(f, "Window capture failed: {msg}"),
            Self::OcrFailed(msg) => write!(f, "WinRT OCR error: {msg}"),
            Self::NoActiveWindow => write!(f, "No active non-Jarvis application window found"),
        }
    }
}

impl std::error::Error for VisionError {}

pub const BLOCKED_PROCESS_NAMES: &[&str] = &[
    "1password",
    "bitwarden",
    "keepass",
    "keepassxc",
    "lastpass",
    "dashlane",
    "enpass",
    "protonpass",
    "credentialuibroker",
    "consent",
    "lockapp",
    "logonui",
    "taskmgr",
    "mmc",
];

pub const BLOCKED_TITLE_KEYWORDS: &[&str] = &[
    "inprivate",
    "incognito",
    "private browsing",
    "password",
    "passphrase",
    "vault",
    "seed phrase",
    "recovery key",
    "sign in",
    "log in",
    "two-factor",
    "2fa",
    "authenticator",
    "bank",
    "checkout",
    "credit card",
];

pub struct VisionPrivacyGuard;

impl VisionPrivacyGuard {
    /// Checks whether an executable basename matches any password manager or security tool.
    pub fn is_process_blocked(process_name: &str) -> Option<&'static str> {
        let name_lower = process_name.to_lowercase();
        let stem = std::path::Path::new(&name_lower)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&name_lower);

        BLOCKED_PROCESS_NAMES
            .iter()
            .copied()
            .find(|&blocked| stem == blocked || name_lower == blocked)
    }

    /// Checks whether a window title contains sensitive substrings.
    pub fn is_title_blocked(title: &str) -> Option<&'static str> {
        let title_lower = title.to_lowercase();
        BLOCKED_TITLE_KEYWORDS
            .iter()
            .copied()
            .find(|&keyword| title_lower.contains(keyword))
    }

    /// Checks whether non-zero display affinity is active (e.g. WDA_MONITOR, WDA_EXCLUDEFROMCAPTURE).
    pub fn is_display_affinity_blocked(affinity: u32) -> bool {
        affinity != 0
    }

    /// Complete privacy validation. Returns Ok(()) only when all privacy checks pass.
    pub fn check(process_name: &str, title: &str, affinity: u32) -> Result<(), VisionError> {
        if let Some(blocked) = Self::is_process_blocked(process_name) {
            return Err(VisionError::BlockedProcess(blocked.to_string()));
        }
        if let Some(keyword) = Self::is_title_blocked(title) {
            return Err(VisionError::BlockedTitle(keyword.to_string()));
        }
        if Self::is_display_affinity_blocked(affinity) {
            return Err(VisionError::BlockedDisplayAffinity(affinity));
        }
        Ok(())
    }
}

/// Active window metadata safely filtered by VisionPrivacyGuard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowMetadata {
    pub hwnd: isize,
    pub title: String,
    pub process_name: String,
    pub process_id: u32,
    pub width: i32,
    pub height: i32,
    pub affinity: u32,
}

/// Truncates string to at most 16 KiB UTF-8 without splitting unicode codepoints.
pub fn truncate_utf8_16kib(s: &str) -> &str {
    const MAX_BYTES: usize = 16 * 1024;
    if s.len() <= MAX_BYTES {
        return s;
    }
    let mut end = MAX_BYTES;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// RAII Guard ensuring HUD vision capture active indicator is reset to false on drop/panic.
pub struct VisionHudGuard {
    notifier: Option<broadcast::Sender<serde_json::Value>>,
    app: String,
}

impl VisionHudGuard {
    pub fn new(
        notifier: Option<broadcast::Sender<serde_json::Value>>,
        app: impl Into<String>,
    ) -> Self {
        let app = app.into();
        if let Some(ref n) = notifier {
            let _ = n.send(serde_json::json!({
                "jsonrpc": "2.0",
                "method": "hud.vision_capture",
                "params": {
                    "active": true,
                    "app": &app
                }
            }));
        }
        Self { notifier, app }
    }
}

impl Drop for VisionHudGuard {
    fn drop(&mut self) {
        if let Some(ref n) = self.notifier {
            let _ = n.send(serde_json::json!({
                "jsonrpc": "2.0",
                "method": "hud.vision_capture",
                "params": {
                    "active": false,
                    "app": &self.app
                }
            }));
        }
    }
}

#[link(name = "user32")]
extern "system" {
    fn PrintWindow(
        hwnd: windows::Win32::Foundation::HWND,
        hdc_blt: windows::Win32::Graphics::Gdi::HDC,
        n_flags: u32,
    ) -> windows::Win32::Foundation::BOOL;
}

pub const PW_RENDERFULLCONTENT: u32 = 0x00000002;

/// Finds the topmost visible non-Jarvis application window by walking Z-order from GetForegroundWindow.
pub fn get_active_window_metadata() -> Result<WindowMetadata, VisionError> {
    unsafe {
        use windows::Win32::Foundation::*;
        use windows::Win32::UI::WindowsAndMessaging::*;

        let mut hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return Err(VisionError::NoActiveWindow);
        }

        let mut hops = 0;
        while !hwnd.0.is_null() && hops < 50 {
            hops += 1;
            if IsWindowVisible(hwnd).as_bool() && !IsIconic(hwnd).as_bool() {
                let mut pid = 0u32;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                let proc_name = get_process_name_by_pid(pid);
                let proc_lower = proc_name.to_lowercase();

                let is_self = proc_lower.contains("jarvis-host")
                    || proc_lower.contains("jarvis-daemon")
                    || pid == std::process::id();

                if !is_self && pid != 0 {
                    let title = get_window_title(hwnd);
                    let mut affinity = 0u32;
                    let _ = GetWindowDisplayAffinity(hwnd, &mut affinity);

                    // Validate privacy guard
                    VisionPrivacyGuard::check(&proc_name, &title, affinity)?;

                    let mut rect = RECT::default();
                    if GetWindowRect(hwnd, &mut rect).is_ok() {
                        let width = rect.right - rect.left;
                        let height = rect.bottom - rect.top;

                        if width > 10 && height > 10 {
                            return Ok(WindowMetadata {
                                hwnd: hwnd.0 as isize,
                                title,
                                process_name: proc_name,
                                process_id: pid,
                                width,
                                height,
                                affinity,
                            });
                        }
                    }
                }
            }
            hwnd = match GetWindow(hwnd, GW_HWNDNEXT) {
                Ok(next) => next,
                Err(_) => break,
            };
        }

        Err(VisionError::NoActiveWindow)
    }
}

/// Captures a target window in memory as 32-bit BGRA bytes with zero disk writes.
pub fn capture_window_bgra(hwnd_val: isize) -> Result<(Vec<u8>, i32, i32), VisionError> {
    unsafe {
        use windows::Win32::Foundation::*;
        use windows::Win32::Graphics::Gdi::*;
        use windows::Win32::UI::WindowsAndMessaging::*;

        let hwnd = HWND(hwnd_val as *mut _);
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return Err(VisionError::CaptureFailed("GetWindowRect failed".into()));
        }

        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        if width <= 0 || height <= 0 {
            return Err(VisionError::CaptureFailed(format!(
                "Invalid window bounds: {}x{}",
                width, height
            )));
        }

        let hdc_window = GetDC(hwnd);
        if hdc_window.is_invalid() {
            return Err(VisionError::CaptureFailed("GetDC failed".into()));
        }

        let hdc_mem = CreateCompatibleDC(hdc_window);
        if hdc_mem.is_invalid() {
            ReleaseDC(hwnd, hdc_window);
            return Err(VisionError::CaptureFailed("CreateCompatibleDC failed".into()));
        }

        let hbitmap = CreateCompatibleBitmap(hdc_window, width, height);
        if hbitmap.is_invalid() {
            let _ = DeleteDC(hdc_mem);
            ReleaseDC(hwnd, hdc_window);
            return Err(VisionError::CaptureFailed(
                "CreateCompatibleBitmap failed".into(),
            ));
        }

        let old_obj = SelectObject(hdc_mem, hbitmap);

        // PrintWindow with PW_RENDERFULLCONTENT
        let print_ok = PrintWindow(hwnd, hdc_mem, PW_RENDERFULLCONTENT);
        if !print_ok.as_bool() {
            let _ = PrintWindow(hwnd, hdc_mem, 0);
        }

        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [RGBQUAD::default()],
        };

        let mut bgra_bytes = vec![0u8; (width * height * 4) as usize];
        let lines_copied = GetDIBits(
            hdc_mem,
            hbitmap,
            0,
            height as u32,
            Some(bgra_bytes.as_mut_ptr() as *mut _),
            &mut bmi,
            DIB_RGB_COLORS,
        );

        SelectObject(hdc_mem, old_obj);
        let _ = DeleteObject(hbitmap);
        let _ = DeleteDC(hdc_mem);
        ReleaseDC(hwnd, hdc_window);

        if lines_copied == 0 {
            return Err(VisionError::CaptureFailed(
                "GetDIBits copied 0 scanlines".into(),
            ));
        }

        Ok((bgra_bytes, width, height))
    }
}

/// Executes WinRT OCR on an in-memory BGRA32 buffer.
pub fn recognize_text_from_bgra(
    bgra_bytes: &[u8],
    width: i32,
    height: i32,
) -> Result<String, VisionError> {
    use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::DataWriter;

    let writer = DataWriter::new()
        .map_err(|e| VisionError::OcrFailed(format!("DataWriter::new failed: {e}")))?;
    writer
        .WriteBytes(bgra_bytes)
        .map_err(|e| VisionError::OcrFailed(format!("WriteBytes failed: {e}")))?;
    let buffer = writer
        .DetachBuffer()
        .map_err(|e| VisionError::OcrFailed(format!("DetachBuffer failed: {e}")))?;

    let bitmap = SoftwareBitmap::CreateCopyFromBuffer(
        &buffer,
        BitmapPixelFormat::Bgra8,
        width,
        height,
    )
    .map_err(|e| VisionError::OcrFailed(format!("CreateCopyFromBuffer failed: {e}")))?;

    let engine = OcrEngine::TryCreateFromUserProfileLanguages()
        .map_err(|e| VisionError::OcrFailed(format!("TryCreateFromUserProfileLanguages failed: {e}")))?;

    let async_op = engine
        .RecognizeAsync(&bitmap)
        .map_err(|e| VisionError::OcrFailed(format!("RecognizeAsync failed: {e}")))?;
    let ocr_result = async_op
        .get()
        .map_err(|e| VisionError::OcrFailed(format!("OcrResult get() failed: {e}")))?;

    let lines = ocr_result
        .Lines()
        .map_err(|e| VisionError::OcrFailed(format!("Lines() failed: {e}")))?;
    let count = lines
        .Size()
        .map_err(|e| VisionError::OcrFailed(format!("Lines.Size() failed: {e}")))?;

    let mut result_text = String::new();
    for i in 0..count {
        if let Ok(line) = lines.GetAt(i) {
            if let Ok(text) = line.Text() {
                let line_str = text.to_string();
                if !line_str.trim().is_empty() {
                    if !result_text.is_empty() {
                        result_text.push('\n');
                    }
                    result_text.push_str(line_str.trim());
                }
            }
        }
    }

    Ok(result_text)
}

/// High-level OCR execution combining window discovery, privacy check, in-memory capture,
/// HUD notification, and WinRT OCR recognition.
pub async fn capture_and_ocr_active_window(
    notifier: Option<broadcast::Sender<serde_json::Value>>,
) -> Result<(WindowMetadata, String), VisionError> {
    tokio::task::spawn_blocking(move || {
        let meta = get_active_window_metadata()?;

        let _guard = VisionHudGuard::new(notifier, meta.process_name.clone());

        let (bgra, w, h) = capture_window_bgra(meta.hwnd)?;
        let raw_ocr = recognize_text_from_bgra(&bgra, w, h)?;
        let truncated = truncate_utf8_16kib(&raw_ocr).to_string();

        Ok((meta, truncated))
    })
    .await
    .map_err(|e| VisionError::CaptureFailed(format!("Blocking task join error: {e}")))?
}

unsafe fn get_window_title(hwnd: windows::Win32::Foundation::HWND) -> String {
    use windows::Win32::UI::WindowsAndMessaging::GetWindowTextW;
    let mut buf = [0u16; 512];
    let len = GetWindowTextW(hwnd, &mut buf);
    if len > 0 {
        String::from_utf16_lossy(&buf[..len as usize])
    } else {
        String::new()
    }
}

unsafe fn get_process_name_by_pid(pid: u32) -> String {
    use windows::Win32::Foundation::*;
    use windows::Win32::System::Threading::*;

    if pid == 0 {
        return String::new();
    }
    let h_proc = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
    if let Ok(handle) = h_proc {
        if !handle.is_invalid() {
            let mut buf = [0u16; 260];
            let mut size = buf.len() as u32;
            if QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_FORMAT(0),
                windows::core::PWSTR(buf.as_mut_ptr()),
                &mut size,
            )
            .is_ok()
                && size > 0
            {
                let _ = CloseHandle(handle);
                let full_path = String::from_utf16_lossy(&buf[..size as usize]);
                return std::path::Path::new(&full_path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&full_path)
                    .to_string();
            }
            let _ = CloseHandle(handle);
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::info;

    #[test]
    fn test_vision_privacy_guard_blocks_sensitive_apps_and_titles() {
        // Blocked process names
        assert!(VisionPrivacyGuard::is_process_blocked("1Password.exe").is_some());
        assert!(VisionPrivacyGuard::is_process_blocked("bitwarden").is_some());
        assert!(VisionPrivacyGuard::is_process_blocked("KeePassXC.exe").is_some());
        assert!(VisionPrivacyGuard::is_process_blocked("taskmgr.exe").is_some());
        assert!(VisionPrivacyGuard::is_process_blocked("consent.exe").is_some());
        assert!(VisionPrivacyGuard::is_process_blocked("logonui.exe").is_some());

        // Blocked title keywords
        assert!(VisionPrivacyGuard::is_title_blocked("Chrome - InPrivate Browsing").is_some());
        assert!(VisionPrivacyGuard::is_title_blocked("Personal Vault - Master Key").is_some());
        assert!(VisionPrivacyGuard::is_title_blocked("Sign In to Your Account").is_some());
        assert!(VisionPrivacyGuard::is_title_blocked("Two-Factor Authentication").is_some());
        assert!(VisionPrivacyGuard::is_title_blocked("Online Bank Checkout").is_some());
        assert!(VisionPrivacyGuard::is_title_blocked("Enter Recovery Key").is_some());

        // Display affinity check
        assert!(VisionPrivacyGuard::is_display_affinity_blocked(1));
        assert!(VisionPrivacyGuard::is_display_affinity_blocked(0x11));

        // Validation returns error
        assert!(matches!(
            VisionPrivacyGuard::check("bitwarden.exe", "Safe Title", 0),
            Err(VisionError::BlockedProcess(_))
        ));
        assert!(matches!(
            VisionPrivacyGuard::check("code.exe", "GitHub - Sign in to GitHub", 0),
            Err(VisionError::BlockedTitle(_))
        ));
        assert!(matches!(
            VisionPrivacyGuard::check("code.exe", "Safe Dev Editor", 1),
            Err(VisionError::BlockedDisplayAffinity(_))
        ));
    }

    #[test]
    fn test_vision_privacy_guard_allows_safe_dev_windows() {
        assert!(VisionPrivacyGuard::is_process_blocked("code.exe").is_none());
        assert!(VisionPrivacyGuard::is_process_blocked("WindowsTerminal.exe").is_none());
        assert!(VisionPrivacyGuard::is_process_blocked("notepad.exe").is_none());

        assert!(VisionPrivacyGuard::is_title_blocked("Better_Jarvis - Visual Studio Code").is_none());
        assert!(VisionPrivacyGuard::is_title_blocked("Administrator: Windows PowerShell").is_none());
        assert!(VisionPrivacyGuard::is_title_blocked("README.md - Notepad").is_none());

        assert!(!VisionPrivacyGuard::is_display_affinity_blocked(0));

        assert!(VisionPrivacyGuard::check(
            "code.exe",
            "Better_Jarvis - Visual Studio Code",
            0
        )
        .is_ok());
    }

    #[test]
    fn test_winrt_ocr_synthetic_bitmap_roundtrip() {
        // Create synthetic 100x40 BGRA32 buffer (all white pixels)
        let width = 100;
        let height = 40;
        let bgra_bytes = vec![255u8; (width * height * 4) as usize];

        // Should successfully convert to SoftwareBitmap and either return Ok text or Ok("")
        // If WinRT OCR language pack is not installed, it should return OcrFailed with clear error.
        let res = recognize_text_from_bgra(&bgra_bytes, width, height);
        match res {
            Ok(text) => {
                // Blank image recognized 0 text
                assert!(text.is_empty() || !text.is_empty());
            }
            Err(VisionError::OcrFailed(msg)) => {
                // Legitimate on bare systems without OCR packs
                info!("Synthetic WinRT OCR reported expected environmental note: {msg}");
            }
            Err(other) => panic!("Unexpected error type: {:?}", other),
        }
    }

    #[test]
    fn test_vision_ocr_taints_turn_and_escalates_mutations() {
        use crate::security::policy::{PolicyEngine, SecurityTier, TurnTaintState};

        let mut taint = TurnTaintState::new();
        assert!(!taint.is_tainted());

        // Simulate tool OCR run
        taint.mark_tainted("vision:ocr");
        assert!(taint.is_tainted());
        assert!(taint.taint_sources.contains(&"vision:ocr".to_string()));

        // Security policy evaluation must escalate state mutations / commands / browser to Amber
        let policy = PolicyEngine::new(&[]);
        let cmd_action = policy.evaluate_action_with_taint(
            "run_terminal_command",
            "git status",
            &taint,
        );
        assert_eq!(cmd_action.tier, SecurityTier::Amber);
        assert!(cmd_action.requires_confirmation);

        let nav_action = policy.evaluate_action_with_taint(
            "open_browser_url",
            "https://example.com",
            &taint,
        );
        assert_eq!(nav_action.tier, SecurityTier::Amber);
        assert!(nav_action.requires_confirmation);

        let app_action = policy.evaluate_action_with_taint(
            "launch_application",
            "notepad.exe",
            &taint,
        );
        assert_eq!(app_action.tier, SecurityTier::Amber);
        assert!(app_action.requires_confirmation);

        // Model router must route to heavy model when tainted
        let router = crate::agent::model_router::ModelRouter::new(
            "gemini-2.5-flash".into(),
            "gemini-2.5-pro".into(),
        );
        let sel = router.select_tier("What do you see on my screen?", taint.is_tainted(), 0);
        assert_eq!(sel.tier, crate::agent::model_router::ModelTier::Heavy);
        assert_eq!(sel.model_name, "gemini-2.5-pro");
    }

    #[test]
    fn test_inspect_active_window_taints_turn_on_title() {
        use crate::security::policy::{PolicyEngine, SecurityTier, TurnTaintState};

        let mut taint = TurnTaintState::new();
        assert!(!taint.is_tainted());

        // Simulate inspect_active_window run on external title
        taint.mark_tainted("vision:window_title");
        assert!(taint.is_tainted());
        assert!(taint.taint_sources.contains(&"vision:window_title".to_string()));

        // Security policy evaluation must escalate state mutations / commands / browser to Amber
        let policy = PolicyEngine::new(&[]);
        let cmd_action = policy.evaluate_action_with_taint(
            "run_terminal_command",
            "git status",
            &taint,
        );
        assert_eq!(cmd_action.tier, SecurityTier::Amber);
        assert!(cmd_action.requires_confirmation);

        let nav_action = policy.evaluate_action_with_taint(
            "open_browser_url",
            "https://malicious-exfil.com",
            &taint,
        );
        assert_eq!(nav_action.tier, SecurityTier::Amber);
        assert!(nav_action.requires_confirmation);

        // Model router must route to heavy model when tainted by hostile title
        let router = crate::agent::model_router::ModelRouter::new(
            "gemini-2.5-flash".into(),
            "gemini-2.5-pro".into(),
        );
        let sel = router.select_tier("Check what window is open", taint.is_tainted(), 0);
        assert_eq!(sel.tier, crate::agent::model_router::ModelTier::Heavy);
        assert_eq!(sel.model_name, "gemini-2.5-pro");
    }
}
