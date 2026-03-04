///! Container lifecycle management — platform-agnostic create/start/stop/exec/delete.

use std::fs;
use std::path::Path;

use crate::config::{self, ContainerConfig, ContainerState};
use crate::error::{ContainerError, Result};
use crate::platform;

/// List all containers and their states.
pub fn list_containers() -> Result<Vec<ContainerConfig>> {
    let base = config::state_base_dir().join("containers");
    if !base.exists() {
        return Ok(Vec::new());
    }

    let mut containers = Vec::new();
    for entry in fs::read_dir(&base)? {
        let entry = entry?;
        if entry.path().is_dir() {
            if let Ok(cfg) = ContainerConfig::load(&entry.path()) {
                containers.push(cfg);
            }
        }
    }
    Ok(containers)
}

/// Create a new container (sets up config, rootfs, cgroup/job object).
pub fn create(config: &mut ContainerConfig) -> Result<()> {
    let state_dir = config::container_state_dir(&config.name);

    if state_dir.exists() {
        return Err(ContainerError::Config(format!(
            "container '{}' already exists", config.name
        )));
    }

    // Validate rootfs
    if !config.rootfs.exists() {
        return Err(ContainerError::Filesystem(format!(
            "rootfs does not exist: {}", config.rootfs.display()
        )));
    }

    // Create isolation context (cgroup / job object)
    let ctx = platform::create_isolation(config)?;

    // Apply resource limits
    platform::set_resource_limits(&ctx, &config.limits)?;

    // Save config
    config.state = ContainerState::Created;
    config.save(&state_dir)?;

    // Store rootfs path in state dir for later reference
    let rootfs_link = state_dir.join("rootfs_path");
    fs::write(&rootfs_link, config.rootfs.to_string_lossy().as_ref())?;

    println!("[+] Container '{}' created successfully.", config.name);
    println!("    Rootfs: {}", config.rootfs.display());
    if config.limits.memory_bytes > 0 {
        println!("    Memory: {} MB", config.limits.memory_bytes / (1024 * 1024));
    }
    if config.limits.cpu_percent > 0 {
        println!("    CPU: {}%", config.limits.cpu_percent);
    }
    if config.limits.max_pids > 0 {
        println!("    PIDs: {}", config.limits.max_pids);
    }

    // Cleanup context (it will be recreated on start)
    platform::cleanup(&ctx)?;

    Ok(())
}

/// Start a container — spawn the container process with full isolation.
pub fn start(name: &str, command: Option<Vec<String>>) -> Result<()> {
    let state_dir = config::container_state_dir(name);

    if !state_dir.exists() {
        return Err(ContainerError::NotFound(name.to_string()));
    }

    let mut config = ContainerConfig::load(&state_dir)?;

    if config.state == ContainerState::Running {
        return Err(ContainerError::InvalidState {
            name: name.to_string(),
            current: "running".into(),
            expected: "created or stopped".into(),
        });
    }

    // Override command if provided
    if let Some(cmd) = command {
        config.command = cmd;
    }

    // Default command if none set
    if config.command.is_empty() {
        #[cfg(target_os = "linux")]
        {
            config.command = vec!["/bin/sh".into()];
        }
        #[cfg(target_os = "windows")]
        {
            config.command = vec!["cmd.exe".into()];
        }
    }

    // Create isolation context
    let ctx = platform::create_isolation(&config)?;

    // Apply resource limits
    platform::set_resource_limits(&ctx, &config.limits)?;

    // Build environment
    let mut env: Vec<(String, String)> = config.env.iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    // Add default env vars if not present
    let has_key = |env: &[(String, String)], key: &str| env.iter().any(|(k, _)| k == key);
    if !has_key(&env, "HOME") {
        #[cfg(target_os = "linux")]
        env.push(("HOME".into(), "/root".into()));
        #[cfg(target_os = "windows")]
        env.push(("USERPROFILE".into(), "C:\\Users\\ContainerUser".into()));
    }
    if !has_key(&env, "TERM") {
        env.push(("TERM".into(), "xterm-256color".into()));
    }
    if !has_key(&env, "HOSTNAME") {
        env.push(("HOSTNAME".into(), config.hostname.clone()));
    }

    #[cfg(target_os = "linux")]
    if !has_key(&env, "PATH") {
        env.push(("PATH".into(), "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".into()));
    }

    // Spawn the container process
    let pid = platform::spawn_process(&ctx, &config.command, &env)?;

    // Update state
    config.state = ContainerState::Running;
    config.pid = pid;
    config.save(&state_dir)?;

    println!("[+] Container '{}' started with PID {}.", name, pid);

    Ok(())
}

/// Stop a running container.
pub fn stop(name: &str) -> Result<()> {
    let state_dir = config::container_state_dir(name);

    if !state_dir.exists() {
        return Err(ContainerError::NotFound(name.to_string()));
    }

    let mut config = ContainerConfig::load(&state_dir)?;

    if config.state != ContainerState::Running {
        return Err(ContainerError::InvalidState {
            name: name.to_string(),
            current: config.state.as_str().to_string(),
            expected: "running".into(),
        });
    }

    // Create isolation context for cleanup
    let ctx = platform::create_isolation(&config)?;

    // Stop the process
    platform::stop_container(&ctx, config.pid)?;

    // Update state
    config.state = ContainerState::Stopped;
    config.pid = 0;
    config.save(&state_dir)?;

    // Cleanup
    platform::cleanup(&ctx)?;

    println!("[+] Container '{}' stopped.", name);
    Ok(())
}

/// Delete a container (must be stopped first).
pub fn delete(name: &str) -> Result<()> {
    let state_dir = config::container_state_dir(name);

    if !state_dir.exists() {
        return Err(ContainerError::NotFound(name.to_string()));
    }

    let config = ContainerConfig::load(&state_dir)?;

    if config.state == ContainerState::Running {
        return Err(ContainerError::InvalidState {
            name: name.to_string(),
            current: "running".into(),
            expected: "created or stopped".into(),
        });
    }

    // Create context for cleanup
    let ctx = platform::create_isolation(&config)?;
    platform::cleanup(&ctx)?;

    // Remove state directory
    fs::remove_dir_all(&state_dir)?;

    println!("[+] Container '{}' deleted.", name);
    Ok(())
}

/// Print container status table.
pub fn print_status() -> Result<()> {
    let containers = list_containers()?;

    if containers.is_empty() {
        println!("No containers found.");
        return Ok(());
    }

    println!("{:<20} {:<12} {:<8} {:<30} {}", "NAME", "STATE", "PID", "ROOTFS", "COMMAND");
    println!("{}", "-".repeat(90));

    for c in &containers {
        let cmd_str = if c.command.is_empty() {
            "<none>".to_string()
        } else {
            c.command.join(" ")
        };
        let rootfs_str = c.rootfs.to_string_lossy();
        let rootfs_short = if rootfs_str.len() > 28 {
            format!("...{}", &rootfs_str[rootfs_str.len()-25..])
        } else {
            rootfs_str.to_string()
        };

        let alive = if c.state == ContainerState::Running {
            platform::is_process_alive(c.pid)
        } else {
            false
        };
        let state = if c.state == ContainerState::Running && !alive {
            "dead"
        } else {
            c.state.as_str()
        };

        println!(
            "{:<20} {:<12} {:<8} {:<30} {}",
            c.name,
            state,
            if c.pid > 0 { c.pid.to_string() } else { "-".into() },
            rootfs_short,
            cmd_str,
        );
    }

    Ok(())
}
