// HolyContainer — cross-platform container runtime.
// Hand-rolled CLI, zero dependencies.
#![allow(dead_code, unused_imports, unused_variables, unused_mut)]

mod error;
mod config;
mod platform;
mod container;
mod image;

use std::path::PathBuf;
use std::process;

use config::{ContainerConfig, parse_memory};
use error::ContainerError;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage();
        process::exit(1);
    }

    let result = match args[1].as_str() {
        "create" => cmd_create(&args[2..]),
        "start" => cmd_start(&args[2..]),
        "stop" => cmd_stop(&args[2..]),
        "rm" | "delete" => cmd_delete(&args[2..]),
        "ps" | "list" => cmd_ps(),
        "images" => cmd_images(),
        "init-rootfs" => cmd_init_rootfs(&args[2..]),
        "image-create" => cmd_image_create(&args[2..]),
        "image-extract" => cmd_image_extract(&args[2..]),
        "help" | "--help" | "-h" => { print_usage(); Ok(()) }
        "version" | "--version" | "-v" => { print_version(); Ok(()) }
        other => {
            eprintln!("Error: unknown command '{}'", other);
            eprintln!();
            print_usage();
            process::exit(1);
        }
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        process::exit(1);
    }
}

fn print_version() {
    println!("holycontainer v1.0.0");
    println!("A from-scratch cross-platform container runtime in pure Rust");
    println!("Zero third-party dependencies");
    println!();
    #[cfg(target_os = "linux")]
    println!("Platform: Linux (namespaces + cgroups + seccomp)");
    #[cfg(target_os = "windows")]
    println!("Platform: Windows (Job Objects + restricted tokens)");
}

fn print_usage() {
    println!(
r#"
╔═══════════════════════════════════════════════════════════════╗
║              HOLYCONTAINER v1.0.0                            ║
║       From-Scratch Container Runtime in Pure Rust            ║
║              Zero Third-Party Dependencies                   ║
╚═══════════════════════════════════════════════════════════════╝

USAGE:
    holycontainer <COMMAND> [OPTIONS]

CONTAINER COMMANDS:
    create <name>    Create a new container
        --rootfs <path>       Path to root filesystem (required)
        --hostname <name>     Container hostname (default: container name)
        --memory <size>       Memory limit (e.g., 128M, 1G)
        --cpus <percent>      CPU limit as percentage (1-100)
        --pids <max>          Max number of processes
        --env <KEY=VALUE>     Environment variable (repeatable)
        --workdir <path>      Working directory inside container
        --port <host:ctr>     Port mapping (repeatable)
        --no-network          Disable networking

    start <name>     Start a created container
        [-- command args...]  Override the default command

    stop <name>      Stop a running container

    rm <name>        Delete a stopped container

    ps               List all containers

IMAGE COMMANDS:
    init-rootfs <path>           Bootstrap a minimal rootfs from host
    image-create <dir> <output>  Create a tar image from a directory
    image-extract <tar> <dir>    Extract a tar image into a directory
    images                       List available images

OTHER:
    help             Show this help message
    version          Show version information

EXAMPLES:
    # Create a minimal rootfs
    holycontainer init-rootfs /tmp/myroot

    # Create and start a container
    holycontainer create myapp --rootfs /tmp/myroot --memory 256M --cpus 50
    holycontainer start myapp -- /bin/sh

    # List and manage containers
    holycontainer ps
    holycontainer stop myapp
    holycontainer rm myapp
"#
    );
}

// ─── Command Handlers ───────────────────────────────────────────────────────

fn cmd_create(args: &[String]) -> Result<(), ContainerError> {
    if args.is_empty() {
        return Err(ContainerError::Config("usage: holycontainer create <name> --rootfs <path> [options]".into()));
    }

    let name = &args[0];
    let mut rootfs: Option<PathBuf> = None;
    let mut hostname: Option<String> = None;
    let mut memory: u64 = 0;
    let mut cpus: u32 = 0;
    let mut pids: u32 = 0;
    let mut env_vars = Vec::new();
    let mut workdir = "/".to_string();
    let mut port_mappings = Vec::new();
    let mut network_enabled = true;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--rootfs" => {
                i += 1;
                rootfs = Some(PathBuf::from(args.get(i).ok_or_else(|| {
                    ContainerError::Config("--rootfs requires a value".into())
                })?));
            }
            "--hostname" => {
                i += 1;
                hostname = Some(args.get(i).ok_or_else(|| {
                    ContainerError::Config("--hostname requires a value".into())
                })?.clone());
            }
            "--memory" | "-m" => {
                i += 1;
                let val = args.get(i).ok_or_else(|| {
                    ContainerError::Config("--memory requires a value".into())
                })?;
                memory = parse_memory(val)?;
            }
            "--cpus" | "-c" => {
                i += 1;
                let val = args.get(i).ok_or_else(|| {
                    ContainerError::Config("--cpus requires a value".into())
                })?;
                cpus = val.parse().map_err(|_| {
                    ContainerError::Config(format!("invalid cpu value: {}", val))
                })?;
            }
            "--pids" => {
                i += 1;
                let val = args.get(i).ok_or_else(|| {
                    ContainerError::Config("--pids requires a value".into())
                })?;
                pids = val.parse().map_err(|_| {
                    ContainerError::Config(format!("invalid pids value: {}", val))
                })?;
            }
            "--env" | "-e" => {
                i += 1;
                let val = args.get(i).ok_or_else(|| {
                    ContainerError::Config("--env requires KEY=VALUE".into())
                })?;
                let (k, v) = val.split_once('=').ok_or_else(|| {
                    ContainerError::Config(format!("invalid env format: {}", val))
                })?;
                env_vars.push((k.to_string(), v.to_string()));
            }
            "--workdir" | "-w" => {
                i += 1;
                workdir = args.get(i).ok_or_else(|| {
                    ContainerError::Config("--workdir requires a value".into())
                })?.clone();
            }
            "--port" | "-p" => {
                i += 1;
                let val = args.get(i).ok_or_else(|| {
                    ContainerError::Config("--port requires host:container".into())
                })?;
                let (h, c) = val.split_once(':').ok_or_else(|| {
                    ContainerError::Config(format!("invalid port format: {}", val))
                })?;
                let hp: u16 = h.parse().map_err(|_| ContainerError::Config(format!("invalid port: {}", h)))?;
                let cp: u16 = c.parse().map_err(|_| ContainerError::Config(format!("invalid port: {}", c)))?;
                port_mappings.push((hp, cp));
            }
            "--no-network" => {
                network_enabled = false;
            }
            _ => {
                return Err(ContainerError::Config(format!("unknown option: {}", args[i])));
            }
        }
        i += 1;
    }

    let rootfs = rootfs.ok_or_else(|| {
        ContainerError::Config("--rootfs is required".into())
    })?;

    let mut cfg = ContainerConfig::new(name, &rootfs);
    cfg.hostname = hostname.unwrap_or_else(|| name.clone());
    cfg.limits.memory_bytes = memory;
    cfg.limits.cpu_percent = cpus;
    cfg.limits.max_pids = pids;
    cfg.workdir = workdir;
    cfg.network.enabled = network_enabled;
    cfg.network.port_mappings = port_mappings;
    for (k, v) in env_vars {
        cfg.env.insert(k, v);
    }

    container::create(&mut cfg)
}

fn cmd_start(args: &[String]) -> Result<(), ContainerError> {
    if args.is_empty() {
        return Err(ContainerError::Config("usage: holycontainer start <name> [-- command args...]".into()));
    }

    let name = &args[0];
    let command = if args.len() > 1 && args[1] == "--" {
        Some(args[2..].to_vec())
    } else if args.len() > 1 {
        Some(args[1..].to_vec())
    } else {
        None
    };

    container::start(name, command)
}

fn cmd_stop(args: &[String]) -> Result<(), ContainerError> {
    if args.is_empty() {
        return Err(ContainerError::Config("usage: holycontainer stop <name>".into()));
    }
    container::stop(&args[0])
}

fn cmd_delete(args: &[String]) -> Result<(), ContainerError> {
    if args.is_empty() {
        return Err(ContainerError::Config("usage: holycontainer rm <name>".into()));
    }
    container::delete(&args[0])
}

fn cmd_ps() -> Result<(), ContainerError> {
    container::print_status()
}

fn cmd_images() -> Result<(), ContainerError> {
    image::list_images()
}

fn cmd_init_rootfs(args: &[String]) -> Result<(), ContainerError> {
    if args.is_empty() {
        return Err(ContainerError::Config("usage: holycontainer init-rootfs <path>".into()));
    }
    let path = PathBuf::from(&args[0]);
    std::fs::create_dir_all(&path)?;
    platform::create_minimal_rootfs(&path)
}

fn cmd_image_create(args: &[String]) -> Result<(), ContainerError> {
    if args.len() < 2 {
        return Err(ContainerError::Config("usage: holycontainer image-create <directory> <output.tar>".into()));
    }
    let dir = PathBuf::from(&args[0]);
    let output = PathBuf::from(&args[1]);
    image::create_image(&dir, &output)
}

fn cmd_image_extract(args: &[String]) -> Result<(), ContainerError> {
    if args.len() < 2 {
        return Err(ContainerError::Config("usage: holycontainer image-extract <archive.tar> <directory>".into()));
    }
    let archive = PathBuf::from(&args[0]);
    let dir = PathBuf::from(&args[1]);
    image::extract_image(&archive, &dir)
}
