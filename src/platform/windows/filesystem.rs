///! Windows filesystem isolation — sandboxed rootfs with directory junctions.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{ContainerError, Result};
use super::winapi::*;

/// Set up the container's root filesystem on Windows.
/// Creates a sandboxed directory structure with essential system binaries.
pub fn setup_rootfs(rootfs: &Path) -> Result<()> {
    if !rootfs.exists() {
        return Err(ContainerError::Filesystem(format!(
            "rootfs does not exist: {}", rootfs.display()
        )));
    }
    // Create essential directories
    let dirs = [
        "Windows\\System32", "Windows\\SysWOW64", "Users\\ContainerUser",
        "ProgramData", "tmp", "app", "etc",
    ];
    for dir in &dirs {
        fs::create_dir_all(rootfs.join(dir))?;
    }
    Ok(())
}

/// Bootstrap a minimal Windows rootfs by copying essential system binaries.
pub fn create_minimal_rootfs(path: &Path) -> Result<()> {
    println!("[*] Creating minimal Windows rootfs at {}", path.display());

    let dirs = [
        "Windows\\System32", "Windows\\SysWOW64", "Users\\ContainerUser",
        "ProgramData", "tmp", "app", "etc", "Windows\\System32\\config",
    ];
    for dir in &dirs {
        fs::create_dir_all(path.join(dir))?;
    }

    // Get system directory
    let sys_dir = get_system_directory();
    let sys_path = Path::new(&sys_dir);

    // Essential executables to copy
    let essentials = [
        "cmd.exe", "conhost.exe", "kernel32.dll", "ntdll.dll",
        "KernelBase.dll", "msvcrt.dll", "ucrtbase.dll",
        "user32.dll", "advapi32.dll", "shell32.dll",
        "ole32.dll", "rpcrt4.dll", "sechost.dll",
        "bcryptprimitives.dll", "msvcp_win.dll",
        "powershell.exe", "where.exe", "findstr.exe",
        "ping.exe", "ipconfig.exe", "tasklist.exe",
        "whoami.exe", "hostname.exe", "net.exe",
        "xcopy.exe", "robocopy.exe", "more.com",
        "sort.exe", "fc.exe", "comp.exe",
    ];

    let dest_sys = path.join("Windows\\System32");

    for file in &essentials {
        let src = sys_path.join(file);
        let dst = dest_sys.join(file);
        if src.exists() {
            match fs::copy(&src, &dst) {
                Ok(_) => println!("  [+] Copied {}", file),
                Err(e) => eprintln!("  [!] Warning: failed to copy {}: {}", file, e),
            }
        }
    }

    // Also copy PowerShell directory if available
    let ps_src = Path::new("C:\\Windows\\System32\\WindowsPowerShell");
    if ps_src.exists() {
        let ps_dst = path.join("Windows\\System32\\WindowsPowerShell");
        copy_dir_recursive(ps_src, &ps_dst);
        println!("  [+] Copied PowerShell directory");
    }

    // Create basic config files
    fs::write(
        path.join("etc\\hostname"),
        "holycontainer\r\n",
    )?;
    fs::write(
        path.join("etc\\hosts"),
        "127.0.0.1 localhost\r\n",
    )?;

    println!("[*] Minimal Windows rootfs created successfully.");
    println!("[*] Container binaries are in: {}\\Windows\\System32", path.display());

    Ok(())
}

/// Get the Windows System32 directory.
fn get_system_directory() -> String {
    let mut buf = [0u16; 260];
    let len = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), 260) };
    if len == 0 {
        return "C:\\Windows\\System32".to_string();
    }
    String::from_utf16_lossy(&buf[..len as usize])
}

/// Recursively copy a directory.
fn copy_dir_recursive(src: &Path, dst: &Path) {
    let _ = fs::create_dir_all(dst);
    if let Ok(entries) = fs::read_dir(src) {
        for entry in entries.flatten() {
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());
            if src_path.is_dir() {
                copy_dir_recursive(&src_path, &dst_path);
            } else {
                let _ = fs::copy(&src_path, &dst_path);
            }
        }
    }
}

/// Create a directory junction (like a symlink for directories).
pub fn create_junction(link: &Path, target: &Path) -> Result<()> {
    let link_wide = to_wide(&link.to_string_lossy());
    let target_wide = to_wide(&target.to_string_lossy());

    let ret = unsafe {
        CreateSymbolicLinkW(
            link_wide.as_ptr(),
            target_wide.as_ptr(),
            SYMBOLIC_LINK_FLAG_DIRECTORY | SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE,
        )
    };

    if ret == 0 {
        // Fall back to directory copy if symlinks aren't available
        eprintln!("[!] Symlink creation failed, using directory copy instead");
        if target.exists() {
            copy_dir_recursive(target, link);
        }
    }

    Ok(())
}
