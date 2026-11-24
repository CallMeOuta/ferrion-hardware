// SPDX-License-Identifier: GPL-2.0-only AND MIT
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::thread;

use common::io::Io;
use common::timeout::Timeout;
use log::{debug, error, info, warn};
use syscall::error::{Error, Result, EIO};
use syscall::EAGAIN;

use self::enumerator::{DeviceEnumerationRequest, DeviceEnumerator};
use super::context::InputContext;
use super::port::{self, PortFlags};
use super::scheme::EndpIfState;
use super::{EndpNum, EndpointState, PortId, PortState, RingOrStreams, Xhci};

mod configuration;
pub(super) mod descriptors;
pub(super) mod drivers;
pub(super) mod enumerator;
mod slot;

impl<const N: usize> Xhci<N> {
    pub fn poll(&self) {
        debug!("Polling Initial Devices!");

        let len = self.ports.lock().unwrap().len();

        for root_hub_port_num in 1..=(len as u8) {
            let port_id = PortId {
                root_hub_port_num,
                route_string: 0,
            };

            //Get the CCS and CSC flags
            let (ccs, csc, flags) = {
                let mut ports = self.ports.lock().unwrap();
                let port = &mut ports[port_id.root_hub_port_index()];
                let flags = port.flags();
                let ccs = flags.contains(PortFlags::CCS);
                let csc = flags.contains(PortFlags::CSC);

                (ccs, csc, flags)
            };

            debug!("Port {} has flags {:?}", port_id, flags);

            match (ccs, csc) {
                (false, false) => { // Nothing is connected, and there was no port status change
                     //Do nothing
                }
                _ => {
                    //Either something is connected, or nothing is connected and a port status change was asserted.
                    self.device_enumerator_sender
                        .send(DeviceEnumerationRequest { port_id })
                        .expect("Failed to generate the port enumeration request!");
                }
            }
        }
    }

    pub fn reset_port(&self, port_id: PortId) -> Result<()> {
        debug!("XHCI Port {} reset", port_id);

        //TODO handle the second unwrap
        let mut ports = self.ports.lock().unwrap();
        let port = ports.get_mut(port_id.root_hub_port_index()).unwrap();
        let instant = std::time::Instant::now();

        debug!("Port {} Link State: {}", port_id, port.state());

        {
            port.set_pr();
            debug!(
                "Flags after setting port {} reset: {:?}",
                port_id,
                port.flags()
            );
            let timeout = Timeout::from_secs(1);
            while !port.flags().contains(port::PortFlags::PRC) {
                timeout.run().map_err(|()| {
                    log::error!("timeout on port {} PRC", port_id);
                    Error::new(EIO)
                })?;
            }
        }
        Ok(())
    }

    pub async fn attach_device(&self, port_id: PortId) -> syscall::Result<()> {
        if self.port_states.contains_key(&port_id) {
            debug!("Already contains port {}", port_id);
            return Err(syscall::Error::new(EAGAIN));
        }

        let (data, state, speed, flags) = {
            let port = &self.ports.lock().unwrap()[port_id.root_hub_port_index()];
            (port.read(), port.state(), port.speed(), port.flags())
        };

        debug!(
            "XHCI Port {}: {:X}, State {}, Speed {}, Flags {:?}",
            port_id, data, state, speed, flags
        );

        if flags.contains(port::PortFlags::CCS) {
            let slot_ty = match self.supported_protocol(port_id) {
                Some(protocol) => protocol.proto_slot_ty(),
                None => {
                    warn!("Failed to find supported protocol information for port");
                    0
                }
            };

            debug!("Slot type: {}", slot_ty);
            debug!("Enabling slot.");
            let slot = match self.enable_port_slot(slot_ty).await {
                Ok(ok) => ok,
                Err(err) => {
                    error!("Failed to enable slot for port {}: {}", port_id, err);
                    return Err(err);
                }
            };

            debug!("Enabled port {}, which the xHC mapped to {}", port_id, slot);

            //TODO: get correct speed for child devices
            let protocol_speed = self
                .lookup_psiv(port_id, speed)
                .expect("Failed to retrieve speed ID");

            let mut input = unsafe { self.alloc_dma_zeroed::<InputContext<N>>()? };

            debug!("Attempting to address the device");
            let mut ring = match self
                .address_device(&mut input, port_id, slot_ty, slot, protocol_speed, speed)
                .await
            {
                Ok(device_ring) => device_ring,
                Err(err) => {
                    error!("Failed to address device for port {}: `{}`", port_id, err);
                    return Err(err);
                }
            };

            debug!("Addressed device");

            // TODO: Should the descriptors be cached in PortState, or refetched?

            let mut port_state = PortState {
                slot,
                protocol_speed,
                input_context: Mutex::new(input),
                dev_desc: None,
                cfg_idx: None,
                endpoint_states: std::iter::once((
                    EndpNum::new(0),
                    EndpointState {
                        transfer: RingOrStreams::Ring(ring),
                        driver_if_state: EndpIfState::Init,
                    },
                ))
                .collect::<BTreeMap<_, _>>(),
            };
            self.port_states.insert(port_id, port_state);
            debug!("Got port states!");

            // Ensure correct packet size is used
            let dev_desc_8_byte = self.fetch_dev_desc_8_byte(port_id, slot).await?;
            {
                let mut port_state = self.port_states.get_mut(&port_id).unwrap();

                let mut input = port_state.input_context.lock().unwrap();

                match self
                    .update_max_packet_size(&mut *input, slot, dev_desc_8_byte)
                    .await
                {
                    Ok(()) => {}
                    Err(err) => {
                        error!(
                            "Failed to update max packet size for port {}: `{}`",
                            port_id, err
                        );
                        return Err(err);
                    }
                }
            }

            debug!("Got the 8 byte dev descriptor: {:X?}", dev_desc_8_byte);

            let dev_desc = match self.get_desc(port_id, slot).await {
                Ok(ok) => {
                    debug!("Got the full device descriptor!");
                    ok
                }
                Err(err) => {
                    error!(
                        "Failed to get full device descriptor for port {}: `{}`",
                        port_id, err
                    );
                    return Err(err);
                }
            };
            self.port_states.get_mut(&port_id).unwrap().dev_desc = Some(dev_desc);

            debug!("Got the port states again!");
            {
                let mut port_state = self.port_states.get_mut(&port_id).unwrap();

                let mut input = port_state.input_context.lock().unwrap();
                debug!("Got the input context!");
                let dev_desc = port_state.dev_desc.as_ref().unwrap();

                self.update_default_control_pipe(&mut *input, slot, dev_desc)
                    .await?;
            }

            debug!("Updated the default control pipe");

            match self.spawn_drivers(port_id) {
                Ok(()) => (),
                Err(err) => {
                    error!("Failed to spawn driver for port {}: `{}`", port_id, err)
                }
            }
        } else {
            warn!("Attempted to attach a device that didnt have CCS=1");
        }

        Ok(())
    }

    pub async fn detach_device(&self, port_id: PortId) -> Result<bool> {
        if let Some(children) = self.drivers.remove(&port_id) {
            for mut child in children {
                info!("killing driver process {} for port {}", child.id(), port_id);
                match child.kill() {
                    Ok(()) => {
                        info!("killed driver process {} for port {}", child.id(), port_id);
                        match child.try_wait() {
                            Ok(status_opt) => match status_opt {
                                Some(status) => {
                                    debug!(
                                        "driver process {} for port {} exited with status {}",
                                        child.id(),
                                        port_id,
                                        status
                                    );
                                }
                                None => {
                                    //TODO: kill harder
                                    warn!(
                                        "driver process {} for port {} still running",
                                        child.id(),
                                        port_id
                                    );
                                }
                            },
                            Err(err) => {
                                warn!(
                                    "failed to wait for the driver process {} for port {}: {}",
                                    child.id(),
                                    port_id,
                                    err
                                );
                            }
                        }
                    }
                    Err(err) => {
                        warn!(
                            "failed to kill the driver process {} for port {}: {}",
                            child.id(),
                            port_id,
                            err
                        );
                    }
                }
            }
        }

        if let Some(state) = self.port_states.remove(&port_id) {
            debug!("disabling port slot {} for port {}", state.slot, port_id);
            let result = self.disable_port_slot(state.slot).await.and(Ok(true));
            debug!(
                "disabled port slot {} for port {} with result: {:?}",
                state.slot, port_id, result
            );
            result
        } else {
            debug!(
                "Attempted to detach from port {}, which wasn't previously attached.",
                port_id
            );
            Ok(false)
        }
    }
}

pub fn start_device_enumerator<const N: usize>(hci: &Arc<Xhci<N>>) {
    let hci_clone = Arc::clone(&hci);

    debug!("About to start Device Enumerator");

    *hci.device_enumerator.lock().unwrap() = Some(thread::spawn(move || {
        debug!("Started Device Enumerator");
        DeviceEnumerator::new(hci_clone).run();
    }));
}
