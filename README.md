# HolyContainer v2.0

**A real container runtime built from absolute zero in pure Rust. Pulls images from Docker Hub. Boots Linux kernels on Windows via a hand-written hypervisor. No Docker. No WSL. No third-party crates. Every syscall, every HTTP request, every gzip byte, every page table entry — hand-written.**

[![Rust](https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Windows](https://img.shields.io/badge/Windows-0078D6?style=for-the-badge&logo=windows&logoColor=white)](#windows-whp-hypervisor)
[![Linux](https://img.shields.io/badge/Linux-FCC624?style=for-the-badge&logo=linux&logoColor=black)](#linux-kernel-isolation)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg?style=for-the-badge)](LICENSE)

---

## What's New in v2.0

| Feature | Status | What It Does |
|---------|--------|-------------|
| **Image Pulling** | ✅ Working | `holycontainer pull ubuntu:22.04` — downloads from Docker Hub |
| **One-Command Run** | ✅ Working | `holycontainer run alpine:latest -- /bin/sh` |
| **Homemade JSON Parser** | ✅ Working | Parses Docker Registry API responses, zero crates |
| **Homemade HTTP Client** | ✅ Working | WinHTTP FFI on Windows, raw sockets on Linux |
| **Homemade gzip/DEFLATE** | ✅ Working | RFC 1951 decompressor — Huffman + LZ77, from scratch |
| **Overlay Filesystem** | ✅ Working | Layer-based storage with copy-on-write |
| **WHP Micro-Hypervisor** | ✅ Working | Boot Linux kernels on Windows via hardware virtualization |
| **Virtio Devices** | ✅ Working | Console, block, and network device emulation |
| **Multi-Container Stacks** | ✅ Working | `holycontainer up -f stack.toml` with dependency ordering |
| **Exec / Logs** | ✅ Working | Execute commands in running containers, view output |

---

## Why Does This Exist?

Docker on Windows requires WSL2 (a full Linux VM). That's not a real Windows container — it's Linux running inside a VM, pretending to be native.

HolyContainer takes a completely different approach: **it talks directly to the operating system kernel**. On Windows, it calls Win32 APIs through raw FFI *and* uses Windows Hypervisor Platform to boot a real Linux kernel in a micro-VM. On Linux, it invokes syscalls directly. There is no middle layer, no daemon, no pre-existing VM.

This means:
- You can **pull Ubuntu from Docker Hub and run it on Windows** — using a hypervisor written entirely in Rust
- On Linux, you get **real kernel namespaces, cgroups, and seccomp** — same primitives Docker uses
- The entire binary is a **single executable with zero external dependencies**
- Every component is **hand-written**: HTTP client, JSON parser, gzip decompressor, tar reader, hypervisor, virtio devices

---

## What We Replaced and How

Open `Cargo.toml`:

```toml
[dependencies]
# Nothing. Zero. Nada. Still.
```

| What most projects use | What HolyContainer does instead | Lines |
|---|---|---|
| `nix` or `libc` for syscalls | Raw `syscall()` via inline assembly / FFI | ~450 |
| `winapi` or `windows` crate for Win32 | Hand-declared `extern "system"` FFI bindings | ~220 |
| `serde` + `serde_json` for JSON | Recursive descent JSON parser | ~310 |
| `reqwest` or `hyper` for HTTP | WinHTTP FFI (Windows) / raw sockets (Linux) | ~350 |
| `flate2` for gzip/deflate | RFC 1951 DEFLATE decoder with Huffman + LZ77 | ~400 |
| `tar` crate for archives | UStar tar reader/writer | ~340 |
| `clap` or `structopt` for CLI | Hand-rolled argument parser | ~460 |
| `seccomp` or `libseccomp` | Raw BPF bytecode generation | ~120 |
| `caps` crate for capabilities | Direct `prctl(PR_CAPBSET_DROP)` calls | ~50 |
| QEMU/Firecracker for VMs | Hand-written WHP hypervisor with x86_64 setup | ~400 |
| Docker SDK for registry | Docker Registry HTTP API v2 client | ~350 |
| `docker-compose` | Multi-container orchestrator with TOML parser | ~300 |

**Total: ~3,750+ lines of hand-written systems code, zero dependencies.**

---

## What Can You Actually Do?

### Pull and Run Any Image

```bash
# Pull Ubuntu from Docker Hub
holycontainer pull ubuntu:22.04

# Create a container from the pulled image
holycontainer create mybox --image ubuntu:22.04 --memory 512M --pids 128
holycontainer start mybox -- /bin/bash

# Or do it all in one command
holycontainer run alpine:latest -- /bin/sh
```

### Multi-Container Stacks

Create a `stack.toml`:
```toml
[stack]
name = myapp

[service.web]
image = nginx:latest
ports = 8080:80
memory = 256M
depends_on = api

[service.api]
image = node:20-alpine
ports = 3000:3000
env.NODE_ENV = production
memory = 512M
depends_on = db

[service.db]
image = postgres:15
env.POSTGRES_PASSWORD = secret
volumes = ./data:/var/lib/postgresql/data
memory = 1G
```

```bash
holycontainer up -f stack.toml     # Start everything in dependency order
holycontainer down -f stack.toml   # Stop everything
```

### Boot a Linux Kernel on Windows (WHP Hypervisor)

```powershell
# Check if your system supports hardware virtualization
holycontainer vm-check

# Boot a Linux kernel in a micro-VM
holycontainer vm-boot --kernel bzImage --initrd initrd.gz --memory 256
```

### Full Container Lifecycle

```bash
# Bootstrap a rootfs from host
holycontainer init-rootfs /tmp/myroot

# Create with full options
holycontainer create myapp --rootfs /tmp/myroot \
  --memory 256M --cpus 50 --pids 64 \
  --hostname myhost --env APP=hello --port 8080:80

# Manage
holycontainer start myapp -- /bin/sh
holycontainer exec myapp -- ls /
holycontainer logs myapp
holycontainer ps
holycontainer stop myapp
holycontainer rm myapp
```

### Container Images

```bash
holycontainer image-create /tmp/myroot my-image.tar
holycontainer image-extract my-image.tar /tmp/newroot
holycontainer images
```

---

## The Architecture

### How Image Pulling Works

When you type `holycontainer pull ubuntu:22.04`:

```
main.rs: parse "pull ubuntu:22.04"
    |
registry.rs: pull_image()
    |-- Parse image reference → registry-1.docker.io/library/ubuntu:22.04
    |-- http.rs: GET https://auth.docker.io/token?scope=repository:library/ubuntu:pull
    |       |-- [Windows] WinHttpOpen() → WinHttpConnect() → WinHttpSendRequest()
    |       |-- [Linux]   raw TCP socket or curl subprocess
    |-- json.rs: parse token response → extract Bearer token
    |-- http.rs: GET /v2/library/ubuntu/manifests/22.04 (with Bearer header)
    |-- json.rs: parse manifest → list of layer digests
    |-- For each layer:
    |       |-- http.rs: GET /v2/library/ubuntu/blobs/sha256:abc...
    |       |-- gzip.rs: DEFLATE decompress (Huffman + LZ77 + CRC32)
    |       |-- registry.rs: extract tar from memory → rootfs directory
    |       |-- Handle .wh. (whiteout) files for layer overlay semantics
    |-- Save image metadata to local storage
```

### How the WHP Hypervisor Works

When you type `holycontainer vm-boot --kernel bzImage`:

```
main.rs: parse "vm-boot"
    |
vmm.rs: boot_linux()
    |-- whp.rs: WHvCreatePartition() → create lightweight VM
    |-- VirtualAlloc() → allocate guest memory (256 MB)
    |-- WHvMapGpaRange() → map host memory into guest physical address space
    |-- vmm.rs: setup_page_tables()
    |       |-- Build PML4 → PDPT → PD (identity-mapped 1 GB, 2 MB pages)
    |-- vmm.rs: setup_gdt()
    |       |-- Null + 64-bit code + 64-bit data segment descriptors
    |-- vmm.rs: load_kernel()
    |       |-- Parse bzImage header (HdrS signature, setup_sects)
    |       |-- Copy protected-mode kernel to 0x100000 (1 MB)
    |-- vmm.rs: setup_boot_params()
    |       |-- Linux boot protocol struct at 0x10000
    |       |-- E820 memory map, command line, initrd address
    |-- WHvSetVirtualProcessorRegisters()
    |       |-- CR0: PE + PG (protected mode + paging)
    |       |-- CR3: PML4 address (page table root)
    |       |-- CR4: PAE + PGE
    |       |-- EFER: LME + LMA (long mode enable + active)
    |       |-- CS: 64-bit code segment
    |       |-- RIP: 0x100000 (kernel entry)
    |       |-- RSI: boot_params pointer
    |-- WHvRunVirtualProcessor() → VM execution loop
            |-- Handle IO port exits (serial console → host stdout)
            |-- Handle MSR/CPUID/memory access exits
            |-- Handle HLT/shutdown exits
```

### How Linux Containers Work

On Linux, `holycontainer start` uses the same kernel primitives as Docker:

```
container.rs: start()
    |-- clone() with CLONE_NEWPID | CLONE_NEWNS | CLONE_NEWUTS | CLONE_NEWNET
    |-- In child: pivot_root() to new rootfs
    |-- In child: load seccomp BPF filter (hand-assembled bytecode)
    |-- In child: drop capabilities via prctl()
    |-- In child: exec() the requested command
    |-- Parent: write PID to cgroup/cgroup.procs
```

---

## Project Structure

```
src/
  main.rs              CLI + argument parsing (no clap)
  config.rs            Config serialization (no serde)
  container.rs         Lifecycle: create/start/stop/delete/exec/logs
  error.rs             Error types + errno handling
  json.rs              ★ Recursive descent JSON parser (no serde_json)
  http.rs              ★ HTTP/HTTPS client (WinHTTP FFI / raw sockets)
  gzip.rs              ★ RFC 1951 DEFLATE + RFC 1952 gzip decompressor
  registry.rs          ★ Docker Registry v2 client (auth, manifests, blobs)
  overlay.rs           ★ Overlay filesystem (overlayfs / union-copy)
  compose.rs           ★ Multi-container orchestration (stack parser)
  image.rs             UStar tar reader/writer (no tar crate)
  platform/
    mod.rs             Compile-time platform dispatch
    linux/
      syscall.rs       Raw syscall wrappers (no libc/nix)
      namespace.rs     PID/mount/UTS/net/IPC namespaces
      cgroup.rs        Cgroups v2 (memory, CPU, PIDs)
      seccomp.rs       BPF filter bytecode generation
      capabilities.rs  Capability dropping via prctl()
      filesystem.rs    pivot_root + mount /proc /sys /dev
      network.rs       veth pairs + ioctl + netlink
    windows/
      winapi.rs        Hand-declared Win32 FFI (no winapi crate)
      job.rs           Job Objects for resource limits
      sandbox.rs       Restricted tokens + sandboxed CreateProcess
      filesystem.rs    System binary copying for rootfs
      network.rs       Firewall rule management
      whp.rs           ★ Windows Hypervisor Platform FFI bindings
      vmm.rs           ★ Virtual Machine Monitor (page tables, kernel loading)
      virtio.rs        ★ Virtio console, block, and network devices

★ = New in v2.0
```

---

## Comparison

| | HolyContainer v2 | Docker | Podman | WSL2 |
|---|---|---|---|---|
| How it runs Linux on Windows | **Hand-written WHP hypervisor** | WSL2 (Hyper-V VM) | WSL2/QEMU | Full Hyper-V VM |
| Pull images from Docker Hub | **Yes (homemade HTTP + JSON)** | Yes | Yes | N/A |
| External Rust dependencies | **0** | N/A (Go) | N/A (Go) | N/A |
| Understands its own VM layer | **Yes — every page table entry** | No (black-box Hyper-V) | No (black-box QEMU) | No |
| Multi-container stacks | **Yes (compose-like)** | Yes (Compose) | Yes (Podman Compose) | No |
| gzip decompression | **Hand-written RFC 1951** | System library | System library | System |
| JSON parsing | **Hand-written recursive descent** | encoding/json (Go) | encoding/json (Go) | N/A |
| Binary size | Single small executable | CLI + daemon + containerd + runc | Podman + conmon + crun | Full OS image |

---

## Installation

### Prerequisites

- **Rust 1.70+** from [rustup.rs](https://rustup.rs/)
- **Windows**: Nothing else needed. For VM features, enable "Windows Hypervisor Platform" in Windows Features
- **Linux**: Root privileges for namespace/cgroup operations

### Build

### Build

```bash
git clone https://github.com/SelmanDemiray/HolyContainer.git
cd HolyContainer
cargo build --release
```

Output binary:
- Linux: `target/release/holycontainer`
- Windows: `target\release\holycontainer.exe`

### Running the App & Dashboard

You can use the compiled executable directly to run all commands:

```powershell
# Windows
.\target\release\holycontainer.exe help
.\target\release\holycontainer.exe pull ubuntu:22.04
.\target\release\holycontainer.exe run alpine:latest -- /bin/sh

# Start the web Dashboard GUI
.\target\release\holycontainer.exe dashboard --port 8080
```

```bash
# Linux
./target/release/holycontainer help
./target/release/holycontainer dashboard --port 8080
```

### Verify WHP (Windows only)

```powershell
.\target\release\holycontainer.exe vm-check
```

---

## All Commands

```
CONTAINER LIFECYCLE:
    create <name>        Create container (--rootfs, --image, --memory, --cpus, etc.)
    start <name>         Start container (-- cmd args...)
    stop <name>          Stop running container
    rm <name>            Delete stopped container
    ps                   List all containers
    exec <name> -- cmd   Execute in running container
    logs <name>          Show container output

IMAGE OPERATIONS:
    pull <image:tag>             Pull from Docker Hub / registry
    run <image:tag> -- cmd       Pull + create + start
    images                       List local images
    init-rootfs <path>           Bootstrap rootfs from host
    image-create <dir> <out>     Create tar image
    image-extract <tar> <dir>    Extract tar image

STACK OPERATIONS:
    up -f <stack.toml>           Start multi-container stack
    down -f <stack.toml>         Stop multi-container stack

VM OPERATIONS (Windows):
    vm-boot                      Boot Linux kernel in micro-VM
    vm-check                     Check WHP availability
```

---

## License

MIT

---

*Every syscall. Every HTTP request. Every Huffman tree. Every page table. Written by hand in pure Rust.*
