use std::convert::TryFrom;
use std::ops::{Add, Deref, Div, Rem};
use std::{cmp, fmt};

use common::dma::Dma;
use common::io::Io;
use log::{debug, error, trace};
use syscall::{Error, Result, EBADF, EBADFD, EINVAL, EIO, ENOSYS};

use self::irq_reactor::{EventDoorbell, RingId};
use self::trb::{TransferKind, Trb, TrbCompletionCode, TrbType};
use super::{usb, EndpNum, EndpointState, PortId, Xhci};
use crate::driver_interface::{EndpDesc, EndpDirection, PortReqDirection};

pub(super) mod event;
pub mod irq_reactor;
pub(super) mod ring;
pub(super) mod trb;

pub enum ControlFlow {
    Continue,
    Break,
}

#[derive(Clone, Copy)]
pub(super) struct DmaSliceDbg<'a, T>(pub(super) &'a Dma<[T]>);

impl<'a, T> fmt::Debug for DmaSliceDbg<'a, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let DmaSliceDbg(dma) = self;

        f.debug_struct("Dma")
            .field("phys_ptr", &(dma.physical() as *const u8))
            .field("virt_ptr", &(dma.deref().as_ptr() as *const u8))
            .field("length", &(dma.len() * size_of::<T>()))
            .finish()
    }
}

impl<const N: usize> Xhci<N> {
    /// Pushes a command TRB to the command ring, rings the doorbell, and then awaits its Command
    /// Completion Event.
    ///
    /// # Locking
    /// This function will lock `Xhci::cmd` and `Xhci::dbs`.
    pub async fn execute_command<F: FnOnce(&mut Trb, bool)>(&self, f: F) -> (Trb, Trb) {
        //TODO: find out why this bit is set earlier!
        if self.interrupt_is_pending(0) {
            debug!("The EHB bit is already set!");
            //self.force_clear_interrupt(0);
        }

        let next_event = {
            let mut command_ring = self.cmd.lock().unwrap();
            let (cmd_index, cycle) = (command_ring.next_index(), command_ring.cycle);

            debug!("Sending command with cycle bit {}", u8::from(cycle));

            {
                let command_trb = &mut command_ring.trbs[cmd_index];
                f(command_trb, cycle);
            }

            // get the future here before awaiting, to destroy the lock before deadlock
            let command_trb = &command_ring.trbs[cmd_index];
            self.next_command_completion_event_trb(
                &*command_ring,
                command_trb,
                EventDoorbell::new(self, 0, 0),
            )
        };

        let trbs = next_event.await;
        let event_trb = trbs.event_trb;
        let command_trb = trbs.src_trb.expect("Command completion event TRBs shall always have a valid pointer to a valid source command TRB");

        assert_eq!(
            event_trb.trb_type(),
            TrbType::CommandCompletion as u8,
            "The IRQ reactor (or the xHC) gave an invalid event TRB"
        );

        (event_trb, command_trb)
    }
    pub async fn execute_control_transfer<D>(
        &self,
        port_num: PortId,
        setup: usb::Setup,
        tk: TransferKind,
        name: &str,
        mut d: D,
    ) -> Result<Trb>
    where
        D: FnMut(&mut Trb, bool) -> ControlFlow,
    {
        let future = {
            let mut port_state = self.port_state_mut(port_num)?;
            let slot = port_state.slot;

            let mut endpoint_state = port_state
                .endpoint_states
                .get_mut(&EndpNum::new(0))
                .ok_or(Error::new(EIO))?;

            let ring = endpoint_state.ring().ok_or(Error::new(EIO))?;

            let first_index = ring.next_index();
            let (cmd, cycle) = (&mut ring.trbs[first_index], ring.cycle);
            cmd.setup(setup, tk, cycle);

            if tk != TransferKind::NoData {
                loop {
                    let (trb, cycle) = ring.next();
                    match d(trb, cycle) {
                        ControlFlow::Break => break,
                        ControlFlow::Continue => continue,
                    }
                }
            }

            let last_index = ring.next_index();
            let (cmd, cycle) = (&mut ring.trbs[last_index], ring.cycle);

            let interrupter = 0;
            // When the data stage is in, the status stage must be out
            let input = tk != TransferKind::In;
            let ioc = true;
            let ch = false;
            let ent = false;
            cmd.status(interrupter, input, ioc, ch, ent, cycle);

            self.next_transfer_event_trb(
                RingId::default_control_pipe(port_num),
                ring,
                &ring.trbs[first_index],
                &ring.trbs[last_index],
                EventDoorbell::new(self, usize::from(slot), Self::def_control_endp_doorbell()),
            )
        };

        let trbs = future.await;
        let event_trb = trbs.event_trb;
        let status_trb = trbs.src_trb.ok_or(Error::new(EIO))?;

        handle_transfer_event_trb("CONTROL_TRANSFER", &event_trb, &status_trb)?;

        //self.event_handler_finished();

        Ok(event_trb)
    }
    /// NOTE: There has to be AT LEAST one successful invocation of `d`, that actually updates the
    /// TRB (it could be a NO-OP in the worst case).
    /// The function is also required to set the Interrupt on Completion flag, or this function
    /// will never complete.
    pub async fn execute_transfer<D>(
        &self,
        port_num: PortId,
        endp_num: EndpNum,
        stream_id: u16,
        name: &str,
        mut d: D,
    ) -> Result<Trb>
    where
        D: FnMut(&mut Trb, bool) -> ControlFlow,
    {
        let mut port_state = self.port_state_mut(port_num)?;

        let slot = port_state.slot;

        let (doorbell_data_stream, doorbell_data_no_stream) = {
            let endp_desc = port_state
                .get_endp_desc(endp_num)
                .ok_or(Error::new(EBADFD))?;

            //TODO: clean this up
            (
                Self::endp_doorbell(endp_num, endp_desc, stream_id),
                Self::endp_doorbell(endp_num, endp_desc, 0),
            )
        };

        let endp_state = port_state
            .endpoint_states
            .get_mut(&endp_num)
            .ok_or(Error::new(EBADF))?;

        let (has_streams, ring) = match endp_state {
            EndpointState {
                transfer: super::RingOrStreams::Ring(ref mut ring),
                ..
            } => (false, ring),
            EndpointState {
                transfer: super::RingOrStreams::Streams(stream_ctx_array),
                ..
            } => (
                true,
                stream_ctx_array
                    .rings
                    .get_mut(&1)
                    .ok_or(Error::new(EBADF))?,
            ),
        };

        let future = loop {
            let last_index = ring.next_index();
            let (trb, cycle) = (&mut ring.trbs[last_index], ring.cycle);

            match d(trb, cycle) {
                ControlFlow::Break => {
                    break self.next_transfer_event_trb(
                        RingId {
                            port: port_num,
                            endpoint_num: endp_num,
                            stream_id,
                        },
                        ring,
                        //TODO: find first TRB
                        &ring.trbs[last_index],
                        &ring.trbs[last_index],
                        EventDoorbell::new(
                            self,
                            usize::from(slot),
                            if has_streams {
                                doorbell_data_stream
                            } else {
                                doorbell_data_no_stream
                            },
                        ),
                    );
                }
                ControlFlow::Continue => continue,
            }
        };

        drop(port_state);

        let trbs = future.await;
        let event_trb = trbs.event_trb;
        let transfer_trb = trbs.src_trb.ok_or(Error::new(EIO))?;

        handle_transfer_event_trb("EXECUTE_TRANSFER", &event_trb, &transfer_trb)?;

        // FIXME: EDTLA if event data was set
        if event_trb.completion_code() != TrbCompletionCode::ShortPacket as u8
            && event_trb.transfer_length() != 0
        {
            error!("Event trb didn't yield a short packet, but some bytes were not transferred");
            return Err(Error::new(EIO));
        }

        // TODO: Handle event data
        trace!("EVENT DATA: {:?}", event_trb.event_data());

        Ok(event_trb)
    }
    pub(super) async fn device_req_no_data(&self, port: PortId, req: usb::Setup) -> Result<()> {
        trace!("DEVICE_REQ_NO_DATA port {}, req: {:?}", port, req);

        self.execute_control_transfer(
            port,
            req,
            TransferKind::NoData,
            "DEVICE_REQ_NO_DATA",
            |_, _| ControlFlow::Break,
        )
        .await?;
        Ok(())
    }

    fn port_state_mut(
        &self,
        port: PortId,
    ) -> Result<chashmap::WriteGuard<'_, PortId, super::PortState<N>>> {
        self.port_states.get_mut(&port).ok_or(Error::new(EBADF))
    }

    pub(super) async fn transfer_read(
        &self,
        port_num: PortId,
        endp_num: EndpNum,
        buf: &mut [u8],
    ) -> Result<(u8, u32)> {
        if buf.is_empty() {
            return Err(Error::new(EINVAL));
        }
        let dma_buffer = unsafe { self.alloc_dma_zeroed_unsized(buf.len())? };

        let (completion_code, bytes_transferred, dma_buffer) = self
            .transfer(
                port_num,
                endp_num,
                Some(dma_buffer),
                PortReqDirection::DeviceToHost,
            )
            .await?;

        buf.copy_from_slice(&*dma_buffer.as_ref().unwrap());
        Ok((completion_code, bytes_transferred))
    }
    pub(super) async fn transfer_write(
        &self,
        port_num: PortId,
        endp_num: EndpNum,
        sbuf: &[u8],
    ) -> Result<(u8, u32)> {
        if sbuf.is_empty() {
            return Err(Error::new(EINVAL));
        }
        let mut dma_buffer = unsafe { self.alloc_dma_zeroed_unsized(sbuf.len()) }?;
        dma_buffer.copy_from_slice(sbuf);

        trace!(
            "TRANSFER_WRITE port {} ep {}, buffer at {:p}, size {}, dma buffer {:?}",
            port_num,
            endp_num.get(),
            sbuf.as_ptr(),
            sbuf.len(),
            DmaSliceDbg(&dma_buffer)
        );

        let (completion_code, bytes_transferred, _) = self
            .transfer(
                port_num,
                endp_num,
                Some(dma_buffer),
                PortReqDirection::HostToDevice,
            )
            .await?;
        Ok((completion_code, bytes_transferred))
    }
    pub const fn def_control_endp_doorbell() -> u32 {
        1
    }
    pub(super) fn endp_doorbell(endp_num: EndpNum, desc: &EndpDesc, stream_id: u16) -> u32 {
        let db_target = desc.xhci_dci();
        let db_task_id: u16 = stream_id;

        (u32::from(db_task_id) << 16) | u32::from(db_target)
    }
    // TODO: Rename DeviceReqData to something more general.
    pub(super) async fn transfer(
        &self,
        port_num: PortId,
        endp_num: EndpNum,
        dma_buf: Option<Dma<[u8]>>,
        direction: PortReqDirection,
    ) -> Result<(u8, u32, Option<Dma<[u8]>>)> {
        let mut port_state = self
            .port_states
            .get_mut(&port_num)
            .ok_or(Error::new(EBADFD))?;

        let endp_desc: &EndpDesc = port_state
            .get_endp_desc(endp_num)
            .ok_or(Error::new(EBADFD))?;

        let direction = endp_desc.direction();

        if endp_desc.is_isoch() {
            return Err(Error::new(ENOSYS));
        }

        if EndpDirection::from(direction) != endp_desc.direction() {
            return Err(Error::new(EBADF));
        }

        let max_packet_size = endp_desc.max_packet_size;
        let max_transfer_size = 65536u32;

        let (buffer, idt, estimated_td_size) = {
            let (buffer, idt) = if dma_buf.as_ref().map(|buf| buf.len()).unwrap_or(0) <= 8
                && max_packet_size >= 8
                && direction != EndpDirection::In
            {
                dma_buf
                    .as_ref()
                    .map(|sbuf| {
                        let mut bytes = [0u8; 8];
                        bytes[..sbuf.len()].copy_from_slice(&sbuf);
                        (u64::from_le_bytes(bytes), true)
                    })
                    .unwrap_or((0, false))
            } else {
                (
                    dma_buf.as_ref().map(|dma| dma.physical()).unwrap_or(0) as u64,
                    false,
                )
            };
            let estimated_td_size = cmp::min(
                u8::try_from(
                    div_round_up(
                        dma_buf.as_ref().map(|buf| buf.len()).unwrap_or(0),
                        max_transfer_size as usize,
                    ) * size_of::<Trb>(),
                )
                .ok()
                .unwrap_or(0x1F),
                0x1F,
            ); // one trb per td
            (buffer, idt, estimated_td_size)
        };

        let stream_id = 1u16;

        let mut bytes_left = dma_buf.as_ref().map(|buf| buf.len()).unwrap_or(0);

        drop(port_state);

        let event = self
            .execute_transfer(
                port_num,
                endp_num,
                stream_id,
                "CUSTOM_TRANSFER",
                |trb, cycle| {
                    let len = cmp::min(bytes_left, max_transfer_size as usize) as u32;

                    // set the interrupt on completion (IOC) flag for the last trb.
                    let ioc = bytes_left <= max_transfer_size as usize;
                    let chain = !ioc;

                    let interrupter = 0;
                    let ent = false;
                    let isp = true;
                    let bei = false;
                    trb.normal(
                        buffer,
                        len,
                        cycle,
                        estimated_td_size,
                        interrupter,
                        ent,
                        isp,
                        chain,
                        ioc,
                        idt,
                        bei,
                    );

                    bytes_left -= len as usize;

                    if bytes_left != 0 {
                        ControlFlow::Continue
                    } else {
                        ControlFlow::Break
                    }
                },
            )
            .await?;
        //self.event_handler_finished();

        let bytes_transferred = dma_buf
            .as_ref()
            .map(|buf| buf.len() as u32 - event.transfer_length())
            .unwrap_or(0);

        Ok((event.completion_code(), bytes_transferred, dma_buf))
    }
    /// Notifies the xHC that the current event handler has finished, so that new interrupts can be
    /// sent. This is required after each invocation of `Self::execute_command`.
    ///
    /// # Locking
    /// This function locks `Xhci::run`.
    pub fn event_handler_finished(&self) {
        trace!("Event handler finished");
        // write 1 to EHB to clear it
        self.run.lock().unwrap().ints[0]
            .erdp_low
            .writef(1 << 3, true);
    }
}

pub fn handle_event_trb(name: &str, event_trb: &Trb, command_trb: &Trb) -> Result<()> {
    if event_trb.completion_code() == TrbCompletionCode::Success as u8 {
        Ok(())
    } else {
        error!(
            "{} command (TRB {:?}) failed with event trb {:?}",
            name, command_trb, event_trb
        );
        Err(Error::new(EIO))
    }
}
pub fn handle_transfer_event_trb(name: &str, event_trb: &Trb, transfer_trb: &Trb) -> Result<()> {
    if event_trb.completion_code() == TrbCompletionCode::Success as u8
        || event_trb.completion_code() == TrbCompletionCode::ShortPacket as u8
    {
        Ok(())
    } else {
        error!(
            "{} transfer {:?} failed with event {:?}",
            name, transfer_trb, event_trb
        );
        Err(Error::new(EIO))
    }
}

pub fn div_round_up<T>(a: T, b: T) -> T
where
    T: Add<Output = T> + Div<Output = T> + Rem<Output = T> + PartialEq + From<u8> + Copy,
{
    if a % b != T::from(0u8) {
        a / b + T::from(1u8)
    } else {
        a / b
    }
}
