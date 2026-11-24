use std::cmp;

use common::io::Io;
use log::{debug, warn};
use syscall::{Error, Result, EBADF, EBADFD, EBADMSG, EIO};

use crate::driver_interface::{ConfigureEndpointsReq, DevDesc, EndpDesc};
use crate::usb;
use crate::usb::endpoint::EndpointTy;
use crate::xhci::context::StreamContextArray;
use crate::xhci::extended::ProtocolSpeed;
use crate::xhci::ring::Ring;
use crate::xhci::scheme::EndpIfState;
use crate::xhci::transfer::handle_event_trb;
use crate::xhci::{EndpNum, EndpointState, PortId, PortState, RingOrStreams, Xhci};

impl<const N: usize> Xhci<N> {
    async fn set_configuration(&self, port: PortId, config: u8) -> Result<()> {
        debug!("Setting configuration value {} to port {}", config, port);
        self.device_req_no_data(port, usb::Setup::set_configuration(config))
            .await
    }

    async fn set_interface(
        &self,
        port: PortId,
        interface_num: u8,
        alternate_setting: u8,
    ) -> Result<()> {
        debug!(
            "Setting interface value {} (alternate setting {}) to port {}",
            interface_num, alternate_setting, port
        );
        self.device_req_no_data(
            port,
            usb::Setup::set_interface(interface_num, alternate_setting),
        )
        .await
    }

    fn endp_ctx_interval(speed_id: &ProtocolSpeed, endp_desc: &EndpDesc) -> u8 {
        /// Logarithmic (base 2) 125 µs periods per millisecond.
        const MILLISEC_PERIODS: u8 = 3;

        // TODO: Also check the Speed ID for superspeed(plus).
        if (speed_id.is_lowspeed() || speed_id.is_fullspeed()) && endp_desc.is_interrupt() {
            // The interval field has values 1-255, ranging from 1 ms to 255 ms.
            // TODO: This is correct, right?
            let last_power_of_two = 8 - endp_desc.interval.leading_zeros() as u8;
            last_power_of_two - 1 + MILLISEC_PERIODS
        } else if speed_id.is_fullspeed() && endp_desc.is_isoch() {
            // bInterval has values 1-16, ranging from 1 ms to 32,768 ms.
            endp_desc.interval - 1 + MILLISEC_PERIODS
        } else if (speed_id.is_fullspeed()
            || endp_desc.is_superspeed()
            || endp_desc.is_superspeedplus())
            && (endp_desc.is_interrupt() || endp_desc.is_isoch())
        {
            // bInterval has values 1-16, but ranging from 125 µs to 4096 ms.
            endp_desc.interval - 1
        } else {
            // This includes superspeed(plus) control and bulk endpoints in particular.
            0
        }
    }
    fn endp_ctx_max_burst(
        speed_id: &ProtocolSpeed,
        dev_desc: &DevDesc,
        endp_desc: &EndpDesc,
    ) -> u8 {
        if speed_id.is_highspeed() && (endp_desc.is_interrupt() || endp_desc.is_isoch()) {
            assert_eq!(dev_desc.major_version(), 2);
            ((endp_desc.max_packet_size & 0x0C00) >> 11) as u8
        } else if endp_desc.is_superspeed() {
            endp_desc.max_burst()
        } else {
            0
        }
    }
    fn endp_ctx_max_packet_size(endp_desc: &EndpDesc) -> u16 {
        // TODO: Control endpoint? Encoding?
        endp_desc.max_packet_size & 0x07FF
    }
    fn endp_ctx_max_esit_payload(
        speed_id: &ProtocolSpeed,
        dev_desc: &DevDesc,
        endp_desc: &EndpDesc,
        max_packet_size: u16,
        max_burst_size: u8,
    ) -> u32 {
        const KIB: u32 = 1024;

        if dev_desc.major_version() == 2 && endp_desc.is_periodic() {
            u32::from(max_packet_size) * (u32::from(max_burst_size) + 1)
        } else if endp_desc.has_ssp_companion() {
            endp_desc.sspc.as_ref().unwrap().bytes_per_interval
        } else if endp_desc.ssc.is_some() {
            u32::from(endp_desc.ssc.as_ref().unwrap().bytes_per_interval)
        } else if speed_id.is_fullspeed() && endp_desc.is_interrupt() {
            64
        } else if speed_id.is_fullspeed() && endp_desc.is_isoch() {
            1 * KIB
        } else if (speed_id.is_highspeed() && (endp_desc.is_interrupt() || endp_desc.is_isoch()))
            || endp_desc.is_superspeed() && endp_desc.is_interrupt()
        {
            3 * KIB
        } else if endp_desc.is_superspeed() && endp_desc.is_isoch() {
            48 * KIB
        } else {
            // TODO: Is "maximum allowed" ESIT payload, the same as "maximum" ESIT payload.
            0
        }
    }

    fn port_state(&self, port: PortId) -> Result<chashmap::ReadGuard<'_, PortId, PortState<N>>> {
        self.port_states.get(&port).ok_or(Error::new(EBADF))
    }
    async fn configure_endpoints_once(
        &self,
        port: PortId,
        req: &ConfigureEndpointsReq,
    ) -> Result<()> {
        let (endp_desc_count, new_context_entries, configuration_value) = {
            let mut port_state = self.port_states.get_mut(&port).ok_or(Error::new(EBADFD))?;

            port_state.cfg_idx = Some(req.config_desc);

            let config_desc = port_state
                .dev_desc
                .as_ref()
                .unwrap()
                .config_descs
                .iter()
                .find(|desc| desc.configuration_value == req.config_desc)
                .ok_or(Error::new(EBADFD))?;

            //TODO: USE ENDPOINTS FROM ALL INTERFACES
            let mut endp_desc_count = 0;
            let mut new_context_entries = 1;
            for if_desc in config_desc.interface_descs.iter() {
                for endpoint in if_desc.endpoints.iter() {
                    endp_desc_count += 1;
                    let entry = endpoint.xhci_dci();
                    if entry > new_context_entries {
                        new_context_entries = entry;
                    }
                }
            }
            new_context_entries += 1;

            if endp_desc_count >= 31 {
                warn!("endpoints length {} >= 31", endp_desc_count);
                return Err(Error::new(EIO));
            }

            (
                endp_desc_count,
                new_context_entries,
                config_desc.configuration_value,
            )
        };
        let lec = self.cap.lec();
        let log_max_psa_size = self.cap.max_psa_size();

        let port_speed_id = self.ports.lock().unwrap()[port.root_hub_port_index()].speed();
        let speed_id: &ProtocolSpeed = self.lookup_psiv(port, port_speed_id).ok_or_else(|| {
            warn!("no speed_id");
            Error::new(EIO)
        })?;

        {
            let port_state = self.port_states.get(&port).ok_or(Error::new(EBADFD))?;
            let mut input_context = port_state.input_context.lock().unwrap();

            // Configure the slot context as well, which holds the last index of the endp descs.
            input_context.add_context.write(1);
            input_context.drop_context.write(0);

            const CONTEXT_ENTRIES_MASK: u32 = 0xF800_0000;
            const CONTEXT_ENTRIES_SHIFT: u8 = 27;

            const HUB_PORTS_MASK: u32 = 0xFF00_0000;
            const HUB_PORTS_SHIFT: u8 = 24;

            let mut current_slot_a = input_context.device.slot.a.read();
            let mut current_slot_b = input_context.device.slot.b.read();

            // Set context entries
            current_slot_a &= !CONTEXT_ENTRIES_MASK;
            current_slot_a |=
                (u32::from(new_context_entries) << CONTEXT_ENTRIES_SHIFT) & CONTEXT_ENTRIES_MASK;

            // Set hub data
            current_slot_a &= !(1 << 26);
            current_slot_b &= !HUB_PORTS_MASK;
            if let Some(hub_ports) = req.hub_ports {
                current_slot_a |= 1 << 26;
                current_slot_b |= (u32::from(hub_ports) << HUB_PORTS_SHIFT) & HUB_PORTS_MASK;
            }

            input_context.device.slot.a.write(current_slot_a);
            input_context.device.slot.b.write(current_slot_b);

            let control = if self.op.lock().unwrap().cie() {
                (u32::from(req.alternate_setting.unwrap_or(0)) << 16)
                    | (u32::from(req.interface_desc.unwrap_or(0)) << 8)
                    | u32::from(configuration_value)
            } else {
                0
            };
            input_context.control.write(control);
        }

        for endp_idx in 0..endp_desc_count as u8 {
            let endp_num = EndpNum::new(endp_idx + 1);

            let mut port_state = self.port_states.get_mut(&port).ok_or(Error::new(EBADFD))?;
            let dev_desc = port_state.dev_desc.as_ref().unwrap();
            let endp_desc = port_state.get_endp_desc(endp_num).ok_or_else(|| {
                warn!("failed to find endpoint {}", endp_num.get());
                Error::new(EIO)
            })?;

            log::debug!("found endpoint {:?}: {:X?}", endp_num, endp_desc);
            let endp_addr = endp_desc.address;
            let endp_xhci_dci = endp_desc.xhci_dci();

            let usb_log_max_streams = endp_desc.log_max_streams();

            // TODO: Secondary streams.
            let primary_streams = if let Some(log_max_streams) = usb_log_max_streams {
                // TODO: Can streams-capable be configured to not use streams?
                if log_max_psa_size != 0 {
                    cmp::min(u8::from(log_max_streams), log_max_psa_size + 1) - 1
                } else {
                    0
                }
            } else {
                0
            };
            let linear_stream_array = if primary_streams != 0 { true } else { false };

            // TODO: Interval related fields
            // TODO: Max ESIT payload size.

            let mult = endp_desc.isoch_mult(lec);

            let max_packet_size = Self::endp_ctx_max_packet_size(endp_desc);
            let max_burst_size = Self::endp_ctx_max_burst(speed_id, dev_desc, endp_desc);

            let max_esit_payload = Self::endp_ctx_max_esit_payload(
                speed_id,
                dev_desc,
                endp_desc,
                max_packet_size,
                max_burst_size,
            );
            let max_esit_payload_lo = max_esit_payload as u16;
            let max_esit_payload_hi = ((max_esit_payload & 0x00FF_0000) >> 16) as u8;

            let interval = Self::endp_ctx_interval(speed_id, endp_desc);

            let max_error_count = 3;
            let ep_ty = endp_desc.xhci_ep_type()?;
            let host_initiate_disable = false;

            // TODO: Maybe this value is out of scope for xhcid, because the actual usb device
            // driver probably knows better. The spec says that the initial value should be 8 bytes
            // for control, 1KiB for interrupt and 3KiB for bulk and isoch.
            let avg_trb_len: u16 = match endp_desc.ty() {
                EndpointTy::Ctrl => {
                    warn!("trying to use control endpoint");
                    return Err(Error::new(EIO)); // only endpoint zero is of type control, and is configured separately with the address device command.
                }
                EndpointTy::Bulk | EndpointTy::Isoch => 3072, // 3 KiB
                EndpointTy::Interrupt => 1024,                // 1 KiB
            };

            assert_eq!(ep_ty & 0x7, ep_ty);
            assert_eq!(mult & 0x3, mult);
            assert_eq!(max_error_count & 0x3, max_error_count);
            assert_ne!(ep_ty, 0); // 0 means invalid.

            let ring_ptr = if usb_log_max_streams.is_some() {
                let mut array =
                    StreamContextArray::new::<N>(self.cap.ac64(), 1 << (primary_streams + 1))?;

                // TODO: Use as many stream rings as needed.
                array.add_ring::<N>(self.cap.ac64(), 1, true)?;
                let array_ptr = array.register();

                assert_eq!(
                    array_ptr & 0xFFFF_FFFF_FFFF_FF81,
                    array_ptr,
                    "stream ctx ptr not aligned to 16 bytes"
                );
                port_state.endpoint_states.insert(
                    endp_num,
                    EndpointState {
                        transfer: RingOrStreams::Streams(array),
                        driver_if_state: EndpIfState::Init,
                    },
                );

                array_ptr
            } else {
                let ring = Ring::new::<N>(self.cap.ac64(), 16, true)?;
                let ring_ptr = ring.register();

                assert_eq!(
                    ring_ptr & 0xFFFF_FFFF_FFFF_FF81,
                    ring_ptr,
                    "ring pointer not aligned to 16 bytes"
                );
                port_state.endpoint_states.insert(
                    endp_num,
                    EndpointState {
                        transfer: RingOrStreams::Ring(ring),
                        driver_if_state: EndpIfState::Init,
                    },
                );
                ring_ptr
            };
            assert_eq!(primary_streams & 0x1F, primary_streams);

            let mut input_context = port_state.input_context.lock().unwrap();
            input_context.add_context.writef(1 << endp_xhci_dci, true);

            let endp_i = endp_xhci_dci as usize - 1;
            input_context.device.endpoints[endp_i].a.write(
                u32::from(mult) << 8
                    | u32::from(primary_streams) << 10
                    | u32::from(linear_stream_array) << 15
                    | u32::from(interval) << 16
                    | u32::from(max_esit_payload_hi) << 24,
            );
            input_context.device.endpoints[endp_i].b.write(
                max_error_count << 1
                    | u32::from(ep_ty) << 3
                    | u32::from(host_initiate_disable) << 7
                    | u32::from(max_burst_size) << 8
                    | u32::from(max_packet_size) << 16,
            );

            input_context.device.endpoints[endp_i]
                .trl
                .write(ring_ptr as u32);
            input_context.device.endpoints[endp_i]
                .trh
                .write((ring_ptr >> 32) as u32);

            input_context.device.endpoints[endp_i]
                .c
                .write(u32::from(avg_trb_len) | (u32::from(max_esit_payload_lo) << 16));

            log::debug!(
                "initialized endpoint {:?} address {:#X} with index {}",
                endp_num,
                endp_addr,
                endp_i,
            );
        }

        {
            let port_state = self.port_states.get(&port).ok_or(Error::new(EBADFD))?;
            let slot = port_state.slot;
            let input_context_physical = port_state.input_context.lock().unwrap().physical();

            let (event_trb, command_trb) = self
                .execute_command(|trb, cycle| {
                    trb.configure_endpoint(slot, input_context_physical, cycle)
                })
                .await;

            //self.event_handler_finished();

            handle_event_trb("CONFIGURE_ENDPOINT", &event_trb, &command_trb)?;
        }

        // Tell the device about this configuration.
        self.set_configuration(port, configuration_value).await?;

        Ok(())
    }

    pub(in crate::xhci) async fn configure_endpoints(
        &self,
        port: PortId,
        json_buf: &[u8],
    ) -> Result<()> {
        let mut req: ConfigureEndpointsReq =
            serde_json::from_slice(json_buf).or(Err(Error::new(EBADMSG)))?;

        debug!(
            "Running configure endpoints command, at port {}, request: {:?}",
            port, req
        );

        if req.interface_desc.is_some() != req.alternate_setting.is_some() {
            return Err(Error::new(EBADMSG));
        }

        let already_configured = {
            let port_state = self.port_states.get(&port).ok_or(Error::new(EBADFD))?;
            port_state.cfg_idx == Some(req.config_desc)
        };

        if !already_configured {
            self.configure_endpoints_once(port, &req).await?;
        }

        if let Some(interface_num) = req.interface_desc {
            if let Some(alternate_setting) = req.alternate_setting {
                // We should only set the interface alternate setting if there are different ones
                // Some devices stall if there is only one alternate setting (0) and set_interface is used
                let must_set_interface = {
                    let port_state = self.port_states.get(&port).ok_or(Error::new(EBADFD))?;
                    port_state
                        .dev_desc
                        .as_ref()
                        .and_then(|x| {
                            x.config_descs
                                .iter()
                                .find(|x| x.configuration_value == req.config_desc)
                        })
                        .map_or(false, |x| {
                            x.interface_descs.iter().any(|x| {
                                x.number == interface_num
                                    && x.alternate_setting != alternate_setting
                            })
                        })
                };
                if must_set_interface {
                    self.set_interface(port, interface_num, alternate_setting)
                        .await?;
                }
            }
        }

        Ok(())
    }
}
