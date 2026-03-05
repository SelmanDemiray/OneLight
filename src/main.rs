// HolyContainer — cross-platform container runtime.
// Hand-rolled CLI, zero dependencies.
// Capable of pulling and running any container image on Linux and Windows.
#![allow(dead_code, unused_imports, unused_variables, unused_mut)]

mod error;
mod config;
mod platform;
mod container;
mod image;
mod json;
mod http;
mod gzip;
mod registry;
mod overlay;
mod compose;
mod dashboard;

use std::path::PathBuf;
use std::process;

use config::{ContainerConfig, parse_memory};
use error::ContainerError;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage();
        process::exit(0);
    }

    let result = match args[1].as_str() {
        // ─── Container Lifecycle ────────────────────────────────────────
        "create" => cmd_create(&args[2..]),
        "start" => cmd_start(&args[2..]),
        "stop" => cmd_stop(&args[2..]),
        "rm" | "delete" => cmd_delete(&args[2..]),
        "ps" | "list" => cmd_ps(),

        // ─── Image Operations ───────────────────────────────────────────
        "pull" => cmd_pull(&args[2..]),
        "run" => cmd_run(&args[2..]),
        "images" => cmd_images(),
        "init-rootfs" => cmd_init_rootfs(&args[2..]),
        "image-create" => cmd_image_create(&args[2..]),
        "image-extract" => cmd_image_extract(&args[2..]),

        // ─── VM Operations (Windows WHP) ────────────────────────────────
        #[cfg(target_os = "windows")]
        "vm-boot" => cmd_vm_boot(&args[2..]),
        #[cfg(target_os = "windows")]
        "vm-check" => cmd_vm_check(),

        // ─── Stack / Compose ────────────────────────────────────────────
        "up" => cmd_up(&args[2..]),
        "down" => cmd_down(&args[2..]),

        // ─── Dashboard ──────────────────────────────────────────────────
        "dashboard" | "ui" | "gui" => cmd_dashboard(&args[2..]),

        // ─── Container Operations ───────────────────────────────────────
        "exec" => cmd_exec(&args[2..]),
        "logs" => cmd_logs(&args[2..]),

        // ─── Info ───────────────────────────────────────────────────────
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
    println!(r#"
╔═══════════════════════════════════════════════════════════════╗
║              HOLYCONTAINER v2.0.0                            ║
║   Full Container Runtime • Image Pulling • VM Hypervisor     ║
║              Zero Third-Party Dependencies                   ║
╚═══════════════════════════════════════════════════════════════╝"#);
    println!();
    println!("  Built-in: JSON parser, HTTP client, gzip/DEFLATE decompressor,");
    println!("  Docker Registry v2 client, overlay filesystem, tar reader/writer,");
    println!("  multi-container orchestration — all hand-written in pure Rust.");
    println!();
    #[cfg(target_os = "linux")]
    println!("  Platform: Linux (namespaces + cgroups + seccomp + overlayfs)");
    #[cfg(target_os = "windows")]
    {
        println!("  Platform: Windows (Job Objects + restricted tokens + WHP hypervisor)");
        if platform::windows::whp::is_whp_available() {
            println!("  WHP Status: ✓ Windows Hypervisor Platform available");
            println!("              Linux containers can run via micro-VM");
        } else {
            println!("  WHP Status: ✗ Not available (enable in Windows Features)");
        }
    }
}

fn print_usage() {
    println!(
r#"
╔═══════════════════════════════════════════════════════════════╗
║              HOLYCONTAINER v2.0.0                            ║
║   Full Container Runtime • Image Pulling • VM Hypervisor     ║
║              Zero Third-Party Dependencies                   ║
╚═══════════════════════════════════════════════════════════════╝

USAGE:
    holycontainer <COMMAND> [OPTIONS]

CONTAINER LIFECYCLE:
    create <name>    Create a new container
        --rootfs <path>       Path to root filesystem
        --image <name:tag>    Use a pulled image as rootfs
        --hostname <name>     Container hostname
        --memory <size>       Memory limit (e.g., 128M, 1G)
        --cpus <percent>      CPU limit (1-100)
        --pids <max>          Max processes
        --env <KEY=VALUE>     Environment variable (repeatable)
        --workdir <path>      Working directory
        --port <host:ctr>     Port mapping (repeatable)
        --volume <h:c>        Bind mount (repeatable)
        --no-network          Disable networking

    start <name>     Start a created container
        [-- cmd args...]      Override the default command

    stop <name>      Stop a running container
    rm <name>        Delete a stopped container
    ps               List all containers

IMAGE OPERATIONS:
    pull <image:tag>             Pull image from Docker Hub / registry
    run <image:tag> [-- cmd]     Pull + create + start in one command
    images                       List local images
    init-rootfs <path>           Bootstrap rootfs from host
    image-create <dir> <out.tar> Create tar image
    image-extract <tar> <dir>    Extract tar image

CONTAINER OPERATIONS:
    exec <name> -- <cmd>         Execute command in running container
    logs <name>                  Show container output

DASHBOARD:
    dashboard [--port 8080]      Start web GUI (opens in browser)

STACK OPERATIONS:
    up -f <stack.toml>           Start a multi-container stack
    down -f <stack.toml>         Stop a multi-container stack"#);

    #[cfg(target_os = "windows")]
    println!(r#"
VM OPERATIONS (Windows):
    vm-boot                      Boot a Linux kernel in a micro-VM
        --kernel <bzImage>       Path to Linux kernel
        --initrd <initrd>        Path to initramfs
        --cmdline <string>       Kernel command line
        --memory <MB>            VM memory in MB (default: 256)
    vm-check                     Check WHP availability"#);

    println!(r#"
OTHER:
    help             Show this help
    version          Show version + platform info

EXAMPLES:
    # Pull and run Ubuntu
    holycontainer pull ubuntu:22.04
    holycontainer create mybox --image ubuntu:22.04 --memory 512M
    holycontainer start mybox -- /bin/bash

    # Quick run (pull + create + start)
    holycontainer run alpine:latest -- /bin/sh

    # Multi-container stack
    holycontainer up -f myapp.toml
    holycontainer down -f myapp.toml
"#);
}

// ─── Container Commands ─────────────────────────────────────────────────────

fn cmd_create(args: &[String]) -> Result<(), ContainerError> {
    if args.is_empty() {
        return Err(ContainerError::Config("usage: holycontainer create <name> --rootfs <path> [options]".into()));
    }

    let name = &args[0];
    let mut rootfs: Option<PathBuf> = None;
    let mut image_name: Option<String> = None;
    let mut hostname: Option<String> = None;
    let mut memory: u64 = 0;
    let mut cpus: u32 = 0;
    let mut pids: u32 = 0;
    let mut env_vars = Vec::new();
    let mut workdir = "/".to_string();
    let mut port_mappings = Vec::new();
    let mut network_enabled = true;
    let mut volumes = Vec::new();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--rootfs" => {
                i += 1;
                rootfs = Some(PathBuf::from(args.get(i).ok_or_else(|| {
                    ContainerError::Config("--rootfs requires a value".into())
                })?));
            }
            "--image" => {
                i += 1;
                image_name = Some(args.get(i).ok_or_else(|| {
                    ContainerError::Config("--image requires a value".into())
                })?.clone());
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
            "--volume" | "-v" => {
                i += 1;
                let val = args.get(i).ok_or_else(|| {
                    ContainerError::Config("--volume requires host:container".into())
                })?;
                volumes.push(val.clone());
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

    // If --image was specified, resolve it to a rootfs path
    let rootfs = if let Some(ref img) = image_name {
        // Look for a locally pulled image
        let images = registry::list_local_images()?;
        let found = images.iter().find(|(n, _)| {
            n == img || n.starts_with(img)
        });
        match found {
            Some((_, rootfs_path)) => rootfs_path.clone(),
            None => {
                // Try to pull it
                println!("[*] Image not found locally, pulling...");
                registry::pull_image(img)?
            }
        }
    } else {
        rootfs.ok_or_else(|| {
            ContainerError::Config("--rootfs or --image is required".into())
        })?
    };

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

// ─── Image Commands ─────────────────────────────────────────────────────────

fn cmd_pull(args: &[String]) -> Result<(), ContainerError> {
    if args.is_empty() {
        return Err(ContainerError::Config("usage: holycontainer pull <image:tag>".into()));
    }
    registry::pull_image(&args[0])?;
    Ok(())
}

fn cmd_run(args: &[String]) -> Result<(), ContainerError> {
    if args.is_empty() {
        return Err(ContainerError::Config("usage: holycontainer run <image:tag> [-- command args...]".into()));
    }

    let image_ref = &args[0];

    // Parse command after --
    let command = if let Some(sep) = args.iter().position(|a| a == "--") {
        Some(args[sep + 1..].to_vec())
    } else {
        None
    };

    // Pull the image (will be fast if already cached)
    println!("[*] Ensuring image is available...");
    let rootfs = registry::pull_image(image_ref)?;

    // Generate a container name from the image
    let container_name = format!("run_{}_{}", 
        image_ref.replace([':', '/', '.'], "_"),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    );

    // Create the container
    let mut cfg = ContainerConfig::new(&container_name, &rootfs);
    cfg.limits.memory_bytes = 512 * 1024 * 1024; // 512 MB default

    container::create(&mut cfg)?;

    // Start it
    container::start(&container_name, command)?;

    Ok(())
}

fn cmd_images() -> Result<(), ContainerError> {
    // Show both old-style tar images and pulled registry images
    println!("{:<30} {:<15} {}", "IMAGE", "TYPE", "ROOTFS PATH");
    println!("{}", "-".repeat(80));

    // Pulled images
    let pulled = registry::list_local_images()?;
    for (name, rootfs) in &pulled {
        println!("{:<30} {:<15} {}", name, "pulled", rootfs.display());
    }

    // Tar images
    image::list_images()?;

    if pulled.is_empty() {
        println!("\nNo images found. Pull one with: holycontainer pull ubuntu:22.04");
    }

    Ok(())
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

// ─── VM Commands (Windows WHP) ──────────────────────────────────────────────

#[cfg(target_os = "windows")]
fn cmd_vm_boot(args: &[String]) -> Result<(), ContainerError> {
    let mut kernel_path: Option<PathBuf> = None;
    let mut initrd_path: Option<PathBuf> = None;
    let mut cmdline = "console=ttyS0 earlyprintk=serial".to_string();
    let mut mem_mb: u64 = 256;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--kernel" => {
                i += 1;
                kernel_path = Some(PathBuf::from(args.get(i).ok_or_else(|| {
                    ContainerError::Config("--kernel requires a path".into())
                })?));
            }
            "--initrd" => {
                i += 1;
                initrd_path = Some(PathBuf::from(args.get(i).ok_or_else(|| {
                    ContainerError::Config("--initrd requires a path".into())
                })?));
            }
            "--cmdline" => {
                i += 1;
                cmdline = args.get(i).ok_or_else(|| {
                    ContainerError::Config("--cmdline requires a value".into())
                })?.clone();
            }
            "--memory" => {
                i += 1;
                mem_mb = args.get(i).ok_or_else(|| {
                    ContainerError::Config("--memory requires a value in MB".into())
                })?.parse().map_err(|_| {
                    ContainerError::Config("--memory must be a number (MB)".into())
                })?;
            }
            _ => {
                return Err(ContainerError::Config(format!("unknown option: {}", args[i])));
            }
        }
        i += 1;
    }

    let kernel = kernel_path.ok_or_else(|| {
        ContainerError::Config("--kernel is required".into())
    })?;

    let initrd = initrd_path.as_deref();

    platform::windows::vmm::boot_linux(&kernel, initrd, &cmdline, mem_mb)
        .map(|exit_code| {
            println!("[+] VM exited with code {}.", exit_code);
        })
}

#[cfg(target_os = "windows")]
fn cmd_vm_check() -> Result<(), ContainerError> {
    if platform::windows::whp::is_whp_available() {
        println!("[+] Windows Hypervisor Platform is available.");
        println!("    You can boot Linux kernels and run Linux containers.");
        println!();
        println!("    Example:");
        println!("    holycontainer vm-boot --kernel bzImage --initrd initrd.gz");
    } else {
        println!("[!] Windows Hypervisor Platform is NOT available.");
        println!();
        println!("    To enable it:");
        println!("    1. Open Settings > Apps > Optional Features");
        println!("    2. Search for 'Windows Hypervisor Platform'");
        println!("    3. Install it and restart");
        println!();
        println!("    Requires: Windows 10/11 Pro or Enterprise");
        println!("    Note: This is different from full Hyper-V");
    }
    Ok(())
}

// ─── Exec / Logs ────────────────────────────────────────────────────────────

fn cmd_exec(args: &[String]) -> Result<(), ContainerError> {
    if args.is_empty() {
        return Err(ContainerError::Config("usage: holycontainer exec <name> -- <command>".into()));
    }

    let name = &args[0];
    let command = if let Some(sep) = args.iter().position(|a| a == "--") {
        args[sep + 1..].to_vec()
    } else if args.len() > 1 {
        args[1..].to_vec()
    } else {
        return Err(ContainerError::Config("exec requires a command".into()));
    };

    // Load container config to verify it exists and is running
    let state_dir = config::container_state_dir(name);
    let cfg = ContainerConfig::load(&state_dir)?;

    if cfg.state != config::ContainerState::Running {
        return Err(ContainerError::InvalidState {
            name: name.to_string(),
            current: cfg.state.as_str().to_string(),
            expected: "running".into(),
        });
    }

    // On Linux: use nsenter or fork+setns to enter the container's namespaces
    // On Windows: spawn a new process in the same job object
    println!("[*] Executing in container '{}'...", name);

    // Create isolation context and spawn process
    let ctx = platform::create_isolation(&cfg)?;
    let pid = platform::spawn_process(&ctx, &command, &[])?;
    println!("[+] Process {} started in container '{}'.", pid, name);

    Ok(())
}

fn cmd_logs(args: &[String]) -> Result<(), ContainerError> {
    if args.is_empty() {
        return Err(ContainerError::Config("usage: holycontainer logs <name>".into()));
    }

    let name = &args[0];
    let state_dir = config::container_state_dir(name);

    if !state_dir.exists() {
        return Err(ContainerError::NotFound(name.to_string()));
    }

    // Check for log file
    let log_file = state_dir.join("output.log");
    if log_file.exists() {
        let content = std::fs::read_to_string(&log_file)?;
        print!("{}", content);
    } else {
        println!("No logs available for container '{}'.", name);
        println!("(Container process output goes to the terminal that started it)");
    }

    Ok(())
}

// ─── Stack / Compose Commands ───────────────────────────────────────────────

fn cmd_up(args: &[String]) -> Result<(), ContainerError> {
    let mut file_path: Option<PathBuf> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-f" | "--file" => {
                i += 1;
                file_path = Some(PathBuf::from(args.get(i).ok_or_else(|| {
                    ContainerError::Config("-f requires a file path".into())
                })?));
            }
            _ => {
                return Err(ContainerError::Config(format!("unknown option: {}", args[i])));
            }
        }
        i += 1;
    }

    let path = file_path.ok_or_else(|| {
        ContainerError::Config("usage: holycontainer up -f <stack.toml>".into())
    })?;

    let stack = compose::parse_stack_file(&path)?;
    compose::stack_up(&stack)
}

fn cmd_down(args: &[String]) -> Result<(), ContainerError> {
    let mut file_path: Option<PathBuf> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-f" | "--file" => {
                i += 1;
                file_path = Some(PathBuf::from(args.get(i).ok_or_else(|| {
                    ContainerError::Config("-f requires a file path".into())
                })?));
            }
            _ => {
                return Err(ContainerError::Config(format!("unknown option: {}", args[i])));
            }
        }
        i += 1;
    }

    let path = file_path.ok_or_else(|| {
        ContainerError::Config("usage: holycontainer down -f <stack.toml>".into())
    })?;

    let stack = compose::parse_stack_file(&path)?;
    compose::stack_down(&stack)
}

// ─── Dashboard Command ──────────────────────────────────────────────────────

fn cmd_dashboard(args: &[String]) -> Result<(), ContainerError> {
    let mut port: u16 = 8080;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--port" | "-p" => {
                i += 1;
                port = args.get(i).ok_or_else(|| {
                    ContainerError::Config("--port requires a value".into())
                })?.parse().map_err(|_| {
                    ContainerError::Config("--port must be a number".into())
                })?;
            }
            _ => {
                return Err(ContainerError::Config(format!("unknown option: {}", args[i])));
            }
        }
        i += 1;
    }

    dashboard::start_dashboard(port)
}
