///! Linux platform implementation — ties together namespaces, cgroups, seccomp, etc.

pub mod syscall;
pub mod namespace;
pub mod cgroup;
pub mod seccomp;
pub mod capabilities;
pub mod filesystem;
pub mod network;

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use crate::config::{ContainerConfig, NetworkConfig, ResourceLimits};
use crate::error::{ContainerError, Result, syscall_error};
use crate::platform::IsolationContext;

/// Linux-specific context stored inside IsolationContext.
pub struct LinuxContext {
    pub container_name: String,
    pub cgroup_created: bool,
    pub rootfs_mounted: bool,
}

pub fn create_isolation(config: &ContainerConfig) -> Result<IsolationContext> {
    // Create cgroup for resource limits - permissive for WSL/rootless
    let cgroup_created = cgroup::create_cgroup(&config.name).is_ok();
    if !cgroup_created {
        eprintln!("[!] Warning: Could not create cgroup for isolation context");
    }

    Ok(IsolationContext {
        name: config.name.clone(),
        inner: LinuxContext {
            container_name: config.name.clone(),
            cgroup_created,
            rootfs_mounted: false,
        },
    })
}

pub fn setup_filesystem(_ctx: &IsolationContext, rootfs: &Path) -> Result<()> {
    filesystem::setup_rootfs(rootfs)
}

pub fn set_resource_limits(ctx: &IsolationContext, limits: &ResourceLimits) -> Result<()> {
    cgroup::apply_limits(&ctx.name, limits)
}

pub fn setup_network(ctx: &IsolationContext, config: &NetworkConfig) -> Result<()> {
    // Network setup happens after the child is spawned (need PID)
    // This is called from the parent with the child's PID
    let _ = ctx;
    let _ = config;
    Ok(())
}

pub fn apply_security(_ctx: &IsolationContext) -> Result<()> {
    capabilities::drop_capabilities()?;
    seccomp::apply_seccomp_filter()?;
    Ok(())
}

pub fn spawn_process(ctx: &IsolationContext, cmd: &[String], env: &[(String, String)]) -> Result<u32> {
    if cmd.is_empty() {
        return Err(ContainerError::Config("no command specified".into()));
    }

    // Create a pipe for parent-child synchronization
    let mut pipefd = [0i32; 2];
    let ret = unsafe { syscall::pipe(&mut pipefd as *mut [i32; 2]) };
    if ret != 0 {
        return Err(syscall_error("pipe"));
    }

    let pid = unsafe { syscall::fork() };
    if pid < 0 {
        return Err(syscall_error("fork"));
    }

    if pid == 0 {
        // ── CHILD PROCESS ──
        unsafe { syscall::close(pipefd[0]); } // Close read end

        // Unshare all namespaces
        if let Err(e) = namespace::unshare_namespaces(namespace::ALL_NAMESPACES) {
            eprintln!("[!] Failed to unshare namespaces: {}", e);
            std::process::exit(1);
        }

        // Set hostname
        let _ = namespace::set_container_hostname(&ctx.name);

        // Set up new session
        unsafe { syscall::setsid(); }

        // Signal parent that we're ready
        unsafe {
            let byte: u8 = 1;
            syscall::write(pipefd[1], &byte as *const u8, 1);
            syscall::close(pipefd[1]);
        }

        // Apply security
        let _ = capabilities::drop_capabilities();
        let _ = seccomp::apply_seccomp_filter();

        // Exec the command
        let cmd_c: Vec<CString> = cmd.iter()
            .map(|s| CString::new(s.as_str()).unwrap_or_else(|_| CString::new("").unwrap()))
            .collect();
        let argv: Vec<*const u8> = cmd_c.iter()
            .map(|c| c.as_ptr() as *const u8)
            .chain(std::iter::once(std::ptr::null()))
            .collect();

        let env_strs: Vec<String> = env.iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();
        let env_c: Vec<CString> = env_strs.iter()
            .map(|s| CString::new(s.as_str()).unwrap())
            .collect();
        let envp: Vec<*const u8> = env_c.iter()
            .map(|c| c.as_ptr() as *const u8)
            .chain(std::iter::once(std::ptr::null()))
            .collect();

        unsafe {
            syscall::execve(argv[0], argv.as_ptr(), envp.as_ptr());
        }
        // If execve returns, it failed
        eprintln!("[!] execve failed");
        std::process::exit(1);
    }

    // ── PARENT PROCESS ──
    unsafe { syscall::close(pipefd[1]); } // Close write end

    // Wait for child to be ready
    let mut byte: u8 = 0;
    unsafe { syscall::read(pipefd[0], &mut byte as *mut u8, 1); }
    unsafe { syscall::close(pipefd[0]); }

    // Add child to cgroup
    cgroup::add_process(&ctx.name, pid as u32)?;

    Ok(pid as u32)
}

pub fn stop_container(_ctx: &IsolationContext, pid: u32) -> Result<()> {
    // Send SIGTERM first
    unsafe { syscall::kill(pid as i32, syscall::SIGTERM); }

    // Wait a bit, then SIGKILL if needed
    std::thread::sleep(std::time::Duration::from_secs(2));
    if is_process_alive(pid) {
        unsafe { syscall::kill(pid as i32, syscall::SIGKILL); }
    }

    // Wait for the process
    let mut status = 0i32;
    unsafe { syscall::waitpid(pid as i32, &mut status, 0); }

    Ok(())
}

pub fn cleanup(ctx: &IsolationContext) -> Result<()> {
    cgroup::remove_cgroup(&ctx.name)?;
    Ok(())
}

pub fn is_process_alive(pid: u32) -> bool {
    let ret = unsafe { syscall::kill(pid as i32, 0) };
    ret == 0
}

pub fn create_minimal_rootfs(path: &Path) -> Result<()> {
    filesystem::create_minimal_rootfs(path)
}
