///! Windows Job Objects — resource limiting (CPU, memory, process count).

use crate::config::ResourceLimits;
use crate::error::{ContainerError, Result};
use super::winapi::*;

/// RAII wrapper for a Job Object handle.
pub struct JobObject {
    pub handle: HANDLE,
}

impl JobObject {
    /// Create a named Job Object.
    pub fn create(name: &str) -> Result<Self> {
        let wide_name = to_wide(name);
        let handle = unsafe { CreateJobObjectW(std::ptr::null_mut(), wide_name.as_ptr()) };
        if handle.is_null() {
            return Err(crate::error::syscall_error("CreateJobObjectW"));
        }
        Ok(JobObject { handle })
    }

    /// Apply resource limits to the job.
    pub fn set_limits(&self, limits: &ResourceLimits) -> Result<()> {
        self.set_extended_limits(limits)?;
        if limits.cpu_percent > 0 {
            self.set_cpu_rate(limits.cpu_percent)?;
        }
        Ok(())
    }

    /// Set memory and process limits via extended limit info.
    fn set_extended_limits(&self, limits: &ResourceLimits) -> Result<()> {
        let mut info = unsafe { std::mem::zeroed::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() };

        // Always kill all processes when the job handle is closed
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

        // Memory limit
        if limits.memory_bytes > 0 {
            info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_JOB_MEMORY;
            info.JobMemoryLimit = limits.memory_bytes as SIZE_T;
        }

        // Process count limit
        if limits.max_pids > 0 {
            info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
            info.BasicLimitInformation.ActiveProcessLimit = limits.max_pids;
        }

        let ret = unsafe {
            SetInformationJobObject(
                self.handle,
                JOBOBJECTCLASS_EXTENDED_LIMIT,
                &mut info as *mut _ as LPVOID,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as DWORD,
            )
        };

        if ret == FALSE {
            return Err(crate::error::syscall_error("SetInformationJobObject(ExtendedLimit)"));
        }
        Ok(())
    }

    /// Set CPU rate control (percentage-based hard cap).
    fn set_cpu_rate(&self, percent: u32) -> Result<()> {
        let mut info = JOBOBJECT_CPU_RATE_CONTROL_INFORMATION {
            ControlFlags: JOB_OBJECT_CPU_RATE_CONTROL_ENABLE | JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP,
            Value: percent * 100, // Value is in units of 1/100th of a percent
            _pad: [0; 2],
        };

        let ret = unsafe {
            SetInformationJobObject(
                self.handle,
                JOBOBJECTCLASS_CPU_RATE,
                &mut info as *mut _ as LPVOID,
                std::mem::size_of::<JOBOBJECT_CPU_RATE_CONTROL_INFORMATION>() as DWORD,
            )
        };

        if ret == FALSE {
            // CPU rate control may not be available on all Windows editions
            let err = unsafe { GetLastError() };
            eprintln!("[!] Warning: CPU rate control failed (error {}). This requires Windows 8+ / Server 2012+", err);
        }
        Ok(())
    }

    /// Assign a process to this job.
    pub fn assign_process(&self, process_handle: HANDLE) -> Result<()> {
        let ret = unsafe { AssignProcessToJobObject(self.handle, process_handle) };
        if ret == FALSE {
            return Err(crate::error::syscall_error("AssignProcessToJobObject"));
        }
        Ok(())
    }

    /// Kill all processes in the job.
    pub fn terminate(&self, exit_code: u32) -> Result<()> {
        let ret = unsafe { TerminateJobObject(self.handle, exit_code) };
        if ret == FALSE {
            return Err(crate::error::syscall_error("TerminateJobObject"));
        }
        Ok(())
    }
}

impl Drop for JobObject {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { CloseHandle(self.handle); }
        }
    }
}
