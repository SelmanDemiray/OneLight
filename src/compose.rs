///! Multi-container orchestration — compose-like stack management.
///! Parses a simple TOML-like stack definition and manages multiple containers
///! with dependency ordering, networking, and health checks.
///! Zero third-party dependencies.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::config::{ContainerConfig, ResourceLimits};
use crate::container;
use crate::error::{ContainerError, Result};

// ─── Stack Definition ───────────────────────────────────────────────────────

/// A stack of containers to run together.
#[derive(Debug)]
pub struct Stack {
    pub name: String,
    pub services: Vec<ServiceDef>,
}

/// A service definition within a stack.
#[derive(Debug)]
pub struct ServiceDef {
    pub name: String,
    pub image: String,
    pub rootfs: Option<String>,
    pub command: Vec<String>,
    pub env: HashMap<String, String>,
    pub ports: Vec<(u16, u16)>,  // host:container
    pub volumes: Vec<(String, String)>,  // host:container
    pub depends_on: Vec<String>,
    pub memory: String,
    pub cpus: u32,
    pub pids: u32,
    pub hostname: String,
    pub restart: String,  // "no", "always", "on-failure"
    pub healthcheck: Option<HealthCheck>,
}

#[derive(Debug)]
pub struct HealthCheck {
    pub command: String,
    pub interval_secs: u32,
    pub timeout_secs: u32,
    pub retries: u32,
}

impl ServiceDef {
    fn new(name: &str) -> Self {
        ServiceDef {
            name: name.to_string(),
            image: String::new(),
            rootfs: None,
            command: Vec::new(),
            env: HashMap::new(),
            ports: Vec::new(),
            volumes: Vec::new(),
            depends_on: Vec::new(),
            memory: "256M".to_string(),
            cpus: 0,
            pids: 0,
            hostname: name.to_string(),
            restart: "no".to_string(),
            healthcheck: None,
        }
    }
}

// ─── Stack File Parser ──────────────────────────────────────────────────────

/// Parse a stack definition file (simplified TOML-like format).
///
/// Format:
/// ```
/// [stack]
/// name = myapp
///
/// [service.web]
/// image = nginx:latest
/// ports = 8080:80
/// memory = 256M
/// depends_on = db
///
/// [service.db]
/// image = postgres:15
/// env.POSTGRES_PASSWORD = secret
/// volumes = ./data:/var/lib/postgresql/data
/// memory = 512M
/// ```
pub fn parse_stack_file(path: &Path) -> Result<Stack> {
    let content = fs::read_to_string(path)
        .map_err(|e| ContainerError::Config(format!("read stack file: {}", e)))?;

    parse_stack_string(&content)
}

pub fn parse_stack_string(content: &str) -> Result<Stack> {
    let mut stack_name = String::from("default");
    let mut services: HashMap<String, ServiceDef> = HashMap::new();
    let mut current_section = String::new();
    let mut current_service = String::new();

    for line in content.lines() {
        let line = line.trim();

        // Skip empty lines and comments
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Section headers
        if line.starts_with('[') && line.ends_with(']') {
            let section = &line[1..line.len() - 1];

            if section == "stack" {
                current_section = "stack".to_string();
                current_service.clear();
            } else if section.starts_with("service.") {
                let svc_name = &section[8..];
                current_section = "service".to_string();
                current_service = svc_name.to_string();
                if !services.contains_key(svc_name) {
                    services.insert(svc_name.to_string(), ServiceDef::new(svc_name));
                }
            }
            continue;
        }

        // Key=value pairs
        if let Some(eq_pos) = line.find('=') {
            let key = line[..eq_pos].trim();
            let value = line[eq_pos + 1..].trim().to_string();

            match current_section.as_str() {
                "stack" => {
                    if key == "name" {
                        stack_name = value;
                    }
                }
                "service" => {
                    if let Some(service) = services.get_mut(&current_service) {
                        match key {
                            "image" => service.image = value,
                            "rootfs" => service.rootfs = Some(value),
                            "command" => {
                                service.command = value.split_whitespace()
                                    .map(|s| s.to_string())
                                    .collect();
                            }
                            "memory" => service.memory = value,
                            "cpus" => service.cpus = value.parse().unwrap_or(0),
                            "pids" => service.pids = value.parse().unwrap_or(0),
                            "hostname" => service.hostname = value,
                            "restart" => service.restart = value,
                            "depends_on" => {
                                service.depends_on = value.split(',')
                                    .map(|s| s.trim().to_string())
                                    .collect();
                            }
                            "ports" => {
                                // Parse "host:container" pairs
                                for pair in value.split(',') {
                                    let pair = pair.trim();
                                    if let Some(colon) = pair.find(':') {
                                        let host: u16 = pair[..colon].parse().unwrap_or(0);
                                        let container: u16 = pair[colon + 1..].parse().unwrap_or(0);
                                        if host > 0 && container > 0 {
                                            service.ports.push((host, container));
                                        }
                                    }
                                }
                            }
                            "volumes" => {
                                for pair in value.split(',') {
                                    let pair = pair.trim();
                                    if let Some(colon) = pair.find(':') {
                                        let host = pair[..colon].to_string();
                                        let container = pair[colon + 1..].to_string();
                                        service.volumes.push((host, container));
                                    }
                                }
                            }
                            k if k.starts_with("env.") => {
                                let env_key = &k[4..];
                                service.env.insert(env_key.to_string(), value);
                            }
                            "healthcheck" => {
                                service.healthcheck = Some(HealthCheck {
                                    command: value,
                                    interval_secs: 30,
                                    timeout_secs: 5,
                                    retries: 3,
                                });
                            }
                            _ => {} // Ignore unknown keys
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Sort services by dependency order
    let mut ordered = topological_sort(&services)?;

    Ok(Stack {
        name: stack_name,
        services: ordered,
    })
}

/// Topological sort of services by depends_on.
fn topological_sort(services: &HashMap<String, ServiceDef>) -> Result<Vec<ServiceDef>> {
    let mut visited: HashMap<String, bool> = HashMap::new(); // true = permanent, false = temporary
    let mut order: Vec<String> = Vec::new();

    for name in services.keys() {
        if !visited.contains_key(name) {
            visit(name, services, &mut visited, &mut order)?;
        }
    }

    let mut result = Vec::new();
    for name in &order {
        if let Some(svc) = services.get(name) {
            // Clone the service def
            result.push(ServiceDef {
                name: svc.name.clone(),
                image: svc.image.clone(),
                rootfs: svc.rootfs.clone(),
                command: svc.command.clone(),
                env: svc.env.clone(),
                ports: svc.ports.clone(),
                volumes: svc.volumes.clone(),
                depends_on: svc.depends_on.clone(),
                memory: svc.memory.clone(),
                cpus: svc.cpus,
                pids: svc.pids,
                hostname: svc.hostname.clone(),
                restart: svc.restart.clone(),
                healthcheck: None, // Simplified
            });
        }
    }

    Ok(result)
}

fn visit(
    name: &str,
    services: &HashMap<String, ServiceDef>,
    visited: &mut HashMap<String, bool>,
    order: &mut Vec<String>,
) -> Result<()> {
    if let Some(&permanent) = visited.get(name) {
        if permanent {
            return Ok(()); // Already processed
        } else {
            return Err(ContainerError::Config(format!(
                "circular dependency detected involving '{}'", name
            )));
        }
    }

    visited.insert(name.to_string(), false); // temporary mark

    if let Some(service) = services.get(name) {
        for dep in &service.depends_on {
            if !services.contains_key(dep) {
                return Err(ContainerError::Config(format!(
                    "service '{}' depends on unknown service '{}'", name, dep
                )));
            }
            visit(dep, services, visited, order)?;
        }
    }

    visited.insert(name.to_string(), true); // permanent mark
    order.push(name.to_string());
    Ok(())
}

// ─── Stack Operations ───────────────────────────────────────────────────────

/// Bring up all services in a stack.
pub fn stack_up(stack: &Stack) -> Result<()> {
    println!("[*] Starting stack '{}'...", stack.name);
    println!("    Services: {}", stack.services.iter()
        .map(|s| s.name.as_str())
        .collect::<Vec<_>>()
        .join(", "));

    for service in &stack.services {
        println!("\n[*] Starting service '{}'...", service.name);

        let container_name = format!("{}_{}", stack.name, service.name);

        // Determine rootfs
        let rootfs_path = if let Some(ref rootfs) = service.rootfs {
            std::path::PathBuf::from(rootfs)
        } else if !service.image.is_empty() {
            // Pull image if needed
            println!("    Pulling image {}...", service.image);
            match crate::registry::pull_image(&service.image) {
                Ok(rootfs) => rootfs,
                Err(e) => {
                    eprintln!("    [!] Failed to pull image: {}", e);
                    continue;
                }
            }
        } else {
            eprintln!("    [!] No image or rootfs specified for service '{}'", service.name);
            continue;
        };

        // Parse resource limits
        let memory_bytes = crate::config::parse_memory(&service.memory).unwrap_or(256 * 1024 * 1024);

        let mut net_config = crate::config::NetworkConfig::default();
        net_config.port_mappings = service.ports.clone();

        let mut config = ContainerConfig {
            name: container_name.clone(),
            rootfs: rootfs_path,
            command: service.command.clone(),
            env: service.env.clone(),
            hostname: service.hostname.clone(),
            limits: ResourceLimits {
                memory_bytes,
                cpu_percent: service.cpus,
                max_pids: service.pids,
            },
            network: net_config,
            workdir: "/".to_string(),
            state: crate::config::ContainerState::Created,
            pid: 0,
        };

        // Create and start the container
        match container::create(&mut config) {
            Ok(()) => {
                let cmd = if config.command.is_empty() { None } else { Some(config.command.clone()) };
                if let Err(e) = container::start(&container_name, cmd) {
                    eprintln!("    [!] Failed to start: {}", e);
                }
            }
            Err(e) => {
                eprintln!("    [!] Failed to create: {}", e);
            }
        }
    }

    println!("\n[+] Stack '{}' is up.", stack.name);
    Ok(())
}

/// Bring down all services in a stack (reverse order).
pub fn stack_down(stack: &Stack) -> Result<()> {
    println!("[*] Stopping stack '{}'...", stack.name);

    for service in stack.services.iter().rev() {
        let container_name = format!("{}_{}", stack.name, service.name);
        println!("[*] Stopping {}...", container_name);

        if let Err(e) = container::stop(&container_name) {
            eprintln!("    [warn] {}", e);
        }
        if let Err(e) = container::delete(&container_name) {
            eprintln!("    [warn] {}", e);
        }
    }

    println!("[+] Stack '{}' is down.", stack.name);
    Ok(())
}

/// Show status of all services in a stack.
pub fn stack_status(stack: &Stack) -> Result<()> {
    println!("Stack: {}\n", stack.name);
    println!("{:<25} {:<15} {:<10} {}", "SERVICE", "IMAGE", "STATUS", "PORTS");
    println!("{}", "-".repeat(70));

    for service in &stack.services {
        let container_name = format!("{}_{}", stack.name, service.name);

        let status = match crate::config::ContainerConfig::load(
            &crate::config::container_state_dir(&container_name)
        ) {
            Ok(cfg) => cfg.state.as_str().to_string(),
            Err(_) => "not created".to_string(),
        };

        let ports: String = service.ports.iter()
            .map(|(h, c)| format!("{}:{}", h, c))
            .collect::<Vec<_>>()
            .join(", ");

        let image = if service.image.is_empty() {
            service.rootfs.as_deref().unwrap_or("-")
        } else {
            &service.image
        };

        println!("{:<25} {:<15} {:<10} {}", service.name, image, status, ports);
    }

    Ok(())
}
