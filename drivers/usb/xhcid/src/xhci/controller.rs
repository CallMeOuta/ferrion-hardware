use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};
use std::{fs, slice};

use chashmap::CHashMap;
use common::io::Io;
use common::timeout::Timeout;
use log::{debug, warn};
use pcid_interface::PciFunctionHandle;
use syscall::error::{Error, Result, EIO};
use syscall::PAGE_SIZE;

use self::capability::CapabilityRegs;
use self::context::{DeviceContextList, ScratchpadBufferArray};
use self::doorbell::Doorbell;
use self::operational::{
    OperationalRegs, USB_CMD_HCRST, USB_CMD_INTE, USB_CMD_RS, USB_STS_CNR, USB_STS_HCH,
};
use self::port::Port;
use self::runtime::RuntimeRegs;
use super::device::drivers::DriversConfig;
use super::event::EventRing;
use super::ring::Ring;
use super::trb::Trb;
use super::{InterruptMethod, PortId, Xhci};

pub(super) mod capability;
pub(super) mod context;
pub(super) mod doorbell;
pub(super) mod extended;
pub(super) mod operational;
pub(super) mod port;
pub(super) mod runtime;

impl<const N: usize> Xhci<N> {
    pub fn new(
        scheme_name: String,
        address: usize,
        interrupt_method: InterruptMethod,
        pcid_handle: PciFunctionHandle,
    ) -> Result<Self> {
        //Locate the capability registers from the mapped PCI Bar
        let cap = unsafe { &mut *(address as *mut CapabilityRegs) };
        debug!("CAP REGS BASE {:X}", address);

        //let page_size = ...

        //The operational registers appear immediately after the capability registers.
        let op_base = address + cap.len.read() as usize;
        let op = unsafe { &mut *(op_base as *mut OperationalRegs) };
        debug!("OP REGS BASE {:X}", op_base);

        //Reset the XHCI device
        let (max_slots, max_ports) = {
            {
                debug!("Waiting for xHC becoming ready.");
                let timeout = Timeout::from_secs(1);
                while op.usb_sts.readf(USB_STS_CNR) {
                    timeout.run().map_err(|()| {
                        log::error!("timeout on USB_STS_CNR");
                        Error::new(EIO)
                    })?;
                }
            }

            debug!("Stopping the xHC");
            // Set run/stop to 0
            op.usb_cmd.writef(USB_CMD_RS, false);

            {
                debug!("Waiting for the xHC to stop.");
                let timeout = Timeout::from_secs(1);
                while !op.usb_sts.readf(USB_STS_HCH) {
                    timeout.run().map_err(|()| {
                        log::error!("timeout on USB_STS_HCH");
                        Error::new(EIO)
                    })?;
                }
            }

            {
                debug!("Resetting the xHC.");
                op.usb_cmd.writef(USB_CMD_HCRST, true);
                let timeout = Timeout::from_secs(1);
                while op.usb_cmd.readf(USB_CMD_HCRST) {
                    timeout.run().map_err(|()| {
                        log::error!("timeout on USB_CMD_HCRST");
                        Error::new(EIO)
                    })?;
                }
            }

            debug!("Reading max slots.");

            let max_slots = cap.max_slots();
            let max_ports = cap.max_ports();

            debug!("xHC max slots: {}, max ports: {}", max_slots, max_ports);
            (max_slots, max_ports)
        };

        //Get the address of the port register table
        let port_base = op_base + 0x400;
        let ports =
            unsafe { slice::from_raw_parts_mut(port_base as *mut Port, max_ports as usize) };
        debug!("PORT BASE {:X}", port_base);

        //Get the address of the dorbell register table
        let db_base = address + cap.db_offset.read() as usize;
        let dbs = unsafe { slice::from_raw_parts_mut(db_base as *mut Doorbell, 256) };
        debug!("DOORBELL REGS BASE {:X}", db_base);

        let run_base = address + cap.rts_offset.read() as usize;
        let run = unsafe { &mut *(run_base as *mut RuntimeRegs) };
        debug!("RUNTIME REGS BASE {:X}", run_base);

        // Create the command ring with 4096 / 16 (TRB size) entries, so that it uses all of the
        // DMA allocation (which is at least a 4k page).
        let entries_per_page = PAGE_SIZE / size_of::<Trb>();
        let cmd = Ring::new::<N>(cap.ac64(), entries_per_page, true)?;

        let (irq_reactor_sender, irq_reactor_receiver) = crossbeam_channel::unbounded();

        let (device_enumerator_sender, device_enumerator_receiver) = crossbeam_channel::unbounded();

        let mut drivers_config = DriversConfig { drivers: vec![] };
        for file in config::config("xhcid").expect("xhcid: Failed to read driver configs") {
            let config = match fs::read(&file) {
                Ok(config) => config,
                Err(err) => {
                    println!("xhcid: Failed to read config {}: {err}", file.display());
                    continue;
                }
            };
            let config = match toml::from_slice::<DriversConfig>(&config) {
                Ok(config) => config,
                Err(err) => {
                    println!("xhcid: Failed to parse config {}: {err}", file.display());
                    continue;
                }
            };
            drivers_config.drivers.extend(config.drivers);
        }

        let mut xhci = Self {
            base: address as *const u8,

            cap,
            //page_size,
            op: Mutex::new(op),
            ports: Mutex::new(ports),
            dbs: Arc::new(Mutex::new(dbs)),
            run: Mutex::new(run),

            dev_ctx: DeviceContextList::new(cap.ac64(), max_slots)?,
            scratchpad_buf_arr: None, // initialized in init()

            cmd: Mutex::new(cmd),
            primary_event_ring: Mutex::new(EventRing::new::<N>(cap.ac64())?),
            drivers_config,
            handles: CHashMap::new(),
            next_handle: AtomicUsize::new(0),
            port_states: CHashMap::new(),
            drivers: CHashMap::new(),
            scheme_name,

            interrupt_method,
            pcid_handle: Mutex::new(pcid_handle),

            irq_reactor: Mutex::new(None),
            irq_reactor_sender,
            irq_reactor_receiver,
            device_enumerator: Mutex::new(None),
            device_enumerator_sender,
            device_enumerator_receiver,
        };

        xhci.init(max_slots)?;

        Ok(xhci)
    }

    pub fn init(&mut self, max_slots: u8) -> Result<()> {
        // Set run/stop to 0
        debug!("Stopping xHC.");
        self.op.get_mut().unwrap().usb_cmd.writef(USB_CMD_RS, false);

        // Warm reset
        {
            debug!("Reset xHC");
            let timeout = Timeout::from_secs(1);
            self.op
                .get_mut()
                .unwrap()
                .usb_cmd
                .writef(USB_CMD_HCRST, true);
            while self.op.get_mut().unwrap().usb_cmd.readf(USB_CMD_HCRST) {
                timeout.run().map_err(|()| {
                    log::error!("timeout on USB_CMD_HCRST");
                    Error::new(EIO)
                })?;
            }
        }

        // Set enabled slots
        debug!("Setting enabled slots to {}.", max_slots);
        self.op
            .get_mut()
            .unwrap()
            .config
            .write(u32::from(max_slots));
        debug!(
            "Enabled Slots: {}",
            self.op.get_mut().unwrap().config.read() & 0xFF
        );

        // Set device context address array pointer
        let dcbaap = self.dev_ctx.dcbaap();
        debug!("Writing DCBAAP: {:X}", dcbaap);
        self.op.get_mut().unwrap().dcbaap_low.write(dcbaap as u32);
        self.op
            .get_mut()
            .unwrap()
            .dcbaap_high
            .write((dcbaap as u64 >> 32) as u32);

        // Set command ring control register
        let crcr = self.cmd.get_mut().unwrap().register();
        assert_eq!(crcr & 0xFFFF_FFFF_FFFF_FFC1, crcr, "unaligned CRCR");
        debug!("Writing CRCR: {:X}", crcr);
        self.op.get_mut().unwrap().crcr_low.write(crcr as u32);
        self.op
            .get_mut()
            .unwrap()
            .crcr_high
            .write((crcr as u64 >> 32) as u32);

        // Set event ring segment table registers
        debug!(
            "Interrupter 0: {:p}",
            self.run.get_mut().unwrap().ints.as_ptr()
        );
        {
            let int = &mut self.run.get_mut().unwrap().ints[0];

            let erstz = 1;
            debug!("Writing ERSTZ: {}", erstz);
            int.erstsz.write(erstz);

            let erdp = self.primary_event_ring.get_mut().unwrap().erdp();
            debug!("Writing ERDP: {:X}", erdp);
            int.erdp_low.write(erdp as u32 | (1 << 3));
            int.erdp_high.write((erdp as u64 >> 32) as u32);

            let erstba = self.primary_event_ring.get_mut().unwrap().erstba();
            debug!("Writing ERSTBA: {:X}", erstba);
            int.erstba_low.write(erstba as u32);
            int.erstba_high.write((erstba as u64 >> 32) as u32);

            debug!("Writing IMODC and IMODI: {} and {}", 0, 0);
            int.imod.write(0);

            debug!("Enabling Primary Interrupter.");
            int.iman.writef(1 << 1 | 1, true);
        }
        self.op
            .get_mut()
            .unwrap()
            .usb_cmd
            .writef(USB_CMD_INTE, true);

        // Setup the scratchpad buffers that are required for the xHC to function.
        self.setup_scratchpads()?;

        // Set run/stop to 1
        debug!("Starting xHC.");
        self.op.get_mut().unwrap().usb_cmd.writef(USB_CMD_RS, true);

        {
            debug!("Waiting for start request to complete.");
            let timeout = Timeout::from_secs(1);
            while self.op.get_mut().unwrap().usb_sts.readf(USB_STS_HCH) {
                timeout.run().map_err(|()| {
                    log::error!("timeout on USB_STS_HCH");
                    Error::new(EIO)
                })?;
            }
        }

        // Ring command doorbell
        debug!("Ringing command doorbell.");
        self.dbs.lock().unwrap()[0].write(0);

        debug!("XHCI initialized.");

        self.op.get_mut().unwrap().set_cie(self.cap.cic());

        self.print_port_capabilities();

        Ok(())
    }

    pub fn setup_scratchpads(&mut self) -> Result<()> {
        let buf_count = self.cap.max_scratchpad_bufs();

        if buf_count == 0 {
            return Ok(());
        }
        let scratchpad_buf_arr = ScratchpadBufferArray::new::<N>(self.cap.ac64(), buf_count)?;
        self.dev_ctx.dcbaa[0] = scratchpad_buf_arr.register() as u64;
        debug!(
            "Setting up {} scratchpads, at {:#0x}",
            buf_count,
            scratchpad_buf_arr.register()
        );
        self.scratchpad_buf_arr = Some(scratchpad_buf_arr);

        Ok(())
    }

    pub fn get_pls(&self, port_id: PortId) -> u8 {
        let mut ports = self.ports.lock().unwrap();
        let port = ports.get_mut(port_id.root_hub_port_index()).unwrap();
        port.state()
    }

    pub fn print_port_capabilities(&self) {
        let len;
        {
            let mut ports = self.ports.lock().unwrap();
            len = ports.len();
        }

        for root_hub_port_num in 1..=(len as u8) {
            let port_id = PortId {
                root_hub_port_num,
                route_string: 0,
            };

            let state = self.get_pls(port_id);
            let mut flags;
            {
                let mut ports = self.ports.lock().unwrap();

                flags = ports[port_id.root_hub_port_index()].flags();
            }

            match self.supported_protocol(port_id) {
                None => {
                    warn!("No detected supported protocol for port {}", port_id);
                }
                Some(protocol) => {
                    debug!(
                        "Port {} is a USB {}.{} port with slot type {} and in current state {}: {:?}",
                        port_id,
                        protocol.rev_major(),
                        protocol.rev_minor(),
                        protocol.proto_slot_ty(),
                        state,
                        flags
                    );
                }
            };
        }
    }
}
