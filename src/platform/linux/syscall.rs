// Raw Linux syscall wrappers — no libc crate, just extern "C" and constants.

#![allow(non_camel_case_types, dead_code)]

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use crate::error::{syscall_error, ContainerError, Result};

// ─── Syscall Numbers (x86_64) ───────────────────────────────────────────────
pub const SYS_CLONE: i64 = 56;
pub const SYS_CLONE3: i64 = 435;
pub const SYS_MOUNT: i64 = 165;
pub const SYS_UMOUNT2: i64 = 166;
pub const SYS_PIVOT_ROOT: i64 = 155;
pub const SYS_UNSHARE: i64 = 272;
pub const SYS_SETHOSTNAME: i64 = 170;
pub const SYS_PRCTL: i64 = 157;
pub const SYS_SECCOMP: i64 = 317;
pub const SYS_MKNOD: i64 = 133;
pub const SYS_SETNS: i64 = 308;
pub const SYS_WAITID: i64 = 247;

// ─── Namespace Flags ────────────────────────────────────────────────────────
pub const CLONE_NEWNS: u64 = 0x00020000;     // Mount namespace
pub const CLONE_NEWUTS: u64 = 0x04000000;    // UTS namespace (hostname)
pub const CLONE_NEWIPC: u64 = 0x08000000;    // IPC namespace
pub const CLONE_NEWUSER: u64 = 0x10000000;   // User namespace
pub const CLONE_NEWPID: u64 = 0x20000000;    // PID namespace
pub const CLONE_NEWNET: u64 = 0x40000000;    // Network namespace
pub const CLONE_NEWCGROUP: u64 = 0x02000000; // Cgroup namespace

// ─── Mount Flags ────────────────────────────────────────────────────────────
pub const MS_RDONLY: u64 = 1;
pub const MS_NOSUID: u64 = 2;
pub const MS_NODEV: u64 = 4;
pub const MS_NOEXEC: u64 = 8;
pub const MS_REMOUNT: u64 = 32;
pub const MS_BIND: u64 = 4096;
pub const MS_REC: u64 = 16384;
pub const MS_PRIVATE: u64 = 1 << 18;
pub const MS_SLAVE: u64 = 1 << 19;

pub const MNT_DETACH: i32 = 2;

// ─── Signal Constants ───────────────────────────────────────────────────────
pub const SIGKILL: i32 = 9;
pub const SIGTERM: i32 = 15;
pub const SIGCHLD: i32 = 17;

// ─── prctl Constants ────────────────────────────────────────────────────────
pub const PR_SET_PDEATHSIG: i32 = 1;
pub const PR_CAPBSET_DROP: i32 = 24;
pub const PR_SET_NO_NEW_PRIVS: i32 = 38;
pub const PR_SET_SECUREBITS: i32 = 28;
pub const PR_SET_SECCOMP: i32 = 22;

// ─── Seccomp Constants ──────────────────────────────────────────────────────
pub const SECCOMP_MODE_FILTER: u64 = 2;
pub const SECCOMP_SET_MODE_FILTER: u32 = 1;

// ─── Capability Constants ───────────────────────────────────────────────────
pub const CAP_CHOWN: u32 = 0;
pub const CAP_DAC_OVERRIDE: u32 = 1;
pub const CAP_DAC_READ_SEARCH: u32 = 2;
pub const CAP_FOWNER: u32 = 3;
pub const CAP_FSETID: u32 = 4;
pub const CAP_KILL: u32 = 5;
pub const CAP_SETGID: u32 = 6;
pub const CAP_SETUID: u32 = 7;
pub const CAP_SETPCAP: u32 = 8;
pub const CAP_LINUX_IMMUTABLE: u32 = 9;
pub const CAP_NET_BIND_SERVICE: u32 = 10;
pub const CAP_NET_BROADCAST: u32 = 11;
pub const CAP_NET_ADMIN: u32 = 12;
pub const CAP_NET_RAW: u32 = 13;
pub const CAP_IPC_LOCK: u32 = 14;
pub const CAP_IPC_OWNER: u32 = 15;
pub const CAP_SYS_MODULE: u32 = 16;
pub const CAP_SYS_RAWIO: u32 = 17;
pub const CAP_SYS_CHROOT: u32 = 18;
pub const CAP_SYS_PTRACE: u32 = 19;
pub const CAP_SYS_PACCT: u32 = 20;
pub const CAP_SYS_ADMIN: u32 = 21;
pub const CAP_SYS_BOOT: u32 = 22;
pub const CAP_SYS_NICE: u32 = 23;
pub const CAP_SYS_RESOURCE: u32 = 24;
pub const CAP_SYS_TIME: u32 = 25;
pub const CAP_SYS_TTY_CONFIG: u32 = 26;
pub const CAP_MKNOD: u32 = 27;
pub const CAP_LEASE: u32 = 28;
pub const CAP_AUDIT_WRITE: u32 = 29;
pub const CAP_AUDIT_CONTROL: u32 = 30;
pub const CAP_SETFCAP: u32 = 31;
pub const CAP_MAC_OVERRIDE: u32 = 32;
pub const CAP_MAC_ADMIN: u32 = 33;
pub const CAP_SYSLOG: u32 = 34;
pub const CAP_WAKE_ALARM: u32 = 35;
pub const CAP_BLOCK_SUSPEND: u32 = 36;
pub const CAP_AUDIT_READ: u32 = 37;
pub const CAP_LAST_CAP: u32 = 37;

// ─── Device type macros ─────────────────────────────────────────────────────
pub const S_IFCHR: u32 = 0o020000; // Character device
pub const S_IFBLK: u32 = 0o060000; // Block device

// ─── BPF Constants for Seccomp ──────────────────────────────────────────────
pub const BPF_LD: u16 = 0x00;
pub const BPF_W: u16 = 0x00;
pub const BPF_ABS: u16 = 0x20;
pub const BPF_JMP: u16 = 0x05;
pub const BPF_JEQ: u16 = 0x10;
pub const BPF_K: u16 = 0x00;
pub const BPF_RET: u16 = 0x06;

pub const SECCOMP_RET_ALLOW: u32 = 0x7fff0000;
pub const SECCOMP_RET_KILL: u32 = 0x00000000;
pub const SECCOMP_RET_ERRNO: u32 = 0x00050000;

// Audit arch for x86_64
pub const AUDIT_ARCH_X86_64: u32 = 0xC000003E;

// ─── Netlink Constants ──────────────────────────────────────────────────────
pub const AF_NETLINK: i32 = 16;
pub const AF_INET: i32 = 2;
pub const SOCK_RAW: i32 = 3;
pub const SOCK_DGRAM: i32 = 2;
pub const NETLINK_ROUTE: i32 = 0;

pub const RTM_NEWLINK: u16 = 16;
pub const RTM_SETLINK: u16 = 19;
pub const RTM_NEWADDR: u16 = 20;
pub const RTM_NEWROUTE: u16 = 24;

pub const NLM_F_REQUEST: u16 = 1;
pub const NLM_F_CREATE: u16 = 0x400;
pub const NLM_F_EXCL: u16 = 0x200;
pub const NLM_F_ACK: u16 = 4;

pub const IFF_UP: u32 = 1;

pub const IFLA_IFNAME: u16 = 3;
pub const IFLA_MASTER: u16 = 10;
pub const IFLA_LINKINFO: u16 = 18;
pub const IFLA_INFO_KIND: u16 = 1;
pub const IFLA_INFO_DATA: u16 = 2;
pub const IFLA_NET_NS_PID: u16 = 19;
pub const IFLA_LINK: u16 = 5;

pub const IFA_ADDRESS: u16 = 1;
pub const IFA_LOCAL: u16 = 2;

// ─── Libc FFI ───────────────────────────────────────────────────────────────
// We define only what we need, without importing the `libc` crate.

extern "C" {
    pub fn mount(
        source: *const u8,
        target: *const u8,
        fstype: *const u8,
        flags: u64,
        data: *const u8,
    ) -> i32;

    pub fn umount2(target: *const u8, flags: i32) -> i32;

    pub fn unshare(flags: i32) -> i32;

    pub fn sethostname(name: *const u8, len: usize) -> i32;

    pub fn chroot(path: *const u8) -> i32;

    pub fn chdir(path: *const u8) -> i32;

    pub fn prctl(option: i32, arg2: u64, arg3: u64, arg4: u64, arg5: u64) -> i32;

    pub fn fork() -> i32;

    pub fn execve(filename: *const u8, argv: *const *const u8, envp: *const *const u8) -> i32;

    pub fn waitpid(pid: i32, status: *mut i32, options: i32) -> i32;

    pub fn kill(pid: i32, sig: i32) -> i32;

    pub fn getpid() -> i32;

    pub fn getuid() -> u32;

    pub fn getgid() -> u32;

    pub fn setuid(uid: u32) -> i32;

    pub fn setgid(gid: u32) -> i32;

    pub fn setsid() -> i32;

    pub fn dup2(oldfd: i32, newfd: i32) -> i32;

    pub fn close(fd: i32) -> i32;

    pub fn read(fd: i32, buf: *mut u8, count: usize) -> isize;

    pub fn write(fd: i32, buf: *const u8, count: usize) -> isize;

    pub fn pipe(pipefd: *mut [i32; 2]) -> i32;

    pub fn socket(domain: i32, stype: i32, protocol: i32) -> i32;

    pub fn bind(sockfd: i32, addr: *const u8, addrlen: u32) -> i32;

    pub fn sendto(
        sockfd: i32,
        buf: *const u8,
        len: usize,
        flags: i32,
        dest_addr: *const u8,
        addrlen: u32,
    ) -> isize;

    pub fn recvfrom(
        sockfd: i32,
        buf: *mut u8,
        len: usize,
        flags: i32,
        src_addr: *mut u8,
        addrlen: *mut u32,
    ) -> isize;

    pub fn mknod(pathname: *const u8, mode: u32, dev: u64) -> i32;

    pub fn symlink(target: *const u8, linkpath: *const u8) -> i32;

    pub fn mkdir(pathname: *const u8, mode: u32) -> i32;

    pub fn ioctl(fd: i32, request: u64, ...) -> i32;

    pub fn setsockopt(
        sockfd: i32,
        level: i32,
        optname: i32,
        optval: *const u8,
        optlen: u32,
    ) -> i32;
}

// ─── Syscall via assembly (for calls not in libc wrappers) ──────────────────

/// Raw syscall wrapper for pivot_root (no libc wrapper exists).
pub unsafe fn pivot_root(new_root: &str, put_old: &str) -> Result<()> {
    let new_root_c = CString::new(new_root)
        .map_err(|_| ContainerError::Filesystem("invalid new_root path".into()))?;
    let put_old_c = CString::new(put_old)
        .map_err(|_| ContainerError::Filesystem("invalid put_old path".into()))?;

    let ret: i64;
    std::arch::asm!(
        "syscall",
        in("rax") SYS_PIVOT_ROOT,
        in("rdi") new_root_c.as_ptr(),
        in("rsi") put_old_c.as_ptr(),
        out("rcx") _,
        out("r11") _,
        lateout("rax") ret,
    );

    if ret < 0 {
        Err(syscall_error("pivot_root"))
    } else {
        Ok(())
    }
}

/// Raw syscall wrapper for setns.
pub unsafe fn setns(fd: i32, nstype: i32) -> Result<()> {
    let ret: i64;
    std::arch::asm!(
        "syscall",
        in("rax") SYS_SETNS,
        in("rdi") fd as i64,
        in("rsi") nstype as i64,
        out("rcx") _,
        out("r11") _,
        lateout("rax") ret,
    );

    if ret < 0 {
        Err(syscall_error("setns"))
    } else {
        Ok(())
    }
}

// ─── Helper wrappers ────────────────────────────────────────────────────────

/// Mount a filesystem.
pub fn do_mount(
    source: Option<&str>,
    target: &Path,
    fstype: Option<&str>,
    flags: u64,
    data: Option<&str>,
) -> Result<()> {
    let source_c = source.map(|s| CString::new(s).unwrap());
    let target_c = CString::new(target.as_os_str().as_bytes())
        .map_err(|_| ContainerError::Filesystem("invalid mount target".into()))?;
    let fstype_c = fstype.map(|s| CString::new(s).unwrap());
    let data_c = data.map(|s| CString::new(s).unwrap());

    let ret = unsafe {
        mount(
            source_c.as_ref().map_or(std::ptr::null(), |c| c.as_ptr() as *const u8),
            target_c.as_ptr() as *const u8,
            fstype_c.as_ref().map_or(std::ptr::null(), |c| c.as_ptr() as *const u8),
            flags,
            data_c.as_ref().map_or(std::ptr::null(), |c| c.as_ptr() as *const u8),
        )
    };

    if ret != 0 {
        Err(syscall_error("mount"))
    } else {
        Ok(())
    }
}

/// Unmount a filesystem.
pub fn do_umount(target: &Path, flags: i32) -> Result<()> {
    let target_c = CString::new(target.as_os_str().as_bytes())
        .map_err(|_| ContainerError::Filesystem("invalid umount target".into()))?;

    let ret = unsafe { umount2(target_c.as_ptr() as *const u8, flags) };

    if ret != 0 {
        Err(syscall_error("umount2"))
    } else {
        Ok(())
    }
}

/// Set hostname.
pub fn do_sethostname(name: &str) -> Result<()> {
    let ret = unsafe { sethostname(name.as_ptr(), name.len()) };
    if ret != 0 {
        Err(syscall_error("sethostname"))
    } else {
        Ok(())
    }
}

/// Unshare namespaces.
pub fn do_unshare(flags: u64) -> Result<()> {
    let ret = unsafe { unshare(flags as i32) };
    if ret != 0 {
        Err(syscall_error("unshare"))
    } else {
        Ok(())
    }
}

/// Create a device node.
pub fn do_mknod(path: &Path, mode: u32, dev: u64) -> Result<()> {
    let path_c = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| ContainerError::Filesystem("invalid mknod path".into()))?;

    let ret = unsafe { mknod(path_c.as_ptr() as *const u8, mode, dev) };

    if ret != 0 {
        // Ignore EEXIST (17)
        let errno = unsafe { *crate::error::libc_errno_ptr() };
        if errno != 17 {
            return Err(syscall_error("mknod"));
        }
    }
    Ok(())
}

/// Create a symlink.
pub fn do_symlink(target: &str, linkpath: &Path) -> Result<()> {
    let target_c = CString::new(target)
        .map_err(|_| ContainerError::Filesystem("invalid symlink target".into()))?;
    let linkpath_c = CString::new(linkpath.as_os_str().as_bytes())
        .map_err(|_| ContainerError::Filesystem("invalid symlink path".into()))?;

    let ret = unsafe { symlink(target_c.as_ptr() as *const u8, linkpath_c.as_ptr() as *const u8) };

    if ret != 0 {
        let errno = unsafe { *crate::error::libc_errno_ptr() };
        if errno != 17 {
            return Err(syscall_error("symlink"));
        }
    }
    Ok(())
}

/// Make directory via libc (with proper error handling for EEXIST).
pub fn do_mkdir(path: &Path, mode: u32) -> Result<()> {
    let path_c = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| ContainerError::Filesystem("invalid mkdir path".into()))?;

    let ret = unsafe { mkdir(path_c.as_ptr() as *const u8, mode) };

    if ret != 0 {
        let errno = unsafe { *crate::error::libc_errno_ptr() };
        if errno != 17 {
            // EEXIST is OK
            return Err(syscall_error("mkdir"));
        }
    }
    Ok(())
}

/// Make a linux device number from major and minor.
pub fn makedev(major: u32, minor: u32) -> u64 {
    let major = major as u64;
    let minor = minor as u64;
    ((major & 0xfffff000) << 32)
        | ((major & 0x00000fff) << 8)
        | ((minor & 0xffffff00) << 12)
        | (minor & 0x000000ff)
}
