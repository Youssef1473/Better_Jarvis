use std::ffi::{c_void, OsStr};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::AsRawHandle;
use std::ptr;
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tracing::{debug, info};

const TOKEN_QUERY: u32 = 0x0008;
const TOKEN_USER_CLASS: u32 = 1;
const TOKEN_LOGON_SID_CLASS: u32 = 28;
const SDDL_REVISION_1: u32 = 1;
const TH32CS_SNAPPROCESS: u32 = 0x00000002;
const INVALID_HANDLE_VALUE: *mut c_void = -1isize as *mut c_void;

#[repr(C)]
struct SidAndAttributes {
    sid: *mut c_void,
    attributes: u32,
}

#[repr(C)]
struct TokenGroups {
    group_count: u32,
    groups: [SidAndAttributes; 1],
}

#[repr(C)]
struct TokenUser {
    user: SidAndAttributes,
}

#[repr(C)]
pub struct SecurityAttributes {
    pub n_length: u32,
    pub lp_security_descriptor: *mut c_void,
    pub b_inherit_handle: i32,
}

#[repr(C)]
pub struct ProcessEntry32W {
    pub dw_size: u32,
    pub cnt_usage: u32,
    pub th32_process_id: u32,
    pub th32_default_heap_id: usize,
    pub th32_module_id: u32,
    pub cnt_threads: u32,
    pub th32_parent_process_id: u32,
    pub pc_pri_class_base: i32,
    pub dw_flags: u32,
    pub sz_exe_file: [u16; 260],
}

#[link(name = "advapi32")]
#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcess() -> *mut c_void;
    fn GetCurrentProcessId() -> u32;
    fn OpenProcessToken(
        process_handle: *mut c_void,
        desired_access: u32,
        token_handle: *mut *mut c_void,
    ) -> i32;
    fn GetTokenInformation(
        token_handle: *mut c_void,
        token_information_class: u32,
        token_information: *mut c_void,
        token_information_length: u32,
        return_length: *mut u32,
    ) -> i32;
    fn ConvertSidToStringSidW(sid: *mut c_void, string_sid: *mut *mut u16) -> i32;
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        string_security_descriptor: *const u16,
        string_sd_revision: u32,
        security_descriptor: *mut *mut c_void,
        security_descriptor_size: *mut u32,
    ) -> i32;
    fn LocalFree(h_mem: *mut c_void) -> *mut c_void;
    fn CloseHandle(h_object: *mut c_void) -> i32;
    fn CreateToolhelp32Snapshot(dw_flags: u32, th32_process_id: u32) -> *mut c_void;
    fn Process32FirstW(h_snapshot: *mut c_void, lppe: *mut ProcessEntry32W) -> i32;
    fn Process32NextW(h_snapshot: *mut c_void, lppe: *mut ProcessEntry32W) -> i32;
    fn GetNamedPipeClientProcessId(pipe: *mut c_void, client_process_id: *mut u32) -> i32;
}

/// RAII Guard for a Windows Security Descriptor allocated with LocalAlloc
pub struct SecurityDescriptorGuard {
    ptr: *mut c_void,
}

unsafe impl Send for SecurityDescriptorGuard {}
unsafe impl Sync for SecurityDescriptorGuard {}

impl SecurityDescriptorGuard {
    pub fn as_ptr(&self) -> *mut c_void {
        self.ptr
    }
}

impl Drop for SecurityDescriptorGuard {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe {
                LocalFree(self.ptr);
            }
            self.ptr = ptr::null_mut();
        }
    }
}

fn to_wide_null(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn from_wide_null(ptr: *const u16) -> String {
    let mut len = 0;
    unsafe {
        while *ptr.add(len) != 0 {
            len += 1;
        }
        let slice = std::slice::from_raw_parts(ptr, len);
        String::from_utf16_lossy(slice)
    }
}

/// Retrieves the current process Logon SID, falling back to User SID if TokenLogonSid is absent.
pub fn get_current_logon_or_user_sid() -> io::Result<String> {
    unsafe {
        let mut token: *mut c_void = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(io::Error::last_os_error());
        }

        // Try TokenLogonSid first
        let mut return_length: u32 = 0;
        let _ = GetTokenInformation(
            token,
            TOKEN_LOGON_SID_CLASS,
            ptr::null_mut(),
            0,
            &mut return_length,
        );

        let mut string_sid_ptr: *mut u16 = ptr::null_mut();

        if return_length > 0 {
            let mut buf = vec![0u8; return_length as usize];
            if GetTokenInformation(
                token,
                TOKEN_LOGON_SID_CLASS,
                buf.as_mut_ptr() as *mut c_void,
                return_length,
                &mut return_length,
            ) != 0 {
                let groups = &*(buf.as_ptr() as *const TokenGroups);
                if groups.group_count > 0
                    && !groups.groups[0].sid.is_null()
                    && ConvertSidToStringSidW(groups.groups[0].sid, &mut string_sid_ptr) != 0
                {
                    let sid_str = from_wide_null(string_sid_ptr);
                    LocalFree(string_sid_ptr as *mut c_void);
                    CloseHandle(token);
                    debug!("Retrieved process TokenLogonSid: {}", sid_str);
                    return Ok(sid_str);
                }
            }
        }

        // Fallback: Query TokenUser
        return_length = 0;
        let _ = GetTokenInformation(
            token,
            TOKEN_USER_CLASS,
            ptr::null_mut(),
            0,
            &mut return_length,
        );

        if return_length == 0 {
            CloseHandle(token);
            return Err(io::Error::last_os_error());
        }

        let mut buf = vec![0u8; return_length as usize];
        if GetTokenInformation(
            token,
            TOKEN_USER_CLASS,
            buf.as_mut_ptr() as *mut c_void,
            return_length,
            &mut return_length,
        ) == 0 {
            let err = io::Error::last_os_error();
            CloseHandle(token);
            return Err(err);
        }

        let token_user = &*(buf.as_ptr() as *const TokenUser);
        if token_user.user.sid.is_null() {
            CloseHandle(token);
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "TokenUser SID is null",
            ));
        }

        if ConvertSidToStringSidW(token_user.user.sid, &mut string_sid_ptr) == 0 {
            let err = io::Error::last_os_error();
            CloseHandle(token);
            return Err(err);
        }

        let sid_str = from_wide_null(string_sid_ptr);
        LocalFree(string_sid_ptr as *mut c_void);
        CloseHandle(token);

        info!("Retrieved process TokenUser SID fallback: {}", sid_str);
        Ok(sid_str)
    }
}

/// Builds a protected SDDL DACL granting Generic All strictly to the target SID.
/// Format: D:P(A;;GA;;;{SID})
/// D: Discretionary ACL
/// P: Protected from inheritance
/// A;;GA: Allow Generic All
pub fn build_strict_sddl_for_sid(sid: &str) -> String {
    format!("D:P(A;;GA;;;{})", sid)
}

/// Compiles an SDDL string into an RAII-guarded Windows Security Descriptor.
pub fn create_pipe_security_descriptor() -> io::Result<SecurityDescriptorGuard> {
    let sid = get_current_logon_or_user_sid()?;
    let sddl = build_strict_sddl_for_sid(&sid);
    let wide_sddl = to_wide_null(&sddl);

    let mut sec_desc: *mut c_void = ptr::null_mut();
    let mut sec_desc_size: u32 = 0;

    let res = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            wide_sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut sec_desc,
            &mut sec_desc_size,
        )
    };

    if res == 0 || sec_desc.is_null() {
        return Err(io::Error::last_os_error());
    }

    Ok(SecurityDescriptorGuard { ptr: sec_desc })
}

/// Creates a hardened NamedPipeServer instance.
/// - Enforces `reject_remote_clients(true)`
/// - Applies explicit SDDL DACL (no inheritance, current logon/user SID only)
/// - Sets `first_pipe_instance(true)` if `is_first` is true
pub fn create_hardened_pipe_server(
    pipe_name: &str,
    is_first: bool,
    sec_desc: &SecurityDescriptorGuard,
) -> io::Result<NamedPipeServer> {
    let mut options = ServerOptions::new();
    if is_first {
        options.first_pipe_instance(true);
    }
    options.reject_remote_clients(true);

    let mut sec_attrs = SecurityAttributes {
        n_length: std::mem::size_of::<SecurityAttributes>() as u32,
        lp_security_descriptor: sec_desc.as_ptr(),
        b_inherit_handle: 0, // FALSE
    };

    unsafe {
        options.create_with_security_attributes_raw(
            pipe_name,
            &mut sec_attrs as *mut _ as *mut c_void,
        )
    }
}

/// Discovers the real OS parent process ID of the current process via Toolhelp32 snapshot.
pub fn get_parent_pid() -> io::Result<u32> {
    unsafe {
        let current_pid = GetCurrentProcessId();
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }

        let mut entry = ProcessEntry32W {
            dw_size: std::mem::size_of::<ProcessEntry32W>() as u32,
            cnt_usage: 0,
            th32_process_id: 0,
            th32_default_heap_id: 0,
            th32_module_id: 0,
            cnt_threads: 0,
            th32_parent_process_id: 0,
            pc_pri_class_base: 0,
            dw_flags: 0,
            sz_exe_file: [0u16; 260],
        };

        if Process32FirstW(snapshot, &mut entry) == 0 {
            CloseHandle(snapshot);
            return Err(io::Error::last_os_error());
        }

        loop {
            if entry.th32_process_id == current_pid {
                let parent_pid = entry.th32_parent_process_id;
                CloseHandle(snapshot);
                return Ok(parent_pid);
            }
            if Process32NextW(snapshot, &mut entry) == 0 {
                break;
            }
        }

        CloseHandle(snapshot);
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Current process not found in snapshot",
        ))
    }
}

/// Queries the client process ID connected to the Named Pipe.
pub fn get_pipe_client_pid(pipe: &NamedPipeServer) -> io::Result<u32> {
    let mut client_pid: u32 = 0;
    let res = unsafe {
        GetNamedPipeClientProcessId(pipe.as_raw_handle(), &mut client_pid)
    };
    if res == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(client_pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_security_descriptor_creation() {
        let guard = create_pipe_security_descriptor().expect("SDDL conversion succeeded");
        assert!(!guard.as_ptr().is_null());
    }

    #[test]
    fn test_sddl_generation() {
        let sid = get_current_logon_or_user_sid().expect("SID retrieved");
        assert!(sid.starts_with("S-1-"));
        let sddl = build_strict_sddl_for_sid(&sid);
        assert_eq!(sddl, format!("D:P(A;;GA;;;{})", sid));
    }

    #[test]
    fn test_get_parent_pid_returns_valid_id() {
        let parent_pid = get_parent_pid().expect("Parent PID queried");
        assert!(parent_pid > 0);
    }

    #[tokio::test]
    async fn test_first_pipe_instance_conflict() {
        let pipe_name = format!(
            r"\\.\pipe\jarvis-test-conflict-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );

        let sec_desc = create_pipe_security_descriptor().unwrap();

        // 1. First instance must succeed
        let _server1 = create_hardened_pipe_server(&pipe_name, true, &sec_desc)
            .expect("First pipe instance must succeed");

        // 2. Second instance specifying first_pipe_instance(true) MUST fail
        let server2_res = create_hardened_pipe_server(&pipe_name, true, &sec_desc);
        assert!(
            server2_res.is_err(),
            "Second first_pipe_instance must fail on existing pipe"
        );
    }
}
