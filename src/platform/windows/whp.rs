//! Windows Hypervisor Platform (WHP) FFI bindings — hand-declared, zero dependencies.
//! These bindings talk directly to WinHvPlatform.dll to create lightweight VMs
//! that boot a Linux kernel for running Linux containers on Windows.
//!
//! This is the core of what makes HolyContainer revolutionary:
//! no WSL2, no Hyper-V manager, just raw hardware virtualization.

use std::ffi::c_void;

// ─── Basic Types ────────────────────────────────────────────────────────────

pub type HRESULT = i32;
pub type BOOL = i32;
pub type UINT8 = u8;
pub type UINT16 = u16;
pub type UINT32 = u32;
pub type UINT64 = u64;
pub type VOID = c_void;

pub type WHV_PARTITION_HANDLE = *mut c_void;

// ─── Result Codes ───────────────────────────────────────────────────────────

pub const S_OK: HRESULT = 0;
pub const WHV_E_INSUFFICIENT_BUFFER: HRESULT = -2143878399i32; // 0x80370301
pub const WHV_E_UNKNOWN_CAPABILITY: HRESULT = -2143878400i32;

// ─── Capability Types ───────────────────────────────────────────────────────

#[repr(C)]
#[derive(Copy, Clone)]
pub enum WHV_CAPABILITY_CODE {
    HypervisorPresent = 0x00000000,
    Features = 0x00000001,
    ExtendedVmExits = 0x00000002,
    ProcessorVendor = 0x00001000,
    ProcessorFeatures = 0x00001001,
    ProcessorClFlushSize = 0x00001002,
    ProcessorXsaveFeatures = 0x00001003,
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct WHV_CAPABILITY_FEATURES {
    pub flags: UINT64,
}

#[repr(C)]
pub union WHV_CAPABILITY {
    pub HypervisorPresent: BOOL,
    pub Features: WHV_CAPABILITY_FEATURES,
    pub ProcessorVendor: UINT32,
    pub Reserved: [UINT8; 256],
}

// ─── Partition Property Types ───────────────────────────────────────────────

#[repr(C)]
#[derive(Copy, Clone)]
pub enum WHV_PARTITION_PROPERTY_CODE {
    ProcessorCount = 0x00001fff,
    ProcessorVendor = 0x00001000,
    ExtendedVmExits = 0x00000002,
    ProcessorFeatures = 0x00001001,
}

#[repr(C)]
pub union WHV_PARTITION_PROPERTY {
    pub ProcessorCount: UINT32,
    pub ExtendedVmExits: UINT64,
    pub Reserved: [UINT8; 256],
}

// ─── Memory Mapping ─────────────────────────────────────────────────────────

pub type WHV_GUEST_PHYSICAL_ADDRESS = UINT64;
pub type WHV_GUEST_VIRTUAL_ADDRESS = UINT64;

#[repr(C)]
#[derive(Copy, Clone)]
pub enum WHV_MAP_GPA_RANGE_FLAGS {
    None = 0x00000000,
    Read = 0x00000001,
    Write = 0x00000002,
    Execute = 0x00000004,
    TrackDirtyPages = 0x00000008,
}

// Read + Write + Execute
pub const WHV_MAP_GPA_ALL: u32 = 0x07;

// ─── Register Types ─────────────────────────────────────────────────────────

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub enum WHV_REGISTER_NAME {
    // General purpose
    Rax = 0x00000000,
    Rcx = 0x00000001,
    Rdx = 0x00000002,
    Rbx = 0x00000003,
    Rsp = 0x00000004,
    Rbp = 0x00000005,
    Rsi = 0x00000006,
    Rdi = 0x00000007,
    R8 = 0x00000008,
    R9 = 0x00000009,
    R10 = 0x0000000A,
    R11 = 0x0000000B,
    R12 = 0x0000000C,
    R13 = 0x0000000D,
    R14 = 0x0000000E,
    R15 = 0x0000000F,
    Rip = 0x00000010,
    Rflags = 0x00000011,

    // Segment registers
    Es = 0x00000012,
    Cs = 0x00000013,
    Ss = 0x00000014,
    Ds = 0x00000015,
    Fs = 0x00000016,
    Gs = 0x00000017,
    Ldtr = 0x00000018,
    Tr = 0x00000019,

    // Table registers
    Idtr = 0x0000001A,
    Gdtr = 0x0000001B,

    // Control registers
    Cr0 = 0x00000020,
    Cr2 = 0x00000021,
    Cr3 = 0x00000022,
    Cr4 = 0x00000023,
    Cr8 = 0x00000024,

    // Extended control
    Efer = 0x00000030,

    // MSRs  
    Tsc = 0x00000040,
    KernelGsBase = 0x00000041,
    ApicBase = 0x00000042,
    Pat = 0x00000043,
    SysenterCs = 0x00000044,
    SysenterEip = 0x00000045,
    SysenterEsp = 0x00000046,
    Star = 0x00000047,
    Lstar = 0x00000048,
    Cstar = 0x00000049,
    Sfmask = 0x0000004A,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub union WHV_REGISTER_VALUE {
    pub Reg128: WHV_UINT128,
    pub Reg64: UINT64,
    pub Reg32: UINT32,
    pub Reg16: UINT16,
    pub Reg8: UINT8,
    pub Segment: WHV_X64_SEGMENT_REGISTER,
    pub Table: WHV_X64_TABLE_REGISTER,
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct WHV_UINT128 {
    pub Low64: UINT64,
    pub High64: UINT64,
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct WHV_X64_SEGMENT_REGISTER {
    pub Base: UINT64,
    pub Limit: UINT32,
    pub Selector: UINT16,
    pub Attributes: UINT16,
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct WHV_X64_TABLE_REGISTER {
    pub Pad: [UINT16; 3],
    pub Limit: UINT16,
    pub Base: UINT64,
}

// ─── VM Exit Types ──────────────────────────────────────────────────────────

#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum WHV_RUN_VP_EXIT_REASON {
    None = 0x00000000,
    MemoryAccess = 0x00000001,
    X64IoPortAccess = 0x00000002,
    UnrecoverableException = 0x00000004,
    InvalidVpRegisterValue = 0x00000005,
    UnsupportedFeature = 0x00000006,
    X64InterruptWindow = 0x00000007,
    X64Halt = 0x00000008,
    X64ApicEoi = 0x00000009,
    X64MsrAccess = 0x0000000C,
    X64Cpuid = 0x0000000D,
    Exception = 0x0000000E,
    Canceled = 0x00000021,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct WHV_RUN_VP_EXIT_CONTEXT {
    pub ExitReason: WHV_RUN_VP_EXIT_REASON,
    pub Reserved: UINT32,
    pub VpContext: WHV_VP_EXIT_CONTEXT,
    pub Anonymous: WHV_RUN_VP_EXIT_UNION,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct WHV_VP_EXIT_CONTEXT {
    pub ExecutionState: UINT64,
    pub InstructionLength_Cr8: UINT32,   // packed field
    pub Reserved: UINT32,
    pub Cs: WHV_X64_SEGMENT_REGISTER,
    pub Rip: UINT64,
    pub Rflags: UINT64,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub union WHV_RUN_VP_EXIT_UNION {
    pub MemoryAccess: WHV_MEMORY_ACCESS_CONTEXT,
    pub IoPortAccess: WHV_X64_IO_PORT_ACCESS_CONTEXT,
    pub MsrAccess: WHV_X64_MSR_ACCESS_CONTEXT,
    pub CpuidAccess: WHV_X64_CPUID_ACCESS_CONTEXT,
    pub Reserved: [UINT8; 256],
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct WHV_MEMORY_ACCESS_CONTEXT {
    pub InstructionByteCount: UINT8,
    pub Reserved: [UINT8; 3],
    pub InstructionBytes: [UINT8; 16],
    pub AccessInfo: UINT32,  // WHV_MEMORY_ACCESS_INFO as u32
    pub Gpa: WHV_GUEST_PHYSICAL_ADDRESS,
    pub Gva: WHV_GUEST_VIRTUAL_ADDRESS,
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct WHV_X64_IO_PORT_ACCESS_CONTEXT {
    pub InstructionByteCount: UINT8,
    pub Reserved: [UINT8; 3],
    pub InstructionBytes: [UINT8; 16],
    pub AccessInfo: UINT32,
    pub PortNumber: UINT16,
    pub Reserved2: [UINT16; 3],
    pub Rax: UINT64,
    pub Rcx: UINT64,
    pub Rsi: UINT64,
    pub Rdi: UINT64,
    pub Ds: WHV_X64_SEGMENT_REGISTER,
    pub Es: WHV_X64_SEGMENT_REGISTER,
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct WHV_X64_MSR_ACCESS_CONTEXT {
    pub AccessInfo: UINT32,
    pub MsrNumber: UINT32,
    pub Rax: UINT64,
    pub Rdx: UINT64,
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct WHV_X64_CPUID_ACCESS_CONTEXT {
    pub Rax: UINT64,
    pub Rcx: UINT64,
    pub Rdx: UINT64,
    pub Rbx: UINT64,
    pub DefaultResultRax: UINT64,
    pub DefaultResultRcx: UINT64,
    pub DefaultResultRdx: UINT64,
    pub DefaultResultRbx: UINT64,
}

// ─── WinHvPlatform.dll Function Declarations ────────────────────────────────

#[link(name = "WinHvPlatform")]
extern "system" {
    pub fn WHvGetCapability(
        CapabilityCode: WHV_CAPABILITY_CODE,
        CapabilityBuffer: *mut VOID,
        CapabilityBufferSizeInBytes: UINT32,
        WrittenSizeInBytes: *mut UINT32,
    ) -> HRESULT;

    pub fn WHvCreatePartition(
        Partition: *mut WHV_PARTITION_HANDLE,
    ) -> HRESULT;

    pub fn WHvSetupPartition(
        Partition: WHV_PARTITION_HANDLE,
    ) -> HRESULT;

    pub fn WHvDeletePartition(
        Partition: WHV_PARTITION_HANDLE,
    ) -> HRESULT;

    pub fn WHvSetPartitionProperty(
        Partition: WHV_PARTITION_HANDLE,
        PropertyCode: WHV_PARTITION_PROPERTY_CODE,
        PropertyBuffer: *const VOID,
        PropertyBufferSizeInBytes: UINT32,
    ) -> HRESULT;

    pub fn WHvMapGpaRange(
        Partition: WHV_PARTITION_HANDLE,
        SourceAddress: *const VOID,
        GuestAddress: WHV_GUEST_PHYSICAL_ADDRESS,
        SizeInBytes: UINT64,
        Flags: UINT32,
    ) -> HRESULT;

    pub fn WHvUnmapGpaRange(
        Partition: WHV_PARTITION_HANDLE,
        GuestAddress: WHV_GUEST_PHYSICAL_ADDRESS,
        SizeInBytes: UINT64,
    ) -> HRESULT;

    pub fn WHvCreateVirtualProcessor(
        Partition: WHV_PARTITION_HANDLE,
        VpIndex: UINT32,
        Flags: UINT32,
    ) -> HRESULT;

    pub fn WHvDeleteVirtualProcessor(
        Partition: WHV_PARTITION_HANDLE,
        VpIndex: UINT32,
    ) -> HRESULT;

    pub fn WHvRunVirtualProcessor(
        Partition: WHV_PARTITION_HANDLE,
        VpIndex: UINT32,
        ExitContext: *mut VOID,
        ExitContextSizeInBytes: UINT32,
    ) -> HRESULT;

    pub fn WHvCancelRunVirtualProcessor(
        Partition: WHV_PARTITION_HANDLE,
        VpIndex: UINT32,
        Flags: UINT32,
    ) -> HRESULT;

    pub fn WHvGetVirtualProcessorRegisters(
        Partition: WHV_PARTITION_HANDLE,
        VpIndex: UINT32,
        RegisterNames: *const WHV_REGISTER_NAME,
        RegisterCount: UINT32,
        RegisterValues: *mut WHV_REGISTER_VALUE,
    ) -> HRESULT;

    pub fn WHvSetVirtualProcessorRegisters(
        Partition: WHV_PARTITION_HANDLE,
        VpIndex: UINT32,
        RegisterNames: *const WHV_REGISTER_NAME,
        RegisterCount: UINT32,
        RegisterValues: *const WHV_REGISTER_VALUE,
    ) -> HRESULT;
}

// ─── Windows VirtualAlloc for guest memory ──────────────────────────────────

pub const MEM_COMMIT: u32 = 0x1000;
pub const MEM_RESERVE: u32 = 0x2000;
pub const MEM_RELEASE: u32 = 0x8000;
pub const PAGE_READWRITE: u32 = 0x04;
pub const PAGE_EXECUTE_READWRITE: u32 = 0x40;

#[link(name = "kernel32")]
extern "system" {
    pub fn VirtualAlloc(
        lpAddress: *mut c_void,
        dwSize: usize,
        flAllocationType: u32,
        flProtect: u32,
    ) -> *mut c_void;

    pub fn VirtualFree(
        lpAddress: *mut c_void,
        dwSize: usize,
        dwFreeType: u32,
    ) -> i32;
}

// ─── Helper Functions ───────────────────────────────────────────────────────

/// Check if the Windows Hypervisor Platform is available.
pub fn is_whp_available() -> bool {
    unsafe {
        let mut present: BOOL = 0;
        let mut written: UINT32 = 0;
        let hr = WHvGetCapability(
            WHV_CAPABILITY_CODE::HypervisorPresent,
            &mut present as *mut BOOL as *mut VOID,
            std::mem::size_of::<BOOL>() as UINT32,
            &mut written,
        );
        hr == S_OK && present != 0
    }
}

/// Create a register value from a u64.
pub fn reg_val64(val: u64) -> WHV_REGISTER_VALUE {
    let mut rv = WHV_REGISTER_VALUE { Reg64: 0 };
    rv.Reg64 = val;
    rv
}

/// Create a segment register value.
pub fn seg_val(base: u64, limit: u32, selector: u16, attributes: u16) -> WHV_REGISTER_VALUE {
    WHV_REGISTER_VALUE {
        Segment: WHV_X64_SEGMENT_REGISTER { Base: base, Limit: limit, Selector: selector, Attributes: attributes }
    }
}

/// Create a table register value.
pub fn table_val(base: u64, limit: u16) -> WHV_REGISTER_VALUE {
    WHV_REGISTER_VALUE {
        Table: WHV_X64_TABLE_REGISTER { Pad: [0; 3], Limit: limit, Base: base }
    }
}
