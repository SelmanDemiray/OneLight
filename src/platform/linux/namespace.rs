///! Linux namespace setup — PID, MNT, NET, UTS, IPC, USER.

use std::fs;
use std::path::Path;

use crate::error::{ContainerError, Result};
use super::syscall::*;

/// All namespace flags combined for full container isolation.
pub const ALL_NAMESPACES: u64 = CLONE_NEWNS
    | CLONE_NEWUTS
    | CLONE_NEWIPC
    | CLONE_NEWPID
    | CLONE_NEWNET
    | CLONE_NEWCGROUP;

/// Unshare into new namespaces (called in the child process).
pub fn unshare_namespaces(flags: u64) -> Result<()> {
    do_unshare(flags)
}

/// Set up user namespace mappings.
/// This allows running containers without root by mapping container UID 0 → host UID.
pub fn setup_user_namespace(pid: u32) -> Result<()> {
    let uid = unsafe { getuid() };
    let gid = unsafe { getgid() };

    // Write UID mapping: container UID 0 → host UID
    let uid_map = format!("0 {} 1\n", uid);
    let uid_map_path = format!("/proc/{}/uid_map", pid);
    fs::write(&uid_map_path, &uid_map).map_err(|e| {
        ContainerError::Syscall {
            call: "write uid_map",
            code: 0,
            detail: format!("{}: {}", uid_map_path, e),
        }
    })?;

    // Must write "deny" to setgroups before writing gid_map
    let setgroups_path = format!("/proc/{}/setgroups", pid);
    let _ = fs::write(&setgroups_path, "deny\n");

    // Write GID mapping: container GID 0 → host GID
    let gid_map = format!("0 {} 1\n", gid);
    let gid_map_path = format!("/proc/{}/gid_map", pid);
    fs::write(&gid_map_path, &gid_map).map_err(|e| {
        ContainerError::Syscall {
            call: "write gid_map",
            code: 0,
            detail: format!("{}: {}", gid_map_path, e),
        }
    })?;

    Ok(())
}

/// Set the hostname inside the UTS namespace.
pub fn set_container_hostname(hostname: &str) -> Result<()> {
    do_sethostname(hostname)
}

/// Enter existing namespaces of a running container (for `exec` command).
pub fn enter_namespaces(pid: u32) -> Result<()> {
    let ns_types = [
        ("pid", CLONE_NEWPID),
        ("mnt", CLONE_NEWNS),
        ("net", CLONE_NEWNET),
        ("uts", CLONE_NEWUTS),
        ("ipc", CLONE_NEWIPC),
    ];

    for (ns_name, ns_flag) in &ns_types {
        let ns_path = format!("/proc/{}/ns/{}", pid, ns_name);
        let ns_path_obj = Path::new(&ns_path);
        if ns_path_obj.exists() {
            let fd = std::fs::File::open(&ns_path).map_err(|e| {
                ContainerError::Syscall {
                    call: "open namespace",
                    code: 0,
                    detail: format!("failed to open {}: {}", ns_path, e),
                }
            })?;
            use std::os::unix::io::AsRawFd;
            unsafe {
                setns(fd.as_raw_fd(), *ns_flag as i32)?;
            }
        }
    }

    Ok(())
}
