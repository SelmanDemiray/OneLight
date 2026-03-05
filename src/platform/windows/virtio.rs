//! Virtio device emulation for VM ↔ host communication.
//! Implements basic virtio console (serial I/O) and block device
//! for passing the container rootfs into the VM.
//! Zero dependencies — all virtio protocol structures hand-defined.

use std::collections::VecDeque;
use std::io::{Read, Write};

// ─── Virtio Constants ───────────────────────────────────────────────────────

const VIRTIO_VENDOR_ID: u32 = 0x554D4551; // "QEMU" (historical)
const VIRTIO_CONSOLE_DEVICE_ID: u16 = 3;
const VIRTIO_BLOCK_DEVICE_ID: u16 = 2;
const VIRTIO_NET_DEVICE_ID: u16 = 1;

// Virtio device status bits
const VIRTIO_STATUS_ACKNOWLEDGE: u8 = 1;
const VIRTIO_STATUS_DRIVER: u8 = 2;
const VIRTIO_STATUS_FEATURES_OK: u8 = 8;
const VIRTIO_STATUS_DRIVER_OK: u8 = 4;
const VIRTIO_STATUS_FAILED: u8 = 128;

// Virtqueue descriptor flags
const VRING_DESC_F_NEXT: u16 = 1;
const VRING_DESC_F_WRITE: u16 = 2;
const VRING_DESC_F_INDIRECT: u16 = 4;

// ─── Virtqueue Structures ───────────────────────────────────────────────────

/// Virtqueue descriptor (16 bytes)
#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct VringDesc {
    pub addr: u64,    // Guest physical address
    pub len: u32,     // Length in bytes
    pub flags: u16,   // VRING_DESC_F_*
    pub next: u16,    // Next descriptor index (if NEXT flag set)
}

/// Virtqueue available ring header
#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct VringAvailHeader {
    pub flags: u16,
    pub idx: u16,
}

/// Virtqueue used ring header
#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct VringUsedHeader {
    pub flags: u16,
    pub idx: u16,
}

/// Virtqueue used element
#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct VringUsedElem {
    pub id: u32,
    pub len: u32,
}

// ─── Virtio Console ─────────────────────────────────────────────────────────

/// A simple virtio console that bridges VM serial output to host stdout.
pub struct VirtioConsole {
    /// Input buffer (host → guest)
    input_queue: VecDeque<u8>,
    /// Output buffer (guest → host)
    output_queue: VecDeque<u8>,
    /// Terminal mode
    raw_mode: bool,
}

impl VirtioConsole {
    pub fn new() -> Self {
        VirtioConsole {
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
            raw_mode: false,
        }
    }

    /// Handle a byte written to the serial port by the guest.
    pub fn handle_output(&mut self, byte: u8) {
        self.output_queue.push_back(byte);

        // Write directly to host stdout
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        let _ = handle.write_all(&[byte]);

        // Flush on newline for immediate output
        if byte == b'\n' {
            let _ = handle.flush();
        }
    }

    /// Queue input from the host for the guest to read.
    pub fn queue_input(&mut self, data: &[u8]) {
        for &b in data {
            self.input_queue.push_back(b);
        }
    }

    /// Get the next byte for the guest to read (from input queue).
    pub fn read_byte(&mut self) -> Option<u8> {
        self.input_queue.pop_front()
    }

    /// Check if there's input available for the guest.
    pub fn has_input(&self) -> bool {
        !self.input_queue.is_empty()
    }

    /// Get all accumulated output as a string.
    pub fn get_output(&mut self) -> String {
        let bytes: Vec<u8> = self.output_queue.drain(..).collect();
        String::from_utf8_lossy(&bytes).to_string()
    }
}

// ─── Virtio Block Device ────────────────────────────────────────────────────

/// Virtio block device for exposing the container rootfs to the guest.
pub struct VirtioBlock {
    /// Backing storage (the rootfs as a disk image)
    data: Vec<u8>,
    /// Block size (512 bytes standard)
    block_size: u32,
    /// Read-only flag
    read_only: bool,
}

/// Virtio block request header
#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct VirtioBlkReq {
    pub req_type: u32,
    pub reserved: u32,
    pub sector: u64,
}

const VIRTIO_BLK_T_IN: u32 = 0;   // Read
const VIRTIO_BLK_T_OUT: u32 = 1;  // Write
const VIRTIO_BLK_T_FLUSH: u32 = 4;
const VIRTIO_BLK_T_GET_ID: u32 = 8;

const VIRTIO_BLK_S_OK: u8 = 0;
const VIRTIO_BLK_S_IOERR: u8 = 1;
const VIRTIO_BLK_S_UNSUPP: u8 = 2;

impl VirtioBlock {
    /// Create a new block device backed by a byte buffer.
    pub fn new(data: Vec<u8>, read_only: bool) -> Self {
        VirtioBlock {
            data,
            block_size: 512,
            read_only,
        }
    }

    /// Create from a file (e.g., a rootfs disk image).
    pub fn from_file(path: &std::path::Path) -> std::io::Result<Self> {
        let data = std::fs::read(path)?;
        Ok(VirtioBlock::new(data, false))
    }

    /// Get the capacity in sectors.
    pub fn capacity_sectors(&self) -> u64 {
        self.data.len() as u64 / self.block_size as u64
    }

    /// Handle a read request.
    pub fn read_sectors(&self, sector: u64, count: u32) -> Result<Vec<u8>, u8> {
        let offset = sector as usize * self.block_size as usize;
        let len = count as usize * self.block_size as usize;

        if offset + len > self.data.len() {
            return Err(VIRTIO_BLK_S_IOERR);
        }

        Ok(self.data[offset..offset + len].to_vec())
    }

    /// Handle a write request.
    pub fn write_sectors(&mut self, sector: u64, data: &[u8]) -> Result<(), u8> {
        if self.read_only {
            return Err(VIRTIO_BLK_S_IOERR);
        }

        let offset = sector as usize * self.block_size as usize;
        if offset + data.len() > self.data.len() {
            // Extend if needed
            self.data.resize(offset + data.len(), 0);
        }

        self.data[offset..offset + data.len()].copy_from_slice(data);
        Ok(())
    }

    /// Get the virtio block config space.
    pub fn config_space(&self) -> Vec<u8> {
        let mut config = vec![0u8; 60]; // virtio_blk_config size

        // capacity (in 512-byte sectors)
        let capacity = self.capacity_sectors();
        config[0..8].copy_from_slice(&capacity.to_le_bytes());

        // size_max (4KB)
        config[8..12].copy_from_slice(&4096u32.to_le_bytes());

        // seg_max
        config[12..16].copy_from_slice(&128u32.to_le_bytes());

        // blk_size (512)
        config[20..24].copy_from_slice(&self.block_size.to_le_bytes());

        config
    }
}

// ─── Virtio Network ─────────────────────────────────────────────────────────

/// Minimal virtio-net device for container networking.
pub struct VirtioNet {
    /// MAC address
    pub mac: [u8; 6],
    /// Received packets (from host network)
    rx_queue: VecDeque<Vec<u8>>,
    /// Transmitted packets (from guest)
    tx_queue: VecDeque<Vec<u8>>,
}

impl VirtioNet {
    pub fn new() -> Self {
        VirtioNet {
            mac: [0x52, 0x54, 0x00, 0x12, 0x34, 0x56], // Default MAC
            rx_queue: VecDeque::new(),
            tx_queue: VecDeque::new(),
        }
    }

    /// Queue a packet for the guest to receive.
    pub fn inject_packet(&mut self, packet: Vec<u8>) {
        self.rx_queue.push_back(packet);
    }

    /// Get the next packet transmitted by the guest.
    pub fn get_tx_packet(&mut self) -> Option<Vec<u8>> {
        self.tx_queue.pop_front()
    }

    /// Record a packet sent by the guest.
    pub fn handle_tx(&mut self, packet: Vec<u8>) {
        self.tx_queue.push_back(packet);
    }

    pub fn config_space(&self) -> Vec<u8> {
        let mut config = vec![0u8; 6];
        config.copy_from_slice(&self.mac);
        config
    }
}
