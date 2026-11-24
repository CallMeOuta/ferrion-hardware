use common::dma::Dma;
use log::{debug, error, trace};
use smallvec::SmallVec;
use syscall::error::{Error, Result, EIO, ENOENT};

use crate::driver_interface::{
    ConfDesc, DevDesc, EndpDesc, IfDesc, SuperSpeedCmp, SuperSpeedPlusIsochCmp, UnknownDesc,
};
use crate::usb;
use crate::xhci::irq_reactor::{EventDoorbell, RingId};
use crate::xhci::transfer::handle_transfer_event_trb;
use crate::xhci::trb::TransferKind;
use crate::xhci::{port, EndpNum, PortId, Xhci};

// TODO: Even though the driver interface descriptors are originally intended for JSON, they should suffice... for
// now.

impl From<usb::EndpointDescriptor> for EndpDesc {
    fn from(d: usb::EndpointDescriptor) -> Self {
        Self {
            kind: d.kind,
            address: d.address,
            attributes: d.attributes,
            interval: d.interval,
            max_packet_size: d.max_packet_size,
            ssc: None,
            sspc: None,
        }
    }
}

impl From<usb::SuperSpeedCompanionDescriptor> for SuperSpeedCmp {
    fn from(d: usb::SuperSpeedCompanionDescriptor) -> Self {
        Self {
            kind: d.kind,
            attributes: d.attributes,
            bytes_per_interval: d.bytes_per_interval,
            max_burst: d.max_burst,
        }
    }
}
impl From<usb::SuperSpeedPlusIsochCmpDescriptor> for SuperSpeedPlusIsochCmp {
    fn from(r: usb::SuperSpeedPlusIsochCmpDescriptor) -> Self {
        Self {
            kind: r.kind,
            bytes_per_interval: r.bytes_per_interval,
        }
    }
}

/// Any descriptor that can be stored in the config desc "data" area.
#[derive(Debug)]
pub enum AnyDescriptor {
    // These are the ones that I have found, but there are more.
    Device(usb::DeviceDescriptor),
    Config(usb::ConfigDescriptor),
    Interface(usb::InterfaceDescriptor),
    Endpoint(usb::EndpointDescriptor),
    SuperSpeedCompanion(usb::SuperSpeedCompanionDescriptor),
    SuperSpeedPlusCompanion(usb::SuperSpeedPlusIsochCmpDescriptor),
    Unknown(Vec<u8>),
}

impl AnyDescriptor {
    fn parse(bytes: &[u8]) -> Option<(Self, usize)> {
        if bytes.len() < 2 {
            return None;
        }

        let len = bytes[0];
        let kind = bytes[1];

        if bytes.len() < len.into() {
            error!("Parsed length is bigger than buffer len");
            return None;
        }

        Some((
            match kind {
                1 => Self::Device(*plain::from_bytes(bytes).ok()?),
                2 => Self::Config(*plain::from_bytes(bytes).ok()?),
                4 => Self::Interface(*plain::from_bytes(bytes).ok()?),
                5 => Self::Endpoint(*plain::from_bytes(bytes).ok()?),
                48 => Self::SuperSpeedCompanion(*plain::from_bytes(bytes).ok()?),
                49 => Self::SuperSpeedPlusCompanion(*plain::from_bytes(bytes).ok()?),
                _ => Self::Unknown(bytes[..len as usize].iter().cloned().collect()),
            },
            len.into(),
        ))
    }
}

impl<const N: usize> Xhci<N> {
    /// Gets descriptors, before the port state is initiated.
    async fn get_desc_raw<T>(
        &self,
        port: PortId,
        slot: u8,
        kind: usb::DescriptorKind,
        value: u8,
        index: u16,
        desc: &mut Dma<T>,
    ) -> Result<()> {
        if self.interrupt_is_pending(0) {
            debug!("EHB is already set!");
            self.force_clear_interrupt(0);
        }
        let len = size_of::<T>();
        log::debug!(
            "get_desc_raw port {} slot {} kind {:?} value {} index {} len {}",
            port,
            slot,
            kind,
            value,
            index,
            len
        );

        let future = {
            let mut port_state = self.port_states.get_mut(&port).ok_or(Error::new(ENOENT))?;
            let ring = port_state
                .endpoint_states
                .get_mut(&EndpNum::new(0))
                .ok_or(Error::new(EIO))?
                .ring()
                .expect("no ring for the default control pipe");

            let first_index = ring.next_index();
            let (cmd, cycle) = (&mut ring.trbs[first_index], ring.cycle);
            cmd.setup(
                usb::Setup::get_descriptor(kind, value, index, len as u16),
                TransferKind::In,
                cycle,
            );

            let (cmd, cycle) = ring.next();
            cmd.data(desc.physical(), len as u16, true, cycle);

            let last_index = ring.next_index();
            let (cmd, cycle) = (&mut ring.trbs[last_index], ring.cycle);

            let interrupter = 0;
            // When the data stage is in, the status stage must be out
            let input = false;
            let ioc = true;
            let ch = false;
            let ent = false;
            cmd.status(interrupter, input, ioc, ch, ent, cycle);

            self.next_transfer_event_trb(
                RingId::default_control_pipe(port),
                &ring,
                &ring.trbs[first_index],
                &ring.trbs[last_index],
                EventDoorbell::new(self, usize::from(slot), Self::def_control_endp_doorbell()),
            )
        };

        debug!("Waiting for the next transfer event TRB...");
        let trbs = future.await;
        let event_trb = trbs.event_trb;
        let status_trb = trbs.src_trb.ok_or(Error::new(EIO))?;
        trace!("Handling the transfer event TRB!");
        handle_transfer_event_trb("GET_DESC", &event_trb, &status_trb)?;

        //self.event_handler_finished();
        Ok(())
    }

    pub(super) async fn fetch_dev_desc_8_byte(
        &self,
        port: PortId,
        slot: u8,
    ) -> Result<usb::DeviceDescriptor8Byte> {
        let mut desc = unsafe { self.alloc_dma_zeroed::<usb::DeviceDescriptor8Byte>()? };
        self.get_desc_raw(port, slot, usb::DescriptorKind::Device, 0, 0, &mut desc)
            .await?;
        Ok(*desc)
    }

    async fn fetch_dev_desc(&self, port: PortId, slot: u8) -> Result<usb::DeviceDescriptor> {
        let mut desc = unsafe { self.alloc_dma_zeroed::<usb::DeviceDescriptor>()? };
        self.get_desc_raw(port, slot, usb::DescriptorKind::Device, 0, 0, &mut desc)
            .await?;
        Ok(*desc)
    }

    async fn fetch_config_desc(
        &self,
        port: PortId,
        slot: u8,
        config: u8,
    ) -> Result<(usb::ConfigDescriptor, [u8; 4087])> {
        let mut desc = unsafe { self.alloc_dma_zeroed::<(usb::ConfigDescriptor, [u8; 4087])>()? };
        self.get_desc_raw(
            port,
            slot,
            usb::DescriptorKind::Configuration,
            config,
            0,
            &mut desc,
        )
        .await?;
        Ok(*desc)
    }

    async fn fetch_bos_desc(
        &self,
        port: PortId,
        slot: u8,
    ) -> Result<(usb::BosDescriptor, [u8; 4087])> {
        let mut desc = unsafe { self.alloc_dma_zeroed::<(usb::BosDescriptor, [u8; 4087])>()? };
        self.get_desc_raw(
            port,
            slot,
            usb::DescriptorKind::BinaryObjectStorage,
            0,
            0,
            &mut desc,
        )
        .await?;
        Ok(*desc)
    }

    async fn fetch_lang_ids_desc(&self, port: PortId, slot: u8) -> Result<Vec<u16>> {
        let mut sdesc = unsafe { self.alloc_dma_zeroed::<(u8, u8, [u16; 127])>()? };
        self.get_desc_raw(port, slot, usb::DescriptorKind::String, 0, 0, &mut sdesc)
            .await?;

        let len = sdesc.0 as usize;
        if len > 2 {
            Ok(sdesc.2[..(len - 2) / 2].to_vec())
        } else {
            Ok(Vec::new())
        }
    }

    async fn fetch_string_desc(
        &self,
        port: PortId,
        slot: u8,
        value: u8,
        lang_id: u16,
    ) -> Result<String> {
        let mut sdesc = unsafe { self.alloc_dma_zeroed::<(u8, u8, [u16; 127])>()? };
        self.get_desc_raw(
            port,
            slot,
            usb::DescriptorKind::String,
            value,
            lang_id,
            &mut sdesc,
        )
        .await?;

        let len = sdesc.0 as usize;
        if len > 2 {
            Ok(String::from_utf16(&sdesc.2[..(len - 2) / 2]).unwrap_or(String::new()))
        } else {
            Ok(String::new())
        }
    }

    async fn new_if_desc(
        &self,
        port_id: PortId,
        slot: u8,
        desc: usb::InterfaceDescriptor,
        endps: impl IntoIterator<Item = EndpDesc>,
        unknown_descs: impl IntoIterator<Item = UnknownDesc>,
        lang_id: u16,
    ) -> Result<IfDesc> {
        Ok(IfDesc {
            alternate_setting: desc.alternate_setting,
            class: desc.class,
            interface_str: if desc.interface_str > 0 {
                /*TODO: broken on some devices
                Some(
                    self.fetch_string_desc(port_id, slot, desc.interface_str, lang_id)
                        .await?,
                )
                */
                None
            } else {
                None
            },
            kind: desc.kind,
            number: desc.number,
            protocol: desc.protocol,
            sub_class: desc.sub_class,
            endpoints: endps.into_iter().collect(),
            unknown_descs: unknown_descs.into_iter().collect(),
        })
    }

    pub async fn get_desc(&self, port_id: PortId, slot: u8) -> Result<DevDesc> {
        let ports = self.ports.lock().unwrap();
        let port = ports
            .get(port_id.root_hub_port_index())
            .ok_or(Error::new(ENOENT))?;
        if !port.flags().contains(port::PortFlags::CCS) {
            return Err(Error::new(ENOENT));
        }

        let raw_dd = self.fetch_dev_desc(port_id, slot).await?;
        log::debug!("port {} slot {} desc {:X?}", port_id, slot, raw_dd);

        // Only fetch language IDs if we need to. Some devices will fail to return this descriptor
        //TODO: also check configurations and interfaces for defined strings?
        let lang_id =
            if raw_dd.manufacturer_str > 0 || raw_dd.product_str > 0 || raw_dd.serial_str > 0 {
                let lang_ids = self.fetch_lang_ids_desc(port_id, slot).await?;
                // Prefer US English, but fall back to first language ID, or zero
                let en_us_id = 0x409;
                if lang_ids.contains(&en_us_id) {
                    en_us_id
                } else {
                    match lang_ids.first() {
                        Some(some) => *some,
                        None => 0,
                    }
                }
            } else {
                0
            };
        log::debug!("port {} using language ID 0x{:04x}", port_id, lang_id);

        let (manufacturer_str, product_str, serial_str) = (
            if raw_dd.manufacturer_str > 0 {
                Some(
                    self.fetch_string_desc(port_id, slot, raw_dd.manufacturer_str, lang_id)
                        .await?,
                )
            } else {
                None
            },
            if raw_dd.product_str > 0 {
                Some(
                    self.fetch_string_desc(port_id, slot, raw_dd.product_str, lang_id)
                        .await?,
                )
            } else {
                None
            },
            if raw_dd.serial_str > 0 {
                Some(
                    self.fetch_string_desc(port_id, slot, raw_dd.serial_str, lang_id)
                        .await?,
                )
            } else {
                None
            },
        );
        log::debug!(
            "manufacturer {:?} product {:?} serial {:?}",
            manufacturer_str,
            product_str,
            serial_str
        );

        //TODO let (bos_desc, bos_data) = self.fetch_bos_desc(port_id, slot).await?;

        let supports_superspeed = false;
        //TODO usb::bos_capability_descs(bos_desc, &bos_data).any(|desc| desc.is_superspeed());
        let supports_superspeedplus = false;
        //TODO usb::bos_capability_descs(bos_desc, &bos_data).any(|desc| desc.is_superspeedplus());

        let mut config_descs = SmallVec::new();

        for index in 0..raw_dd.configurations {
            debug!("Fetching the config descriptor at index {}", index);
            let (desc, data) = self.fetch_config_desc(port_id, slot, index).await?;
            log::debug!(
                "port {} slot {} config {} desc {:X?}",
                port_id,
                slot,
                index,
                desc
            );

            let extra_length = desc.total_length as usize - size_of_val(&desc);
            let data = &data[..extra_length];

            let mut i = 0;
            let mut descriptors = Vec::new();

            while let Some((descriptor, len)) = AnyDescriptor::parse(&data[i..]) {
                descriptors.push(descriptor);
                i += len;
            }

            let mut interface_descs = SmallVec::new();
            let mut unknown_descs = SmallVec::new();
            let mut iface_unknown_descs = SmallVec::<[UnknownDesc; 1]>::new();
            let mut iter = descriptors.into_iter().peekable();

            let mut cur_iface = None;
            let mut cur_endpoint = None;
            let mut endpoints = SmallVec::<[EndpDesc; 4]>::new();

            while let Some(item) = iter.next() {
                debug!("Processing descriptor {item:?}");

                match item {
                    AnyDescriptor::Interface(idesc) => {
                        if let Some(cur_idesc) = cur_iface.take() {
                            if let Some(endp) = cur_endpoint.take() {
                                if !endpoints.contains(&endp) {
                                    endpoints.push(endp);
                                }
                            }
                            interface_descs.push(
                                self.new_if_desc(
                                    port_id,
                                    slot,
                                    cur_idesc,
                                    endpoints,
                                    iface_unknown_descs,
                                    lang_id,
                                )
                                .await?,
                            );
                        }
                        cur_iface = Some(idesc);
                        endpoints = SmallVec::new();
                        iface_unknown_descs = SmallVec::new();
                    }
                    AnyDescriptor::Endpoint(endpdesc) => {
                        if let Some(desc) = cur_endpoint.take() {
                            endpoints.push(desc);
                        }
                        cur_endpoint = Some(EndpDesc::from(endpdesc));
                    }
                    AnyDescriptor::SuperSpeedCompanion(ssc) => {
                        if let Some(endp) = cur_endpoint.as_mut() {
                            endp.ssc = Some(SuperSpeedCmp::from(ssc.clone()));
                        } else {
                            error!("Found a superspeedcompanion descriptor before encountering any endpoint descriptor.");
                        }
                    }
                    AnyDescriptor::SuperSpeedPlusCompanion(sscp) => {
                        if let Some(endp) = cur_endpoint.as_mut() {
                            endp.sspc = Some(SuperSpeedPlusIsochCmp::from(sscp.clone()));
                        } else {
                            error!("Found a superspeedpluscompanion descriptor before encountering any endpoint descriptor.");
                        }
                    }
                    AnyDescriptor::Unknown(bytes) => {
                        // If we're processing an interface, add the desc to it, otherwise add it to the configuration
                        if cur_iface.is_none() {
                            unknown_descs.push(UnknownDesc {
                                len: bytes[0],
                                kind: bytes[1],
                                all_bytes: bytes,
                            });
                        } else {
                            iface_unknown_descs.push(UnknownDesc {
                                len: bytes[0],
                                kind: bytes[1],
                                all_bytes: bytes,
                            });
                        }
                    }
                    _ => {
                        error!("Found unexpected descriptor {item:?} while parsing config descriptors, skipping");
                        continue;
                    }
                }
            }

            // Push the last endpoint
            if let Some(endp) = cur_endpoint.take() {
                if !endpoints.contains(&endp) {
                    endpoints.push(endp);
                }
            }

            // Push the last interface
            if let Some(idesc) = cur_iface {
                let new_if_desc = self
                    .new_if_desc(
                        port_id,
                        slot,
                        idesc,
                        endpoints,
                        iface_unknown_descs,
                        lang_id,
                    )
                    .await?;
                if !interface_descs.contains(&new_if_desc) {
                    interface_descs.push(new_if_desc);
                }
            }

            config_descs.push(ConfDesc {
                kind: desc.kind,
                configuration: if desc.configuration_str > 0 {
                    Some(
                        self.fetch_string_desc(port_id, slot, desc.configuration_str, lang_id)
                            .await?,
                    )
                } else {
                    None
                },
                configuration_value: desc.configuration_value,
                attributes: desc.attributes,
                max_power: desc.max_power,
                interface_descs,
                unknown_descs,
            });
        }

        Ok(DevDesc {
            kind: raw_dd.kind,
            usb: raw_dd.usb,
            class: raw_dd.class,
            sub_class: raw_dd.sub_class,
            protocol: raw_dd.protocol,
            packet_size: raw_dd.packet_size,
            vendor: raw_dd.vendor,
            product: raw_dd.product,
            release: raw_dd.release,
            manufacturer_str,
            product_str,
            serial_str,
            config_descs,
        })
    }
}
