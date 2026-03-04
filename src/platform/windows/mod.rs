///! Windows platform implementation — Job Objects + restricted tokens + sandboxed FS.

pub mod winapi;
pub mod job;
pub mod sandbox;
pub mod filesystem;
pub mod network;

use std::path::Path;

use crate::config::{ContainerConfig, NetworkConfig, ResourceLimits};
use crate::error::{ContainerError, Result};
use crate::platform::IsolationContext;
use self::winapi::*;

/// Windows-specific context stored inside IsolationContext.
pub struct WindowsContext {
    pub job: Option<job::JobObject>,
    pub process_handle: HANDLE,
    pub thread_handle: HANDLE,
}

// Safety: the handles are unique to this container and not shared
unsafe impl Send for WindowsContext {}
unsafe impl Sync for WindowsContext {}

pub fn create_isolation(config: &ContainerConfig) -> Result<IsolationContext> {
    let job_name = format!("HolyContainer_{}", config.name);
    let job = job::JobObject::create(&job_name)?;
    job.set_limits(&config.limits)?;

    Ok(IsolationContext {
        name: config.name.clone(),
        inner: WindowsContext {
            job: Some(job),
            process_handle: NULL_HANDLE,
            thread_handle: NULL_HANDLE,
        },
    })
}

pub fn setup_filesystem(_ctx: &IsolationContext, rootfs: &Path) -> Result<()> {
    filesystem::setup_rootfs(rootfs)
}

pub fn set_resource_limits(ctx: &IsolationContext, limits: &ResourceLimits) -> Result<()> {
    if let Some(ref job) = ctx.inner.job {
        job.set_limits(limits)?;
    }
    Ok(())
}

pub fn setup_network(ctx: &IsolationContext, config: &NetworkConfig) -> Result<()> {
    network::setup_container_network(config, 0, &ctx.name)
}

pub fn apply_security(_ctx: &IsolationContext) -> Result<()> {
    // Security is applied during process creation via restricted token
    Ok(())
}

pub fn spawn_process(ctx: &IsolationContext, cmd: &[String], env: &[(String, String)]) -> Result<u32> {
    if cmd.is_empty() {
        return Err(ContainerError::Config("no command specified".into()));
    }

    // Build command line (Windows-style: space-separated, quoted if needed)
    let command_line = cmd.iter()
        .map(|arg| {
            if arg.contains(' ') || arg.contains('"') {
                format!("\"{}\"", arg.replace('"', "\\\""))
            } else {
                arg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    // Build environment block
    let mut env_pairs = env.to_vec();
    if !env_pairs.iter().any(|(k, _)| k == "PATH") {
        env_pairs.push(("PATH".into(), "C:\\Windows\\System32;C:\\Windows".into()));
    }
    if !env_pairs.iter().any(|(k, _)| k == "SYSTEMROOT") {
        env_pairs.push(("SYSTEMROOT".into(), "C:\\Windows".into()));
    }
    let env_block = sandbox::build_env_block(&env_pairs);

    // Get working directory
    let work_dir = if let Some(ref job) = ctx.inner.job {
        ".".to_string()
    } else {
        ".".to_string()
    };

    // Get job handle
    let job_handle = ctx.inner.job
        .as_ref()
        .map(|j| j.handle)
        .unwrap_or(NULL_HANDLE);

    // Spawn sandboxed process
    let (proc_handle, thread_handle, pid) = sandbox::spawn_sandboxed_process(
        &command_line,
        &work_dir,
        Some(&env_block),
        job_handle,
    )?;

    // Store handles (note: we can't mutate through &self, so handles are
    // managed through the PID for stop/cleanup)
    // In a production system, we'd use interior mutability here
    println!("[*] Container process started with PID {}", pid);

    Ok(pid)
}

pub fn stop_container(ctx: &IsolationContext, pid: u32) -> Result<()> {
    // Try graceful termination first via the job object
    if let Some(ref job) = ctx.inner.job {
        job.terminate(0)?;
    } else {
        // Direct process termination
        let handle = unsafe { OpenProcess(PROCESS_TERMINATE, FALSE, pid) };
        if !handle.is_null() {
            unsafe {
                TerminateProcess(handle, 1);
                CloseHandle(handle);
            }
        }
    }
    Ok(())
}

pub fn cleanup(ctx: &IsolationContext) -> Result<()> {
    network::cleanup_network(&ctx.name)?;
    // Job object is cleaned up by Drop
    Ok(())
}

pub fn is_process_alive(pid: u32) -> bool {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION, FALSE, pid) };
    if handle.is_null() {
        return false;
    }
    let mut exit_code: DWORD = 0;
    let ret = unsafe { GetExitCodeProcess(handle, &mut exit_code) };
    unsafe { CloseHandle(handle); }
    ret != FALSE && exit_code == STILL_ACTIVE
}

pub fn create_minimal_rootfs(path: &Path) -> Result<()> {
    filesystem::create_minimal_rootfs(path)
}
