use common::dma::Dma;
use common::io::Io;
use log::{debug, error, info, trace, warn};
use syscall::error::{Error, Result, EIO};

use crate::driver_interface::DevDesc;
use crate::xhci::context::{InputContext, SLOT_CONTEXT_STATE_MASK, SLOT_CONTEXT_STATE_SHIFT};
use crate::xhci::extended::ProtocolSpeed;
use crate::xhci::ring::Ring;
use crate::xhci::transfer::handle_event_trb;
use crate::xhci::trb::TrbCompletionCode;
use crate::xhci::{usb, PortId, Xhci};

impl<const N: usize> Xhci<N> {
    pub fn slot(&self, port_num: PortId) -> Result<u8> {
        Ok(self.port_states.get(&port_num).ok_or(Error::new(EIO))?.slot)
    }

    pub async fn enable_port_slot(&self, slot_ty: u8) -> Result<u8> {
        assert_eq!(slot_ty & 0x1F, slot_ty);

        let (event_trb, command_trb) = self
            .execute_command(|cmd, cycle| cmd.enable_slot(slot_ty, cycle))
            .await;

        trace!("Slot is enabled!");
        handle_event_trb("ENABLE_SLOT", &event_trb, &command_trb)?;
        //self.event_handler_finished();

        Ok(event_trb.event_slot())
    }
    pub async fn disable_port_slot(&self, slot: u8) -> Result<()> {
        trace!("Disable slot {}", slot);
        let (event_trb, command_trb) = self
            .execute_command(|cmd, cycle| cmd.disable_slot(slot, cycle))
            .await;

        handle_event_trb("DISABLE_SLOT", &event_trb, &command_trb)?;
        //self.event_handler_finished();

        Ok(())
    }

    pub fn slot_state(&self, slot: usize) -> u8 {
        ((self.dev_ctx.contexts[slot].slot.d.read() & SLOT_CONTEXT_STATE_MASK)
            >> SLOT_CONTEXT_STATE_SHIFT) as u8
    }
    pub async fn update_max_packet_size(
        &self,
        input_context: &mut Dma<InputContext<N>>,
        slot_id: u8,
        dev_desc: usb::DeviceDescriptor8Byte,
    ) -> Result<()> {
        let new_max_packet_size = if dev_desc.major_usb_vers() <= 2 {
            // For USB 2.0 and below, packet_size is in bytes
            u32::from(dev_desc.packet_size)
        } else {
            // For later USB versions, packet_size is the shift
            1u32 << dev_desc.packet_size
        };
        debug!(
            "Setting new packet size of slot {} to {}",
            slot_id, new_max_packet_size
        );
        let mut b = input_context.device.endpoints[0].b.read();
        b &= 0x0000_FFFF;
        b |= (new_max_packet_size) << 16;
        input_context.device.endpoints[0].b.write(b);

        let (event_trb, command_trb) = self
            .execute_command(|trb, cycle| {
                trb.evaluate_context(slot_id, input_context.physical(), false, cycle)
            })
            .await;

        handle_event_trb("EVALUATE_CONTEXT", &event_trb, &command_trb)?;
        //self.event_handler_finished();

        Ok(())
    }

    pub async fn update_default_control_pipe(
        &self,
        input_context: &mut Dma<InputContext<N>>,
        slot_id: u8,
        dev_desc: &DevDesc,
    ) -> Result<()> {
        debug!("Updating default control pipe!");
        input_context.add_context.write(1 << 1);
        input_context.drop_context.write(0);

        let new_max_packet_size = if dev_desc.major_version() <= 2 {
            // For USB 2.0 and below, packet_size is in bytes
            u32::from(dev_desc.packet_size)
        } else {
            // For later USB versions, packet_size is the shift
            1u32 << dev_desc.packet_size
        };
        let mut b = input_context.device.endpoints[0].b.read();
        b &= 0x0000_FFFF;
        b |= (new_max_packet_size) << 16;
        input_context.device.endpoints[0].b.write(b);

        let (event_trb, command_trb) = self
            .execute_command(|trb, cycle| {
                trb.evaluate_context(slot_id, input_context.physical(), false, cycle)
            })
            .await;
        debug!("Completed the command to update the default control pipe");

        handle_event_trb("EVALUATE_CONTEXT", &event_trb, &command_trb)?;
        //self.event_handler_finished();

        Ok(())
    }

    pub async fn address_device(
        &self,
        input_context: &mut Dma<InputContext<N>>,
        port: PortId,
        slot_ty: u8,
        slot: u8,
        protocol_speed: &ProtocolSpeed,
        speed: u8,
    ) -> Result<Ring> {
        // Collect MTT, parent port number, parent slot ID
        let mut mtt = false;
        let mut parent_hub_slot_id = 0u8;
        let mut parent_port_num = 0u8;
        if let Some((parent_port, port_num)) = port.parent() {
            match self.port_states.get(&parent_port) {
                Some(parent_state) => {
                    // parent info must be supplied if:
                    let mut needs_parent_info = false;
                    // 1. the device is low or full speed and connected through a high speed hub
                    //TODO: determine device speed (speed is not accurate as it comes from the port)
                    // 2. the device is superspeed and connected through a higher rank hub
                    //TODO: determine device speed (speed is not accurate as it comes from the port)
                    // For now, this is just set to true to force things to work
                    needs_parent_info = true;
                    if needs_parent_info {
                        parent_hub_slot_id = parent_state.slot;
                        parent_port_num = port_num;
                    }
                    info!(
                        "port {} parent_hub_slot_id {} parent_port_num {}",
                        port, parent_hub_slot_id, parent_port_num
                    );
                }
                None => {
                    warn!("port {} missing parent port {} state", port, parent_port);
                }
            }
        }

        let mut ring = Ring::new::<N>(self.cap.ac64(), 16, true)?;

        {
            input_context.add_context.write(1 << 1 | 1); // Enable the slot (zeroth bit) and the control endpoint (first bit).

            let route_string = port.route_string;
            let context_entries = 1u8;
            let hub = false;

            assert_eq!(route_string & 0x000F_FFFF, route_string);
            input_context.device.slot.a.write(
                route_string
                    | (u32::from(speed) << 20)
                    | (u32::from(mtt) << 25)
                    | (u32::from(hub) << 26)
                    | (u32::from(context_entries) << 27),
            );

            let max_exit_latency = 0u16;
            let root_hub_port_num = port.root_hub_port_num;
            let number_of_ports = 0u8;
            input_context.device.slot.b.write(
                u32::from(max_exit_latency)
                    | (u32::from(root_hub_port_num) << 16)
                    | (u32::from(number_of_ports) << 24),
            );

            // TODO
            let ttt = 0u8;
            let interrupter = 0u8;

            assert_eq!(ttt & 0b11, ttt);
            input_context.device.slot.c.write(
                u32::from(parent_hub_slot_id)
                    | (u32::from(parent_port_num) << 8)
                    | (u32::from(ttt) << 16)
                    | (u32::from(interrupter) << 22),
            );

            let max_error_count = 3u8; // recommended value according to the XHCI spec
            let ep_ty = 4u8; // control endpoint, bidirectional
            let max_packet_size: u32 =
                if protocol_speed.is_lowspeed() || protocol_speed.is_fullspeed() {
                    8
                } else if protocol_speed.is_highspeed() {
                    64
                } else {
                    512
                };
            let host_initiate_disable = false; // only applies to streams
            let max_burst_size = 0u8; // TODO

            assert_eq!(max_error_count & 0b11, max_error_count);
            input_context.device.endpoints[0].b.write(
                (u32::from(max_error_count) << 1)
                    | (u32::from(ep_ty) << 3)
                    | (u32::from(host_initiate_disable) << 7)
                    | (u32::from(max_burst_size) << 8)
                    | (u32::from(max_packet_size) << 16),
            );

            let dequeue_cycle_state = true;
            let tr = ring.register();
            input_context.device.endpoints[0]
                .trh
                .write((tr >> 32) as u32);
            input_context.device.endpoints[0]
                .trl
                .write((tr as u32) | u32::from(dequeue_cycle_state));

            // The default control pipe can always use 8 bytes
            let avg_trb_len = 8u8;
            input_context.device.endpoints[0]
                .c
                .write(u32::from(avg_trb_len));
        }

        let input_context_physical = input_context.physical();

        let (event_trb, _) = self
            .execute_command(|trb, cycle| {
                trb.address_device(slot, input_context_physical, false, cycle)
            })
            .await;

        if event_trb.completion_code() != TrbCompletionCode::Success as u8 {
            error!(
                "Failed to address device at slot {} (port {}), completion code 0x{:X}",
                slot,
                port,
                event_trb.completion_code()
            );
            //self.event_handler_finished();
            return Err(Error::new(EIO));
        }
        //self.event_handler_finished();

        Ok(ring)
    }
}
