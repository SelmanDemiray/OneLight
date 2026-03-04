///! Windows security sandbox — restricted tokens for process isolation.

use crate::error::{ContainerError, Result};
use super::winapi::*;

/// Create a restricted token from the current process token.
/// Strips most privileges to sandbox the container process.
pub fn create_restricted_token() -> Result<HANDLE> {
    let mut current_token: HANDLE = NULL_HANDLE;

    // Open current process token
    let ret = unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ALL_ACCESS,
            &mut current_token,
        )
    };
    if ret == FALSE {
        return Err(crate::error::syscall_error("OpenProcessToken"));
    }

    // Create restricted token with max privilege removal
    let mut restricted_token: HANDLE = NULL_HANDLE;
    let ret = unsafe {
        CreateRestrictedToken(
            current_token,
            DISABLE_MAX_PRIVILEGE, // Remove all privileges
            0,                      // No SIDs to disable
            std::ptr::null_mut(),   // SidsToDisable
            0,                      // No privileges to explicitly delete
            std::ptr::null_mut(),   // PrivilegesToDelete
            0,                      // No restricting SIDs
            std::ptr::null_mut(),   // SidsToRestrict
            &mut restricted_token,
        )
    };

    unsafe { CloseHandle(current_token); }

    if ret == FALSE {
        return Err(crate::error::syscall_error("CreateRestrictedToken"));
    }

    Ok(restricted_token)
}

/// Spawn a process with a restricted token inside a job object.
pub fn spawn_sandboxed_process(
    command_line: &str,
    working_dir: &str,
    env_block: Option<&[u16]>,
    job_handle: HANDLE,
) -> Result<(HANDLE, HANDLE, u32)> {
    // Try with restricted token first
    let token_result = create_restricted_token();

    let mut si = startup_info();
    let mut pi = process_info();

    let mut cmd_wide = to_wide(command_line);
    let dir_wide = to_wide(working_dir);

    let env_ptr = env_block
        .map(|e| e.as_ptr() as LPVOID)
        .unwrap_or(std::ptr::null_mut());

    let creation_flags = CREATE_SUSPENDED
        | CREATE_NEW_PROCESS_GROUP
        | CREATE_UNICODE_ENVIRONMENT;

    let ret = match token_result {
        Ok(token) => {
            let r = unsafe {
                CreateProcessAsUserW(
                    token,
                    std::ptr::null(),
                    cmd_wide.as_mut_ptr(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    FALSE,
                    creation_flags,
                    env_ptr,
                    dir_wide.as_ptr(),
                    &si,
                    &mut pi,
                )
            };
            unsafe { CloseHandle(token); }
            r
        }
        Err(_) => {
            // Fall back to normal CreateProcessW if restricted token fails
            eprintln!("[!] Warning: restricted token unavailable, using normal process");
            unsafe {
                CreateProcessW(
                    std::ptr::null(),
                    cmd_wide.as_mut_ptr(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    FALSE,
                    creation_flags,
                    env_ptr,
                    dir_wide.as_ptr(),
                    &si,
                    &mut pi,
                )
            }
        }
    };

    if ret == FALSE {
        return Err(crate::error::syscall_error("CreateProcess"));
    }

    // Assign to job object before resuming
    if !job_handle.is_null() {
        let jr = unsafe { AssignProcessToJobObject(job_handle, pi.hProcess) };
        if jr == FALSE {
            eprintln!("[!] Warning: failed to assign process to job object");
        }
    }

    // Resume the suspended process
    unsafe { ResumeThread(pi.hThread); }

    Ok((pi.hProcess, pi.hThread, pi.dwProcessId))
}

/// Build a UTF-16 environment block from key=value pairs.
/// Format: "KEY1=VALUE1\0KEY2=VALUE2\0\0"
pub fn build_env_block(env: &[(String, String)]) -> Vec<u16> {
    let mut block = Vec::new();
    for (key, value) in env {
        let entry = format!("{}={}", key, value);
        block.extend(entry.encode_utf16());
        block.push(0); // Null terminator for this entry
    }
    block.push(0); // Double null terminator
    block
}
