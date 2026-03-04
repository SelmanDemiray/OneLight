///! Tar-based container image format — custom reader/writer, zero dependencies.
///! Implements the POSIX.1 UStar tar format from scratch.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::config;
use crate::error::{ContainerError, Result};

const TAR_BLOCK_SIZE: usize = 512;
const TAR_MAGIC: &[u8; 6] = b"ustar\0";
const TAR_VERSION: &[u8; 2] = b"00";

/// Tar header (512 bytes).
#[repr(C)]
struct TarHeader {
    name: [u8; 100],       // File name
    mode: [u8; 8],         // File mode (octal)
    uid: [u8; 8],          // Owner user ID
    gid: [u8; 8],          // Owner group ID
    size: [u8; 12],        // File size in bytes (octal)
    mtime: [u8; 12],       // Modification time (octal)
    checksum: [u8; 8],     // Header checksum
    typeflag: u8,           // File type
    linkname: [u8; 100],   // Link name
    magic: [u8; 6],         // "ustar\0"
    version: [u8; 2],       // "00"
    uname: [u8; 32],       // Owner user name
    gname: [u8; 32],       // Owner group name
    devmajor: [u8; 8],     // Device major number
    devminor: [u8; 8],     // Device minor number
    prefix: [u8; 155],     // Filename prefix
    _padding: [u8; 12],    // Padding to 512 bytes
}

// Type flags
const REGTYPE: u8 = b'0';   // Regular file
const DIRTYPE: u8 = b'5';   // Directory
const SYMTYPE: u8 = b'2';   // Symbolic link

/// Create a tar archive from a directory.
pub fn create_image(src_dir: &Path, output: &Path) -> Result<()> {
    let mut file = fs::File::create(output)?;
    tar_directory(&mut file, src_dir, Path::new(""))?;
    // Write two empty blocks (end of archive marker)
    file.write_all(&[0u8; TAR_BLOCK_SIZE * 2])?;
    println!("[+] Image created: {}", output.display());
    Ok(())
}

/// Recursively tar a directory.
fn tar_directory(writer: &mut dyn Write, base: &Path, prefix: &Path) -> Result<()> {
    let entries = fs::read_dir(base)?;
    let mut sorted_entries: Vec<_> = entries.filter_map(|e| e.ok()).collect();
    sorted_entries.sort_by_key(|e| e.file_name());

    for entry in sorted_entries {
        let path = entry.path();
        let rel_path = prefix.join(entry.file_name());
        let rel_str = rel_path.to_string_lossy().replace('\\', "/");

        let metadata = entry.metadata()?;

        if metadata.is_dir() {
            // Write directory header
            let mut header = new_header();
            set_name(&mut header, &format!("{}/", rel_str));
            set_mode(&mut header, 0o755);
            set_size(&mut header, 0);
            header.typeflag = DIRTYPE;
            finalize_header(&mut header);
            write_header(writer, &header)?;

            // Recurse
            tar_directory(writer, &path, &rel_path)?;
        } else if metadata.is_file() {
            let size = metadata.len();

            // Write file header
            let mut header = new_header();
            set_name(&mut header, &rel_str);
            set_mode(&mut header, 0o644);
            set_size(&mut header, size);
            header.typeflag = REGTYPE;
            finalize_header(&mut header);
            write_header(writer, &header)?;

            // Write file contents
            let mut f = fs::File::open(&path)?;
            let mut remaining = size as usize;
            let mut buf = [0u8; 8192];
            while remaining > 0 {
                let to_read = remaining.min(buf.len());
                let n = f.read(&mut buf[..to_read])?;
                if n == 0 { break; }
                writer.write_all(&buf[..n])?;
                remaining -= n;
            }

            // Pad to block boundary
            let padding = (TAR_BLOCK_SIZE - (size as usize % TAR_BLOCK_SIZE)) % TAR_BLOCK_SIZE;
            if padding > 0 {
                writer.write_all(&vec![0u8; padding])?;
            }
        }
        #[cfg(target_os = "linux")]
        {
            if metadata.file_type().is_symlink() {
                if let Ok(target) = fs::read_link(&path) {
                    let mut header = new_header();
                    set_name(&mut header, &rel_str);
                    set_mode(&mut header, 0o777);
                    set_size(&mut header, 0);
                    header.typeflag = SYMTYPE;
                    let target_str = target.to_string_lossy();
                    let target_bytes = target_str.as_bytes();
                    let len = target_bytes.len().min(100);
                    header.linkname[..len].copy_from_slice(&target_bytes[..len]);
                    finalize_header(&mut header);
                    write_header(writer, &header)?;
                }
            }
        }
    }
    Ok(())
}

/// Extract a tar archive into a directory.
pub fn extract_image(archive: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    let mut file = fs::File::open(archive)?;
    let mut header_buf = [0u8; TAR_BLOCK_SIZE];

    loop {
        let n = file.read(&mut header_buf)?;
        if n < TAR_BLOCK_SIZE { break; }

        // Check for end-of-archive (all zeros)
        if header_buf.iter().all(|&b| b == 0) { break; }

        // Parse header
        let name = parse_name(&header_buf);
        let size = parse_octal(&header_buf[124..136]);
        let typeflag = header_buf[156];

        if name.is_empty() { break; }

        let full_path = dest.join(&name);

        match typeflag {
            DIRTYPE | b'\0' if name.ends_with('/') => {
                fs::create_dir_all(&full_path)?;
            }
            REGTYPE | b'\0' => {
                if let Some(parent) = full_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut out = fs::File::create(&full_path)?;
                let mut remaining = size as usize;
                let mut buf = [0u8; 8192];
                while remaining > 0 {
                    let to_read = remaining.min(buf.len());
                    let n = file.read(&mut buf[..to_read])?;
                    if n == 0 { break; }
                    out.write_all(&buf[..n])?;
                    remaining -= n;
                }
                // Skip padding
                let padding = (TAR_BLOCK_SIZE - (size as usize % TAR_BLOCK_SIZE)) % TAR_BLOCK_SIZE;
                if padding > 0 && padding < TAR_BLOCK_SIZE {
                    let mut skip = vec![0u8; padding];
                    file.read_exact(&mut skip)?;
                }

                // Set executable permission on Linux
                #[cfg(target_os = "linux")]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mode = parse_octal(&header_buf[100..108]);
                    let _ = fs::set_permissions(&full_path, fs::Permissions::from_mode(mode as u32));
                }
            }
            SYMTYPE => {
                #[cfg(target_os = "linux")]
                {
                    let link_target = parse_linkname(&header_buf);
                    if let Some(parent) = full_path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    let _ = std::os::unix::fs::symlink(&link_target, &full_path);
                }
            }
            _ => {
                // Skip unknown types
                if size > 0 {
                    let blocks = (size as usize + TAR_BLOCK_SIZE - 1) / TAR_BLOCK_SIZE;
                    let mut skip = vec![0u8; blocks * TAR_BLOCK_SIZE];
                    let _ = file.read(&mut skip);
                }
            }
        }
    }

    println!("[+] Image extracted to: {}", dest.display());
    Ok(())
}

/// List available images.
pub fn list_images() -> Result<()> {
    let dir = config::images_dir();
    if !dir.exists() {
        println!("No images found.");
        return Ok(());
    }

    println!("{:<30} {:<15} {}", "NAME", "SIZE", "PATH");
    println!("{}", "-".repeat(70));

    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().map_or(false, |e| e == "tar") {
            let name = path.file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let size = entry.metadata()
                .map(|m| format_size(m.len()))
                .unwrap_or_else(|_| "?".into());
            println!("{:<30} {:<15} {}", name, size, path.display());
        }
    }
    Ok(())
}

// ─── Helpers ────────────────────────────────────────────────────────────────

fn new_header() -> TarHeader {
    let mut h: TarHeader = unsafe { std::mem::zeroed() };
    h.magic.copy_from_slice(TAR_MAGIC);
    h.version.copy_from_slice(TAR_VERSION);
    h.uname[..4].copy_from_slice(b"root");
    h.gname[..4].copy_from_slice(b"root");
    h
}

fn set_name(h: &mut TarHeader, name: &str) {
    let bytes = name.as_bytes();
    if bytes.len() <= 100 {
        h.name[..bytes.len()].copy_from_slice(bytes);
    } else if bytes.len() <= 255 {
        // UStar: split into prefix (max 155) + name (max 100) at a '/' boundary.
        // Find the best '/' split point where name part fits in 100 and prefix in 155.
        let mut split_at = None;
        for i in (0..bytes.len()).rev() {
            if bytes[i] == b'/' {
                let name_part_len = bytes.len() - i - 1; // after the '/'
                let prefix_part_len = i;                  // before the '/'
                if name_part_len <= 100 && prefix_part_len <= 155 {
                    split_at = Some(i);
                    break;
                }
            }
        }
        if let Some(pos) = split_at {
            h.prefix[..pos].copy_from_slice(&bytes[..pos]);
            let name_part = &bytes[pos + 1..]; // skip the '/'
            h.name[..name_part.len()].copy_from_slice(name_part);
        } else {
            // No good split point; truncate into name field
            let len = bytes.len().min(100);
            h.name[..len].copy_from_slice(&bytes[..len]);
        }
    } else {
        // Name too long even for prefix+name; truncate
        h.name[..100].copy_from_slice(&bytes[..100]);
    }
}

fn set_mode(h: &mut TarHeader, mode: u32) {
    write_octal(&mut h.mode, mode as u64, 7);
}

fn set_size(h: &mut TarHeader, size: u64) {
    write_octal(&mut h.size, size, 11);
}

fn write_octal(buf: &mut [u8], val: u64, width: usize) {
    let s = format!("{:0>width$o}", val, width = width);
    let bytes = s.as_bytes();
    let len = bytes.len().min(buf.len() - 1);
    buf[..len].copy_from_slice(&bytes[bytes.len() - len..]);
}

fn finalize_header(h: &mut TarHeader) {
    // Set checksum field to spaces for calculation
    h.checksum = [b' '; 8];
    let bytes = unsafe {
        std::slice::from_raw_parts(h as *const TarHeader as *const u8, TAR_BLOCK_SIZE)
    };
    let sum: u32 = bytes.iter().map(|&b| b as u32).sum();
    write_octal(&mut h.checksum, sum as u64, 6);
    h.checksum[7] = 0;
}

fn write_header(writer: &mut dyn Write, h: &TarHeader) -> Result<()> {
    let bytes = unsafe {
        std::slice::from_raw_parts(h as *const TarHeader as *const u8, TAR_BLOCK_SIZE)
    };
    writer.write_all(bytes)?;
    Ok(())
}

fn parse_name(header: &[u8]) -> String {
    let prefix = extract_string(&header[345..500]);
    let name = extract_string(&header[0..100]);
    if prefix.is_empty() { name } else { format!("{}/{}", prefix, name) }
}

fn parse_linkname(header: &[u8]) -> String {
    extract_string(&header[157..257])
}

fn extract_string(buf: &[u8]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).trim().to_string()
}

fn parse_octal(buf: &[u8]) -> u64 {
    let s = extract_string(buf);
    u64::from_str_radix(s.trim(), 8).unwrap_or(0)
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 { format!("{} B", bytes) }
    else if bytes < 1024 * 1024 { format!("{:.1} KB", bytes as f64 / 1024.0) }
    else if bytes < 1024 * 1024 * 1024 { format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0)) }
    else { format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0)) }
}
