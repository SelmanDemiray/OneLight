///! Seccomp BPF system call filtering — hand-built BPF instructions.

use crate::error::{syscall_error, Result};
use super::syscall::*;

/// BPF instruction (8 bytes each).
#[repr(C)]
struct BpfInsn {
    code: u16,
    jt: u8,
    jf: u8,
    k: u32,
}

/// BPF program header.
#[repr(C)]
struct BpfProg {
    len: u16,
    filter: *const BpfInsn,
}

/// Seccomp data offsets (architecture-dependent).
const SECCOMP_DATA_NR_OFFSET: u32 = 0;        // syscall number
const SECCOMP_DATA_ARCH_OFFSET: u32 = 4;       // audit architecture

/// Syscall numbers to BLOCK inside the container (x86_64).
/// These are dangerous operations that could escape isolation.
const BLOCKED_SYSCALLS: &[u32] = &[
    169,  // reboot
    175,  // init_module
    176,  // delete_module
    246,  // kexec_load
    304,  // open_by_handle_at (container escape vector)
    310,  // process_vm_readv
    311,  // process_vm_writev
    139,  // sysfs
    156,  // _sysctl
    171,  // setdomainname
    245,  // mq_getsetattr — not dangerous but in default Docker policy
    320,  // kexec_file_load
    161,  // chroot (prevent nested chroot escapes)
    167,  // swapon
    168,  // swapoff
    103,  // syslog
];

/// Build and load a seccomp BPF filter.
/// Default policy: ALLOW all, but BLOCK the dangerous syscalls listed above.
pub fn apply_seccomp_filter() -> Result<()> {
    let mut insns: Vec<BpfInsn> = Vec::new();

    // ── Instruction 0: Validate architecture ──
    // Load the audit architecture from seccomp_data
    insns.push(BpfInsn {
        code: BPF_LD | BPF_W | BPF_ABS,
        jt: 0,
        jf: 0,
        k: SECCOMP_DATA_ARCH_OFFSET,
    });

    // If arch != x86_64, kill the process
    insns.push(BpfInsn {
        code: BPF_JMP | BPF_JEQ | BPF_K,
        jt: 1, // skip next instruction (continue)
        jf: 0, // fall through to kill
        k: AUDIT_ARCH_X86_64,
    });

    // Kill: wrong architecture
    insns.push(BpfInsn {
        code: BPF_RET | BPF_K,
        jt: 0,
        jf: 0,
        k: SECCOMP_RET_KILL,
    });

    // ── Instruction 3: Load syscall number ──
    insns.push(BpfInsn {
        code: BPF_LD | BPF_W | BPF_ABS,
        jt: 0,
        jf: 0,
        k: SECCOMP_DATA_NR_OFFSET,
    });

    // ── Instructions 4+: Check each blocked syscall ──
    let num_blocked = BLOCKED_SYSCALLS.len();
    for (i, &nr) in BLOCKED_SYSCALLS.iter().enumerate() {
        // If syscall matches, jump to the kill return
        // jt = jump forward to the KILL instruction
        // jf = continue checking (next instruction)
        let jump_to_kill = (num_blocked - i) as u8;
        insns.push(BpfInsn {
            code: BPF_JMP | BPF_JEQ | BPF_K,
            jt: jump_to_kill,
            jf: 0,
            k: nr,
        });
    }

    // ── Default: ALLOW ──
    insns.push(BpfInsn {
        code: BPF_RET | BPF_K,
        jt: 0,
        jf: 0,
        k: SECCOMP_RET_ALLOW,
    });

    // ── Kill return (reached by matched blocked syscalls) ──
    insns.push(BpfInsn {
        code: BPF_RET | BPF_K,
        jt: 0,
        jf: 0,
        k: SECCOMP_RET_ERRNO | 1, // Return EPERM
    });

    // ── Load the filter ──
    let prog = BpfProg {
        len: insns.len() as u16,
        filter: insns.as_ptr(),
    };

    // First, set no_new_privs (required for non-root seccomp)
    let ret = unsafe { prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
    if ret != 0 {
        return Err(syscall_error("prctl(PR_SET_NO_NEW_PRIVS)"));
    }

    // Apply the seccomp filter
    let ret = unsafe {
        prctl(
            PR_SET_SECCOMP,
            SECCOMP_MODE_FILTER,
            &prog as *const BpfProg as u64,
            0,
            0,
        )
    };
    if ret != 0 {
        return Err(syscall_error("prctl(PR_SET_SECCOMP)"));
    }

    Ok(())
}
