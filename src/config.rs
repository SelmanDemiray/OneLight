///! Container configuration — serializable without serde.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{ContainerError, Result};

/// Container state machine.
#[derive(Debug, Clone, PartialEq)]
pub enum ContainerState {
    Created,
    Running,
    Stopped,
}

impl ContainerState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ContainerState::Created => "created",
            ContainerState::Running => "running",
            ContainerState::Stopped => "stopped",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "created" => Some(ContainerState::Created),
            "running" => Some(ContainerState::Running),
            "stopped" => Some(ContainerState::Stopped),
            _ => None,
        }
    }
}

/// Resource limits for the container.
#[derive(Debug, Clone)]
pub struct ResourceLimits {
    /// Memory limit in bytes (0 = unlimited).
    pub memory_bytes: u64,
    /// CPU percentage (1-100, 0 = unlimited).
    pub cpu_percent: u32,
    /// Maximum number of processes/threads (0 = unlimited).
    pub max_pids: u32,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            memory_bytes: 0,
            cpu_percent: 0,
            max_pids: 0,
        }
    }
}

/// Network configuration.
#[derive(Debug, Clone)]
pub struct NetworkConfig {
    /// Whether to enable networking.
    pub enabled: bool,
    /// Container IP address (e.g., "10.0.0.2").
    pub ip_address: String,
    /// Bridge IP on host side (e.g., "10.0.0.1").
    pub bridge_ip: String,
    /// Subnet mask bits (e.g., 24 for /24).
    pub subnet_bits: u8,
    /// Port mappings: host_port -> container_port.
    pub port_mappings: Vec<(u16, u16)>,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            ip_address: "10.0.0.2".into(),
            bridge_ip: "10.0.0.1".into(),
            subnet_bits: 24,
            port_mappings: Vec::new(),
        }
    }
}

/// Full container configuration.
#[derive(Debug, Clone)]
pub struct ContainerConfig {
    /// Unique container name.
    pub name: String,
    /// Path to the root filesystem.
    pub rootfs: PathBuf,
    /// Command to execute inside the container.
    pub command: Vec<String>,
    /// Hostname inside the container.
    pub hostname: String,
    /// Environment variables.
    pub env: HashMap<String, String>,
    /// Resource limits.
    pub limits: ResourceLimits,
    /// Network configuration.
    pub network: NetworkConfig,
    /// Working directory inside the container.
    pub workdir: String,
    /// Container state.
    pub state: ContainerState,
    /// PID of the container's init process (0 if not running).
    pub pid: u32,
}

impl ContainerConfig {
    pub fn new(name: &str, rootfs: &Path) -> Self {
        Self {
            name: name.to_string(),
            rootfs: rootfs.to_path_buf(),
            command: Vec::new(),
            hostname: name.to_string(),
            env: HashMap::new(),
            limits: ResourceLimits::default(),
            network: NetworkConfig::default(),
            workdir: "/".into(),
            state: ContainerState::Created,
            pid: 0,
        }
    }
}

// ─── Hand-Rolled Config Serialization ───────────────────────────────────────
// Simple key=value format. No JSON/TOML/YAML crate needed.
//
// Format:
//   name=mycontainer
//   rootfs=/path/to/rootfs
//   hostname=mycontainer
//   state=created
//   pid=0
//   memory_bytes=134217728
//   cpu_percent=50
//   max_pids=64
//   network_enabled=true
//   ip_address=10.0.0.2
//   bridge_ip=10.0.0.1
//   subnet_bits=24
//   port=8080:80
//   port=443:443
//   env.PATH=/usr/bin
//   env.HOME=/root
//   cmd=/bin/sh
//   cmd=-c
//   cmd=echo hello
//   workdir=/

impl ContainerConfig {
    /// Serialize to our simple key=value format.
    pub fn serialize(&self) -> String {
        let mut lines = Vec::new();

        lines.push(format!("name={}", self.name));
        lines.push(format!("rootfs={}", self.rootfs.display()));
        lines.push(format!("hostname={}", self.hostname));
        lines.push(format!("state={}", self.state.as_str()));
        lines.push(format!("pid={}", self.pid));
        lines.push(format!("workdir={}", self.workdir));

        // Resource limits
        lines.push(format!("memory_bytes={}", self.limits.memory_bytes));
        lines.push(format!("cpu_percent={}", self.limits.cpu_percent));
        lines.push(format!("max_pids={}", self.limits.max_pids));

        // Network
        lines.push(format!(
            "network_enabled={}",
            if self.network.enabled { "true" } else { "false" }
        ));
        lines.push(format!("ip_address={}", self.network.ip_address));
        lines.push(format!("bridge_ip={}", self.network.bridge_ip));
        lines.push(format!("subnet_bits={}", self.network.subnet_bits));

        for (host_port, container_port) in &self.network.port_mappings {
            lines.push(format!("port={}:{}", host_port, container_port));
        }

        // Environment
        let mut env_keys: Vec<&String> = self.env.keys().collect();
        env_keys.sort();
        for key in env_keys {
            lines.push(format!("env.{}={}", key, self.env[key]));
        }

        // Command
        for arg in &self.command {
            lines.push(format!("cmd={}", arg));
        }

        lines.join("\n")
    }

    /// Deserialize from our simple key=value format.
    pub fn deserialize(data: &str) -> Result<Self> {
        let mut name = String::new();
        let mut rootfs = PathBuf::new();
        let mut hostname = String::new();
        let mut state = ContainerState::Created;
        let mut pid: u32 = 0;
        let mut workdir = "/".to_string();
        let mut limits = ResourceLimits::default();
        let mut network = NetworkConfig::default();
        let mut env = HashMap::new();
        let mut command = Vec::new();

        for line in data.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| ContainerError::Config(format!("invalid line: {}", line)))?;

            match key {
                "name" => name = value.to_string(),
                "rootfs" => rootfs = PathBuf::from(value),
                "hostname" => hostname = value.to_string(),
                "state" => {
                    state = ContainerState::from_str(value).ok_or_else(|| {
                        ContainerError::Config(format!("invalid state: {}", value))
                    })?;
                }
                "pid" => {
                    pid = value
                        .parse()
                        .map_err(|_| ContainerError::Config(format!("invalid pid: {}", value)))?;
                }
                "workdir" => workdir = value.to_string(),
                "memory_bytes" => {
                    limits.memory_bytes = value.parse().map_err(|_| {
                        ContainerError::Config(format!("invalid memory_bytes: {}", value))
                    })?;
                }
                "cpu_percent" => {
                    limits.cpu_percent = value.parse().map_err(|_| {
                        ContainerError::Config(format!("invalid cpu_percent: {}", value))
                    })?;
                }
                "max_pids" => {
                    limits.max_pids = value.parse().map_err(|_| {
                        ContainerError::Config(format!("invalid max_pids: {}", value))
                    })?;
                }
                "network_enabled" => network.enabled = value == "true",
                "ip_address" => network.ip_address = value.to_string(),
                "bridge_ip" => network.bridge_ip = value.to_string(),
                "subnet_bits" => {
                    network.subnet_bits = value.parse().map_err(|_| {
                        ContainerError::Config(format!("invalid subnet_bits: {}", value))
                    })?;
                }
                "port" => {
                    let (h, c) = value.split_once(':').ok_or_else(|| {
                        ContainerError::Config(format!("invalid port mapping: {}", value))
                    })?;
                    let host_port: u16 = h.parse().map_err(|_| {
                        ContainerError::Config(format!("invalid host port: {}", h))
                    })?;
                    let container_port: u16 = c.parse().map_err(|_| {
                        ContainerError::Config(format!("invalid container port: {}", c))
                    })?;
                    network.port_mappings.push((host_port, container_port));
                }
                k if k.starts_with("env.") => {
                    let env_key = &k[4..];
                    env.insert(env_key.to_string(), value.to_string());
                }
                "cmd" => command.push(value.to_string()),
                _ => {
                    // Ignore unknown keys for forward compat
                }
            }
        }

        if name.is_empty() {
            return Err(ContainerError::Config("missing container name".into()));
        }

        Ok(ContainerConfig {
            name,
            rootfs,
            command,
            hostname,
            env,
            limits,
            network,
            workdir,
            state,
            pid,
        })
    }

    /// Save config to the container's state directory.
    pub fn save(&self, state_dir: &Path) -> Result<()> {
        let config_path = state_dir.join("config");
        fs::create_dir_all(state_dir)?;
        fs::write(&config_path, self.serialize())?;
        Ok(())
    }

    /// Load config from the container's state directory.
    pub fn load(state_dir: &Path) -> Result<Self> {
        let config_path = state_dir.join("config");
        if !config_path.exists() {
            return Err(ContainerError::NotFound(format!(
                "config not found at {}",
                config_path.display()
            )));
        }
        let data = fs::read_to_string(&config_path)?;
        Self::deserialize(&data)
    }
}

/// Parse a memory string like "128M", "1G", "512K" into bytes.
pub fn parse_memory(s: &str) -> Result<u64> {
    let s = s.trim();
    if s.is_empty() || s == "0" {
        return Ok(0);
    }

    let (num_part, multiplier) = if s.ends_with('K') || s.ends_with('k') {
        (&s[..s.len() - 1], 1024u64)
    } else if s.ends_with('M') || s.ends_with('m') {
        (&s[..s.len() - 1], 1024 * 1024)
    } else if s.ends_with('G') || s.ends_with('g') {
        (&s[..s.len() - 1], 1024 * 1024 * 1024)
    } else {
        (s, 1u64) // raw bytes
    };

    let num: u64 = num_part
        .parse()
        .map_err(|_| ContainerError::Config(format!("invalid memory value: {}", s)))?;

    Ok(num * multiplier)
}

/// Get the base state directory for all containers.
pub fn state_base_dir() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        PathBuf::from("/var/lib/holycontainer")
    }
    #[cfg(target_os = "windows")]
    {
        let base = std::env::var("PROGRAMDATA").unwrap_or_else(|_| "C:\\ProgramData".into());
        PathBuf::from(base).join("holycontainer")
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        PathBuf::from("/var/lib/holycontainer")
    }
}

/// Get the state directory for a specific container.
pub fn container_state_dir(name: &str) -> PathBuf {
    state_base_dir().join("containers").join(name)
}

/// Get the images directory.
pub fn images_dir() -> PathBuf {
    state_base_dir().join("images")
}
