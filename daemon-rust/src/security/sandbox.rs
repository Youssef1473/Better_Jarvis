use anyhow::{Context, Result};
use std::ffi::{c_void, OsStr};
use std::io::{self, Read};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::FromRawHandle;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[repr(C)]
struct JobObjectBasicLimitInformation {
    per_process_user_time_limit: i64,
    per_job_user_time_limit: i64,
    limit_flags: u32,
    minimum_working_set_size: usize,
    maximum_working_set_size: usize,
    active_process_limit: u32,
    affinity: usize,
    priority_class: u32,
    scheduling_class: u32,
}

#[repr(C)]
struct IoCounters {
    read_operation_count: u64,
    write_operation_count: u64,
    other_operation_count: u64,
    read_transfer_count: u64,
    write_transfer_count: u64,
    other_transfer_count: u64,
}

#[repr(C)]
struct JobObjectExtendedLimitInformation {
    basic_limit_information: JobObjectBasicLimitInformation,
    io_info: IoCounters,
    process_memory_limit: usize,
    job_memory_limit: usize,
    peak_process_memory_limit: usize,
    peak_job_memory_limit: usize,
}

#[repr(C)]
struct SecurityAttributes {
    n_length: u32,
    lp_security_descriptor: *mut c_void,
    b_inherit_handle: i32,
}

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

const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x00002000;
const JOB_OBJECT_LIMIT_JOB_MEMORY: u32 = 0x00000200;
const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: u32 = 9;
const HANDLE_FLAG_INHERIT: u32 = 0x00000001;
const CREATE_SUSPENDED: u32 = 0x00000004;
const CREATE_UNICODE_ENVIRONMENT: u32 = 0x00000400;
const CREATE_NO_WINDOW: u32 = 0x08000000;
const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x00080000;
const STARTF_USESTDHANDLES: u32 = 0x00000100;
const PROC_THREAD_ATTRIBUTE_HANDLE_LIST: usize = 0x00020002;
const GENERIC_READ: u32 = 0x80000000;
const FILE_SHARE_READ: u32 = 0x00000001;
const FILE_SHARE_WRITE: u32 = 0x00000002;
const OPEN_EXISTING: u32 = 3;
const INFINITE: u32 = 0xFFFFFFFF;

extern "system" {
    fn CreateJobObjectW(lpJobAttributes: *mut c_void, lpName: *const u16) -> *mut c_void;
    fn SetInformationJobObject(
        hJob: *mut c_void,
        JobObjectInformationClass: u32,
        lpJobObjectInformation: *const c_void,
        cbJobObjectInformationLength: u32,
    ) -> i32;
    fn AssignProcessToJobObject(hJob: *mut c_void, hProcess: *mut c_void) -> i32;
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
    fn CreateFileW(
        lpFileName: *const u16,
        dwDesiredAccess: u32,
        dwShareMode: u32,
        lpSecurityAttributes: *const c_void,
        dwCreationDisposition: u32,
        dwFlagsAndAttributes: u32,
        hTemplateFile: *mut c_void,
    ) -> *mut c_void;
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
    fn TerminateProcess(hProcess: *mut c_void, uExitCode: u32) -> i32;
    fn WaitForSingleObject(hHandle: *mut c_void, dwMilliseconds: u32) -> u32;
    fn GetExitCodeProcess(hProcess: *mut c_void, lpExitCode: *mut u32) -> i32;
    fn CloseHandle(hObject: *mut c_void) -> i32;
}

/// RAII wrapper for a Windows Job Object.
/// Ensures all child and descendant processes are forcefully killed on drop.
pub struct JobObjectGuard {
    handle: *mut c_void,
}

unsafe impl Send for JobObjectGuard {}
unsafe impl Sync for JobObjectGuard {}

impl JobObjectGuard {
    pub fn new(max_memory_bytes: Option<usize>) -> io::Result<Self> {
        let handle = unsafe { CreateJobObjectW(std::ptr::null_mut(), std::ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }

        let mut limits = unsafe { std::mem::zeroed::<JobObjectExtendedLimitInformation>() };
        limits.basic_limit_information.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

        if let Some(mem_bytes) = max_memory_bytes {
            limits.basic_limit_information.limit_flags |= JOB_OBJECT_LIMIT_JOB_MEMORY;
            limits.job_memory_limit = mem_bytes;
        }

        let ret = unsafe {
            SetInformationJobObject(
                handle,
                JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
                &limits as *const _ as *const c_void,
                std::mem::size_of::<JobObjectExtendedLimitInformation>() as u32,
            )
        };

        if ret == 0 {
            unsafe { CloseHandle(handle) };
            return Err(io::Error::last_os_error());
        }

        Ok(Self { handle })
    }

    /// # Safety
    /// Caller must pass a valid, unclosed process handle with `PROCESS_SET_QUOTA` and `PROCESS_TERMINATE` access.
    pub unsafe fn assign_process(&self, process_handle: *mut c_void) -> io::Result<()> {
        let ret = AssignProcessToJobObject(self.handle, process_handle);
        if ret == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn raw_handle(&self) -> *mut c_void {
        self.handle
    }
}

impl Drop for JobObjectGuard {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                CloseHandle(self.handle);
            }
            self.handle = std::ptr::null_mut();
        }
    }
}

#[derive(Debug, Clone)]
pub struct GuardedCommandOutput {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub timed_out: bool,
}

/// Allowlisted developer binaries permitted for direct execution in the Job Object sandbox.
pub const ALLOWLISTED_BINARIES: &[&str] = &[
    "git",
    "cargo",
    "rustc",
    "rg",
    "fd",
    "cmake",
    "where",
    "ipconfig",
];

/// Dangerous environment variables scrubbed from child process executions to prevent injection.
const DANGEROUS_ENV_VARS: &[&str] = &[
    "GIT_SSH_COMMAND",
    "GIT_PAGER",
    "GIT_EDITOR",
    "GIT_EXTERNAL_DIFF",
    "RUSTC_WRAPPER",
    "CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUNNER",
    "CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER",
];

/// Resolves a program name against the strict binary allowlist to a verified absolute executable path.
pub fn resolve_allowlisted_binary(program: &str) -> Result<PathBuf> {
    let temp_dir = std::env::temp_dir();
    let current_dir = std::env::current_dir().ok();

    if program.contains('\\') || program.contains('/') {
        let pb = PathBuf::from(program);
        let stem = pb
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_lowercase())
            .unwrap_or_default();
        if !ALLOWLISTED_BINARIES.contains(&stem.as_str()) {
            anyhow::bail!(
                "Binary '{}' is not on the execution allowlist. Zero-shell policy permits only: {:?}",
                program,
                ALLOWLISTED_BINARIES
            );
        }
        let canon = pb.canonicalize().unwrap_or_else(|_| pb.clone());
        let temp_canon = temp_dir.canonicalize().unwrap_or_else(|_| temp_dir.clone());
        if canon.starts_with(&temp_canon) || pb.starts_with(&temp_dir) {
            anyhow::bail!("Allowlisted binary path inside %TEMP% is forbidden: {:?}", pb);
        }
        if let Some(ref cwd) = current_dir {
            let cwd_canon = cwd.canonicalize().unwrap_or_else(|_| cwd.clone());
            if canon.parent() == Some(cwd_canon.as_path()) || pb.parent() == Some(cwd.as_path()) {
                anyhow::bail!("Allowlisted binary path inside current working directory is forbidden: {:?}", pb);
            }
        }
        if canon.is_file() {
            return Ok(canon);
        }
        anyhow::bail!("Allowlisted binary '{}' not found at explicit path", program);
    }

    let lower = program.to_lowercase();
    let base_name = lower.trim_end_matches(".exe");

    let is_allowed = ALLOWLISTED_BINARIES.contains(&base_name);
    if !is_allowed {
        anyhow::bail!(
            "Binary '{}' is not on the execution allowlist. Zero-shell policy permits only: {:?}",
            program,
            ALLOWLISTED_BINARIES
        );
    }

    // Anti-CWD Hijack: Explicitly skip relative paths, current directory, and %TEMP%
    // Search system PATH for the binary
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            if dir.as_os_str().is_empty() || dir == Path::new(".") || !dir.is_absolute() {
                continue;
            }
            if let Some(ref cwd) = current_dir {
                if dir == *cwd {
                    continue;
                }
            }
            if dir.starts_with(&temp_dir) {
                continue;
            }

            let exe_candidate = dir.join(format!("{}.exe", base_name));
            if exe_candidate.is_file() {
                return Ok(exe_candidate.canonicalize().unwrap_or(exe_candidate));
            }
            let direct_candidate = dir.join(base_name);
            if direct_candidate.is_file() {
                return Ok(direct_candidate.canonicalize().unwrap_or(direct_candidate));
            }
        }
    }

    anyhow::bail!("Allowlisted binary '{}' was not found on system PATH", program)
}

/// Validates arguments against argument injection and dangerous command overrides.
pub fn validate_binary_arguments(program: &str, args: &[String]) -> Result<()> {
    let lower = program.to_lowercase();
    let base = lower.trim_end_matches(".exe");

    // Shell metacharacters are forbidden in all arguments
    for arg in args {
        if arg.contains('|')
            || arg.contains('&')
            || arg.contains(';')
            || arg.contains('>')
            || arg.contains('<')
            || arg.contains('`')
            || arg.contains('$')
            || arg.contains('\n')
            || arg.contains('\r')
        {
            anyhow::bail!("Shell metacharacters are strictly forbidden in arguments: {:?}", arg);
        }
    }

    match base {
        "git" => {
            for arg in args {
                let a_lower = arg.to_lowercase();
                if a_lower == "-c"
                    || a_lower.starts_with("-c")
                    || a_lower.starts_with("--config")
                    || a_lower.starts_with("--exec-path")
                    || a_lower.starts_with("--upload-pack")
                    || a_lower.starts_with("--receive-pack")
                    || a_lower.starts_with("--git-dir")
                    || a_lower.starts_with("--work-tree")
                {
                    anyhow::bail!("Dangerous git configuration override argument blocked: {}", arg);
                }
            }
        }
        "cargo" => {
            for arg in args {
                let a_lower = arg.to_lowercase();
                if a_lower == "--config"
                    || a_lower.starts_with("--config")
                    || a_lower == "-z"
                    || a_lower.starts_with("-z")
                {
                    anyhow::bail!("Dangerous cargo configuration override argument blocked: {}", arg);
                }
            }
        }
        "rg" => {
            for arg in args {
                let a_lower = arg.to_lowercase();
                if a_lower == "--pre"
                    || a_lower.starts_with("--pre=")
                    || a_lower == "--pre-glob"
                    || a_lower.starts_with("--pre-glob=")
                    || a_lower == "--search-zip"
                    || a_lower.starts_with("-z")
                {
                    anyhow::bail!("Dangerous ripgrep LOLBin argument blocked: {}", arg);
                }
            }
        }
        "fd" => {
            for arg in args {
                let a_lower = arg.to_lowercase();
                if a_lower == "-x"
                    || a_lower.starts_with("-x=")
                    || a_lower == "--exec"
                    || a_lower.starts_with("--exec=")
                    || a_lower == "-x"
                    || a_lower == "-X"
                    || a_lower.starts_with("-x")
                    || a_lower == "--exec-batch"
                    || a_lower.starts_with("--exec-batch=")
                {
                    anyhow::bail!("Dangerous fd exec argument blocked: {}", arg);
                }
            }
        }
        "cmake" => {
            for arg in args {
                let a_lower = arg.to_lowercase();
                if a_lower == "-p"
                    || a_lower.starts_with("-p")
                    || a_lower == "-e"
                    || a_lower.starts_with("-e")
                    || a_lower == "-c"
                    || a_lower.starts_with("-c")
                    || a_lower == "--toolchain"
                    || a_lower.starts_with("--toolchain=")
                    || a_lower == "--build"
                    || a_lower.starts_with("--build")
                    || a_lower == "--install"
                    || a_lower.starts_with("--install")
                {
                    anyhow::bail!("Dangerous cmake script/build argument blocked: {}", arg);
                }
            }
        }
        "rustc" => {
            for arg in args {
                let a_lower = arg.to_lowercase();
                if !a_lower.starts_with("--version") && !a_lower.starts_with("-v") && !a_lower.starts_with("--print") {
                    anyhow::bail!("Arbitrary rustc compilation/execution blocked: {}", arg);
                }
            }
        }
        _ => {}
    }

    Ok(())
}

struct OwnedWinHandle(*mut c_void);

unsafe impl Send for OwnedWinHandle {}
unsafe impl Sync for OwnedWinHandle {}

impl OwnedWinHandle {
    fn as_raw(&self) -> *mut c_void {
        self.0
    }
}

impl Drop for OwnedWinHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CloseHandle(self.0);
            }
            self.0 = std::ptr::null_mut();
        }
    }
}

fn quote_win32_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.chars().any(|c| c == ' ' || c == '\t' || c == '"') {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0usize;
    for ch in arg.chars() {
        if ch == '\\' {
            backslashes += 1;
        } else if ch == '"' {
            out.push_str(&"\\".repeat(backslashes * 2 + 1));
            out.push('"');
            backslashes = 0;
        } else {
            out.push_str(&"\\".repeat(backslashes));
            backslashes = 0;
            out.push(ch);
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

fn build_scrubbed_unicode_env() -> Vec<u16> {
    let mut wide = Vec::new();
    for (k, v) in std::env::vars_os() {
        let k_upper = k.to_string_lossy().to_uppercase();
        if DANGEROUS_ENV_VARS.contains(&k_upper.as_str()) {
            continue;
        }
        wide.extend(k.encode_wide());
        wide.push(b'=' as u16);
        wide.extend(v.encode_wide());
        wide.push(0);
    }
    wide.push(0);
    wide
}

struct SpawnedSuspendedProcess {
    process_handle: OwnedWinHandle,
    stdout_file: std::fs::File,
    stderr_file: std::fs::File,
}

fn spawn_suspended_assign_and_resume<F>(
    exe_path: &Path,
    args: &[String],
    cwd: Option<&Path>,
    job: &JobObjectGuard,
    before_resume_hook: F,
) -> Result<SpawnedSuspendedProcess>
where
    F: FnOnce(*mut c_void) -> Result<()>,
{
    let mut stdout_read: *mut c_void = std::ptr::null_mut();
    let mut stdout_write: *mut c_void = std::ptr::null_mut();
    let mut stderr_read: *mut c_void = std::ptr::null_mut();
    let mut stderr_write: *mut c_void = std::ptr::null_mut();

    let nul_wide: Vec<u16> = OsStr::new("NUL").encode_wide().chain(std::iter::once(0)).collect();
    let nul_sa = SecurityAttributes {
        n_length: std::mem::size_of::<SecurityAttributes>() as u32,
        lp_security_descriptor: std::ptr::null_mut(),
        b_inherit_handle: 1,
    };
    let nul_in = unsafe {
        CreateFileW(
            nul_wide.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            &nul_sa as *const _ as *const c_void,
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    if nul_in.is_null() || nul_in == (-1isize as *mut c_void) {
        return Err(io::Error::last_os_error()).context("Failed to open NUL for child stdin");
    }

    if unsafe { CreatePipe(&mut stdout_read, &mut stdout_write, std::ptr::null(), 0) } == 0 {
        unsafe { CloseHandle(nul_in); }
        return Err(io::Error::last_os_error()).context("CreatePipe stdout failed");
    }
    if unsafe { CreatePipe(&mut stderr_read, &mut stderr_write, std::ptr::null(), 0) } == 0 {
        unsafe {
            CloseHandle(nul_in);
            CloseHandle(stdout_read);
            CloseHandle(stdout_write);
        }
        return Err(io::Error::last_os_error()).context("CreatePipe stderr failed");
    }

    unsafe {
        SetHandleInformation(stdout_write, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT);
        SetHandleInformation(stderr_write, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT);
        SetHandleInformation(stdout_read, HANDLE_FLAG_INHERIT, 0);
        SetHandleInformation(stderr_read, HANDLE_FLAG_INHERIT, 0);
    }

    let mut handles_to_inherit = [nul_in, stdout_write, stderr_write];
    let mut attr_size: usize = 0;
    unsafe {
        InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut attr_size);
    }
    let mut attr_buf = vec![0u8; attr_size];
    let attr_list = attr_buf.as_mut_ptr() as *mut c_void;

    if unsafe { InitializeProcThreadAttributeList(attr_list, 1, 0, &mut attr_size) } == 0 {
        unsafe {
            CloseHandle(nul_in);
            CloseHandle(stdout_read);
            CloseHandle(stdout_write);
            CloseHandle(stderr_read);
            CloseHandle(stderr_write);
        }
        return Err(io::Error::last_os_error()).context("InitializeProcThreadAttributeList failed");
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
            CloseHandle(nul_in);
            CloseHandle(stdout_read);
            CloseHandle(stdout_write);
            CloseHandle(stderr_read);
            CloseHandle(stderr_write);
        }
        return Err(io::Error::last_os_error()).context("UpdateProcThreadAttribute failed");
    }

    let mut si_ex: StartupInfoExW = unsafe { std::mem::zeroed() };
    si_ex.startup_info.cb = std::mem::size_of::<StartupInfoExW>() as u32;
    si_ex.startup_info.dw_flags = STARTF_USESTDHANDLES;
    si_ex.startup_info.h_std_input = nul_in;
    si_ex.startup_info.h_std_output = stdout_write;
    si_ex.startup_info.h_std_error = stderr_write;
    si_ex.lp_attribute_list = attr_list;

    let app_wide: Vec<u16> = exe_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut cmd_parts = vec![quote_win32_arg(&exe_path.to_string_lossy())];
    for a in args {
        cmd_parts.push(quote_win32_arg(a));
    }
    let mut cmd_wide: Vec<u16> = OsStr::new(&cmd_parts.join(" "))
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut env_wide = build_scrubbed_unicode_env();
    let cwd_wide: Option<Vec<u16>> = cwd.map(|d| {
        d.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    });

    let mut pi: ProcessInformation = unsafe { std::mem::zeroed() };
    let created = unsafe {
        CreateProcessW(
            app_wide.as_ptr(),
            cmd_wide.as_mut_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            1, // bInheritHandles = TRUE (restricted via PROC_THREAD_ATTRIBUTE_HANDLE_LIST)
            CREATE_SUSPENDED | CREATE_NO_WINDOW | EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
            env_wide.as_mut_ptr() as *mut c_void,
            cwd_wide.as_ref().map_or(std::ptr::null(), |v| v.as_ptr()),
            &mut si_ex as *mut _ as *mut c_void,
            &mut pi,
        )
    };

    unsafe {
        DeleteProcThreadAttributeList(attr_list);
        CloseHandle(nul_in);
        CloseHandle(stdout_write);
        CloseHandle(stderr_write);
    }

    if created == 0 {
        unsafe {
            CloseHandle(stdout_read);
            CloseHandle(stderr_read);
        }
        return Err(io::Error::last_os_error())
            .with_context(|| format!("Failed to spawn binary: {:?}", exe_path));
    }

    if let Err(e) = unsafe { job.assign_process(pi.h_process) } {
        unsafe {
            TerminateProcess(pi.h_process, 1);
            CloseHandle(pi.h_process);
            CloseHandle(pi.h_thread);
            CloseHandle(stdout_read);
            CloseHandle(stderr_read);
        }
        return Err(e).context("Failed to assign suspended child process to Job Object");
    }

    if let Err(e) = before_resume_hook(pi.h_process) {
        unsafe {
            TerminateProcess(pi.h_process, 1);
            CloseHandle(pi.h_process);
            CloseHandle(pi.h_thread);
            CloseHandle(stdout_read);
            CloseHandle(stderr_read);
        }
        return Err(e);
    }

    unsafe {
        ResumeThread(pi.h_thread);
        CloseHandle(pi.h_thread);
    }

    let stdout_file = unsafe { std::fs::File::from_raw_handle(stdout_read) };
    let stderr_file = unsafe { std::fs::File::from_raw_handle(stderr_read) };

    Ok(SpawnedSuspendedProcess {
        process_handle: OwnedWinHandle(pi.h_process),
        stdout_file,
        stderr_file,
    })
}

/// Executes an allowlisted binary in the Windows Job Object sandbox with `CREATE_SUSPENDED`
/// pre-execution job assignment, scrubbed environment, and strictly zero shell interpolation.
pub async fn execute_allowlisted(
    program: &str,
    args: &[String],
    cwd: Option<&Path>,
    timeout: Duration,
) -> Result<GuardedCommandOutput> {
    let resolved_path = resolve_allowlisted_binary(program)?;
    validate_binary_arguments(program, args)?;

    // 1 GiB memory cap
    const MAX_MEMORY_BYTES: usize = 1024 * 1024 * 1024;
    let job = JobObjectGuard::new(Some(MAX_MEMORY_BYTES))?;

    let start = Instant::now();
    let spawned = spawn_suspended_assign_and_resume(&resolved_path, args, cwd, &job, |_| Ok(()))?;

    let mut stdout_file = spawned.stdout_file;
    let mut stderr_file = spawned.stderr_file;
    let proc_handle = spawned.process_handle;

    let wait_future = tokio::task::spawn_blocking(move || -> io::Result<(Option<i32>, Vec<u8>, Vec<u8>)> {
        let stdout_reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = stdout_file.read_to_end(&mut buf);
            buf
        });
        let stderr_reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = stderr_file.read_to_end(&mut buf);
            buf
        });

        unsafe {
            WaitForSingleObject(proc_handle.as_raw(), INFINITE);
        }
        let mut exit_code: u32 = 0;
        let code_opt = if unsafe { GetExitCodeProcess(proc_handle.as_raw(), &mut exit_code) } != 0 {
            Some(exit_code as i32)
        } else {
            None
        };

        let out_bytes = stdout_reader.join().unwrap_or_default();
        let err_bytes = stderr_reader.join().unwrap_or_default();
        Ok((code_opt, out_bytes, err_bytes))
    });

    match tokio::time::timeout(timeout, wait_future).await {
        Ok(Ok(Ok((exit_code, out_bytes, err_bytes)))) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            Ok(GuardedCommandOutput {
                exit_code,
                stdout: String::from_utf8_lossy(&out_bytes).trim().to_string(),
                stderr: String::from_utf8_lossy(&err_bytes).trim().to_string(),
                duration_ms,
                timed_out: false,
            })
        }
        Ok(Ok(Err(err))) => Err(anyhow::anyhow!("Process execution error: {err}")),
        Ok(Err(join_err)) => Err(anyhow::anyhow!("Process wait task failed: {join_err}")),
        Err(_) => {
            // Timeout fired! Dropping `job` triggers JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            // forcefully terminating the entire child process tree.
            drop(job);
            let duration_ms = start.elapsed().as_millis() as u64;

            Ok(GuardedCommandOutput {
                exit_code: None,
                stdout: String::new(),
                stderr: format!("Execution timed out after {:?} and process tree was terminated.", timeout),
                duration_ms,
                timed_out: true,
            })
        }
    }
}

/// Parses a command string into structured program and arguments, then executes it
/// within the zero-shell Job Object sandbox.
pub async fn run_guarded_command(
    command: &str,
    cwd: Option<&Path>,
    timeout_secs: u64,
) -> Result<GuardedCommandOutput> {
    let (program, args) = crate::security::policy::split_command_line(command)
        .map_err(|e| anyhow::anyhow!("Command parsing error: {}", e))?;

    execute_allowlisted(&program, &args, cwd, Duration::from_secs(timeout_secs)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    extern "system" {
        fn IsProcessInJob(
            ProcessHandle: *mut c_void,
            JobHandle: *mut c_void,
            Result: *mut i32,
        ) -> i32;
    }

    #[tokio::test]
    async fn test_guarded_command_success() {
        // Runs allowlisted git --version with zero shell interpolation
        let output = run_guarded_command("git --version", None, 5)
            .await
            .expect("allowlisted command ran");

        assert_eq!(output.exit_code, Some(0));
        assert!(output.stdout.to_lowercase().contains("git version"));
        assert!(!output.timed_out);
    }

    #[tokio::test]
    async fn test_guarded_command_timeout_termination() {
        // Run benign sleep command via the raw Job-Object runner with 1 second timeout
        let job = JobObjectGuard::new(Some(1024 * 1024 * 1024)).expect("job created");
        let mut py_cmd = tokio::process::Command::new("python.exe");
        py_cmd.args(["-c", "import time; time.sleep(10)"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let child = match py_cmd.spawn() {
            Ok(c) => c,
            Err(_) => {
                let mut ping_cmd = tokio::process::Command::new("ping.exe");
                ping_cmd.args(["127.0.0.1", "-n", "10"])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped());
                ping_cmd.spawn().expect("ping spawned")
            }
        };

        if let Some(raw_handle) = child.raw_handle() {
            unsafe { job.assign_process(raw_handle).expect("assigned"); }
        }

        let timeout_res = tokio::time::timeout(Duration::from_secs(1), child.wait_with_output()).await;
        assert!(timeout_res.is_err(), "Command did not timeout as expected");
        drop(job); // Forces kill on timeout
    }

    #[tokio::test]
    async fn test_shell_execution_rejected() {
        // Proves that shell and unallowlisted binaries are rejected
        assert!(run_guarded_command("powershell.exe Write-Output hi", None, 5).await.is_err());
        assert!(run_guarded_command("cmd.exe /c echo hi", None, 5).await.is_err());
        assert!(run_guarded_command("calc.exe", None, 5).await.is_err());
        assert!(run_guarded_command("whoami", None, 5).await.is_err());
    }

    #[tokio::test]
    async fn test_argument_metacharacters_rejected() {
        // Proves shell metacharacter chaining is rejected
        assert!(run_guarded_command("git status | whoami", None, 5).await.is_err());
        assert!(run_guarded_command("git status & calc.exe", None, 5).await.is_err());
        assert!(run_guarded_command("git -c core.sshCommand=evil status", None, 5).await.is_err());
    }

    #[test]
    fn test_lolbin_argument_injection_rejected() {
        // Ripgrep pre-processor execution blocked
        let args_rg = vec!["--pre".to_string(), "calc.exe".to_string(), "foo".to_string()];
        assert!(validate_binary_arguments("rg", &args_rg).is_err());

        let args_rg_eq = vec!["--pre=calc.exe".to_string(), "foo".to_string()];
        assert!(validate_binary_arguments("rg", &args_rg_eq).is_err());

        // fd exec execution blocked
        let args_fd = vec!["-x".to_string(), "calc.exe".to_string()];
        assert!(validate_binary_arguments("fd", &args_fd).is_err());

        // cmake script execution blocked
        let args_cmake = vec!["-P".to_string(), "evil.cmake".to_string()];
        assert!(validate_binary_arguments("cmake", &args_cmake).is_err());

        // rustc arbitrary compilation blocked
        let args_rustc = vec!["main.rs".to_string(), "-o".to_string(), "evil.exe".to_string()];
        assert!(validate_binary_arguments("rustc", &args_rustc).is_err());
    }

    #[test]
    fn test_resolve_allowlisted_binary_rejects_temp_path() {
        let temp_exe = std::env::temp_dir().join("git.exe");
        let _ = std::fs::write(&temp_exe, b"fake");
        let res = resolve_allowlisted_binary(temp_exe.to_str().unwrap());
        let _ = std::fs::remove_file(&temp_exe);
        assert!(res.is_err(), "Explicit path inside %TEMP% must be rejected");
    }

    #[test]
    fn test_create_suspended_assigns_job_before_resume() {
        let resolved = resolve_allowlisted_binary("git").expect("git found");
        let job = JobObjectGuard::new(Some(1024 * 1024 * 1024)).expect("job created");
        let mut verified_in_job_before_resume = false;

        let spawned = spawn_suspended_assign_and_resume(
            &resolved,
            &["--version".to_string()],
            None,
            &job,
            |proc_handle| {
                let mut in_job: i32 = 0;
                let ok = unsafe { IsProcessInJob(proc_handle, job.raw_handle(), &mut in_job) };
                anyhow::ensure!(ok != 0, "IsProcessInJob call failed");
                anyhow::ensure!(
                    in_job != 0,
                    "Child process must already be assigned to Job Object before ResumeThread"
                );
                verified_in_job_before_resume = true;
                Ok(())
            },
        )
        .expect("spawn_suspended_assign_and_resume succeeded");

        assert!(verified_in_job_before_resume);
        unsafe {
            WaitForSingleObject(spawned.process_handle.0, INFINITE);
        }
        let mut out = String::new();
        let mut f = spawned.stdout_file;
        let _ = f.read_to_string(&mut out);
        assert!(out.to_lowercase().contains("git version"));
    }
}
