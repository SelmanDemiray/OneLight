///! Overlay filesystem — layer-based image storage with copy-on-write semantics.
///! On Linux: uses kernel overlayfs via mount() syscall.
///! On Windows: userspace union-copy strategy with hardlinks for efficiency.
///! Zero third-party dependencies.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{ContainerError, Result};

// ─── Overlay Types ──────────────────────────────────────────────────────────

/// An overlay mount combining multiple read-only layers with a writable upper layer.
pub struct OverlayMount {
    /// Read-only lower layers (bottom to top)
    pub lower_dirs: Vec<PathBuf>,
    /// Writable upper layer
    pub upper_dir: PathBuf,
    /// Work directory (required by overlayfs)
    pub work_dir: PathBuf,
    /// Merged view directory
    pub merged_dir: PathBuf,
}

impl OverlayMount {
    /// Create a new overlay mount configuration.
    pub fn new(
        lower_dirs: Vec<PathBuf>,
        upper_dir: PathBuf,
        work_dir: PathBuf,
        merged_dir: PathBuf,
    ) -> Self {
        OverlayMount { lower_dirs, upper_dir, work_dir, merged_dir }
    }

    /// Set up the overlay filesystem.
    pub fn mount(&self) -> Result<()> {
        // Create all required directories
        fs::create_dir_all(&self.upper_dir)
            .map_err(|e| ContainerError::Filesystem(format!("create upper dir: {}", e)))?;
        fs::create_dir_all(&self.work_dir)
            .map_err(|e| ContainerError::Filesystem(format!("create work dir: {}", e)))?;
        fs::create_dir_all(&self.merged_dir)
            .map_err(|e| ContainerError::Filesystem(format!("create merged dir: {}", e)))?;

        #[cfg(target_os = "linux")]
        {
            self.mount_linux()?;
        }

        #[cfg(target_os = "windows")]
        {
            self.mount_windows()?;
        }

        Ok(())
    }

    /// Unmount the overlay filesystem.
    pub fn unmount(&self) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            self.unmount_linux()?;
        }

        // On Windows, nothing to unmount — it's a userspace copy
        Ok(())
    }

    // ─── Linux: Kernel overlayfs ────────────────────────────────────────

    #[cfg(target_os = "linux")]
    fn mount_linux(&self) -> Result<()> {
        use crate::platform::linux::syscall;

        // Build the overlayfs mount options
        // lowerdir=dir1:dir2:dir3 (bottom to top, but overlayfs reads left=top)
        let lower_str: String = self.lower_dirs.iter()
            .rev() // overlayfs wants top layer first
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join(":");

        let options = format!(
            "lowerdir={},upperdir={},workdir={}",
            lower_str,
            self.upper_dir.display(),
            self.work_dir.display()
        );

        // mount("overlay", merged, "overlay", 0, options)
        let source = "overlay\0";
        let target = format!("{}\0", self.merged_dir.display());
        let fstype = "overlay\0";
        let options_c = format!("{}\0", options);

        unsafe {
            let ret = syscall::mount(
                source.as_ptr(),
                target.as_ptr(),
                fstype.as_ptr(),
                0,
                options_c.as_ptr() as *const u8,
            );
            if ret < 0 {
                return Err(ContainerError::Filesystem(format!(
                    "mount overlayfs failed (errno: {})", -ret
                )));
            }
        }

        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn unmount_linux(&self) -> Result<()> {
        use crate::platform::linux::syscall;

        let target = format!("{}\0", self.merged_dir.display());
        unsafe {
            let ret = syscall::umount2(target.as_ptr(), 0);
            if ret < 0 {
                eprintln!("[warn] unmount overlayfs failed (errno: {})", -ret);
            }
        }
        Ok(())
    }

    // ─── Windows: Userspace union-copy ──────────────────────────────────

    #[cfg(target_os = "windows")]
    fn mount_windows(&self) -> Result<()> {
        // On Windows, we create a merged view by copying layers bottom-up
        // into the merged directory. The upper layer is overlaid on top.

        // Copy lower layers (bottom to top)
        for lower in &self.lower_dirs {
            if lower.exists() {
                copy_dir_recursive(lower, &self.merged_dir)?;
            }
        }

        // Copy upper layer on top (overwrites)
        if self.upper_dir.exists() {
            copy_dir_recursive(&self.upper_dir, &self.merged_dir)?;
        }

        Ok(())
    }
}

// ─── Layer Management ───────────────────────────────────────────────────────

/// Manages image layers and their storage.
pub struct LayerStore {
    base_dir: PathBuf,
}

impl LayerStore {
    pub fn new(base_dir: PathBuf) -> Self {
        LayerStore { base_dir }
    }

    /// Get or create a layer directory by digest.
    pub fn layer_dir(&self, digest: &str) -> PathBuf {
        let safe_name = digest.replace(":", "_").replace("/", "_");
        self.base_dir.join("layers").join(safe_name)
    }

    /// Check if a layer is already cached.
    pub fn has_layer(&self, digest: &str) -> bool {
        self.layer_dir(digest).exists()
    }

    /// Create an overlay mount for a container using the given image layers.
    pub fn create_overlay(
        &self,
        container_name: &str,
        layer_digests: &[String],
    ) -> Result<OverlayMount> {
        let container_dir = self.base_dir.join("containers").join(container_name);
        let upper = container_dir.join("upper");
        let work = container_dir.join("work");
        let merged = container_dir.join("merged");

        let lower_dirs: Vec<PathBuf> = layer_digests.iter()
            .map(|d| self.layer_dir(d))
            .collect();

        let overlay = OverlayMount::new(lower_dirs, upper, work, merged);
        overlay.mount()?;

        Ok(overlay)
    }

    /// Clean up a container's overlay.
    pub fn remove_overlay(&self, container_name: &str) -> Result<()> {
        let container_dir = self.base_dir.join("containers").join(container_name);
        let merged = container_dir.join("merged");

        // Unmount if mounted
        #[cfg(target_os = "linux")]
        {
            let target = format!("{}\0", merged.display());
            unsafe {
                crate::platform::linux::syscall::umount2(target.as_ptr(), 0);
            }
        }

        // Remove container directory
        if container_dir.exists() {
            let _ = fs::remove_dir_all(&container_dir);
        }

        Ok(())
    }
}

// ─── Utility Functions ──────────────────────────────────────────────────────

/// Recursively copy a directory, merging into the destination.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    if !src.is_dir() {
        return Ok(());
    }

    fs::create_dir_all(dst)
        .map_err(|e| ContainerError::Filesystem(format!("create dir {}: {}", dst.display(), e)))?;

    for entry in fs::read_dir(src).map_err(|e| ContainerError::Filesystem(format!("read dir: {}", e)))? {
        let entry = entry.map_err(|e| ContainerError::Filesystem(format!("dir entry: {}", e)))?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if src_path.is_file() {
            // Try hardlink first (efficient), fall back to copy
            if fs::hard_link(&src_path, &dst_path).is_err() {
                let _ = fs::remove_file(&dst_path); // remove existing if any
                fs::copy(&src_path, &dst_path)
                    .map_err(|e| ContainerError::Filesystem(format!(
                        "copy {} -> {}: {}", src_path.display(), dst_path.display(), e
                    )))?;
            }
        }
    }

    Ok(())
}

/// Prepare a rootfs from pulled image layers for container use.
pub fn prepare_rootfs_from_image(image_rootfs: &Path, container_rootfs: &Path) -> Result<()> {
    if container_rootfs.exists() {
        // Already prepared
        return Ok(());
    }

    fs::create_dir_all(container_rootfs)
        .map_err(|e| ContainerError::Filesystem(format!("create rootfs: {}", e)))?;

    copy_dir_recursive(image_rootfs, container_rootfs)?;

    println!("    Rootfs prepared at {}", container_rootfs.display());
    Ok(())
}
