# HolyContainer

**A real container runtime built from absolute zero in pure Rust. No Docker. No WSL. No third-party crates. Every syscall wrapper, every tar byte parser, every BPF instruction — hand-written.**

[![Rust](https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Windows](https://img.shields.io/badge/Windows-0078D6?style=for-the-badge&logo=windows&logoColor=white)](#how-windows-containers-work)
[![Linux](https://img.shields.io/badge/Linux-FCC624?style=for-the-badge&logo=linux&logoColor=black)](#how-linux-containers-work)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg?style=for-the-badge)](LICENSE)

---

## Why Does This Exist?

Docker on Windows requires WSL2 (a full Linux virtual machine) or Hyper-V. That's not a real Windows container — it's Linux running inside a VM, pretending to be native. WSL itself is also a VM.

HolyContainer takes a completely different approach: **it talks directly to the operating system kernel**. On Windows, it calls Win32 APIs (`kernel32.dll`, `advapi32.dll`) through raw FFI. On Linux, it invokes syscalls directly. There is no middle layer, no daemon, no virtual machine.

This means:
- On Windows, you get **actual Windows process isolation** using Job Objects and restricted tokens — the same primitives the OS itself uses
- On Linux, you get **actual kernel namespaces, cgroups, and seccomp** — the same primitives Docker uses internally, but without Docker's massive dependency stack
- You can run real containers (Ubuntu, Alpine, or any rootfs) on Linux, and sandboxed Windows processes on Windows
- The entire binary is a single executable with **zero external dependencies**

---

## Why Everything Is Homemade

Open `Cargo.toml`:

```toml
[dependencies]
# Nothing. Zero. Nada.
```

Most container runtimes pull in hundreds of crates. HolyContainer pulls in **none**. Here's what was built by hand and why:

### What We Replaced and How

| What most projects use | What HolyContainer does instead | Why |
|---|---|---|
| `nix` or `libc` crate for syscalls | Raw `syscall()` via inline assembly / FFI to `__errno_location` | Direct kernel interface, no wrappers on top of wrappers |
| `winapi` or `windows` crate for Win32 | Hand-declared `extern "system"` FFI bindings to `kernel32.dll` and `advapi32.dll` | Every struct, constant, and function signature defined from scratch |
| `serde` + `serde_json` / `toml` for config | Custom `key=value` serializer/deserializer in `config.rs` | ~200 lines replaces a 50,000-line dependency tree |
| `tar` crate for archives | Full UStar tar reader/writer in `image.rs` | Reads/writes 512-byte tar headers, computes checksums, handles prefix/name splits |
| `clap` or `structopt` for CLI | Hand-rolled argument parser in `main.rs` | Simple `match` on `args[1]` with manual option extraction |
| `seccomp` or `libseccomp` bindings | Raw BPF bytecode generation in `seccomp.rs` | Builds `sock_filter` arrays and loads them via `prctl(PR_SET_SECCOMP)` |
| `caps` crate for capabilities | Direct `prctl(PR_CAPBSET_DROP)` calls in `capabilities.rs` | Iterates through capability numbers and drops each one |

Every one of these was implemented because the goal is to understand and control every byte that flows between the runtime and the kernel.

---

## The Container Engine — How It Actually Works

### The Lifecycle

When you type `holycontainer create mycontainer --rootfs /path`, here's what actually happens in the code:

```
main.rs: parse CLI args
    |
    v
container.rs: create()
    |-- Validate rootfs directory exists
    |-- Call platform::create_isolation()
    |       |
    |       |-- [Linux]  Create a cgroup directory under /sys/fs/cgroup/holycontainer_<name>
    |       |-- [Windows] Call CreateJobObjectW() to get a kernel Job Object handle
    |       |
    |-- Call platform::set_resource_limits()
    |       |
    |       |-- [Linux]  Write to cgroup files: memory.max, cpu.max, pids.max
    |       |-- [Windows] Call SetInformationJobObject() with memory/CPU/PID limits
    |       |
    |-- Serialize config to key=value format, save to state directory
    |-- Cleanup isolation context (will be recreated on start)
```

When you type `holycontainer start mycontainer -- /bin/bash`:

```
container.rs: start()
    |
    |-- Load saved config from disk
    |-- Recreate isolation context (cgroup or Job Object)
    |-- Reapply resource limits
    |-- Build environment variables (PATH, HOME, TERM, HOSTNAME, user-defined)
    |
    |-- Call platform::spawn_process()
            |
            |-- [Linux]
            |       |-- clone() with CLONE_NEWPID | CLONE_NEWNS | CLONE_NEWUTS | CLONE_NEWNET
            |       |-- In child: setup_filesystem() -> pivot_root to new rootfs
            |       |-- In child: apply_security() -> load seccomp BPF filter
            |       |-- In child: drop capabilities
            |       |-- In child: exec() the requested command (/bin/bash)
            |       |-- Parent: write child PID to cgroup/cgroup.procs
            |
            |-- [Windows]
                    |-- OpenProcessToken() on current process
                    |-- CreateRestrictedToken() with DISABLE_MAX_PRIVILEGE
                    |-- CreateProcessAsUserW() with restricted token + CREATE_SUSPENDED
                    |-- AssignProcessToJobObject() to enforce resource limits
                    |-- ResumeThread() to let the process run
```

### What "Isolation" Actually Means in the Code

#### On Linux — Real Kernel Isolation

**Namespaces** (`namespace.rs`): The `clone()` syscall is called with flags that create entirely new kernel namespaces:

```rust
// This is actual code from syscall.rs - raw syscall, no libc
pub unsafe fn clone3(flags: u64, stack: *mut u8, stack_size: usize) -> i64 {
    // CLONE_NEWPID  = new process ID namespace (container sees PID 1)
    // CLONE_NEWNS   = new mount namespace (container has its own mount table)
    // CLONE_NEWUTS  = new UTS namespace (container has its own hostname)
    // CLONE_NEWNET  = new network namespace (container has its own network stack)
    // CLONE_NEWIPC  = new IPC namespace (isolated shared memory / semaphores)
    syscall(SYS_CLONE3, &args as *const _ as u64, size as u64)
}
```

This is the same mechanism Docker uses. The container process literally cannot see other processes, cannot see the host filesystem, and has its own hostname — enforced by the Linux kernel.

**Cgroups v2** (`cgroup.rs`): Resource limits are enforced by writing to cgroup filesystem nodes:

```rust
// Actual logic from cgroup.rs
fn set_memory_limit(cgroup_path: &Path, bytes: u64) {
    // The kernel reads this file and will OOM-kill the container
    // if it exceeds this many bytes of RAM
    fs::write(cgroup_path.join("memory.max"), bytes.to_string())
}
```

This is not a suggestion — the kernel **will** terminate the container if it exceeds its memory limit. The container cannot override this.

**Seccomp BPF** (`seccomp.rs`): A BPF (Berkeley Packet Filter) program is assembled by hand and loaded into the kernel:

```rust
// Actual logic from seccomp.rs - building raw BPF instructions
let filter = [
    // Load syscall number from seccomp_data.nr
    bpf_stmt(BPF_LD | BPF_W | BPF_ABS, 0),
    // If syscall == SYS_REBOOT, kill the process
    bpf_jump(BPF_JMP | BPF_JEQ | BPF_K, SYS_REBOOT, 0, 1),
    bpf_stmt(BPF_RET | BPF_K, SECCOMP_RET_KILL),
    // ... more blocked syscalls ...
    // Default: allow
    bpf_stmt(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
];
// Load the filter into the kernel via prctl()
prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER, &prog);
```

This means the container process physically cannot call dangerous syscalls like `reboot()`, `kexec_load()`, or `mount()` outside its namespace. The kernel intercepts the attempt and kills the process.

**Filesystem** (`filesystem.rs`): Uses `pivot_root` (not `chroot`) because `chroot` can be escaped with enough privileges, but `pivot_root` combined with namespace isolation cannot:

```rust
// Mount the new rootfs, pivot into it, unmount the old root
mount(rootfs, rootfs, "bind", MS_BIND | MS_REC);
pivot_root(rootfs, old_root);         // Swap filesystem root
umount2(old_root, MNT_DETACH);       // Old root is gone forever
```

After this, the container has no way to access the host filesystem. The old root doesn't exist in its mount namespace.

#### On Windows — Native Win32 Isolation

**Job Objects** (`job.rs`): These are kernel objects that group processes and enforce limits:

```rust
// Actual code from job.rs - raw Win32 FFI
let handle = CreateJobObjectW(null_mut(), wide_name.as_ptr());

// Set memory limit - kernel will terminate process if exceeded
info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_JOB_MEMORY
                                      | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
info.JobMemoryLimit = 268_435_456; // 256 MB hard cap

SetInformationJobObject(handle, JOBOBJECTCLASS_EXTENDED_LIMIT, &info, size);
```

The `KILL_ON_JOB_CLOSE` flag means all container processes are automatically killed when the runtime exits — no orphan processes.

**Restricted Tokens** (`sandbox.rs`): The container process runs with stripped privileges:

```rust
// Open our own process token
OpenProcessToken(GetCurrentProcess(), TOKEN_ALL_ACCESS, &mut token);

// Create a copy with ALL privileges removed
CreateRestrictedToken(token, DISABLE_MAX_PRIVILEGE, ...);

// Spawn the container process using this stripped token
CreateProcessAsUserW(restricted_token, ..., CREATE_SUSPENDED, ...);

// Put it in the job object BEFORE it can run
AssignProcessToJobObject(job_handle, process_handle);

// Now let it run - it's already locked down
ResumeThread(thread_handle);
```

The process is created **suspended**, so it never gets a chance to run with any privileges before being assigned to the Job Object. By the time it executes its first instruction, it's already caged.

**All Win32 FFI Bindings** (`winapi.rs`): Every single Win32 type, constant, and function is declared by hand:

```rust
// These are NOT from the 'winapi' crate - they're hand-written
pub type HANDLE = *mut std::ffi::c_void;
pub type DWORD = u32;
pub const JOB_OBJECT_LIMIT_JOB_MEMORY: DWORD = 0x00000200;

#[link(name = "kernel32")]
extern "system" {
    pub fn CreateJobObjectW(...) -> HANDLE;
    pub fn SetInformationJobObject(...) -> BOOL;
    pub fn CreateProcessW(...) -> BOOL;
    // ... every function signature manually defined
}
```

There are ~220 lines of hand-declared Win32 types, structs, constants, and function signatures. No code generation, no binding generators.

---

## What Can You Actually Run?

### On Linux — Real Linux Containers

You can run any Linux distribution's rootfs as a container:

```bash
# Download an Ubuntu rootfs
wget https://cdimage.ubuntu.com/ubuntu-base/releases/22.04/release/ubuntu-base-22.04-base-amd64.tar.gz
mkdir /tmp/ubuntu-rootfs
tar xf ubuntu-base-22.04-base-amd64.tar.gz -C /tmp/ubuntu-rootfs

# Run Ubuntu in HolyContainer
sudo holycontainer create ubuntu-box --rootfs /tmp/ubuntu-rootfs --memory 512M --pids 128
sudo holycontainer start ubuntu-box -- /bin/bash
```

You can also run Alpine, Debian, Fedora, Arch — any distro that provides a rootfs tarball. The container gets full namespace isolation, resource limits, seccomp filtering, and capability dropping — just like Docker.

### On Windows — Sandboxed Windows Processes

```powershell
# Bootstrap a Windows rootfs (copies cmd.exe, powershell, DLLs)
holycontainer init-rootfs C:\tmp\myroot

# Run a sandboxed process with resource limits
holycontainer create winbox --rootfs C:\tmp\myroot --memory 256M --cpus 50
holycontainer start winbox -- cmd.exe /c "echo Running in a container && whoami"
```

The Windows container process runs with a restricted token (privileges stripped), inside a Job Object (memory/CPU/PID capped), using its own copy of system binaries.

---

## The Custom Tar Format (image.rs)

Most tools use the `tar` crate (which pulls in more crates). HolyContainer implements the **POSIX.1 UStar tar format** from scratch — both reading and writing:

```
512-byte tar header:
+----------+------+-----+-----+------+-------+----------+----------+
|  name    | mode | uid | gid | size | mtime | checksum | typeflag |
| (100 B)  | (8B) | (8B)| (8B)| (12B)| (12B) |  (8 B)   |  (1 B)   |
+----------+------+-----+-----+------+-------+----------+----------+
| linkname | magic  | ver | uname  | gname  | devmaj | devmin | prefix  |
| (100 B)  | (6 B)  | (2B)| (32 B) | (32 B) | (8 B)  | (8 B)  | (155 B) |
+----------+--------+-----+--------+--------+--------+--------+---------+
```

The code:
- Constructs 512-byte headers with correct octal-encoded sizes and checksums
- Splits long filenames across the `prefix` and `name` fields at `/` boundaries (UStar spec)
- Handles regular files, directories, and symlinks
- Pads file content to 512-byte block boundaries
- Writes the two-block end-of-archive marker

This lets you create portable container images (`holycontainer image-create`) and extract them (`holycontainer image-extract`) without any external tool.

---

## Installation

### Prerequisites

- **Rust 1.70+** from [rustup.rs](https://rustup.rs/)
- **Windows**: Nothing else needed — no WSL, no Docker, no VMs
- **Linux**: Root privileges for namespace/cgroup operations

### Build

```bash
git clone https://github.com/SelmanDemiray/OneLight.git
cd OneLight
cargo build --release
```

Output binary:
- Linux: `target/release/holycontainer`
- Windows: `target\release\holycontainer.exe`

---

## Usage

### Full Workflow

```bash
# Bootstrap a rootfs
holycontainer init-rootfs /tmp/myroot                     # copies host binaries

# Create a container with limits
holycontainer create myapp --rootfs /tmp/myroot \
  --memory 256M --cpus 50 --pids 64 \
  --hostname myhost --env APP=hello --port 8080:80

# Start it
holycontainer start myapp -- /bin/sh                      # Linux
holycontainer start myapp -- cmd.exe /c "echo hi"         # Windows

# Check status
holycontainer ps

# Stop and remove
holycontainer stop myapp
holycontainer rm myapp
```

### Container Images

```bash
holycontainer image-create /tmp/myroot my-image.tar       # package rootfs
holycontainer image-extract my-image.tar /tmp/newroot     # unpack image
holycontainer images                                       # list images
```

### All Options

```
create <name> --rootfs <path>     Create container (required: --rootfs)
    --memory <size>                Memory limit (128M, 1G, etc.)
    --cpus <percent>               CPU cap (1-100)
    --pids <max>                   Max processes
    --env <KEY=VALUE>              Environment variable (repeatable)
    --hostname <name>              Container hostname
    --workdir <path>               Working directory
    --port <host:container>        Port mapping (repeatable)
    --no-network                   Disable networking

start <name> [-- cmd args...]     Start container (optional command override)
stop <name>                        Stop running container
rm <name>                          Delete stopped container
ps                                 List all containers
init-rootfs <path>                 Bootstrap rootfs from host
image-create <dir> <out.tar>       Create tar image
image-extract <in.tar> <dir>       Extract tar image
images                             List stored images
```

---

## Project Structure

```
src/
  main.rs              CLI + argument parsing (no clap)
  config.rs            Config serialization (no serde)
  container.rs         Lifecycle: create/start/stop/delete
  error.rs             Error types + errno handling (both platforms)
  image.rs             UStar tar reader/writer (no tar crate)
  platform/
    mod.rs             Compile-time platform dispatch
    linux/
      syscall.rs       Raw syscall wrappers (no libc/nix)
      namespace.rs     PID/mount/UTS/net/IPC/user namespaces
      cgroup.rs        Cgroups v2 (memory, CPU, PIDs)
      seccomp.rs       BPF filter bytecode generation (no libseccomp)
      capabilities.rs  Capability dropping via prctl()
      filesystem.rs    pivot_root + mount /proc /sys /dev
      network.rs       veth pairs + ioctl + netlink
    windows/
      winapi.rs        Hand-declared Win32 FFI (no winapi crate)
      job.rs           Job Objects for resource limits
      sandbox.rs       Restricted tokens + sandboxed CreateProcess
      filesystem.rs    System binary copying for rootfs
      network.rs       Firewall rule management
```

---

## Comparison

| | HolyContainer | Docker | WSL |
|---|---|---|---|
| How it works | Direct kernel API calls | Daemon + containerd + runc | Full Linux VM via Hyper-V |
| Windows containers | Native Win32 (Job Objects) | Requires WSL2 Linux VM | IS a VM, not containers |
| Linux containers | Raw syscalls (clone, cgroups, seccomp) | Same syscalls, but through layers of Go code | Runs full Linux kernel |
| External dependencies | **0 crates** | Hundreds of Go modules | N/A |
| Requires admin/daemon | No daemon (direct execution) | Yes (dockerd must be running) | Yes (WSL service) |
| Binary size | Single small executable | Docker CLI + daemon + containerd + runc | Full OS image |
| Can run Ubuntu rootfs | Yes (Linux) | Yes | Yes (but it's a VM) |

---

## License

MIT

---

*Every syscall. Every struct. Every byte. Written by hand in pure Rust.*
