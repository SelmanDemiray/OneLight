// Raw Win32 API FFI bindings — no winapi crate, all defined from scratch.

#![allow(non_camel_case_types, non_snake_case, dead_code)]

// ─── Type Aliases ───────────────────────────────────────────────────────────
pub type HANDLE = *mut std::ffi::c_void;
pub type BOOL = i32;
pub type DWORD = u32;
pub type WORD = u16;
pub type BYTE = u8;
pub type LPCWSTR = *const u16;
pub type LPWSTR = *mut u16;
pub type LPVOID = *mut std::ffi::c_void;
pub type LPCVOID = *const std::ffi::c_void;
pub type PSID = *mut std::ffi::c_void;
pub type SIZE_T = usize;
pub type LPSECURITY_ATTRIBUTES = *mut SECURITY_ATTRIBUTES;

pub const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;
pub const NULL_HANDLE: HANDLE = std::ptr::null_mut();
pub const TRUE: BOOL = 1;
pub const FALSE: BOOL = 0;
pub const INFINITE: DWORD = 0xFFFFFFFF;

// ─── Job Object Constants ───────────────────────────────────────────────────
pub const JOB_OBJECT_LIMIT_PROCESS_MEMORY: DWORD = 0x00000100;
pub const JOB_OBJECT_LIMIT_JOB_MEMORY: DWORD = 0x00000200;
pub const JOB_OBJECT_LIMIT_ACTIVE_PROCESS: DWORD = 0x00000008;
pub const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: DWORD = 0x00002000;
pub const JOB_OBJECT_LIMIT_BREAKAWAY_OK: DWORD = 0x00000800;

pub const JOB_OBJECT_CPU_RATE_CONTROL_ENABLE: DWORD = 0x1;
pub const JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP: DWORD = 0x4;

pub const JOBOBJECTCLASS_EXTENDED_LIMIT: u32 = 9;
pub const JOBOBJECTCLASS_CPU_RATE: u32 = 15;

// ─── Process Creation Constants ─────────────────────────────────────────────
pub const CREATE_SUSPENDED: DWORD = 0x00000004;
pub const CREATE_NEW_CONSOLE: DWORD = 0x00000010;
pub const CREATE_NEW_PROCESS_GROUP: DWORD = 0x00000200;
pub const CREATE_NO_WINDOW: DWORD = 0x08000000;
pub const CREATE_UNICODE_ENVIRONMENT: DWORD = 0x00000400;
pub const EXTENDED_STARTUPINFO_PRESENT: DWORD = 0x00080000;
pub const CREATE_BREAKAWAY_FROM_JOB: DWORD = 0x01000000;

// ─── Token Constants ────────────────────────────────────────────────────────
pub const TOKEN_ALL_ACCESS: DWORD = 0x000F01FF;
pub const TOKEN_QUERY: DWORD = 0x0008;
pub const TOKEN_DUPLICATE: DWORD = 0x0002;
pub const TOKEN_ASSIGN_PRIMARY: DWORD = 0x0001;
pub const TOKEN_ADJUST_DEFAULT: DWORD = 0x0080;
pub const TOKEN_ADJUST_SESSIONID: DWORD = 0x0100;

pub const DISABLE_MAX_PRIVILEGE: DWORD = 0x1;
pub const SANDBOX_INERT: DWORD = 0x2;

pub const SE_PRIVILEGE_REMOVED: DWORD = 0x00000004;
pub const SE_GROUP_USE_FOR_DENY_ONLY: DWORD = 0x00000010;

pub const SECURITY_MANDATORY_LOW_RID: DWORD = 0x00001000;
pub const SECURITY_MANDATORY_MEDIUM_RID: DWORD = 0x00002000;

// ─── Wait Constants ─────────────────────────────────────────────────────────
pub const WAIT_OBJECT_0: DWORD = 0;
pub const WAIT_TIMEOUT: DWORD = 258;
pub const STILL_ACTIVE: DWORD = 259;

// ─── Access Rights ──────────────────────────────────────────────────────────
pub const PROCESS_ALL_ACCESS: DWORD = 0x001FFFFF;
pub const PROCESS_QUERY_INFORMATION: DWORD = 0x0400;
pub const PROCESS_TERMINATE: DWORD = 0x0001;

// Symlink flags
pub const SYMBOLIC_LINK_FLAG_DIRECTORY: DWORD = 0x1;
pub const SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE: DWORD = 0x2;

// ─── Structures ─────────────────────────────────────────────────────────────

#[repr(C)]
pub struct SECURITY_ATTRIBUTES {
    pub nLength: DWORD,
    pub lpSecurityDescriptor: LPVOID,
    pub bInheritHandle: BOOL,
}

#[repr(C)]
pub struct STARTUPINFOW {
    pub cb: DWORD,
    pub lpReserved: LPWSTR,
    pub lpDesktop: LPWSTR,
    pub lpTitle: LPWSTR,
    pub dwX: DWORD,
    pub dwY: DWORD,
    pub dwXSize: DWORD,
    pub dwYSize: DWORD,
    pub dwXCountChars: DWORD,
    pub dwYCountChars: DWORD,
    pub dwFillAttribute: DWORD,
    pub dwFlags: DWORD,
    pub wShowWindow: WORD,
    pub cbReserved2: WORD,
    pub lpReserved2: *mut BYTE,
    pub hStdInput: HANDLE,
    pub hStdOutput: HANDLE,
    pub hStdError: HANDLE,
}

#[repr(C)]
pub struct PROCESS_INFORMATION {
    pub hProcess: HANDLE,
    pub hThread: HANDLE,
    pub dwProcessId: DWORD,
    pub dwThreadId: DWORD,
}

#[repr(C)]
pub struct JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
    pub BasicLimitInformation: JOBOBJECT_BASIC_LIMIT_INFORMATION,
    pub IoInfo: IO_COUNTERS,
    pub ProcessMemoryLimit: SIZE_T,
    pub JobMemoryLimit: SIZE_T,
    pub PeakProcessMemoryUsed: SIZE_T,
    pub PeakJobMemoryUsed: SIZE_T,
}

#[repr(C)]
pub struct JOBOBJECT_BASIC_LIMIT_INFORMATION {
    pub PerProcessUserTimeLimit: i64,
    pub PerJobUserTimeLimit: i64,
    pub LimitFlags: DWORD,
    pub MinimumWorkingSetSize: SIZE_T,
    pub MaximumWorkingSetSize: SIZE_T,
    pub ActiveProcessLimit: DWORD,
    pub Affinity: usize,
    pub PriorityClass: DWORD,
    pub SchedulingClass: DWORD,
}

#[repr(C)]
pub struct IO_COUNTERS {
    pub ReadOperationCount: u64,
    pub WriteOperationCount: u64,
    pub OtherOperationCount: u64,
    pub ReadTransferCount: u64,
    pub WriteTransferCount: u64,
    pub OtherTransferCount: u64,
}

#[repr(C)]
pub struct JOBOBJECT_CPU_RATE_CONTROL_INFORMATION {
    pub ControlFlags: DWORD,
    pub Value: DWORD, // Union: CpuRate, Weight, or MinRate/MaxRate
    pub _pad: [DWORD; 2],
}

// ─── kernel32.dll ───────────────────────────────────────────────────────────
#[link(name = "kernel32")]
extern "system" {
    pub fn CreateJobObjectW(lpJobAttributes: LPSECURITY_ATTRIBUTES, lpName: LPCWSTR) -> HANDLE;
    pub fn SetInformationJobObject(hJob: HANDLE, JobObjectInfoClass: u32, lpJobObjectInfo: LPVOID, cbJobObjectInfoLength: DWORD) -> BOOL;
    pub fn AssignProcessToJobObject(hJob: HANDLE, hProcess: HANDLE) -> BOOL;
    pub fn TerminateJobObject(hJob: HANDLE, uExitCode: u32) -> BOOL;
    pub fn CloseHandle(hObject: HANDLE) -> BOOL;
    pub fn CreateProcessW(lpAppName: LPCWSTR, lpCmdLine: LPWSTR, lpProcAttr: LPSECURITY_ATTRIBUTES, lpThreadAttr: LPSECURITY_ATTRIBUTES, bInheritHandles: BOOL, dwCreationFlags: DWORD, lpEnvironment: LPVOID, lpCurrentDir: LPCWSTR, lpStartupInfo: *const STARTUPINFOW, lpProcessInfo: *mut PROCESS_INFORMATION) -> BOOL;
    pub fn WaitForSingleObject(hHandle: HANDLE, dwMilliseconds: DWORD) -> DWORD;
    pub fn GetExitCodeProcess(hProcess: HANDLE, lpExitCode: *mut DWORD) -> BOOL;
    pub fn TerminateProcess(hProcess: HANDLE, uExitCode: u32) -> BOOL;
    pub fn ResumeThread(hThread: HANDLE) -> DWORD;
    pub fn GetLastError() -> DWORD;
    pub fn OpenProcess(dwDesiredAccess: DWORD, bInheritHandle: BOOL, dwProcessId: DWORD) -> HANDLE;
    pub fn GetCurrentProcess() -> HANDLE;
    pub fn CreateDirectoryW(lpPathName: LPCWSTR, lpSecurityAttributes: LPSECURITY_ATTRIBUTES) -> BOOL;
    pub fn CreateSymbolicLinkW(lpSymlinkFileName: LPCWSTR, lpTargetFileName: LPCWSTR, dwFlags: DWORD) -> BYTE;
    pub fn CopyFileW(lpExistingFileName: LPCWSTR, lpNewFileName: LPCWSTR, bFailIfExists: BOOL) -> BOOL;
    pub fn GetSystemDirectoryW(lpBuffer: LPWSTR, uSize: u32) -> u32;
}

// ─── advapi32.dll ───────────────────────────────────────────────────────────
#[link(name = "advapi32")]
extern "system" {
    pub fn OpenProcessToken(ProcessHandle: HANDLE, DesiredAccess: DWORD, TokenHandle: *mut HANDLE) -> BOOL;
    pub fn CreateRestrictedToken(ExistingTokenHandle: HANDLE, Flags: DWORD, DisableSidCount: DWORD, SidsToDisable: LPVOID, DeletePrivilegeCount: DWORD, PrivilegesToDelete: LPVOID, RestrictedSidCount: DWORD, SidsToRestrict: LPVOID, NewTokenHandle: *mut HANDLE) -> BOOL;
    pub fn CreateProcessAsUserW(hToken: HANDLE, lpAppName: LPCWSTR, lpCmdLine: LPWSTR, lpProcAttr: LPSECURITY_ATTRIBUTES, lpThreadAttr: LPSECURITY_ATTRIBUTES, bInheritHandles: BOOL, dwCreationFlags: DWORD, lpEnvironment: LPVOID, lpCurrentDir: LPCWSTR, lpStartupInfo: *const STARTUPINFOW, lpProcessInfo: *mut PROCESS_INFORMATION) -> BOOL;
}

// ─── Helper Functions ───────────────────────────────────────────────────────

/// Convert a Rust string to a null-terminated wide (UTF-16) string.
pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Create zeroed STARTUPINFOW.
pub fn startup_info() -> STARTUPINFOW {
    STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as DWORD,
        lpReserved: std::ptr::null_mut(),
        lpDesktop: std::ptr::null_mut(),
        lpTitle: std::ptr::null_mut(),
        dwX: 0, dwY: 0, dwXSize: 0, dwYSize: 0,
        dwXCountChars: 0, dwYCountChars: 0,
        dwFillAttribute: 0, dwFlags: 0,
        wShowWindow: 0, cbReserved2: 0,
        lpReserved2: std::ptr::null_mut(),
        hStdInput: NULL_HANDLE,
        hStdOutput: NULL_HANDLE,
        hStdError: NULL_HANDLE,
    }
}

/// Create zeroed PROCESS_INFORMATION.
pub fn process_info() -> PROCESS_INFORMATION {
    PROCESS_INFORMATION {
        hProcess: NULL_HANDLE,
        hThread: NULL_HANDLE,
        dwProcessId: 0,
        dwThreadId: 0,
    }
}
