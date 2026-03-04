///! HolyContainer error types — cross-platform, zero dependencies.

use std::fmt;
use std::io;

/// Every error that can occur in the container runtime.
#[derive(Debug)]
pub enum ContainerError {
    /// A raw OS syscall / API call failed.
    Syscall {
        call: &'static str,
        code: i32,
        detail: String,
    },
    /// I/O failure (file, directory, pipe).
    Io(io::Error),
    /// Invalid configuration.
    Config(String),
    /// Filesystem setup failure.
    Filesystem(String),
    /// Networking setup failure.
    Network(String),
    /// Container does not exist.
    NotFound(String),
    /// Container is in wrong state for this operation.
    InvalidState {
        name: String,
        current: String,
        expected: String,
    },
    /// Image operation failure.
    Image(String),
    /// Permission denied.
    PermissionDenied(String),
    /// Feature not supported on this platform.
    Unsupported(String),
}

impl fmt::Display for ContainerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContainerError::Syscall { call, code, detail } => {
                write!(f, "syscall `{}` failed (errno {}): {}", call, code, detail)
            }
            ContainerError::Io(e) => write!(f, "I/O error: {}", e),
            ContainerError::Config(msg) => write!(f, "config error: {}", msg),
            ContainerError::Filesystem(msg) => write!(f, "filesystem error: {}", msg),
            ContainerError::Network(msg) => write!(f, "network error: {}", msg),
            ContainerError::NotFound(name) => write!(f, "container '{}' not found", name),
            ContainerError::InvalidState {
                name,
                current,
                expected,
            } => write!(
                f,
                "container '{}' is in state '{}', expected '{}'",
                name, current, expected
            ),
            ContainerError::Image(msg) => write!(f, "image error: {}", msg),
            ContainerError::PermissionDenied(msg) => write!(f, "permission denied: {}", msg),
            ContainerError::Unsupported(msg) => write!(f, "unsupported: {}", msg),
        }
    }
}

impl std::error::Error for ContainerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ContainerError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for ContainerError {
    fn from(e: io::Error) -> Self {
        ContainerError::Io(e)
    }
}

/// Convenience result type.
pub type Result<T> = std::result::Result<T, ContainerError>;

/// Helper: create a syscall error from the last OS errno.
#[cfg(target_os = "linux")]
pub fn syscall_error(call: &'static str) -> ContainerError {
    let code = unsafe { *libc_errno() };
    ContainerError::Syscall {
        call,
        code,
        detail: errno_to_string(code),
    }
}

#[cfg(target_os = "linux")]
unsafe fn libc_errno() -> *mut i32 {
    extern "C" {
        fn __errno_location() -> *mut i32;
    }
    __errno_location()
}

/// Public accessor for errno pointer (used by syscall and capabilities modules).
#[cfg(target_os = "linux")]
pub unsafe fn libc_errno_ptr() -> *mut i32 {
    libc_errno()
}

#[cfg(target_os = "linux")]
fn errno_to_string(code: i32) -> String {
    // Common errno descriptions
    match code {
        1 => "Operation not permitted".into(),
        2 => "No such file or directory".into(),
        3 => "No such process".into(),
        4 => "Interrupted system call".into(),
        5 => "Input/output error".into(),
        9 => "Bad file descriptor".into(),
        11 => "Resource temporarily unavailable".into(),
        12 => "Cannot allocate memory".into(),
        13 => "Permission denied".into(),
        14 => "Bad address".into(),
        16 => "Device or resource busy".into(),
        17 => "File exists".into(),
        19 => "No such device".into(),
        20 => "Not a directory".into(),
        21 => "Is a directory".into(),
        22 => "Invalid argument".into(),
        23 => "Too many open files in system".into(),
        24 => "Too many open files".into(),
        28 => "No space left on device".into(),
        30 => "Read-only file system".into(),
        36 => "File name too long".into(),
        38 => "Function not implemented".into(),
        39 => "Directory not empty".into(),
        _ => format!("OS error code {}", code),
    }
}

#[cfg(target_os = "windows")]
pub fn syscall_error(call: &'static str) -> ContainerError {
    let code = unsafe { GetLastError() };
    ContainerError::Syscall {
        call,
        code: code as i32,
        detail: format!("Win32 error code {}", code),
    }
}

#[cfg(target_os = "windows")]
extern "system" {
    fn GetLastError() -> u32;
}
