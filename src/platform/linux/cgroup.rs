///! cgroup v2 resource limiting — pure filesystem writes, no libraries.

use std::fs;
use std::path::{Path, PathBuf};

use crate::config::ResourceLimits;
use crate::error::{ContainerError, Result};

/// Base path for cgroup v2 filesystem.
const CGROUP_ROOT: &str = "/sys/fs/cgroup";

/// Our cgroup subtree name.
const CGROUP_PREFIX: &str = "holycontainer";

/// Get the cgroup directory for a container.
fn cgroup_dir(name: &str) -> PathBuf {
    Path::new(CGROUP_ROOT)
        .join(CGROUP_PREFIX)
        .join(name)
}

/// Create a cgroup for a container.
pub fn create_cgroup(name: &str) -> Result<()> {
    let dir = cgroup_dir(name);
    fs::create_dir_all(&dir).map_err(|e| {
        ContainerError::Syscall {
            call: "create_cgroup",
            code: 0,
            detail: format!("failed to create cgroup dir {}: {}", dir.display(), e),
        }
    })?;

    // Enable controllers in parent if needed
    let parent = Path::new(CGROUP_ROOT).join(CGROUP_PREFIX);
    let subtree_control = parent.join("cgroup.subtree_control");
    if subtree_control.exists() {
        // Try to enable cpu, memory, pids controllers
        let _ = fs::write(&subtree_control, "+cpu +memory +pids");
    }

    Ok(())
}

/// Set memory limit for a container's cgroup.
pub fn set_memory_limit(name: &str, bytes: u64) -> Result<()> {
    if bytes == 0 {
        return Ok(()); // No limit
    }
    let dir = cgroup_dir(name);

    // Set hard memory limit
    let memory_max = dir.join("memory.max");
    fs::write(&memory_max, format!("{}", bytes)).map_err(|e| {
        ContainerError::Syscall {
            call: "set_memory_limit",
            code: 0,
            detail: format!("failed to write memory.max: {}", e),
        }
    })?;

    // Disable swap
    let swap_max = dir.join("memory.swap.max");
    if swap_max.exists() {
        let _ = fs::write(&swap_max, "0");
    }

    Ok(())
}

/// Set CPU limit for a container's cgroup.
/// `percent` is 1-100, mapped to cpu.max quota/period.
pub fn set_cpu_limit(name: &str, percent: u32) -> Result<()> {
    if percent == 0 {
        return Ok(()); // No limit
    }

    let dir = cgroup_dir(name);
    let cpu_max = dir.join("cpu.max");

    // cpu.max format: "$QUOTA $PERIOD"
    // Period is typically 100000 (100ms). Quota = percent * period / 100.
    let period: u64 = 100000;
    let quota: u64 = (percent as u64) * period / 100;

    fs::write(&cpu_max, format!("{} {}", quota, period)).map_err(|e| {
        ContainerError::Syscall {
            call: "set_cpu_limit",
            code: 0,
            detail: format!("failed to write cpu.max: {}", e),
        }
    })?;

    Ok(())
}

/// Set PID limit for a container's cgroup.
pub fn set_pids_limit(name: &str, max_pids: u32) -> Result<()> {
    if max_pids == 0 {
        return Ok(()); // No limit
    }

    let dir = cgroup_dir(name);
    let pids_max = dir.join("pids.max");

    fs::write(&pids_max, format!("{}", max_pids)).map_err(|e| {
        ContainerError::Syscall {
            call: "set_pids_limit",
            code: 0,
            detail: format!("failed to write pids.max: {}", e),
        }
    })?;

    Ok(())
}

/// Add a process to the container's cgroup.
pub fn add_process(name: &str, pid: u32) -> Result<()> {
    let dir = cgroup_dir(name);
    if !dir.exists() {
        return Ok(());
    }
    let cgroup_procs = dir.join("cgroup.procs");

    if let Err(e) = fs::write(&cgroup_procs, format!("{}", pid)) {
        eprintln!("[!] Warning: failed to add process to cgroup: {}", e);
    }

    Ok(())
}

/// Apply all resource limits.
pub fn apply_limits(name: &str, limits: &ResourceLimits) -> Result<()> {
    if let Err(e) = create_cgroup(name) {
        eprintln!("[!] Warning: Could not create cgroup: {}", e);
        return Ok(()); // Early exit if cgroups aren't supported/writable
    }
    if let Err(e) = set_memory_limit(name, limits.memory_bytes) {
        eprintln!("[!] Warning: Could not set memory limit: {}", e);
    }
    if let Err(e) = set_cpu_limit(name, limits.cpu_percent) {
        eprintln!("[!] Warning: Could not set CPU limit: {}", e);
    }
    if let Err(e) = set_pids_limit(name, limits.max_pids) {
        eprintln!("[!] Warning: Could not set PIDs limit: {}", e);
    }
    Ok(())
}

/// Remove the container's cgroup.
pub fn remove_cgroup(name: &str) -> Result<()> {
    let dir = cgroup_dir(name);
    if dir.exists() {
        // Move all processes to parent first
        let procs_file = dir.join("cgroup.procs");
        if let Ok(content) = fs::read_to_string(&procs_file) {
            let parent_procs = Path::new(CGROUP_ROOT)
                .join(CGROUP_PREFIX)
                .join("cgroup.procs");
            for line in content.lines() {
                let _ = fs::write(&parent_procs, line);
            }
        }
        // Now remove directory
        let _ = fs::remove_dir(&dir);
    }
    Ok(())
}

/// Read current memory usage.
pub fn get_memory_usage(name: &str) -> Result<u64> {
    let dir = cgroup_dir(name);
    let current = dir.join("memory.current");
    let data = fs::read_to_string(&current).map_err(|e| {
        ContainerError::Syscall {
            call: "get_memory_usage",
            code: 0,
            detail: format!("failed to read memory.current: {}", e),
        }
    })?;
    data.trim()
        .parse()
        .map_err(|_| ContainerError::Syscall {
            call: "get_memory_usage",
            code: 0,
            detail: "failed to parse memory.current".into(),
        })
}
