///! Linux capabilities management — drop dangerous caps inside the container.

use crate::error::{syscall_error, Result};
use super::syscall::*;

/// Capabilities to RETAIN inside the container (minimal safe set).
/// Everything else gets dropped.
const RETAINED_CAPS: &[u32] = &[
    CAP_CHOWN,
    CAP_DAC_OVERRIDE,
    CAP_FOWNER,
    CAP_FSETID,
    CAP_KILL,
    CAP_SETGID,
    CAP_SETUID,
    CAP_SETPCAP,
    CAP_NET_BIND_SERVICE,
    CAP_SYS_CHROOT,
    CAP_MKNOD,
    CAP_AUDIT_WRITE,
    CAP_SETFCAP,
];

/// Drop all dangerous capabilities from the bounding set.
/// This is called inside the container process after fork.
pub fn drop_capabilities() -> Result<()> {
    for cap in 0..=CAP_LAST_CAP {
        // Skip caps we want to keep
        if RETAINED_CAPS.contains(&cap) {
            continue;
        }

        let ret = unsafe { prctl(PR_CAPBSET_DROP, cap as u64, 0, 0, 0) };
        if ret != 0 {
            // Some capabilities may not exist on older kernels, ignore EINVAL
            let errno = unsafe { *crate::error::libc_errno_ptr() };
            if errno != 22 {
                // EINVAL
                return Err(syscall_error("prctl(PR_CAPBSET_DROP)"));
            }
        }
    }

    // Set no_new_privs to prevent privilege escalation via execve
    let ret = unsafe { prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
    if ret != 0 {
        return Err(syscall_error("prctl(PR_SET_NO_NEW_PRIVS)"));
    }

    Ok(())
}
