//! The eXtensible Host Controller Interface (XHCI) Module
//!
//! This module implements the XHCI functionality of Redox's USB driver daemon.
//!
//! XHCI is a standard for the USB Host Controller interface specified by Intel that provides a
//! common register interface for systems to use to interact with the Universal Serial Bus (USB)
//! subsystem.
//!
//! The standard can be found [here](https://www.intel.com/content/dam/www/public/us/en/documents/technical-specifications/extensible-host-controler-interface-usb-xhci.pdf).
//! The standard is referenced frequently throughout this documentation. The acronyms used for specific
//! documents are specified in the crate-level documentation.
use std::collections::BTreeMap;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};
use std::{process, thread};

use chashmap::CHashMap;
use common::dma::Dma;
use common::io::Io;
use crossbeam_channel::{Receiver, Sender};
use log::{debug, trace, warn};
use pcid_interface::irq_helpers::InterruptVector;
use pcid_interface::PciFunctionHandle;
use syscall::error::Result;

use crate::usb;

mod controller;
mod device;
pub mod scheme;
mod transfer;

pub use self::capability::CapabilityRegs;
use self::context::{DeviceContextList, InputContext, ScratchpadBufferArray, StreamContextArray};
pub use self::context::{CONTEXT_32, CONTEXT_64};
use self::controller::{capability, context, doorbell, extended, operational, port, runtime};
use self::device::drivers::DriversConfig;
use self::device::enumerator::DeviceEnumerationRequest;
pub use self::device::start_device_enumerator;
use self::doorbell::Doorbell;
use self::event::EventRing;
use self::extended::ProtocolSpeed;
use self::irq_reactor::{IrqReactor, NewPendingTrb};
use self::operational::OperationalRegs;
use self::port::Port;
use self::ring::Ring;
use self::runtime::RuntimeRegs;
use self::scheme::EndpIfState;
pub use self::transfer::irq_reactor;
use self::transfer::{event, ring, trb};
pub use crate::driver_interface::PortId;
use crate::driver_interface::*;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EndpNum(u8);

impl EndpNum {
    pub const fn new(value: u8) -> Self {
        Self(value)
    }

    pub fn get(&self) -> u8 {
        self.0
    }

    pub fn index(&self) -> Option<usize> {
        usize::from(self.0).checked_sub(1)
    }
}

/// Specifies the configurable interrupt mechanism used by the xhci subsystem for registering
/// device state change notifications.
pub enum InterruptMethod {
    /// No interrupts whatsoever; the driver will instead rely on polling event rings.
    Polling,

    /// Legacy PCI INTx# interrupt pin.
    Intx,

    /// (Extended) Message signaled interrupts.
    Msi,
}

/// The eXtensible Host Controller Interface (XHCI) data structure
pub struct Xhci<const N: usize> {
    // immutable
    /// The Host Controller Interface Capability Registers. These read-only registers specify the
    /// limits and capabilities of the host controller implementation (See XHCI section 5.3)
    cap: &'static CapabilityRegs,
    //page_size: usize,

    // XXX: It would be really useful to be able to mutably access individual elements of a slice,
    // without having to wrap every element in a lock (which wouldn't work since they're packed).
    /// The Host Controller Interface Operational Registers. These registers provide the software
    /// interface to configure and monitor the state of the XHCI (See XHCI section 5.4)
    op: Mutex<&'static mut OperationalRegs>,
    ports: Mutex<&'static mut [Port]>,
    /// The Host Controller Interface Doorbell Registers. There is one register per device slot,
    /// and these registers are used by system software to notify the XHC that it has work to perform
    /// for a specific device slot. (See XHCI sections 4.7 and 5.6)
    dbs: Arc<Mutex<&'static mut [Doorbell]>>,
    /// The Host Controller Interface Runtime Registers. These handle interrupt and event processing,
    /// and provide time-sensitive information such as the current microframe. (See XHCI section 5.5)
    run: Mutex<&'static mut RuntimeRegs>,
    cmd: Mutex<Ring>,
    primary_event_ring: Mutex<EventRing>,

    // immutable
    dev_ctx: DeviceContextList<N>,
    scratchpad_buf_arr: Option<ScratchpadBufferArray>,

    // used for the extended capabilities, and so far none of them are mutated, and thus no lock.
    base: *const u8,

    drivers_config: DriversConfig,
    handles: CHashMap<usize, scheme::Handle>,
    next_handle: AtomicUsize,
    port_states: CHashMap<PortId, PortState<N>>,
    drivers: CHashMap<PortId, Vec<process::Child>>,
    scheme_name: String,

    interrupt_method: InterruptMethod,
    pcid_handle: Mutex<PciFunctionHandle>,

    irq_reactor: Mutex<Option<thread::JoinHandle<()>>>,

    irq_reactor_sender: Sender<NewPendingTrb>,

    // not used, but still stored so that the thread, when created, can get the channel without the
    // channel being in a mutex.
    irq_reactor_receiver: Receiver<NewPendingTrb>,
    device_enumerator: Mutex<Option<thread::JoinHandle<()>>>,
    device_enumerator_sender: Sender<DeviceEnumerationRequest>,
    device_enumerator_receiver: Receiver<DeviceEnumerationRequest>,
}

unsafe impl<const N: usize> Send for Xhci<N> {}
unsafe impl<const N: usize> Sync for Xhci<N> {}

struct PortState<const N: usize> {
    slot: u8,
    protocol_speed: &'static ProtocolSpeed,
    cfg_idx: Option<u8>,
    input_context: Mutex<Dma<InputContext<N>>>,
    dev_desc: Option<DevDesc>,
    endpoint_states: BTreeMap<EndpNum, EndpointState>,
}

impl<const N: usize> PortState<N> {
    fn get_endp_desc(&self, endp_num: EndpNum) -> Option<&EndpDesc> {
        let cfg_idx = self.cfg_idx?;
        let config_desc = self
            .dev_desc
            .as_ref()?
            .config_descs
            .iter()
            .find(|desc| desc.configuration_value == cfg_idx)?;
        let mut endp_count = 0;
        for if_desc in config_desc.interface_descs.iter() {
            for endp_desc in if_desc.endpoints.iter() {
                endp_count += 1;
                if endp_num.get() == endp_count {
                    return Some(endp_desc);
                }
            }
        }
        None
    }
}

pub(crate) enum RingOrStreams {
    Ring(Ring),
    Streams(StreamContextArray),
}

pub(crate) struct EndpointState {
    pub transfer: RingOrStreams,
    pub driver_if_state: EndpIfState,
}
impl EndpointState {
    fn ring(&mut self) -> Option<&mut Ring> {
        match self.transfer {
            RingOrStreams::Ring(ref mut ring) => Some(ring),
            _ => None,
        }
    }
}

impl<const N: usize> Xhci<N> {
    pub fn force_clear_interrupt(&self, index: usize) {
        {
            // If ERDP EHB bit is set, clear it before sending command
            //TODO: find out why this bit is set earlier!
            let mut run = self.run.lock().unwrap();
            let mut int = &mut run.ints[index];

            if int.erdp_low.readf(1 << 3) {
                int.erdp_low.writef(1 << 3, true);
            } else {
                warn!("Attempted to clear the interrupt bit when no interrupt was pending");
            }
        }
    }

    pub fn interrupt_is_pending(&self, index: usize) -> bool {
        let mut run = self.run.lock().unwrap();
        let mut int = &mut run.ints[index];
        int.erdp_low.readf(1 << 3)
    }

    pub unsafe fn alloc_dma_zeroed_raw<T>(_ac64: bool) -> Result<Dma<T>> {
        // TODO: ac64
        Ok(Dma::zeroed()?.assume_init())
    }
    pub unsafe fn alloc_dma_zeroed<T>(&self) -> Result<Dma<T>> {
        Self::alloc_dma_zeroed_raw(self.cap.ac64())
    }
    pub unsafe fn alloc_dma_zeroed_unsized_raw<T>(_ac64: bool, count: usize) -> Result<Dma<[T]>> {
        // TODO: ac64
        Ok(Dma::zeroed_slice(count)?.assume_init())
    }
    pub unsafe fn alloc_dma_zeroed_unsized<T>(&self, count: usize) -> Result<Dma<[T]>> {
        Self::alloc_dma_zeroed_unsized_raw(self.cap.ac64(), count)
    }

    fn uses_msi_interrupts(&self) -> bool {
        matches!(self.interrupt_method, InterruptMethod::Msi)
    }

    /// Checks whether an IRQ has been received from *this* device, in case of an interrupt. Always
    /// true when using MSI/MSI-X.
    pub fn received_irq(&self) -> bool {
        let mut runtime_regs = self.run.lock().unwrap();

        if self.uses_msi_interrupts() {
            // Since using MSI and MSI-X implies having no IRQ sharing whatsoever, the IP bit
            // doesn't have to be touched.
            trace!(
                "Successfully received MSI/MSI-X interrupt, IP={}, EHB={}",
                runtime_regs.ints[0].iman.readf(1),
                runtime_regs.ints[0].erdp_low.readf(1 << 3)
            );
            true
        } else if runtime_regs.ints[0].iman.readf(1) {
            trace!(
                "Successfully received INTx# interrupt, IP={}, EHB={}",
                runtime_regs.ints[0].iman.readf(1),
                runtime_regs.ints[0].erdp_low.readf(1 << 3)
            );
            // If MSI and/or MSI-X are not used, the interrupt might have to be shared, and thus there is
            // a special register to specify whether the IRQ actually came from the xHC.
            runtime_regs.ints[0].iman.writef(1, true);

            // The interrupt came from the xHC.
            true
        } else {
            // The interrupt came from a different device.
            false
        }
    }
}
pub fn start_irq_reactor<const N: usize>(hci: &Arc<Xhci<N>>, irq_file: Option<InterruptVector>) {
    let hci_clone = Arc::clone(&hci);

    debug!("About to start IRQ reactor");

    *hci.irq_reactor.lock().unwrap() = Some(thread::spawn(move || {
        debug!("Started IRQ reactor thread");
        IrqReactor::new(hci_clone, irq_file).run()
    }));
}
