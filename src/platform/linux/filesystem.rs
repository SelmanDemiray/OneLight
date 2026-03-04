///! Linux container filesystem setup — mounts, devices, pivot_root, overlay.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{ContainerError, Result};
use super::syscall::*;

/// Set up the container's root filesystem with all necessary mounts.
pub fn setup_rootfs(rootfs: &Path) -> Result<()> {
    // Ensure rootfs exists
    if !rootfs.exists() {
        return Err(ContainerError::Filesystem(format!(
            "rootfs does not exist: {}",
            rootfs.display()
        )));
    }

    // Make the mount namespace private so mounts don't propagate to host
    do_mount(None, Path::new("/"), None, MS_REC | MS_PRIVATE, None)?;

    // Bind-mount rootfs onto itself (needed for pivot_root)
    do_mount(
        Some(&rootfs.to_string_lossy()),
        rootfs,
        None,
        MS_BIND | MS_REC,
        None,
    )?;

    // Create essential directories inside rootfs
    let dirs = ["proc", "sys", "dev", "dev/pts", "dev/shm", "tmp", "run", "etc", "root", "var"];
    for dir in &dirs {
        let full = rootfs.join(dir);
        fs::create_dir_all(&full).map_err(|e| {
            ContainerError::Filesystem(format!("failed to create {}: {}", full.display(), e))
        })?;
    }

    // Mount /proc
    let proc_dir = rootfs.join("proc");
    do_mount(
        Some("proc"),
        &proc_dir,
        Some("proc"),
        MS_NOSUID | MS_NOEXEC | MS_NODEV,
        None,
    )?;

    // Mount /sys (read-only)
    let sys_dir = rootfs.join("sys");
    do_mount(
        Some("sysfs"),
        &sys_dir,
        Some("sysfs"),
        MS_NOSUID | MS_NOEXEC | MS_NODEV | MS_RDONLY,
        None,
    )?;

    // Mount /dev as tmpfs
    let dev_dir = rootfs.join("dev");
    do_mount(
        Some("tmpfs"),
        &dev_dir,
        Some("tmpfs"),
        MS_NOSUID,
        Some("mode=755,size=65536k"),
    )?;

    // Create device nodes
    create_default_devices(rootfs)?;

    // Mount devpts for PTY support
    let devpts_dir = rootfs.join("dev/pts");
    fs::create_dir_all(&devpts_dir)?;
    do_mount(
        Some("devpts"),
        &devpts_dir,
        Some("devpts"),
        MS_NOSUID | MS_NOEXEC,
        Some("newinstance,ptmxmode=0666,mode=620"),
    )?;

    // Mount /dev/shm as tmpfs
    let shm_dir = rootfs.join("dev/shm");
    fs::create_dir_all(&shm_dir)?;
    do_mount(
        Some("shm"),
        &shm_dir,
        Some("tmpfs"),
        MS_NOSUID | MS_NODEV | MS_NOEXEC,
        Some("mode=1777,size=65536k"),
    )?;

    // Mount /tmp as tmpfs
    let tmp_dir = rootfs.join("tmp");
    do_mount(
        Some("tmpfs"),
        &tmp_dir,
        Some("tmpfs"),
        MS_NOSUID | MS_NODEV,
        Some("mode=1777"),
    )?;

    // Create /etc/resolv.conf for DNS
    let resolv = rootfs.join("etc/resolv.conf");
    let _ = fs::write(&resolv, "nameserver 8.8.8.8\nnameserver 8.8.4.4\n");

    // Create /etc/hostname
    let hostname_file = rootfs.join("etc/hostname");
    let _ = fs::write(&hostname_file, "holycontainer\n");

    Ok(())
}

/// Create essential device nodes in /dev.
fn create_default_devices(rootfs: &Path) -> Result<()> {
    let dev_dir = rootfs.join("dev");

    // Standard character devices: (path, major, minor, mode)
    let devices = [
        ("null", 1, 3, S_IFCHR | 0o666),
        ("zero", 1, 5, S_IFCHR | 0o666),
        ("full", 1, 7, S_IFCHR | 0o666),
        ("random", 1, 8, S_IFCHR | 0o666),
        ("urandom", 1, 9, S_IFCHR | 0o666),
        ("tty", 5, 0, S_IFCHR | 0o666),
    ];

    for (name, major, minor, mode) in &devices {
        let path = dev_dir.join(name);
        do_mknod(&path, *mode, makedev(*major, *minor))?;
    }

    // Create symlinks
    do_symlink("/proc/self/fd", &dev_dir.join("fd"))?;
    do_symlink("/proc/self/fd/0", &dev_dir.join("stdin"))?;
    do_symlink("/proc/self/fd/1", &dev_dir.join("stdout"))?;
    do_symlink("/proc/self/fd/2", &dev_dir.join("stderr"))?;
    do_symlink("/dev/pts/ptmx", &dev_dir.join("ptmx"))?;

    Ok(())
}

/// Perform pivot_root to switch to the new rootfs.
/// After this, the old root is hidden and unmounted.
pub fn do_pivot_root(rootfs: &Path) -> Result<()> {
    let rootfs_str = rootfs.to_string_lossy().to_string();
    let put_old = rootfs.join("oldroot");

    fs::create_dir_all(&put_old).map_err(|e| {
        ContainerError::Filesystem(format!("failed to create oldroot: {}", e))
    })?;

    let put_old_str = put_old.to_string_lossy().to_string();

    unsafe {
        pivot_root(&rootfs_str, &put_old_str)?;
    }

    // chdir to new root
    let root_c = std::ffi::CString::new("/").unwrap();
    unsafe {
        chdir(root_c.as_ptr() as *const u8);
    }

    // Unmount old root
    do_umount(Path::new("/oldroot"), MNT_DETACH)?;

    // Remove the oldroot directory
    let _ = fs::remove_dir("/oldroot");

    Ok(())
}

/// Set up overlay filesystem with lower (image) + upper (writable) layers.
pub fn setup_overlay(
    lower_dir: &Path,
    upper_dir: &Path,
    work_dir: &Path,
    merged_dir: &Path,
) -> Result<()> {
    fs::create_dir_all(upper_dir)?;
    fs::create_dir_all(work_dir)?;
    fs::create_dir_all(merged_dir)?;

    let options = format!(
        "lowerdir={},upperdir={},workdir={}",
        lower_dir.display(),
        upper_dir.display(),
        work_dir.display()
    );

    do_mount(
        Some("overlay"),
        merged_dir,
        Some("overlay"),
        0,
        Some(&options),
    )?;

    Ok(())
}

/// Create a minimal root filesystem by copying essential binaries from the host.
pub fn create_minimal_rootfs(path: &Path) -> Result<()> {
    println!("[*] Creating minimal rootfs at {}", path.display());

    // Create directory structure
    let dirs = [
        "bin", "sbin", "usr/bin", "usr/sbin", "usr/lib", "usr/lib64",
        "lib", "lib64", "etc", "dev", "proc", "sys", "tmp", "root",
        "var/log", "var/tmp", "run", "home",
    ];
    for dir in &dirs {
        fs::create_dir_all(path.join(dir))?;
    }

    // Copy essential binaries and their library dependencies
    let binaries = [
        "/bin/sh",
        "/bin/bash",
        "/bin/ls",
        "/bin/cat",
        "/bin/echo",
        "/bin/mkdir",
        "/bin/rm",
        "/bin/cp",
        "/bin/mv",
        "/bin/ps",
        "/bin/hostname",
        "/bin/env",
        "/usr/bin/id",
        "/usr/bin/whoami",
    ];

    for bin in &binaries {
        let src = Path::new(bin);
        if src.exists() {
            let dst = path.join(bin.trim_start_matches('/'));
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent)?;
            }
            if let Err(e) = fs::copy(src, &dst) {
                eprintln!("[!] Warning: failed to copy {}: {}", bin, e);
            } else {
                println!("  [+] Copied {}", bin);
                // Copy shared library dependencies
                copy_lib_deps(src, path);
            }
        }
    }

    // Create minimal /etc files
    fs::write(
        path.join("etc/passwd"),
        "root:x:0:0:root:/root:/bin/sh\nnobody:x:65534:65534:nobody:/:/usr/sbin/nologin\n",
    )?;
    fs::write(
        path.join("etc/group"),
        "root:x:0:\nnogroup:x:65534:\n",
    )?;
    fs::write(path.join("etc/hostname"), "holycontainer\n")?;
    fs::write(
        path.join("etc/resolv.conf"),
        "nameserver 8.8.8.8\n",
    )?;
    fs::write(path.join("etc/hosts"), "127.0.0.1 localhost\n")?;

    println!("[*] Minimal rootfs created.");
    println!("[*] NOTE: Run `ldd <binary>` to verify all library deps were copied.");

    Ok(())
}

/// Copy shared library dependencies of a binary into the rootfs.
fn copy_lib_deps(binary: &Path, rootfs: &Path) {
    // Read the ELF binary to find the PT_INTERP (dynamic linker) path
    // and then use it to resolve library paths.
    // For simplicity, we try common library paths.
    let common_libs = [
        "/lib/x86_64-linux-gnu",
        "/lib64",
        "/usr/lib/x86_64-linux-gnu",
        "/usr/lib64",
    ];

    for lib_dir in &common_libs {
        let src = Path::new(lib_dir);
        if src.exists() {
            let dst = rootfs.join(lib_dir.trim_start_matches('/'));
            if !dst.exists() {
                let _ = fs::create_dir_all(&dst);
            }
            // Copy ld-linux and libc at minimum
            if let Ok(entries) = fs::read_dir(src) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    if name_str.starts_with("ld-linux")
                        || name_str.starts_with("libc.")
                        || name_str.starts_with("libc-")
                        || name_str.starts_with("libdl")
                        || name_str.starts_with("libpthread")
                        || name_str.starts_with("librt")
                        || name_str.starts_with("libm.")
                        || name_str.starts_with("libm-")
                        || name_str.starts_with("libresolv")
                        || name_str.starts_with("libnss")
                        || name_str.starts_with("libtinfo")
                        || name_str.starts_with("libncurses")
                        || name_str.starts_with("libreadline")
                    {
                        let src_file = entry.path();
                        let dst_file = dst.join(&name);
                        if !dst_file.exists() {
                            match fs::copy(&src_file, &dst_file) {
                                Ok(_) => {}
                                Err(_) => {
                                    // Try to copy the symlink target
                                    if let Ok(target) = fs::read_link(&src_file) {
                                        let _ = std::os::unix::fs::symlink(&target, &dst_file);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Always ensure the dynamic linker is available
    let linker_paths = [
        "/lib64/ld-linux-x86-64.so.2",
        "/lib/x86_64-linux-gnu/ld-linux-x86-64.so.2",
    ];
    for linker in &linker_paths {
        let src = Path::new(linker);
        if src.exists() {
            let dst = rootfs.join(linker.trim_start_matches('/'));
            if let Some(parent) = dst.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if !dst.exists() {
                let _ = fs::copy(src, &dst);
            }
        }
    }
}
