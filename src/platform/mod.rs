///! Platform abstraction — compile-time dispatch between Linux and Windows backends.

#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(target_os = "windows")]
pub mod windows;

use std::path::Path;

use crate::config::{ContainerConfig, NetworkConfig, ResourceLimits};
use crate::error::Result;

/// Opaque handle to OS-specific isolation state.
/// Each platform stores whatever it needs here.
pub struct IsolationContext {
    /// Container name.
    pub name: String,
    /// Platform-specific inner data.
    pub inner: PlatformContext,
}

/// Platform-specific context data.
#[cfg(target_os = "linux")]
pub type PlatformContext = linux::LinuxContext;

#[cfg(target_os = "windows")]
pub type PlatformContext = windows::WindowsContext;

// ─── Platform API ───────────────────────────────────────────────────────────
// These free functions dispatch to the correct OS backend at compile time.

/// Create isolation structures (cgroup / job object, namespaces, etc.).
pub fn create_isolation(config: &ContainerConfig) -> Result<IsolationContext> {
    #[cfg(target_os = "linux")]
    {
        linux::create_isolation(config)
    }
    #[cfg(target_os = "windows")]
    {
        windows::create_isolation(config)
    }
}

/// Set up the container's root filesystem.
pub fn setup_filesystem(ctx: &IsolationContext, rootfs: &Path) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        linux::setup_filesystem(ctx, rootfs)
    }
    #[cfg(target_os = "windows")]
    {
        windows::setup_filesystem(ctx, rootfs)
    }
}

/// Apply resource limits (CPU, memory, PIDs).
pub fn set_resource_limits(ctx: &IsolationContext, limits: &ResourceLimits) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        linux::set_resource_limits(ctx, limits)
    }
    #[cfg(target_os = "windows")]
    {
        windows::set_resource_limits(ctx, limits)
    }
}

/// Set up container networking.
pub fn setup_network(ctx: &IsolationContext, config: &NetworkConfig) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        linux::setup_network(ctx, config)
    }
    #[cfg(target_os = "windows")]
    {
        windows::setup_network(ctx, config)
    }
}

/// Apply security restrictions (seccomp / AppContainer).
pub fn apply_security(ctx: &IsolationContext) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        linux::apply_security(ctx)
    }
    #[cfg(target_os = "windows")]
    {
        windows::apply_security(ctx)
    }
}

/// Spawn the container's init process. Returns the PID.
pub fn spawn_process(ctx: &IsolationContext, cmd: &[String], env: &[(String, String)]) -> Result<u32> {
    #[cfg(target_os = "linux")]
    {
        linux::spawn_process(ctx, cmd, env)
    }
    #[cfg(target_os = "windows")]
    {
        windows::spawn_process(ctx, cmd, env)
    }
}

/// Stop the container (kill the process tree).
pub fn stop_container(ctx: &IsolationContext, pid: u32) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        linux::stop_container(ctx, pid)
    }
    #[cfg(target_os = "windows")]
    {
        windows::stop_container(ctx, pid)
    }
}

/// Clean up all isolation resources.
pub fn cleanup(ctx: &IsolationContext) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        linux::cleanup(ctx)
    }
    #[cfg(target_os = "windows")]
    {
        windows::cleanup(ctx)
    }
}

/// Check if a process is still alive.
pub fn is_process_alive(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        linux::is_process_alive(pid)
    }
    #[cfg(target_os = "windows")]
    {
        windows::is_process_alive(pid)
    }
}

/// Create a minimal root filesystem from the host.
pub fn create_minimal_rootfs(path: &Path) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        linux::create_minimal_rootfs(path)
    }
    #[cfg(target_os = "windows")]
    {
        windows::create_minimal_rootfs(path)
    }
}
