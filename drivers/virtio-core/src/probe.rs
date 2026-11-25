// SPDX-License-Identifier: GPL-2.0-only AND MIT
use std::sync::{Arc, Mutex};

use common::MemoryType;
use pcid_interface::irq_helpers::{InterruptVector, Msix};
use pcid_interface::{PciBar, PciFunctionHandle};

use crate::spec::*;
use crate::transport::{Error, Queue, StandardTransport, Transport};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct SharedMemoryRegion {
    pub id: u8,
    pub bar: u8,
    pub bar_offset: u64,
    pub phys_addr: u64,
    pub length: u64,
}

pub struct Device {
    pub transport: Arc<dyn Transport>,
    pub device_space: *const u8,
    msix: Mutex<Msix>,
    shared_memory_regions: Vec<SharedMemoryRegion>,
}

// FIXME(andypython): `device_space` should not be `Send` nor `Sync`. Take
// it out of `Device`.
unsafe impl Send for Device {}
unsafe impl Sync for Device {}

impl Device {
    /// Next MSI-X vector and a virtqueue on it.
    /// Later allocs reuse vector 0 if the MSI-X table is exhausted.
    pub fn setup_queue(&self) -> Result<Arc<Queue>, Error> {
        let irq = self.alloc_irq();
        self.transport.setup_queue(irq)
    }

    pub fn setup_queue_manual_irq(&self) -> Result<(Arc<Queue>, InterruptVector), Error> {
        let irq = self.alloc_irq();
        let queue = self.transport.setup_queue_unattached(&irq)?;
        Ok((queue, irq))
    }

    /// Next MSI-X vector without a virtqueue.
    ///
    /// For device-config interrupts (`setup_config_notify`).
    /// This does not spawn an IRQ thread. (see virtio-gpud).
    pub fn alloc_irq(&self) -> InterruptVector {
        self.msix.lock().unwrap().alloc()
    }

    pub fn shared_memory_region(&self, id: u8) -> Option<SharedMemoryRegion> {
        self.shared_memory_regions
            .iter()
            .copied()
            .find(|region| region.id == id)
    }

    pub fn shared_memory_regions(&self) -> &[SharedMemoryRegion] {
        &self.shared_memory_regions
    }
}

/// VirtIO Device Probe
///
/// ## Device State
/// After this function, the device will have been successfully reseted and is ready for use.
///
/// The caller is required to do the following:
/// * Negotiate the device and driver supported features (finialize via [`StandardTransport::finalize_features`])
/// * Create the device specific virtio queues (via [`Device::setup_queue`]). This is *required* to be done
///   before starting the device.
/// * Finally start the device (via [`StandardTransport::run_device`]). At this point, the device
///   is alive.
///
/// ## Panics
/// This function panics if the device is not a virtio device.
pub fn probe_device(pcid_handle: &mut PciFunctionHandle) -> Result<Device, Error> {
    let pci_config = pcid_handle.config();

    assert_eq!(
        pci_config.func.full_device_id.vendor_id, 6900,
        "virtio_core::probe_device: not a virtio device"
    );

    let mut common_addr = None;
    let mut notify_addr = None;
    let mut device_addr = None;
    let mut shared_memory_regions: Vec<SharedMemoryRegion> = Vec::new();

    for raw_capability in pcid_handle.get_vendor_capabilities() {
        if raw_capability.data.len() < size_of::<PciCapability>() {
            log::warn!(
                "virtio-core: ignoring short vendor capability ({} bytes)",
                raw_capability.data.len()
            );
            continue;
        }
        let Some(cfg_type) = CfgType::from_u8(raw_capability.data[0]) else {
            continue;
        };

        if matches!(cfg_type, CfgType::SharedMemory) {
            if raw_capability.data.len() < size_of::<PciCapability64>() {
                log::warn!(
                    "virtio-core: ignoring short shared-memory capability ({} bytes)",
                    raw_capability.data.len()
                );
                continue;
            }
            let cap64 = unsafe {
                raw_capability
                    .data
                    .as_ptr()
                    .cast::<PciCapability64>()
                    .read_unaligned()
            };
            let bar = cap64.cap.bar;
            let id = cap64.cap.id;
            let bar_offset = cap64.offset();
            let length = cap64.length();
            let Some(&pci_bar) = pci_config.func.bars.get(usize::from(bar)) else {
                log::warn!("virtio-core: shared-memory region {id} names invalid BAR {bar}");
                continue;
            };
            let (bar_addr, bar_size) = match pci_bar {
                PciBar::Memory32 { addr, size } => (u64::from(addr), u64::from(size)),
                PciBar::Memory64 { addr, size } => (addr, size),
                _ => {
                    log::warn!("virtio-core: shared-memory region {id} BAR {bar} is not memory");
                    continue;
                }
            };
            let Some(bar_end) = bar_offset.checked_add(length) else {
                log::warn!("virtio-core: shared-memory region {id} overflows BAR {bar}");
                continue;
            };
            let Some(phys_addr) = bar_addr.checked_add(bar_offset) else {
                log::warn!("virtio-core: shared-memory region {id} physical address overflow");
                continue;
            };
            if length == 0 || bar_end > bar_size {
                log::warn!(
                    "virtio-core: shared-memory region {id} [{bar_offset:#x}, +{length:#x}) outside BAR {bar} ({bar_size:#x})"
                );
                continue;
            }
            if shared_memory_regions.iter().any(|region| region.id == id) {
                log::warn!("virtio-core: ignoring duplicate shared-memory region id {id}");
                continue;
            }
            shared_memory_regions.push(SharedMemoryRegion {
                id,
                bar,
                bar_offset,
                phys_addr,
                length,
            });
            continue;
        }

        match cfg_type {
            CfgType::Common | CfgType::Notify | CfgType::Device => {}
            _ => continue,
        }

        let capability = unsafe {
            raw_capability
                .data
                .as_ptr()
                .cast::<PciCapability>()
                .read_unaligned()
        };

        let mapped_bar = unsafe { pcid_handle.map_bar(capability.bar, MemoryType::Uncacheable) };
        let address = mapped_bar.ptr.expose_provenance().get() + capability.offset as usize;

        match cfg_type {
            CfgType::Common => {
                debug_assert!(common_addr.is_none());
                common_addr = Some(address);
            }

            CfgType::Notify => {
                debug_assert!(notify_addr.is_none());
                if raw_capability.data.len() < size_of::<PciCapabilityNotify>() {
                    log::warn!("virtio-core: ignoring short notify capability");
                    continue;
                }
                let multiplier = unsafe {
                    raw_capability
                        .data
                        .as_ptr()
                        .cast::<PciCapabilityNotify>()
                        .read_unaligned()
                        .notify_off_multiplier()
                };
                notify_addr = Some((address, multiplier));
            }

            CfgType::Device => {
                debug_assert!(device_addr.is_none());
                device_addr = Some(address);
            }

            _ => unreachable!(),
        }
    }

    let common_addr = common_addr.expect("virtio common capability missing");
    let device_addr = device_addr.expect("virtio device capability missing");
    let (notify_addr, notify_multiplier) = notify_addr.expect("virtio notify capability missing");

    // FIXME this is explicitly allowed by the virtio specification to happen
    assert!(
        notify_multiplier != 0,
        "virtio-core::device_probe: device uses the same Queue Notify addresses for all queues"
    );

    let common = unsafe { &mut *(common_addr as *mut CommonCfg) };
    let device_space = unsafe { &mut *(device_addr as *mut u8) };

    let transport = StandardTransport::new(
        common,
        notify_addr as *const u8,
        notify_multiplier,
        device_space,
    );

    // Setup interrupts.
    let all_pci_features = pcid_handle.fetch_all_features();
    let has_msix = all_pci_features.iter().any(|feature| feature.is_msix());

    // According to the virtio specification, the device REQUIRED to support MSI-X.
    assert!(has_msix, "virtio: device does not support MSI-X");
    let msix = Msix::enable(pcid_handle, "virtio");

    log::debug!("virtio: using standard PCI transport");

    let device = Device {
        transport,
        device_space,
        msix: Mutex::new(msix),
        shared_memory_regions,
    };

    device.transport.reset();
    reinit(&device)?;

    Ok(device)
}

pub fn reinit(device: &Device) -> Result<(), Error> {
    // XXX: According to the virtio specification v1.2, setting the ACKNOWLEDGE and DRIVER bits
    //      in `device_status` is required to be done in two steps.
    device
        .transport
        .insert_status(DeviceStatusFlags::ACKNOWLEDGE);

    device.transport.insert_status(DeviceStatusFlags::DRIVER);
    Ok(())
}
