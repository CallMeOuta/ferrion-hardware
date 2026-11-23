// SPDX-License-Identifier: GPL-2.0-only
use std::ffi::{c_char, c_void};
use std::fs::File;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use common::{MemoryType, VirtaddrTranslationHandle, memory_root_fd};
use libredox::call::MmapArgs;
use libredox::flag;
use pcid_interface::PciFunctionHandle;
use pcid_interface::PciBar;
use syscall::flag::{MapFlags, MremapFlags};

const DMA_BUDGET: usize = 1 << 30;
const PAGE_SIZE: usize = 4096;
const IRQ_READ: usize = core::mem::size_of::<usize>();
const KOBJ_CHANGE: i32 = 2;
const BAR_IO: u32 = 1;
const BAR_64: u32 = 2;
const SUBSYSTEM_ID: u16 = 0x2c;

#[repr(C)]
pub struct PciBarInfo {
    phys: u64,
    size: u64,
    flags: u32,
}

#[repr(C)]
pub struct PciInfo {
    vendor: u16,
    device: u16,
    subsystem_vendor: u16,
    subsystem_device: u16,
    class: u32,
    revision: u8,
    bus: u8,
    devfn: u8,
    has_irq: u8,
    bars: [PciBarInfo; 6],
}

struct Pci(PciFunctionHandle);

unsafe impl Send for Pci {}

impl std::ops::Deref for Pci {
    type Target = PciFunctionHandle;

    fn deref(&self) -> &PciFunctionHandle {
        &self.0
    }
}

impl std::ops::DerefMut for Pci {
    fn deref_mut(&mut self) -> &mut PciFunctionHandle {
        &mut self.0
    }
}

struct Backend {
    pci: Mutex<Pci>,
    translation: VirtaddrTranslationHandle,
    hotplug: File,
}

static BACKEND: OnceLock<Backend> = OnceLock::new();
static DMA_USED: AtomicUsize = AtomicUsize::new(0);
static IRQ_STARTED: AtomicI32 = AtomicI32::new(0);

unsafe extern "C" {
    static page_offset_base: usize;
    fn ferrion_irq_fire(irq: u32);
}

fn backend() -> &'static Backend {
    BACKEND.get().expect("ferrion-driver-host: back end used before init")
}

pub fn init(pci: PciFunctionHandle) -> Result<i32, String> {
    let translation = VirtaddrTranslationHandle::new().map_err(|e| format!("translation handle: {e}"))?;
    let mut fds = [-1; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) } != 0 {
        return Err("hotplug pipe".into());
    }
    let hotplug = unsafe { std::os::fd::FromRawFd::from_raw_fd(fds[1]) };
    BACKEND
        .set(Backend { pci: Mutex::new(Pci(pci)), translation, hotplug })
        .map_err(|_| "back end initialized twice".to_string())?;
    Ok(fds[0])
}

#[unsafe(no_mangle)]
pub extern "C" fn ferrion_log(level: i32, text: *const c_char, len: usize) {
    let bytes = unsafe { std::slice::from_raw_parts(text as *const u8, len) };
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim_end_matches('\n');
    let level = match level {
        0..=3 => log::Level::Error,
        4 => log::Level::Warn,
        5 | 6 => log::Level::Info,
        _ => log::Level::Debug,
    };
    log::log!(level, "linux: {text}");
}

#[unsafe(no_mangle)]
pub extern "C" fn ferrion_phys_alloc(size: usize, phys_out: *mut usize) -> *mut c_void {
    if size == 0 || size % PAGE_SIZE != 0 {
        return core::ptr::null_mut();
    }
    if DMA_USED.fetch_add(size, Ordering::Relaxed) + size > DMA_BUDGET {
        DMA_USED.fetch_sub(size, Ordering::Relaxed);
        log::warn!("ferrion-driver-host: the {DMA_BUDGET}-byte DMA budget is spent");
        return core::ptr::null_mut();
    }
    match phys_alloc(size) {
        Ok((va, phys)) => {
            unsafe { *phys_out = phys };
            va as *mut c_void
        }
        Err(error) => {
            DMA_USED.fetch_sub(size, Ordering::Relaxed);
            log::warn!("ferrion-driver-host: {size} bytes of DMA memory: {error}");
            core::ptr::null_mut()
        }
    }
}

fn phys_alloc(size: usize) -> Result<(usize, usize), String> {
    let fd = memory_root_fd()
        .openat("zeroed@wb?phys_contiguous", flag::O_CLOEXEC, 0)
        .map_err(|e| format!("open: {e}"))?;
    let virt = unsafe {
        libredox::call::mmap(MmapArgs {
            fd: fd.raw(),
            offset: 0,
            addr: core::ptr::null_mut(),
            length: size,
            flags: flag::MAP_PRIVATE,
            prot: flag::PROT_READ | flag::PROT_WRITE,
        })
    }
    .map_err(|e| format!("mmap: {e}"))? as usize;
    let phys = backend()
        .translation
        .translate(virt)
        .map_err(|e| format!("translate: {e}"))?;
    let target = unsafe { page_offset_base } + phys;
    let flags = MremapFlags::FIXED_REPLACE.bits() | (MapFlags::PROT_READ | MapFlags::PROT_WRITE).bits();
    let moved = unsafe { syscall::syscall5(syscall::number::SYS_MREMAP, virt, size, target, size, flags) };
    match moved {
        Ok(addr) if addr == target => Ok((target, phys)),
        Ok(addr) => Err(format!("mremap moved to {addr:#x}, not {target:#x}")),
        Err(error) => {
            let _ = unsafe { libredox::call::munmap(virt as *mut (), size) };
            Err(format!("mremap: {error}"))
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn ferrion_phys_free(va: *mut c_void, size: usize) {
    match map_zeroed(va as usize, size, MapFlags::MAP_FIXED, flag::PROT_NONE) {
        Ok(()) => {
            DMA_USED.fetch_sub(size, Ordering::Relaxed);
        }
        Err(error) => log::warn!("ferrion-driver-host: freeing {size} bytes of DMA memory: {error}"),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn ferrion_reserve(addr: *mut c_void, size: usize, writable: i32) -> i32 {
    let prot = if writable != 0 { flag::PROT_READ | flag::PROT_WRITE } else { flag::PROT_NONE };
    match map_zeroed(addr as usize, size, MapFlags::MAP_FIXED_NOREPLACE, prot) {
        Ok(()) => 0,
        Err(error) => {
            log::error!("ferrion-driver-host: reserving {size:#x} bytes at {addr:p}: {error}");
            -22
        }
    }
}

fn map_zeroed(addr: usize, size: usize, fixed: MapFlags, prot: u32) -> Result<(), String> {
    let fd = memory_root_fd()
        .openat("zeroed", flag::O_CLOEXEC, 0)
        .map_err(|e| format!("open: {e}"))?;
    let mapped = unsafe {
        libredox::call::mmap(MmapArgs {
            fd: fd.raw(),
            offset: 0,
            addr: addr as *mut (),
            length: size,
            flags: flag::MAP_PRIVATE | fixed.bits() as u32,
            prot,
        })
    }
    .map_err(|e| format!("mmap: {e}"))?;
    if mapped as usize == addr { Ok(()) } else { Err(format!("mapped at {mapped:p}")) }
}

#[unsafe(no_mangle)]
pub extern "C" fn ferrion_pci_info(info: *mut PciInfo) -> i32 {
    let mut pci = backend().pci.lock().unwrap();
    let func = pci.config().func;
    let id = func.full_device_id;
    let subsystem = unsafe { pci.read_config(SUBSYSTEM_ID) };
    let info = unsafe { &mut *info };
    info.vendor = id.vendor_id;
    info.device = id.device_id;
    info.subsystem_vendor = subsystem as u16;
    info.subsystem_device = (subsystem >> 16) as u16;
    info.class = (u32::from(id.class) << 16) | (u32::from(id.subclass) << 8) | u32::from(id.interface);
    info.revision = id.revision;
    info.bus = func.addr.bus();
    info.devfn = (func.addr.device() << 3) | func.addr.function();
    info.has_irq = u8::from(func.legacy_interrupt_line.is_some());
    for (slot, bar) in info.bars.iter_mut().zip(func.bars.iter()) {
        *slot = match *bar {
            PciBar::Memory32 { addr, size } => PciBarInfo { phys: addr.into(), size: size.into(), flags: 0 },
            PciBar::Memory64 { addr, size } => PciBarInfo { phys: addr, size, flags: BAR_64 },
            PciBar::Port(port) => PciBarInfo { phys: port.into(), size: 0, flags: BAR_IO },
            PciBar::None => PciBarInfo { phys: 0, size: 0, flags: 0 },
        };
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn ferrion_pci_read_config(offset: u32) -> u32 {
    unsafe { backend().pci.lock().unwrap().read_config(offset as u16) }
}

#[unsafe(no_mangle)]
pub extern "C" fn ferrion_pci_write_config(offset: u32, value: u32) {
    unsafe { backend().pci.lock().unwrap().write_config(offset as u16, value) }
}

#[unsafe(no_mangle)]
pub extern "C" fn ferrion_pci_map_bar(bar: u32) -> *mut c_void {
    let mut pci = backend().pci.lock().unwrap();
    let mapped = unsafe { pci.map_bar(bar as u8, MemoryType::Uncacheable) };
    mapped.ptr.as_ptr() as *mut c_void
}

#[unsafe(no_mangle)]
pub extern "C" fn ferrion_pci_irq_start(irq: u32) -> i32 {
    if IRQ_STARTED.swap(1, Ordering::AcqRel) != 0 {
        return -16;
    }
    let line = backend().pci.lock().unwrap().config().func.legacy_interrupt_line;
    let Some(line) = line else {
        return -19;
    };
    let mut file = line.irq_handle("ferrion-driver-host");
    let spawned = std::thread::Builder::new()
        .name("ferrion-irq".into())
        .spawn(move || irq_loop(&mut file, irq));
    match spawned {
        Ok(_) => 0,
        Err(_) => -11,
    }
}

fn irq_loop(file: &mut File, irq: u32) {
    let mut buf = [0u8; IRQ_READ];
    loop {
        match file.read(&mut buf) {
            Ok(n) if n == IRQ_READ => {
                unsafe { ferrion_irq_fire(irq) };
                let _ = file.write(&buf);
            }
            Ok(_) => {}
            Err(error) => {
                log::error!("ferrion-driver-host: interrupt line failed: {error}");
                return;
            }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn ferrion_uevent(_name: *const c_char, action: i32) {
    if action == KOBJ_CHANGE {
        let _ = (&backend().hotplug).write(&[1]);
    }
}
