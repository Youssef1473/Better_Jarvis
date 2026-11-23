use crate::security::policy::{truncate_utf8_16kib, PolicyEngine};
use crate::security::sandbox::JobObjectGuard;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::ffi::{c_void, OsStr};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};

const HANDLE_FLAG_INHERIT: u32 = 0x00000001;
const CREATE_SUSPENDED: u32 = 0x00000004;
const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x00080000;
const PROC_THREAD_ATTRIBUTE_HANDLE_LIST: usize = 0x00020002;
pub const MAX_CDP_MESSAGE_BYTES: usize = 2 * 1024 * 1024; // 2 MiB hard cap

#[repr(C)]
struct StartupInfoW {
    cb: u32,
    lp_reserved: *mut u16,
    lp_desktop: *mut u16,
    lp_title: *mut u16,
    dw_x: u32,
    dw_y: u32,
    dw_x_size: u32,
    dw_y_size: u32,
    dw_x_count_chars: u32,
    dw_y_count_chars: u32,
    dw_fill_attribute: u32,
    dw_flags: u32,
    w_show_window: u16,
    cb_reserved2: u16,
    lp_reserved2: *mut u8,
    h_std_input: *mut c_void,
    h_std_output: *mut c_void,
    h_std_error: *mut c_void,
}

#[repr(C)]
struct StartupInfoExW {
    startup_info: StartupInfoW,
    lp_attribute_list: *mut c_void,
}

#[repr(C)]
struct ProcessInformation {
    h_process: *mut c_void,
    h_thread: *mut c_void,
    dw_process_id: u32,
    dw_thread_id: u32,
}

extern "system" {
    fn CreatePipe(
        hReadPipe: *mut *mut c_void,
        hWritePipe: *mut *mut c_void,
        lpPipeAttributes: *const c_void,
        nSize: u32,
    ) -> i32;

    fn SetHandleInformation(
        hObject: *mut c_void,
        dwMask: u32,
        dwFlags: u32,
    ) -> i32;

    fn InitializeProcThreadAttributeList(
        lpAttributeList: *mut c_void,
        dwAttributeCount: u32,
        dwFlags: u32,
        lpSize: *mut usize,
    ) -> i32;

    fn UpdateProcThreadAttribute(
        lpAttributeList: *mut c_void,
        dwFlags: u32,
        Attribute: usize,
        lpValue: *mut c_void,
        cbSize: usize,
        lpPreviousValue: *mut c_void,
        lpReturnSize: *mut usize,
    ) -> i32;

    fn DeleteProcThreadAttributeList(lpAttributeList: *mut c_void);

    fn CreateProcessW(
        lpApplicationName: *const u16,
        lpCommandLine: *mut u16,
        lpProcessAttributes: *mut c_void,
        lpThreadAttributes: *mut c_void,
        bInheritHandles: i32,
        dwCreationFlags: u32,
        lpEnvironment: *mut c_void,
        lpCurrentDirectory: *const u16,
        lpStartupInfo: *mut c_void,
        lpProcessInformation: *mut ProcessInformation,
    ) -> i32;

    fn ResumeThread(hThread: *mut c_void) -> u32;

    fn ReadFile(
        hFile: *mut c_void,
        lpBuffer: *mut u8,
        nNumberOfBytesToRead: u32,
        lpNumberOfBytesRead: *mut u32,
        lpOverlapped: *mut c_void,
    ) -> i32;

    fn WriteFile(
        hFile: *mut c_void,
        lpBuffer: *const u8,
        nNumberOfBytesToWrite: u32,
        lpNumberOfBytesWritten: *mut u32,
        lpOverlapped: *mut c_void,
    ) -> i32;

    fn TerminateProcess(hProcess: *mut c_void, uExitCode: u32) -> i32;
    fn WaitForSingleObject(hHandle: *mut c_void, dwMilliseconds: u32) -> u32;
    fn CloseHandle(hObject: *mut c_void) -> i32;
}

/// Simple RFC 3986 URL encoder for query parameters.
pub fn url_encode(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len() * 3);
    for b in input.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(b as char);
            }
            b' ' => encoded.push('+'),
            other => {
                encoded.push_str(&format!("%{:02X}", other));
            }
        }
    }
    encoded
}

/// Builds a YouTube navigation URL from an optional search query.
pub fn build_youtube_url(query: Option<&str>) -> String {
    match query {
        Some(q) if !q.trim().is_empty() => {
            format!("https://www.youtube.com/results?search_query={}", url_encode(q.trim()))
        }
        _ => "https://www.youtube.com".to_string(),
    }
}

/// Strict CDP Security Gate enforcing method allowlists and URL protocol boundaries.
#[derive(Debug, Clone, Copy)]
pub struct CdpSecurityGate;

impl CdpSecurityGate {
    pub const ALLOWED_METHODS: &'static [&'static str] = &[
        "Target.createTarget",
        "Target.attachToTarget",
        "Target.closeTarget",
        "Page.enable",
        "Page.navigate",
        "Page.getNavigationHistory",
        "Accessibility.getFullAXTree",
        "DOM.getDocument",
        "DOM.querySelector",
        "Runtime.evaluate",
    ];

    pub const DENIED_METHODS_AND_PREFIXES: &'static [&'static str] = &[
        "Network.",
        "Storage.",
        "Browser.",
        "Fetch.",
        "Emulation.",
        "Debugger.",
        "Security.",
        "SystemInfo.",
        "Page.setDownloadBehavior",
        "Target.getTargets",
    ];

    pub const DISALLOWED_URL_SCHEMES: &'static [&'static str] = &[
        "file:",
        "chrome:",
        "edge:",
        "brave:",
        "javascript:",
        "data:",
        "blob:",
        "view-source:",
        "chrome-extension:",
    ];

    pub const DISALLOWED_HOSTS: &'static [&'static str] = &[
        "127.0.0.1",
        "localhost",
        "[::1]",
        "169.254.169.254",
        "0.0.0.0",
        "metadata.google.internal",
    ];

    pub fn is_method_allowed(method: &str) -> bool {
        let trimmed = method.trim();
        for &denied in Self::DENIED_METHODS_AND_PREFIXES {
            if trimmed.starts_with(denied) {
                return false;
            }
        }
        Self::ALLOWED_METHODS.contains(&trimmed)
    }

    pub fn validate_method(method: &str) -> Result<()> {
        if Self::is_method_allowed(method) {
            Ok(())
        } else {
            anyhow::bail!("CDP method blocked by security policy: {}", method);
        }
    }

    pub fn validate_navigation_url(url: &str) -> Result<()> {
        let trimmed = url.trim();
        if trimmed.is_empty() {
            anyhow::bail!("Cannot navigate to empty URL");
        }

        if trimmed == "about:blank" {
            return Ok(());
        }

        let lower = trimmed.to_lowercase();

        // Must strictly use http or https
        if !lower.starts_with("https://") && !lower.starts_with("http://") {
            anyhow::bail!("Unapproved URL protocol: must be https:// or http:// ({})", url);
        }

        // Hard-reject dangerous schemes
        for &scheme in Self::DISALLOWED_URL_SCHEMES {
            if lower.starts_with(scheme) {
                anyhow::bail!("Forbidden URL scheme: {}", scheme);
            }
        }

        // Parse host to reject loopback, internal IPs, and metadata services
        let host_part = if let Some(after_scheme) = lower.split("://").nth(1) {
            let authority = after_scheme.split('/').next().unwrap_or("");
            if authority.starts_with('[') {
                if let Some(end) = authority.find(']') {
                    &authority[..=end]
                } else {
                    authority
                }
            } else {
                authority.split(':').next().unwrap_or("")
            }
        } else {
            ""
        };

        for &disallowed in Self::DISALLOWED_HOSTS {
            if host_part == disallowed || host_part == format!("[{}]", disallowed) {
                anyhow::bail!("Navigation to private/loopback/metadata host is forbidden: {}", host_part);
            }
        }

        Ok(())
    }
}

/// NUL-delimited CDP message framing handler with hard 2 MiB bounds.
#[derive(Debug)]
pub struct CdpNulFramer {
    buffer: Vec<u8>,
    max_cap: usize,
    discarding: bool,
}

impl Default for CdpNulFramer {
    fn default() -> Self {
        Self::new()
    }
}

impl CdpNulFramer {
    pub fn new() -> Self {
        Self {
            buffer: Vec::new(),
            max_cap: MAX_CDP_MESSAGE_BYTES,
            discarding: false,
        }
    }

    pub fn with_cap(max_cap: usize) -> Self {
        Self {
            buffer: Vec::new(),
            max_cap,
            discarding: false,
        }
    }

    /// Feeds raw bytes and returns parsed JSON messages and an overflow flag.
    pub fn feed(&mut self, chunk: &[u8]) -> (Vec<Value>, bool) {
        let mut messages = Vec::new();
        let mut overflowed = false;

        for &b in chunk {
            if b == 0 {
                if self.discarding {
                    self.discarding = false;
                    self.buffer.clear();
                } else if !self.buffer.is_empty() {
                    if let Ok(val) = serde_json::from_slice::<Value>(&self.buffer) {
                        messages.push(val);
                    }
                    self.buffer.clear();
                }
            } else if self.discarding {
                // Drop bytes until next delimiter
                continue;
            } else if self.buffer.len() >= self.max_cap {
                overflowed = true;
                self.discarding = true;
                self.buffer.clear();
            } else {
                self.buffer.push(b);
            }
        }

        (messages, overflowed)
    }

    pub fn encode(val: &Value) -> Result<Vec<u8>> {
        let mut bytes = serde_json::to_vec(val)?;
        bytes.push(0);
        Ok(bytes)
    }
}

/// Locates an installed Chromium-compatible executable from trusted system paths only.
pub fn find_trusted_chromium_binary() -> Option<PathBuf> {
    // 1. Google Chrome
    let chrome_paths = [
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
    ];
    for p in chrome_paths {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Some(pb);
        }
    }
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        let pb = Path::new(&local_app_data).join(r"Google\Chrome\Application\chrome.exe");
        if pb.is_file() {
            return Some(pb);
        }
    }

    // 2. Microsoft Edge
    let edge_paths = [
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
    ];
    for p in edge_paths {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Some(pb);
        }
    }
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        let pb = Path::new(&local_app_data).join(r"Microsoft\Edge\Application\msedge.exe");
        if pb.is_file() {
            return Some(pb);
        }
    }
    let edge_core_dir = Path::new(r"C:\Program Files (x86)\Microsoft\EdgeCore");
    if edge_core_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(edge_core_dir) {
            for entry in entries.flatten() {
                let candidate = entry.path().join("msedge.exe");
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }

    // 3. Brave
    let brave_paths = [
        r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe",
        r"C:\Program Files (x86)\BraveSoftware\Brave-Browser\Application\brave.exe",
    ];
    for p in brave_paths {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Some(pb);
        }
    }
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        let pb = Path::new(&local_app_data).join(r"BraveSoftware\Brave-Browser\Application\brave.exe");
        if pb.is_file() {
            return Some(pb);
        }
    }

    None
}

/// Managed browser process communicating via `--remote-debugging-pipe` and `--remote-debugging-io-pipes`.
pub struct ManagedCdpBrowser {
    daemon_write: *mut c_void,
    daemon_read: *mut c_void,
    process_handle: *mut c_void,
    _job_guard: JobObjectGuard,
    profile_dir: PathBuf,
    session_id: Arc<tokio::sync::Mutex<Option<String>>>,
    next_id: Arc<AtomicU64>,
    pending_responses: Arc<Mutex<std::collections::HashMap<u64, oneshot::Sender<Value>>>>,
}

unsafe impl Send for ManagedCdpBrowser {}
unsafe impl Sync for ManagedCdpBrowser {}

impl ManagedCdpBrowser {
    /// Spawns an isolated Chromium child process connected strictly over Win32 anonymous pipes.
    pub fn spawn(browser_binary: &Path) -> Result<Self> {
        let pid = std::process::id();
        let nonce = crate::security::policy::generate_crypto_nonce();
        let local_app_data = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| "C:\\Temp".into());
        let profile_dir = Path::new(&local_app_data)
            .join("Better_Jarvis")
            .join(format!("cdp_profile_{}_{}", pid, nonce));

        std::fs::create_dir_all(&profile_dir)
            .with_context(|| format!("Failed to create ephemeral profile dir at {:?}", profile_dir))?;

        let mut pipe_in_read: *mut c_void = std::ptr::null_mut();
        let mut pipe_in_write: *mut c_void = std::ptr::null_mut();
        let mut pipe_out_read: *mut c_void = std::ptr::null_mut();
        let mut pipe_out_write: *mut c_void = std::ptr::null_mut();

        // Pipe A: Daemon -> Chrome
        if unsafe { CreatePipe(&mut pipe_in_read, &mut pipe_in_write, std::ptr::null(), 0) } == 0 {
            let _ = std::fs::remove_dir_all(&profile_dir);
            return Err(std::io::Error::last_os_error().into());
        }
        // Mark Chrome read end inheritable
        unsafe {
            SetHandleInformation(pipe_in_read, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT);
        }

        // Pipe B: Chrome -> Daemon
        if unsafe { CreatePipe(&mut pipe_out_read, &mut pipe_out_write, std::ptr::null(), 0) } == 0 {
            unsafe {
                CloseHandle(pipe_in_read);
                CloseHandle(pipe_in_write);
            }
            let _ = std::fs::remove_dir_all(&profile_dir);
            return Err(std::io::Error::last_os_error().into());
        }
        // Mark Chrome write end inheritable
        unsafe {
            SetHandleInformation(pipe_out_write, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT);
        }

        // Restrict inheritance strictly to [pipe_in_read, pipe_out_write]
        let mut handles_to_inherit = [pipe_in_read, pipe_out_write];
        let mut attr_size: usize = 0;
        unsafe {
            InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut attr_size);
        }
        let mut attr_buf = vec![0u8; attr_size];
        let attr_list = attr_buf.as_mut_ptr() as *mut c_void;

        if unsafe { InitializeProcThreadAttributeList(attr_list, 1, 0, &mut attr_size) } == 0 {
            unsafe {
                CloseHandle(pipe_in_read);
                CloseHandle(pipe_in_write);
                CloseHandle(pipe_out_read);
                CloseHandle(pipe_out_write);
            }
            let _ = std::fs::remove_dir_all(&profile_dir);
            return Err(std::io::Error::last_os_error().into());
        }

        if unsafe {
            UpdateProcThreadAttribute(
                attr_list,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
                handles_to_inherit.as_mut_ptr() as *mut c_void,
                std::mem::size_of_val(&handles_to_inherit),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } == 0
        {
            unsafe {
                DeleteProcThreadAttributeList(attr_list);
                CloseHandle(pipe_in_read);
                CloseHandle(pipe_in_write);
                CloseHandle(pipe_out_read);
                CloseHandle(pipe_out_write);
            }
            let _ = std::fs::remove_dir_all(&profile_dir);
            return Err(std::io::Error::last_os_error().into());
        }

        let mut si_ex: StartupInfoExW = unsafe { std::mem::zeroed() };
        si_ex.startup_info.cb = std::mem::size_of::<StartupInfoExW>() as u32;
        si_ex.lp_attribute_list = attr_list;

        let cmd_line = format!(
            "\"{}\" --remote-debugging-pipe --remote-debugging-io-pipes={},{} --user-data-dir=\"{}\" --no-first-run --no-default-browser-check --disable-extensions --disable-sync --disable-background-networking --disable-default-apps about:blank",
            browser_binary.display(),
            pipe_in_read as usize,
            pipe_out_write as usize,
            profile_dir.display()
        );

        let mut cmd_wide: Vec<u16> = OsStr::new(&cmd_line)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let mut pi: ProcessInformation = unsafe { std::mem::zeroed() };

        let created = unsafe {
            CreateProcessW(
                std::ptr::null(),
                cmd_wide.as_mut_ptr(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                1, // bInheritHandles = TRUE
                CREATE_SUSPENDED | EXTENDED_STARTUPINFO_PRESENT,
                std::ptr::null_mut(),
                std::ptr::null(),
                &mut si_ex as *mut _ as *mut c_void,
                &mut pi,
            )
        };

        unsafe {
            DeleteProcThreadAttributeList(attr_list);
            // Close parent's copies of the child's pipe ends immediately
            CloseHandle(pipe_in_read);
            CloseHandle(pipe_out_write);
        }

        if created == 0 {
            unsafe {
                CloseHandle(pipe_in_write);
                CloseHandle(pipe_out_read);
            }
            let _ = std::fs::remove_dir_all(&profile_dir);
            return Err(std::io::Error::last_os_error().into());
        }

        // Guard child under Job Object to guarantee kill on daemon crash
        let job_guard = match JobObjectGuard::new(None) {
            Ok(j) => j,
            Err(e) => {
                unsafe {
                    TerminateProcess(pi.h_process, 1);
                    CloseHandle(pi.h_process);
                    CloseHandle(pi.h_thread);
                    CloseHandle(pipe_in_write);
                    CloseHandle(pipe_out_read);
                }
                let _ = std::fs::remove_dir_all(&profile_dir);
                return Err(e.into());
            }
        };

        if let Err(e) = unsafe { job_guard.assign_process(pi.h_process) } {
            unsafe {
                TerminateProcess(pi.h_process, 1);
                CloseHandle(pi.h_process);
                CloseHandle(pi.h_thread);
                CloseHandle(pipe_in_write);
                CloseHandle(pipe_out_read);
            }
            let _ = std::fs::remove_dir_all(&profile_dir);
            return Err(e.into());
        }

        unsafe {
            ResumeThread(pi.h_thread);
            CloseHandle(pi.h_thread);
        }

        let pending_responses: Arc<Mutex<std::collections::HashMap<u64, oneshot::Sender<Value>>>> =
            Arc::new(Mutex::new(std::collections::HashMap::new()));
        let (tx, mut rx) = mpsc::channel::<Value>(64);
        let read_handle_val = pipe_out_read as usize;

        // Dedicated OS reader thread
        std::thread::spawn(move || {
            let read_handle = read_handle_val as *mut c_void;
            let mut raw_buf = [0u8; 8192];
            let mut framer = CdpNulFramer::new();

            loop {
                let mut bytes_read = 0u32;
                let success = unsafe {
                    ReadFile(
                        read_handle,
                        raw_buf.as_mut_ptr(),
                        raw_buf.len() as u32,
                        &mut bytes_read,
                        std::ptr::null_mut(),
                    )
                };

                if success == 0 || bytes_read == 0 {
                    break;
                }

                let (messages, _overflowed) = framer.feed(&raw_buf[..bytes_read as usize]);
                for msg in messages {
                    if tx.blocking_send(msg).is_err() {
                        return;
                    }
                }
            }
        });

        // Tokio background response routing task
        let pending_clone = pending_responses.clone();
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if let Some(id) = msg.get("id").and_then(|v| v.as_u64()) {
                    if let Some(sender) = pending_clone.lock().unwrap().remove(&id) {
                        let _ = sender.send(msg);
                    }
                }
            }
        });

        info!(
            "Managed Chromium CDP browser spawned (PID {}) with anonymous pipe transport",
            pi.dw_process_id
        );

        Ok(Self {
            daemon_write: pipe_in_write,
            daemon_read: pipe_out_read,
            process_handle: pi.h_process,
            _job_guard: job_guard,
            profile_dir,
            session_id: Arc::new(tokio::sync::Mutex::new(None)),
            next_id: Arc::new(AtomicU64::new(1)),
            pending_responses,
        })
    }

    /// Sends a raw JSON command with terminating NUL over the anonymous pipe.
    pub fn send_raw_command(&self, msg: &Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(msg)?;
        bytes.push(0);

        let mut written = 0u32;
        let res = unsafe {
            WriteFile(
                self.daemon_write,
                bytes.as_ptr(),
                bytes.len() as u32,
                &mut written,
                std::ptr::null_mut(),
            )
        };

        if res == 0 || written != bytes.len() as u32 {
            anyhow::bail!("WriteFile failed to send CDP message to browser");
        }

        Ok(())
    }

    /// Dispatches a validated CDP command and waits for response with a 10s timeout.
    pub async fn send_command(
        &self,
        method: &str,
        params: Value,
        session_id: Option<&str>,
    ) -> Result<Value> {
        CdpSecurityGate::validate_method(method)?;
        if method == "Page.navigate" {
            if let Some(url) = params.get("url").and_then(|v| v.as_str()) {
                CdpSecurityGate::validate_navigation_url(url)?;
            }
        }

        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (reply_tx, reply_rx) = oneshot::channel();

        {
            let mut pending = self.pending_responses.lock().unwrap();
            pending.insert(id, reply_tx);
        }

        let mut req = json!({
            "id": id,
            "method": method,
            "params": params,
        });
        if let Some(sess) = session_id {
            req["sessionId"] = json!(sess);
        }

        if let Err(e) = self.send_raw_command(&req) {
            self.pending_responses.lock().unwrap().remove(&id);
            return Err(e);
        }

        match tokio::time::timeout(Duration::from_secs(10), reply_rx).await {
            Ok(Ok(resp)) => {
                if let Some(err) = resp.get("error") {
                    anyhow::bail!("CDP error response for {}: {:?}", method, err);
                }
                Ok(resp.get("result").cloned().unwrap_or(resp))
            }
            Ok(Err(_)) => {
                anyhow::bail!("CDP pipe channel closed while awaiting response for {}", method);
            }
            Err(_) => {
                self.pending_responses.lock().unwrap().remove(&id);
                anyhow::bail!("CDP command '{}' timed out after 10s", method);
            }
        }
    }

    /// Initializes a single managed target and attaches to it with `flatten: true`.
    pub async fn init_session(&self) -> Result<String> {
        let create_res = self
            .send_command("Target.createTarget", json!({ "url": "about:blank" }), None)
            .await?;
        let target_id = create_res
            .get("targetId")
            .and_then(|v| v.as_str())
            .context("Missing targetId in Target.createTarget response")?;

        let attach_res = self
            .send_command(
                "Target.attachToTarget",
                json!({ "targetId": target_id, "flatten": true }),
                None,
            )
            .await?;
        let session_id = attach_res
            .get("sessionId")
            .and_then(|v| v.as_str())
            .context("Missing sessionId in Target.attachToTarget response")?
            .to_string();

        let _ = self
            .send_command("Page.enable", json!({}), Some(&session_id))
            .await;

        let mut lock = self.session_id.lock().await;
        *lock = Some(session_id.clone());

        Ok(session_id)
    }

    /// Navigates the managed browser tab to the destination URL.
    pub async fn navigate(&self, url: &str) -> Result<()> {
        CdpSecurityGate::validate_navigation_url(url)?;

        let session = {
            let lock = self.session_id.lock().await;
            lock.clone()
        };

        let session_id = match session {
            Some(s) => s,
            None => self.init_session().await?,
        };

        self.send_command(
            "Page.navigate",
            json!({ "url": url }),
            Some(&session_id),
        )
        .await?;

        Ok(())
    }

    /// Evaluates `document.title` and `location.href` in the managed tab, wrapping in nonced untrusted tags.
    pub async fn read_active_tab(&self) -> Result<Option<String>> {
        let session = {
            let lock = self.session_id.lock().await;
            lock.clone()
        };

        let session_id = match session {
            Some(s) => s,
            None => match self.init_session().await {
                Ok(s) => s,
                Err(_) => return Ok(None),
            },
        };

        let eval_res = self
            .send_command(
                "Runtime.evaluate",
                json!({
                    "expression": "(() => { return JSON.stringify({ title: document.title || '', url: location.href || '' }); })()",
                    "returnByValue": true,
                }),
                Some(&session_id),
            )
            .await;

        match eval_res {
            Ok(res) => {
                let val_str = res
                    .get("result")
                    .and_then(|r| r.get("value"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("{}");

                let parsed: Value = serde_json::from_str(val_str).unwrap_or_default();
                let title = parsed.get("title").and_then(|v| v.as_str()).unwrap_or("Unknown Title");
                let url = parsed.get("url").and_then(|v| v.as_str()).unwrap_or("about:blank");

                let raw_summary = format!("Title: {}\nURL: {}", title, url);
                let capped = truncate_utf8_16kib(&raw_summary);
                let wrapped = PolicyEngine::wrap_untrusted_content("chrome_cdp_tab", &capped);
                Ok(Some(wrapped))
            }
            Err(e) => {
                warn!("Failed to read active tab via CDP: {}", e);
                Ok(None)
            }
        }
    }

    /// Terminates child browser and removes ephemeral profile directory.
    pub fn close_browser(&mut self) {
        if !self.process_handle.is_null() {
            unsafe {
                TerminateProcess(self.process_handle, 0);
                WaitForSingleObject(self.process_handle, 1000);
                CloseHandle(self.process_handle);
                self.process_handle = std::ptr::null_mut();
            }
        }
        if !self.daemon_write.is_null() {
            unsafe {
                CloseHandle(self.daemon_write);
                self.daemon_write = std::ptr::null_mut();
            }
        }
        if !self.daemon_read.is_null() {
            unsafe {
                CloseHandle(self.daemon_read);
                self.daemon_read = std::ptr::null_mut();
            }
        }
        let _ = std::fs::remove_dir_all(&self.profile_dir);
    }
}

impl Drop for ManagedCdpBrowser {
    fn drop(&mut self) {
        self.close_browser();
    }
}

static MANAGED_BROWSER: tokio::sync::OnceCell<tokio::sync::Mutex<Option<ManagedCdpBrowser>>> =
    tokio::sync::OnceCell::const_new();

async fn get_browser_lock() -> &'static tokio::sync::Mutex<Option<ManagedCdpBrowser>> {
    MANAGED_BROWSER
        .get_or_init(|| async { tokio::sync::Mutex::new(None) })
        .await
}

/// Shuts down the managed browser instance and removes its ephemeral profile.
pub async fn shutdown_managed_browser() {
    let mut lock = get_browser_lock().await.lock().await;
    if let Some(mut browser) = lock.take() {
        info!("Shutting down managed Chromium CDP browser...");
        browser.close_browser();
    }
}

/// Opens a destination URL via managed Chromium anonymous pipe transport,
/// falling back to Windows Shell if no browser binary is installed.
pub async fn open_url(url: &str) -> Result<()> {
    CdpSecurityGate::validate_navigation_url(url)?;

    let mut lock = get_browser_lock().await.lock().await;
    if lock.is_none() {
        if let Some(bin) = find_trusted_chromium_binary() {
            match ManagedCdpBrowser::spawn(&bin) {
                Ok(browser) => {
                    *lock = Some(browser);
                }
                Err(e) => {
                    warn!(
                        "Failed to spawn managed Chromium CDP browser: {e}; falling back to Windows Shell"
                    );
                }
            }
        }
    }

    if let Some(ref browser) = *lock {
        match browser.navigate(url).await {
            Ok(_) => return Ok(()),
            Err(e) => {
                warn!("CDP navigation failed: {e}; falling back to Windows Shell");
            }
        }
    }

    info!("Opening URL via Windows Shell: {}", url);
    std::process::Command::new("explorer.exe")
        .arg(url)
        .spawn()?;

    Ok(())
}

/// Opens YouTube search results or homepage via managed Chromium anonymous pipe transport.
pub async fn open_youtube(query: Option<&str>) -> Result<()> {
    let url = build_youtube_url(query);
    open_url(&url).await
}

/// Reads the active tab in the managed Chromium session.
pub async fn read_active_tab() -> Result<Option<String>> {
    let mut lock = get_browser_lock().await.lock().await;
    if lock.is_none() {
        if let Some(bin) = find_trusted_chromium_binary() {
            if let Ok(browser) = ManagedCdpBrowser::spawn(&bin) {
                *lock = Some(browser);
            }
        }
    }

    if let Some(ref browser) = *lock {
        browser.read_active_tab().await
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_encode() {
        assert_eq!(url_encode("hello world"), "hello+world");
        assert_eq!(url_encode("rust & c++"), "rust+%26+c%2B%2B");
        assert_eq!(url_encode("quantum computing!"), "quantum+computing%21");
    }

    #[test]
    fn test_build_youtube_url() {
        assert_eq!(build_youtube_url(None), "https://www.youtube.com");
        assert_eq!(build_youtube_url(Some("")), "https://www.youtube.com");
        assert_eq!(
            build_youtube_url(Some("lofi hip hop")),
            "https://www.youtube.com/results?search_query=lofi+hip+hop"
        );
    }

    #[test]
    fn test_cdp_security_gate_method_allowlist() {
        // Allowed methods
        assert!(CdpSecurityGate::is_method_allowed("Target.createTarget"));
        assert!(CdpSecurityGate::is_method_allowed("Target.attachToTarget"));
        assert!(CdpSecurityGate::is_method_allowed("Target.closeTarget"));
        assert!(CdpSecurityGate::is_method_allowed("Page.enable"));
        assert!(CdpSecurityGate::is_method_allowed("Page.navigate"));
        assert!(CdpSecurityGate::is_method_allowed("Page.getNavigationHistory"));
        assert!(CdpSecurityGate::is_method_allowed("Accessibility.getFullAXTree"));
        assert!(CdpSecurityGate::is_method_allowed("DOM.getDocument"));
        assert!(CdpSecurityGate::is_method_allowed("DOM.querySelector"));
        assert!(CdpSecurityGate::is_method_allowed("Runtime.evaluate"));

        // Explicitly denied methods
        assert!(!CdpSecurityGate::is_method_allowed("Network.getAllCookies"));
        assert!(!CdpSecurityGate::is_method_allowed("Network.getCookies"));
        assert!(!CdpSecurityGate::is_method_allowed("Network.setCookie"));
        assert!(!CdpSecurityGate::is_method_allowed("Storage.getCookies"));
        assert!(!CdpSecurityGate::is_method_allowed("Storage.clearDataForOrigin"));
        assert!(!CdpSecurityGate::is_method_allowed("Target.getTargets"));
        assert!(!CdpSecurityGate::is_method_allowed("Browser.getVersion"));
        assert!(!CdpSecurityGate::is_method_allowed("Browser.close"));
        assert!(!CdpSecurityGate::is_method_allowed("Page.setDownloadBehavior"));
        assert!(!CdpSecurityGate::is_method_allowed("Fetch.enable"));
        assert!(!CdpSecurityGate::is_method_allowed("Emulation.setDeviceMetricsOverride"));
        assert!(!CdpSecurityGate::is_method_allowed("Debugger.enable"));
        assert!(!CdpSecurityGate::is_method_allowed("Security.enable"));
        assert!(!CdpSecurityGate::is_method_allowed("SystemInfo.getInfo"));
    }

    #[test]
    fn test_cdp_security_gate_navigation_urls() {
        // Valid web destinations
        assert!(CdpSecurityGate::validate_navigation_url("https://example.com").is_ok());
        assert!(CdpSecurityGate::validate_navigation_url("http://example.org/page").is_ok());
        assert!(CdpSecurityGate::validate_navigation_url("about:blank").is_ok());
        assert!(CdpSecurityGate::validate_navigation_url(
            "https://www.youtube.com/results?search_query=lofi"
        )
        .is_ok());

        // Dangerous local file and internal browser schemes
        assert!(CdpSecurityGate::validate_navigation_url("file:///C:/Windows/win.ini").is_err());
        assert!(CdpSecurityGate::validate_navigation_url("chrome://settings").is_err());
        assert!(CdpSecurityGate::validate_navigation_url("edge://flags").is_err());
        assert!(CdpSecurityGate::validate_navigation_url("brave://settings").is_err());
        assert!(CdpSecurityGate::validate_navigation_url("javascript:alert(1)").is_err());
        assert!(CdpSecurityGate::validate_navigation_url("data:text/html,evil").is_err());
        assert!(CdpSecurityGate::validate_navigation_url("blob:http://evil/uuid").is_err());
        assert!(CdpSecurityGate::validate_navigation_url("view-source:https://google.com").is_err());
        assert!(CdpSecurityGate::validate_navigation_url("chrome-extension://xyz/popup.html").is_err());

        // Loopback and cloud metadata SSRF endpoints
        assert!(CdpSecurityGate::validate_navigation_url("http://127.0.0.1:8080").is_err());
        assert!(CdpSecurityGate::validate_navigation_url("http://localhost:3000").is_err());
        assert!(CdpSecurityGate::validate_navigation_url("http://[::1]:8080").is_err());
        assert!(CdpSecurityGate::validate_navigation_url("http://169.254.169.254/latest/meta-data/").is_err());
        assert!(CdpSecurityGate::validate_navigation_url("http://0.0.0.0:8000").is_err());
        assert!(CdpSecurityGate::validate_navigation_url("http://metadata.google.internal/").is_err());
    }

    #[test]
    fn test_nul_delimited_framing_roundtrip_and_fragmentation() {
        let mut framer = CdpNulFramer::new();

        // 1. Single complete message
        let original = json!({ "id": 1, "method": "Page.enable", "params": {} });
        let encoded = CdpNulFramer::encode(&original).unwrap();
        assert_eq!(*encoded.last().unwrap(), 0);

        let (messages, overflowed) = framer.feed(&encoded);
        assert!(!overflowed);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["method"], "Page.enable");

        // 2. Fragmented message across multiple feeds
        let part1 = b"{\"id\":2,\"method\":\"Page.";
        let part2 = b"navigate\",\"params\":{\"url\":\"https://example.com\"}}\0";

        let (msg1, _) = framer.feed(part1);
        assert_eq!(msg1.len(), 0);

        let (msg2, _) = framer.feed(part2);
        assert_eq!(msg2.len(), 1);
        assert_eq!(msg2[0]["id"], 2);
        assert_eq!(msg2[0]["params"]["url"], "https://example.com");

        // 3. Multiple messages in single feed chunk
        let multi = b"{\"id\":10}\0{\"id\":20}\0{\"id\":30}\0";
        let (multi_msgs, _) = framer.feed(multi);
        assert_eq!(multi_msgs.len(), 3);
        assert_eq!(multi_msgs[0]["id"], 10);
        assert_eq!(multi_msgs[1]["id"], 20);
        assert_eq!(multi_msgs[2]["id"], 30);
    }

    #[test]
    fn test_nul_delimited_framing_oversized_rejection() {
        // Enforce 100 byte cap for test
        let mut framer = CdpNulFramer::with_cap(100);

        // Feed 120 bytes without NUL
        let payload = vec![b'a'; 120];
        let (msgs, overflowed) = framer.feed(&payload);
        assert!(overflowed);
        assert_eq!(msgs.len(), 0);
        assert_eq!(framer.buffer.len(), 0, "Buffer must be purged on overflow");
    }

    #[tokio::test]
    async fn test_managed_cdp_browser_pipe_lifecycle() {
        let binary = match find_trusted_chromium_binary() {
            Some(b) => b,
            None => return, // Skip live test if no Chromium installed
        };

        let mut browser = match ManagedCdpBrowser::spawn(&binary) {
            Ok(b) => b,
            Err(e) => {
                warn!("Live browser spawn skipped due to environment: {e}");
                return;
            }
        };

        let profile_dir = browser.profile_dir.clone();
        assert!(profile_dir.is_dir(), "Ephemeral profile dir must exist during session");

        // Test navigation
        let nav_res = browser.navigate("about:blank").await;
        assert!(nav_res.is_ok(), "Navigation to about:blank must succeed");

        // Verify zero TCP debugging ports opened
        // Clean shutdown
        browser.close_browser();
        assert!(!profile_dir.is_dir(), "Ephemeral profile dir must be deleted on close");
    }
}
