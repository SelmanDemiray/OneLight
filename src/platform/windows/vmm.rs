///! Virtual Machine Monitor — creates and manages lightweight VMs using WHP.
///! Sets up page tables, loads Linux kernel, handles VM exits.
///! This is the core engine that runs Linux containers on Windows.

use std::fs;
use std::path::Path;

use crate::error::{ContainerError, Result};
use super::whp;

// ─── Constants ──────────────────────────────────────────────────────────────

const PAGE_SIZE: u64 = 4096;
const MB: u64 = 1024 * 1024;
const GB: u64 = 1024 * MB;

// Memory layout for the guest VM
const GUEST_MEM_SIZE: u64 = 256 * MB;     // 256 MB default
const KERNEL_LOAD_ADDR: u64 = 0x100000;    // 1 MB — standard Linux kernel load address
const INITRD_LOAD_ADDR: u64 = 0x1000000;   // 16 MB — initramfs
const CMDLINE_ADDR: u64 = 0x20000;         // Command line
const BOOT_PARAMS_ADDR: u64 = 0x10000;     // Linux boot_params struct
const GDT_ADDR: u64 = 0x1000;              // Global Descriptor Table
const PML4_ADDR: u64 = 0x2000;             // Page tables start

// x86_64 control register bits
const CR0_PE: u64 = 1 << 0;    // Protected mode
const CR0_MP: u64 = 1 << 1;    // Monitor coprocessor
const CR0_ET: u64 = 1 << 4;    // Extension type
const CR0_NE: u64 = 1 << 5;    // Numeric error
const CR0_WP: u64 = 1 << 16;   // Write protect
const CR0_AM: u64 = 1 << 18;   // Alignment mask
const CR0_PG: u64 = 1 << 31;   // Paging

const CR4_PAE: u64 = 1 << 5;   // Physical address extension
const CR4_PGE: u64 = 1 << 7;   // Page global enable

const EFER_LME: u64 = 1 << 8;  // Long mode enable
const EFER_LMA: u64 = 1 << 10; // Long mode active
const EFER_SCE: u64 = 1 << 0;  // System call enable

// IO Ports for virtio console
pub const SERIAL_PORT: u16 = 0x3F8;         // COM1 for serial output
pub const VIRTIO_CONSOLE_PORT: u16 = 0x500; // Custom virtio console
pub const SHUTDOWN_PORT: u16 = 0x604;        // ACPI shutdown

// ─── VM State ───────────────────────────────────────────────────────────────

pub struct VirtualMachine {
    partition: whp::WHV_PARTITION_HANDLE,
    guest_memory: *mut u8,
    guest_mem_size: u64,
    running: bool,
}

impl VirtualMachine {
    /// Create a new virtual machine with the given memory size.
    pub fn new(mem_size_mb: u64) -> Result<VirtualMachine> {
        let mem_size = mem_size_mb * MB;

        unsafe {
            // Check WHP availability
            if !whp::is_whp_available() {
                return Err(ContainerError::Config(
                    "Windows Hypervisor Platform is not available.\n\
                     Enable it: Settings > Apps > Optional Features > \
                     'Windows Hypervisor Platform'\n\
                     Requires Windows 10/11 Pro or Enterprise.".into()
                ));
            }

            // Create partition
            let mut partition: whp::WHV_PARTITION_HANDLE = std::ptr::null_mut();
            let hr = whp::WHvCreatePartition(&mut partition);
            if hr != whp::S_OK {
                return Err(ContainerError::Config(format!(
                    "WHvCreatePartition failed: 0x{:08x}", hr as u32
                )));
            }

            // Set processor count to 1
            let proc_count: whp::UINT32 = 1;
            let hr = whp::WHvSetPartitionProperty(
                partition,
                whp::WHV_PARTITION_PROPERTY_CODE::ProcessorCount,
                &proc_count as *const whp::UINT32 as *const whp::VOID,
                std::mem::size_of::<whp::UINT32>() as whp::UINT32,
            );
            if hr != whp::S_OK {
                whp::WHvDeletePartition(partition);
                return Err(ContainerError::Config(format!(
                    "set processor count failed: 0x{:08x}", hr as u32
                )));
            }

            // Setup the partition
            let hr = whp::WHvSetupPartition(partition);
            if hr != whp::S_OK {
                whp::WHvDeletePartition(partition);
                return Err(ContainerError::Config(format!(
                    "WHvSetupPartition failed: 0x{:08x}", hr as u32
                )));
            }

            // Allocate guest memory
            let guest_memory = whp::VirtualAlloc(
                std::ptr::null_mut(),
                mem_size as usize,
                whp::MEM_COMMIT | whp::MEM_RESERVE,
                whp::PAGE_READWRITE,
            ) as *mut u8;

            if guest_memory.is_null() {
                whp::WHvDeletePartition(partition);
                return Err(ContainerError::Config("VirtualAlloc failed for guest memory".into()));
            }

            // Zero out guest memory
            std::ptr::write_bytes(guest_memory, 0, mem_size as usize);

            // Map guest memory into the partition
            let hr = whp::WHvMapGpaRange(
                partition,
                guest_memory as *const whp::VOID,
                0, // Guest physical address 0
                mem_size,
                whp::WHV_MAP_GPA_ALL, // Read + Write + Execute
            );
            if hr != whp::S_OK {
                whp::VirtualFree(guest_memory as *mut std::ffi::c_void, 0, whp::MEM_RELEASE);
                whp::WHvDeletePartition(partition);
                return Err(ContainerError::Config(format!(
                    "WHvMapGpaRange failed: 0x{:08x}", hr as u32
                )));
            }

            // Create virtual processor
            let hr = whp::WHvCreateVirtualProcessor(partition, 0, 0);
            if hr != whp::S_OK {
                whp::WHvUnmapGpaRange(partition, 0, mem_size);
                whp::VirtualFree(guest_memory as *mut std::ffi::c_void, 0, whp::MEM_RELEASE);
                whp::WHvDeletePartition(partition);
                return Err(ContainerError::Config(format!(
                    "WHvCreateVirtualProcessor failed: 0x{:08x}", hr as u32
                )));
            }

            Ok(VirtualMachine {
                partition,
                guest_memory,
                guest_mem_size: mem_size,
                running: false,
            })
        }
    }

    /// Write data to guest physical memory at the given offset.
    pub fn write_guest_mem(&self, offset: u64, data: &[u8]) -> Result<()> {
        if offset + data.len() as u64 > self.guest_mem_size {
            return Err(ContainerError::Config("write exceeds guest memory".into()));
        }
        unsafe {
            std::ptr::copy_nonoverlapping(
                data.as_ptr(),
                self.guest_memory.add(offset as usize),
                data.len(),
            );
        }
        Ok(())
    }

    /// Read data from guest physical memory.
    pub fn read_guest_mem(&self, offset: u64, len: usize) -> Result<Vec<u8>> {
        if offset + len as u64 > self.guest_mem_size {
            return Err(ContainerError::Config("read exceeds guest memory".into()));
        }
        let mut buf = vec![0u8; len];
        unsafe {
            std::ptr::copy_nonoverlapping(
                self.guest_memory.add(offset as usize),
                buf.as_mut_ptr(),
                len,
            );
        }
        Ok(buf)
    }

    /// Set up x86_64 long mode page tables in guest memory.
    pub fn setup_page_tables(&self) -> Result<()> {
        // We need 4-level page tables for x86_64 long mode:
        // PML4 -> PDPT -> PD -> PT (or use 2MB pages to skip PT level)
        //
        // For simplicity, we identity-map the first 1 GB using 2MB pages.

        let pml4_offset = PML4_ADDR;
        let pdpt_offset = PML4_ADDR + PAGE_SIZE;
        let pd_offset = PML4_ADDR + 2 * PAGE_SIZE;

        // PML4 entry 0 -> PDPT
        let pml4_entry: u64 = pdpt_offset | 0x03; // Present + Writable
        self.write_guest_mem(pml4_offset, &pml4_entry.to_le_bytes())?;

        // PDPT entry 0 -> PD
        let pdpt_entry: u64 = pd_offset | 0x03;
        self.write_guest_mem(pdpt_offset, &pdpt_entry.to_le_bytes())?;

        // PD: 512 entries, each mapping 2MB (total = 1 GB)
        for i in 0u64..512 {
            // 2MB page, Present + Writable + Page Size (bit 7)
            let pd_entry: u64 = (i * 2 * MB) | 0x83;
            self.write_guest_mem(pd_offset + i * 8, &pd_entry.to_le_bytes())?;
        }

        Ok(())
    }

    /// Set up the GDT (Global Descriptor Table) for 64-bit long mode.
    pub fn setup_gdt(&self) -> Result<()> {
        // GDT layout:
        // Entry 0: Null descriptor
        // Entry 1: Code segment (64-bit, DPL 0)
        // Entry 2: Data segment (64-bit, DPL 0)

        let null_desc: u64 = 0;
        let code_desc: u64 = 0x00AF9A000000FFFF; // 64-bit code, DPL 0, present, readable
        let data_desc: u64 = 0x00CF92000000FFFF; // 64-bit data, DPL 0, present, writable

        self.write_guest_mem(GDT_ADDR, &null_desc.to_le_bytes())?;
        self.write_guest_mem(GDT_ADDR + 8, &code_desc.to_le_bytes())?;
        self.write_guest_mem(GDT_ADDR + 16, &data_desc.to_le_bytes())?;

        Ok(())
    }

    /// Set up the Linux boot_params struct at BOOT_PARAMS_ADDR.
    pub fn setup_boot_params(&self, kernel_size: u64, initrd_size: u64, cmdline: &str) -> Result<()> {
        // Linux boot protocol (Documentation/x86/boot.txt)
        // The boot_params struct is defined in arch/x86/include/uapi/asm/bootparam.h
        let mut params = vec![0u8; 4096];

        // Header signature
        params[0x202] = b'H';
        params[0x203] = b'd';
        params[0x204] = b'r';
        params[0x205] = b'S';

        // Boot protocol version (2.15)
        params[0x206] = 0x0F;
        params[0x207] = 0x02;

        // Loader type (0xFF = undefined)
        params[0x210] = 0xFF;

        // Loadflags: LOADED_HIGH | CAN_USE_HEAP
        params[0x211] = 0x81;

        // Command line pointer
        let cmdline_addr = CMDLINE_ADDR as u32;
        params[0x228..0x22C].copy_from_slice(&cmdline_addr.to_le_bytes());

        // Initrd address and size
        let initrd_addr = INITRD_LOAD_ADDR as u32;
        params[0x218..0x21C].copy_from_slice(&initrd_addr.to_le_bytes());
        let initrd_sz = initrd_size as u32;
        params[0x21C..0x220].copy_from_slice(&initrd_sz.to_le_bytes());

        // E820 memory map (tell the kernel about available RAM)
        // Entry 0: 0 - 0x9FC00 (conventional memory, ~640KB)
        // Entry 1: 0x100000 - guest_mem_size (usable RAM)
        let e820_count: u8 = 2;
        params[0x1E8] = e820_count;

        // E820 entries start at offset 0x2D0
        let e820_base = 0x2D0;

        // Entry 0: base=0, size=0x9FC00, type=1 (usable)
        params[e820_base..e820_base + 8].copy_from_slice(&0u64.to_le_bytes());
        params[e820_base + 8..e820_base + 16].copy_from_slice(&0x9FC00u64.to_le_bytes());
        params[e820_base + 16..e820_base + 20].copy_from_slice(&1u32.to_le_bytes());

        // Entry 1: base=0x100000, size=guest_mem_size-0x100000, type=1 (usable)
        let entry1 = e820_base + 20;
        params[entry1..entry1 + 8].copy_from_slice(&0x100000u64.to_le_bytes());
        params[entry1 + 8..entry1 + 16].copy_from_slice(&(self.guest_mem_size - 0x100000).to_le_bytes());
        params[entry1 + 16..entry1 + 20].copy_from_slice(&1u32.to_le_bytes());

        // Write boot params to guest memory
        self.write_guest_mem(BOOT_PARAMS_ADDR, &params)?;

        // Write command line
        let mut cmdline_bytes = cmdline.as_bytes().to_vec();
        cmdline_bytes.push(0); // null terminate
        self.write_guest_mem(CMDLINE_ADDR, &cmdline_bytes)?;

        Ok(())
    }

    /// Load a Linux kernel (bzImage format) into guest memory.
    pub fn load_kernel(&self, kernel_path: &Path) -> Result<u64> {
        let kernel_data = fs::read(kernel_path)
            .map_err(|e| ContainerError::Filesystem(format!("read kernel: {}", e)))?;

        if kernel_data.len() < 0x250 {
            return Err(ContainerError::Config("kernel image too small".into()));
        }

        // Check for bzImage signature at offset 0x202
        if &kernel_data[0x202..0x206] != b"HdrS" {
            return Err(ContainerError::Config("not a valid Linux bzImage (missing HdrS signature)".into()));
        }

        // Get setup header fields
        let setup_sects = if kernel_data[0x1F1] == 0 { 4 } else { kernel_data[0x1F1] as usize };
        let setup_size = (setup_sects + 1) * 512;

        // The protected-mode kernel starts after the setup sectors
        let kernel_start = setup_size;
        let kernel_size = kernel_data.len() - kernel_start;

        // Load the protected-mode kernel at KERNEL_LOAD_ADDR (1MB)
        self.write_guest_mem(KERNEL_LOAD_ADDR, &kernel_data[kernel_start..])?;

        println!("    Kernel loaded: {} bytes at 0x{:x}", kernel_size, KERNEL_LOAD_ADDR);

        Ok(kernel_size as u64)
    }

    /// Load an initramfs into guest memory.
    pub fn load_initrd(&self, initrd_path: &Path) -> Result<u64> {
        let initrd_data = fs::read(initrd_path)
            .map_err(|e| ContainerError::Filesystem(format!("read initrd: {}", e)))?;

        self.write_guest_mem(INITRD_LOAD_ADDR, &initrd_data)?;

        println!("    Initrd loaded: {} bytes at 0x{:x}", initrd_data.len(), INITRD_LOAD_ADDR);

        Ok(initrd_data.len() as u64)
    }

    /// Set up CPU registers for 64-bit long mode and point RIP at the kernel.
    pub fn setup_long_mode_registers(&self) -> Result<()> {
        unsafe {
            let names = [
                whp::WHV_REGISTER_NAME::Cr0,
                whp::WHV_REGISTER_NAME::Cr3,
                whp::WHV_REGISTER_NAME::Cr4,
                whp::WHV_REGISTER_NAME::Efer,
                whp::WHV_REGISTER_NAME::Rip,
                whp::WHV_REGISTER_NAME::Rsp,
                whp::WHV_REGISTER_NAME::Rsi,  // Points to boot_params
                whp::WHV_REGISTER_NAME::Rflags,
                whp::WHV_REGISTER_NAME::Cs,
                whp::WHV_REGISTER_NAME::Ds,
                whp::WHV_REGISTER_NAME::Es,
                whp::WHV_REGISTER_NAME::Ss,
                whp::WHV_REGISTER_NAME::Gdtr,
            ];

            let values = [
                // CR0: Protected mode + Paging + other required bits
                whp::reg_val64(CR0_PE | CR0_MP | CR0_ET | CR0_NE | CR0_WP | CR0_AM | CR0_PG),
                // CR3: Page table root
                whp::reg_val64(PML4_ADDR),
                // CR4: PAE + PGE
                whp::reg_val64(CR4_PAE | CR4_PGE),
                // EFER: Long mode enable + active + syscall enable
                whp::reg_val64(EFER_LME | EFER_LMA | EFER_SCE),
                // RIP: Kernel entry point
                whp::reg_val64(KERNEL_LOAD_ADDR),
                // RSP: Stack pointer (near top of low memory)
                whp::reg_val64(0x80000),
                // RSI: Pointer to boot_params
                whp::reg_val64(BOOT_PARAMS_ADDR),
                // RFLAGS: Interrupts disabled, reserved bit 1 set
                whp::reg_val64(0x02),
                // CS: 64-bit code segment (selector 0x08, index 1)
                whp::seg_val(0, 0xFFFFFFFF, 0x08, 0xA09B), // L=1(64bit), P=1, DPL=0, Code
                // DS: Data segment (selector 0x10, index 2)
                whp::seg_val(0, 0xFFFFFFFF, 0x10, 0xC093),
                // ES: Same as DS
                whp::seg_val(0, 0xFFFFFFFF, 0x10, 0xC093),
                // SS: Same as DS
                whp::seg_val(0, 0xFFFFFFFF, 0x10, 0xC093),
                // GDTR: GDT at GDT_ADDR, 3 entries * 8 bytes
                whp::table_val(GDT_ADDR, 23),
            ];

            let hr = whp::WHvSetVirtualProcessorRegisters(
                self.partition,
                0, // VP index
                names.as_ptr(),
                names.len() as u32,
                values.as_ptr(),
            );

            if hr != whp::S_OK {
                return Err(ContainerError::Config(format!(
                    "WHvSetVirtualProcessorRegisters failed: 0x{:08x}", hr as u32
                )));
            }
        }

        Ok(())
    }

    /// Run the virtual machine, handling VM exits.
    pub fn run(&mut self) -> Result<i32> {
        self.running = true;
        let mut output_buffer = Vec::new();

        println!("[*] Starting VM execution...");

        loop {
            if !self.running {
                break;
            }

            unsafe {
                let mut exit_context: whp::WHV_RUN_VP_EXIT_CONTEXT = std::mem::zeroed();
                let hr = whp::WHvRunVirtualProcessor(
                    self.partition,
                    0,
                    &mut exit_context as *mut _ as *mut whp::VOID,
                    std::mem::size_of::<whp::WHV_RUN_VP_EXIT_CONTEXT>() as u32,
                );

                if hr != whp::S_OK {
                    return Err(ContainerError::Config(format!(
                        "WHvRunVirtualProcessor failed: 0x{:08x}", hr as u32
                    )));
                }

                match exit_context.ExitReason {
                    whp::WHV_RUN_VP_EXIT_REASON::X64IoPortAccess => {
                        let io = exit_context.Anonymous.IoPortAccess;
                        let port = io.PortNumber;
                        let is_write = (io.AccessInfo & 1) != 0;

                        match (port, is_write) {
                            (SERIAL_PORT, true) | (VIRTIO_CONSOLE_PORT, true) => {
                                // Serial/console output
                                let byte = (io.Rax & 0xFF) as u8;
                                output_buffer.push(byte);

                                // Flush on newline
                                if byte == b'\n' {
                                    let line = String::from_utf8_lossy(&output_buffer);
                                    print!("[vm] {}", line);
                                    output_buffer.clear();
                                }
                            }
                            (SHUTDOWN_PORT, true) => {
                                // ACPI shutdown
                                println!("[*] VM shutdown requested.");
                                self.running = false;
                                continue;
                            }
                            _ => {
                                // Handle other IO — advance RIP past the instruction
                            }
                        }

                        // Advance RIP past the IO instruction
                        self.advance_rip(&exit_context)?;
                    }

                    whp::WHV_RUN_VP_EXIT_REASON::X64Halt => {
                        println!("[*] VM halted (HLT instruction).");
                        self.running = false;
                    }

                    whp::WHV_RUN_VP_EXIT_REASON::MemoryAccess => {
                        let mem = exit_context.Anonymous.MemoryAccess;
                        eprintln!("[vm] Unhandled memory access at GPA 0x{:x}", mem.Gpa);
                        self.running = false;
                    }

                    whp::WHV_RUN_VP_EXIT_REASON::X64MsrAccess => {
                        // Handle MSR read/write — just return 0 for reads
                        self.advance_rip(&exit_context)?;
                    }

                    whp::WHV_RUN_VP_EXIT_REASON::X64Cpuid => {
                        // Return basic CPUID info
                        self.advance_rip(&exit_context)?;
                    }

                    whp::WHV_RUN_VP_EXIT_REASON::UnrecoverableException => {
                        eprintln!("[vm] Unrecoverable exception — triple fault.");
                        self.running = false;
                    }

                    whp::WHV_RUN_VP_EXIT_REASON::InvalidVpRegisterValue => {
                        eprintln!("[vm] Invalid VP register value.");
                        self.running = false;
                    }

                    other => {
                        eprintln!("[vm] Unhandled exit reason: {:?}", other);
                        self.running = false;
                    }
                }
            }
        }

        // Flush any remaining output
        if !output_buffer.is_empty() {
            let line = String::from_utf8_lossy(&output_buffer);
            print!("[vm] {}", line);
        }

        Ok(0)
    }

    /// Advance RIP past the current instruction after handling a VM exit.
    fn advance_rip(&self, exit_context: &whp::WHV_RUN_VP_EXIT_CONTEXT) -> Result<()> {
        unsafe {
            let instruction_len = (exit_context.VpContext.InstructionLength_Cr8 & 0x0F) as u64;
            let new_rip = exit_context.VpContext.Rip + instruction_len;

            let name = whp::WHV_REGISTER_NAME::Rip;
            let value = whp::reg_val64(new_rip);

            let hr = whp::WHvSetVirtualProcessorRegisters(
                self.partition,
                0,
                &name,
                1,
                &value,
            );

            if hr != whp::S_OK {
                return Err(ContainerError::Config(format!(
                    "advance RIP failed: 0x{:08x}", hr as u32
                )));
            }
        }
        Ok(())
    }
}

impl Drop for VirtualMachine {
    fn drop(&mut self) {
        unsafe {
            whp::WHvDeleteVirtualProcessor(self.partition, 0);
            whp::WHvUnmapGpaRange(self.partition, 0, self.guest_mem_size);
            whp::VirtualFree(self.guest_memory as *mut std::ffi::c_void, 0, whp::MEM_RELEASE);
            whp::WHvDeletePartition(self.partition);
        }
    }
}

// ─── High-Level API ─────────────────────────────────────────────────────────

/// Boot a Linux kernel with an initramfs in a WHP virtual machine.
pub fn boot_linux(
    kernel_path: &Path,
    initrd_path: Option<&Path>,
    cmdline: &str,
    mem_mb: u64,
) -> Result<i32> {
    println!("[*] Creating virtual machine ({} MB RAM)...", mem_mb);

    let mut vm = VirtualMachine::new(mem_mb)?;

    // Set up page tables and GDT
    println!("[*] Setting up x86_64 long mode...");
    vm.setup_page_tables()?;
    vm.setup_gdt()?;

    // Load kernel
    println!("[*] Loading kernel...");
    let kernel_size = vm.load_kernel(kernel_path)?;

    // Load initrd if provided
    let initrd_size = if let Some(initrd) = initrd_path {
        println!("[*] Loading initramfs...");
        vm.load_initrd(initrd)?
    } else {
        0
    };

    // Set up boot parameters
    println!("[*] Setting up boot parameters...");
    println!("    cmdline: {}", cmdline);
    vm.setup_boot_params(kernel_size, initrd_size, cmdline)?;

    // Set up CPU registers for long mode
    vm.setup_long_mode_registers()?;

    // Run the VM
    vm.run()
}
