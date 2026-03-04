<![CDATA[<div align="center">

# 🔥 HolyContainer

### A From-Scratch Container Runtime in Pure Rust

**Zero third-party dependencies · No Docker · No WSL · Native Windows & Linux**

[![Rust](https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Windows](https://img.shields.io/badge/Windows-0078D6?style=for-the-badge&logo=windows&logoColor=white)](#windows-isolation)
[![Linux](https://img.shields.io/badge/Linux-FCC624?style=for-the-badge&logo=linux&logoColor=black)](#linux-isolation)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg?style=for-the-badge)](LICENSE)

</div>

---

## What Is This?

HolyContainer is a **fully hand-rolled container runtime** built entirely from scratch in Rust. Every single component — from syscall wrappers and Win32 FFI bindings to the tar archive format parser and BPF seccomp filters — is implemented manually with **zero external crates**.

This is not a wrapper around Docker, containerd, LXC, or WSL. It directly interfaces with the operating system's kernel primitives to provide process isolation, resource limiting, filesystem sandboxing, and networking.

```
[dependencies]
# Nothing. Zero. Nada. Every byte is hand-rolled.
```

---

## How It Works

### The Big Picture

```
┌──────────────────────────────────────────────────────────────┐
│                     holycontainer CLI                        │
│              (hand-rolled argument parser)                   │
├──────────────────────────────────────────────────────────────┤
│                  Container Lifecycle                         │
│          create → start → stop → delete                     │
├──────────────────────────────────────────────────────────────┤
│           Platform Abstraction Layer                         │
│         (compile-time #[cfg] dispatch)                       │
├────────────────────┬─────────────────────────────────────────┤
│   Linux Backend    │          Windows Backend                │
│                    │                                         │
│  • Namespaces      │  • Job Objects                          │
│  • Cgroups v2      │  • Restricted Tokens                    │
│  • Seccomp BPF     │  • Sandboxed Process Creation           │
│  • Capabilities    │  • Win32 API FFI                        │
│  • pivot_root      │  • Directory Isolation                  │
│  • veth networking │  • Firewall Rules                       │
│  • Raw syscalls    │  • CreateProcessAsUserW                 │
└────────────────────┴─────────────────────────────────────────┘
```

Each container gets:
1. **Its own isolated filesystem** (rootfs with copied/bootstrapped binaries)
2. **Resource limits** enforced by the OS kernel (memory, CPU, process count)
3. **A sandboxed process** spawned with reduced privileges
4. **Its own network configuration** (optional)

---

## Windows Isolation — How It Works (No WSL, No Docker)

On Windows, HolyContainer uses **native Win32 API calls** through hand-written FFI bindings. No WSL, no Hyper-V, no Docker Desktop — just raw kernel32.dll and advapi32.dll calls.

### Job Objects (Resource Limiting)
Every container process is assigned to a Windows **Job Object** — an OS-level construct that enforces hard resource limits:

| Resource | Win32 API | How |
|----------|-----------|-----|
| **Memory** | `SetInformationJobObject` + `JOBOBJECT_EXTENDED_LIMIT_INFORMATION` | `JOB_OBJECT_LIMIT_JOB_MEMORY` sets a hard byte cap |
| **CPU** | `SetInformationJobObject` + `JOBOBJECT_CPU_RATE_CONTROL_INFORMATION` | Hard cap as percentage (1-100%) |
| **Processes** | `JOBOBJECT_BASIC_LIMIT_INFORMATION.ActiveProcessLimit` | Maximum concurrent processes |
| **Cleanup** | `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` | All child processes killed when job handle closes |

### Restricted Tokens (Privilege Stripping)
Container processes run with a **restricted security token** created via `CreateRestrictedToken` with `DISABLE_MAX_PRIVILEGE`. This strips all elevated privileges from the spawned process, preventing it from:
- Modifying system settings
- Accessing privileged resources
- Escalating permissions

### Sandboxed Process Creation
Processes are spawned using `CreateProcessAsUserW` with the restricted token, started in a **suspended state**, assigned to the Job Object, then resumed:

```
OpenProcessToken → CreateRestrictedToken → CreateProcessAsUserW (SUSPENDED)
    → AssignProcessToJobObject → ResumeThread
```

If restricted token creation fails (e.g., insufficient caller privileges), the runtime gracefully falls back to `CreateProcessW` with Job Object isolation still enforced.

### Filesystem Isolation
The `init-rootfs` command bootstraps a minimal Windows rootfs by copying essential system binaries from `C:\Windows\System32`:
- Core: `cmd.exe`, `conhost.exe`, `kernel32.dll`, `ntdll.dll`, `KernelBase.dll`
- C Runtime: `msvcrt.dll`, `ucrtbase.dll`, `msvcp_win.dll`
- System: `advapi32.dll`, `shell32.dll`, `ole32.dll`, `rpcrt4.dll`, `sechost.dll`
- Tools: `powershell.exe`, `ping.exe`, `ipconfig.exe`, `whoami.exe`, `hostname.exe`, `tasklist.exe`, `findstr.exe`, `where.exe`, and more

The container process runs against this isolated copy — it doesn't have access to the host's full system directory.

---

## Linux Isolation — How It Works

On Linux, HolyContainer uses **raw syscall wrappers** (no libc wrappers, no nix crate) to set up full kernel-level isolation.

### Namespaces
Creates new namespaces via `clone()` / `unshare()` syscalls:
- **PID namespace**: Container sees only its own processes (PID 1 inside)
- **Mount namespace**: Isolated mount table, container can't see host mounts
- **UTS namespace**: Separate hostname
- **Network namespace**: Isolated network stack
- **IPC namespace**: Isolated inter-process communication
- **User namespace**: UID/GID mapping for rootless containers

### Cgroups v2
Writes directly to `/sys/fs/cgroup` to enforce resource limits:
- `memory.max` — Hard memory ceiling in bytes
- `cpu.max` — CPU bandwidth control (quota/period microseconds)
- `pids.max` — Maximum number of processes

### Seccomp BPF
Builds and loads a **BPF filter program** from scratch (no libseccomp) that restricts which syscalls the container process can invoke. Dangerous syscalls like `reboot`, `kexec_load`, `mount` (outside the namespace), etc. are blocked.

### Linux Capabilities
Drops unnecessary capabilities via `prctl(PR_CAPBSET_DROP, ...)` to enforce the principle of least privilege. The container runs with only the capabilities it actually needs.

### Filesystem (pivot_root)
Uses `pivot_root` (not `chroot`) to create a complete filesystem isolation:
1. Mount the rootfs as a new root
2. Set up `/proc`, `/sys`, `/dev` inside the container
3. `pivot_root` to swap the root filesystem
4. Unmount the old root

### Networking
Creates a **veth pair** (virtual Ethernet) connecting the container's network namespace to the host, with IP address assignment and routing configuration — all through raw `ioctl` and `netlink` syscalls.

---

## Installation

### Prerequisites
- **Rust toolchain** (1.70+): Install from [rustup.rs](https://rustup.rs/)
- **Windows**: No additional requirements (no WSL, no Docker)
- **Linux**: Root privileges (for namespace/cgroup operations)

### Build

```bash
git clone https://github.com/SelmanDemiray/OneLight.git
cd OneLight/holycontainer
cargo build --release
```

The binary will be at `target/release/holycontainer` (Linux) or `target\release\holycontainer.exe` (Windows).

### Cross-Compile

```bash
# Build for Linux from Windows (or vice versa)
rustup target add x86_64-unknown-linux-gnu
cargo check --target x86_64-unknown-linux-gnu

rustup target add x86_64-unknown-linux-musl
cargo check --target x86_64-unknown-linux-musl
```

---

## Usage

### Quick Start

```bash
# 1. Bootstrap a minimal rootfs from your host system
holycontainer init-rootfs /tmp/myroot       # Linux
holycontainer init-rootfs C:\tmp\myroot     # Windows

# 2. Create a container with resource limits
holycontainer create myapp \
  --rootfs /tmp/myroot \
  --memory 256M \
  --cpus 50 \
  --pids 64 \
  --hostname myhost \
  --env APP_NAME=hello

# 3. Start the container
holycontainer start myapp -- /bin/sh              # Linux
holycontainer start myapp -- cmd.exe /c "echo hi" # Windows

# 4. List containers
holycontainer ps

# 5. Stop and clean up
holycontainer stop myapp
holycontainer rm myapp
```

### All Commands

```
CONTAINER COMMANDS:
    create <name>    Create a new container
        --rootfs <path>       Path to root filesystem (required)
        --hostname <name>     Container hostname
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
    help             Show help
    version          Show version information
```

### Image Management

HolyContainer includes a custom **UStar tar implementation** (written from scratch, no external libraries) for creating and extracting container images:

```bash
# Package a rootfs into a portable tar image
holycontainer image-create /tmp/myroot my-container.tar

# Extract an image to create a new rootfs
holycontainer image-extract my-container.tar /tmp/newroot

# List stored images
holycontainer images
```

### Examples

```bash
# Container with custom environment and port mapping
holycontainer create webapp \
  --rootfs /tmp/myroot \
  --memory 512M \
  --cpus 75 \
  --pids 128 \
  --env NODE_ENV=production \
  --env PORT=3000 \
  --port 8080:3000 \
  --hostname webapp-prod

holycontainer start webapp -- node server.js

# Minimal container with no networking
holycontainer create isolated \
  --rootfs /tmp/myroot \
  --memory 64M \
  --no-network

holycontainer start isolated -- /bin/sh
```

---

## Project Structure

```
holycontainer/
├── Cargo.toml                    # Zero dependencies
├── src/
│   ├── main.rs                   # CLI entry point (hand-rolled arg parser)
│   ├── config.rs                 # Container config (hand-rolled serialization)
│   ├── container.rs              # Container lifecycle (create/start/stop/delete)
│   ├── error.rs                  # Error types (cross-platform errno handling)
│   ├── image.rs                  # UStar tar format (reader + writer from scratch)
│   └── platform/
│       ├── mod.rs                # Platform abstraction (compile-time dispatch)
│       ├── linux/
│       │   ├── mod.rs            # Linux orchestration
│       │   ├── syscall.rs        # Raw Linux syscall wrappers (no libc)
│       │   ├── namespace.rs      # PID/mount/UTS/net/IPC/user namespaces
│       │   ├── cgroup.rs         # Cgroups v2 resource control
│       │   ├── seccomp.rs        # BPF seccomp filter (hand-built bytecode)
│       │   ├── capabilities.rs   # Linux capability dropping
│       │   ├── filesystem.rs     # pivot_root + mount setup
│       │   └── network.rs        # veth networking (netlink + ioctl)
│       └── windows/
│           ├── mod.rs            # Windows orchestration
│           ├── winapi.rs         # Raw Win32 FFI bindings (no winapi crate)
│           ├── job.rs            # Job Objects (memory/CPU/PID limits)
│           ├── sandbox.rs        # Restricted tokens + sandboxed processes
│           ├── filesystem.rs     # Rootfs setup + system binary copying
│           └── network.rs        # Firewall rule management
```

---

## Configuration Storage

Container state is stored in a simple `key=value` format (no JSON/TOML/YAML parser needed):

| Platform | State Directory |
|----------|----------------|
| Linux | `/var/lib/holycontainer/containers/<name>/` |
| Windows | `%PROGRAMDATA%\holycontainer\containers\<name>\` |

---

## What Makes This Different

| Feature | HolyContainer | Docker | WSL |
|---------|--------------|--------|-----|
| External dependencies | **0** | 100+ | N/A (full VM) |
| Requires Docker daemon | ❌ | ✅ | ❌ |
| Requires WSL/Hyper-V | ❌ | ✅ (Windows) | ✅ |
| Native Windows support | ✅ (Win32 API) | ❌ (needs Linux VM) | ❌ (IS a Linux VM) |
| Native Linux support | ✅ (raw syscalls) | ✅ | N/A |
| Hand-rolled tar format | ✅ | ❌ | N/A |
| Hand-rolled seccomp BPF | ✅ | ❌ (uses libseccomp) | N/A |
| Single static binary | ✅ | ❌ | N/A |
| Lines of pure Rust | ~3,500 | Millions (Go + C) | N/A |

---

## License

MIT

---

<div align="center">

**Built with nothing but `std` and raw system calls.**

*No Docker. No WSL. No crates. Just Rust and the kernel.*

</div>
]]>
