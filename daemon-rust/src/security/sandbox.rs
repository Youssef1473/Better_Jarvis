use anyhow::{Context, Result};
use std::ffi::c_void;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
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

const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x00002000;
const JOB_OBJECT_LIMIT_JOB_MEMORY: u32 = 0x00000200;
const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: u32 = 9;

extern "system" {
    fn CreateJobObjectW(lpJobAttributes: *mut c_void, lpName: *const u16) -> *mut c_void;
    fn SetInformationJobObject(
        hJob: *mut c_void,
        JobObjectInformationClass: u32,
        lpJobObjectInformation: *const c_void,
        cbJobObjectInformationLength: u32,
    ) -> i32;
    fn AssignProcessToJobObject(hJob: *mut c_void, hProcess: *mut c_void) -> i32;
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

    // If an explicit path was provided, verify it exists directly
    if program.contains('\\') || program.contains('/') {
        let pb = PathBuf::from(program);
        if pb.is_file() {
            return Ok(pb.canonicalize().unwrap_or(pb));
        }
    }

    // Anti-CWD Hijack: Explicitly skip relative paths, current directory, and %TEMP%
    let current_dir = std::env::current_dir().ok();
    let temp_dir = std::env::temp_dir();

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

/// Executes an allowlisted binary in the Windows Job Object sandbox with scrubbed environment
/// and strictly zero shell interpolation.
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

    let mut cmd = tokio::process::Command::new(&resolved_path);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Scrub dangerous injection environment variables
    for var in DANGEROUS_ENV_VARS {
        cmd.env_remove(var);
    }

    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }

    let start = Instant::now();
    let child = cmd.spawn().with_context(|| format!("Failed to spawn binary: {:?}", resolved_path))?;

    if let Some(raw_handle) = child.raw_handle() {
        unsafe { job.assign_process(raw_handle)?; }
    }

    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(output)) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            Ok(GuardedCommandOutput {
                exit_code: output.status.code(),
                stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
                duration_ms,
                timed_out: false,
            })
        }
        Ok(Err(err)) => Err(anyhow::anyhow!("Process execution error: {err}")),
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
        // Run benign ping.exe 127.0.0.1 -n 10 via the raw Job-Object runner with 1 second timeout
        let job = JobObjectGuard::new(Some(1024 * 1024 * 1024)).expect("job created");
        let mut cmd = tokio::process::Command::new("ping.exe");
        cmd.args(["127.0.0.1", "-n", "10"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let child = cmd.spawn().expect("ping spawned");
        if let Some(raw_handle) = child.raw_handle() {
            unsafe { job.assign_process(raw_handle).expect("assigned"); }
        }

        let timeout_res = tokio::time::timeout(Duration::from_secs(1), child.wait_with_output()).await;
        assert!(timeout_res.is_err(), "Ping command did not timeout as expected");
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
}
