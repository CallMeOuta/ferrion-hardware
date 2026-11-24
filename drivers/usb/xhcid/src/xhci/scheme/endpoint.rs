// SPDX-License-Identifier: GPL-2.0-only AND MIT
use std::io::{self, Seek};

use common::io::Io;
use syscall::{
    Error, Result, EBADF, EBADFD, EBADMSG, EINVAL, EIO, EISDIR, ENOENT, EPROTO, O_DIRECTORY, O_STAT,
};

use super::{handle_event_trb, usb, EndpNum, Handle, PortId, Xhci};
use crate::driver_interface::{
    EndpDirection, EndpointStatus, PortReqDirection, PortTransferStatus, PortTransferStatusKind,
    XhciEndpCtlDirection, XhciEndpCtlReq, XhciEndpCtlRes,
};
use crate::xhci::context::{self, StreamContextType};
use crate::xhci::trb::TrbCompletionCode;
use crate::xhci::RingOrStreams;

#[derive(Clone, Copy, Debug)]
pub enum EndpIfState {
    Init,
    WaitingForDataPipe {
        direction: XhciEndpCtlDirection,
        bytes_transferred: u32,
        bytes_to_transfer: u32,
    },
    WaitingForStatus,
    WaitingForTransferResult(PortTransferStatus),
}

/// Subdirs of an endpoint
#[derive(Debug)]
pub enum EndpointHandleTy {
    /// portX/endpoints/Y/data. Allows clients to read and write data associated with ctl requests.
    Data,

    /// portX/endpoints/Y/status
    Ctl,

    /// portX/endpoints/Y/
    Root(Vec<u8>), // content
}

#[derive(Clone, Copy, Debug)]
pub enum PortTransferState {
    /// Ready to read or write to do another transfer
    Ready,

    /// Transfer has completed, and the status has to be read.
    WaitingForStatusReq(PortTransferStatus),
}

impl<const N: usize> Xhci<N> {
    async fn reset_endpoint(&self, port_num: PortId, endp_num: EndpNum, tsp: bool) -> Result<()> {
        let port_state = self.port_states.get(&port_num).ok_or(Error::new(EBADFD))?;

        let endp_desc = port_state
            .get_endp_desc(endp_num)
            .ok_or(Error::new(EBADFD))?;
        let endp_xhci_dci = endp_desc.xhci_dci();

        let slot = self
            .port_states
            .get(&port_num)
            .ok_or(Error::new(EBADF))?
            .slot;

        let (event_trb, command_trb) = self
            .execute_command(|trb, cycle| {
                trb.reset_endpoint(slot, endp_xhci_dci, tsp, cycle);
            })
            .await;
        //self.event_handler_finished();

        handle_event_trb("RESET_ENDPOINT", &event_trb, &command_trb)
    }

    /// implements open() for /port<n>/endpoints/<n>
    ///
    /// # Arguments
    /// - 'port_num: [PortId]' - The port number specified in the scheme path
    /// - 'endpoint_num: [EndpNum]' - The endpoint number to access
    /// - 'flags: [usize]'    - The flags parameter passed to open()
    ///
    /// # Returns
    /// This function returns a [Result] containing either:
    ///
    /// - [Handle::Port]     - The handle was opened successfully
    /// - [EISDIR]          - open() was called on this scheme endpoint, but no directory-specific flags were passed to open
    /// - [ENOENT]           - The scheme is valid, but there is no port associated with the given port_num
    fn open_handle_endpoint_root(
        &self,
        port_num: PortId,
        endpoint_num: EndpNum,
        flags: usize,
    ) -> Result<Handle> {
        if flags & O_DIRECTORY == 0 && flags & O_STAT == 0 {
            return Err(Error::new(EISDIR));
        }

        let port_state = self
            .port_states
            .get_mut(&port_num)
            .ok_or(Error::new(ENOENT))?;

        /*if self.dev_ctx.contexts[port_state.slot as usize].endpoints.get(endpoint_num as usize).ok_or(Error::new(ENOENT))?.a.read() & 0b111 != 1 {
            return Err(Error::new(ENXIO)); // TODO: Find a proper error code for "endpoint not initialized".
        }*/

        if !port_state.endpoint_states.contains_key(&endpoint_num) {
            return Err(Error::new(ENOENT));
        }
        let contents = "ctl\ndata\n".as_bytes().to_owned();

        Ok(Handle::Endpoint(
            port_num,
            endpoint_num,
            EndpointHandleTy::Root(contents),
        ))
    }

    /// implements open() for /port<n>/endpoints/<n>/data and /port<n>/endpoints/<n>/ctl
    ///
    /// # Arguments
    /// - 'port_num: [PortId]' - The port number specified in the scheme path
    /// - 'endpoint_num: [EndpNum]' - The endpoint number to access
    /// - 'handle_type: [String]' - The type of the handle
    /// - 'flags: [usize]'    - The flags parameter passed to open()
    ///
    /// # Returns
    /// This function returns a [Result] containing either:
    ///
    /// - [Handle::Port]     - The handle was opened successfully
    /// - [EISDIR]          - open() was called on this scheme endpoint, but no directory-specific flags were passed to open
    /// - [ENOENT]           - The scheme is valid, but there is no port associated with the given port_num, or no endpoint with the given endpoint_num
    pub(super) fn open_handle_single_endpoint(
        &self,
        port_num: PortId,
        endpoint_num: EndpNum,
        handle_type: String,
        flags: usize,
    ) -> Result<Handle> {
        match handle_type.as_str() {
            "root" => self.open_handle_endpoint_root(port_num, endpoint_num, flags),
            "ctl" | "data" => {
                if flags & O_DIRECTORY != 0 && flags & O_STAT == 0 {
                    return Err(Error::new(EISDIR));
                }

                let port_state = self.port_states.get(&port_num).ok_or(Error::new(ENOENT))?;

                if port_state.endpoint_states.get(&endpoint_num).is_none() {
                    return Err(Error::new(ENOENT));
                }

                let st = match handle_type.as_str() {
                    "ctl" => EndpointHandleTy::Ctl,
                    "data" => EndpointHandleTy::Data,
                    _ => return Err(Error::new(ENOENT)),
                };
                Ok(Handle::Endpoint(port_num, endpoint_num, st))
            }
            _ => panic!(
                "Scheme parser returned an invalid string '{}' for the endpoint handle type",
                handle_type
            ),
        }
    }

    pub fn get_endp_status(&self, port_num: PortId, endp_num: EndpNum) -> Result<EndpointStatus> {
        let port_state = self.port_states.get(&port_num).ok_or(Error::new(EBADFD))?;

        let slot = port_state.slot;

        let endp_xhci_dci = if endp_num.get() != 0 {
            //TODO: simplify this
            let endp_desc = port_state
                .dev_desc
                .as_ref()
                .unwrap()
                .config_descs
                .get(0)
                .ok_or(Error::new(EIO))?
                .interface_descs
                .get(0)
                .ok_or(Error::new(EIO))?
                .endpoints
                .get(endp_num.index().unwrap())
                .ok_or(Error::new(EBADFD))?;
            endp_desc.xhci_dci()
        } else {
            1
        };

        let raw = self
            .dev_ctx
            .contexts
            .get(slot as usize)
            .ok_or(Error::new(EBADFD))?
            .endpoints[endp_xhci_dci as usize - 1]
            .a
            .read()
            & context::ENDPOINT_CONTEXT_STATUS_MASK;

        Ok(match raw {
            0 => EndpointStatus::Disabled,
            1 => EndpointStatus::Enabled,
            2 => EndpointStatus::Halted,
            3 => EndpointStatus::Stopped,
            4 => EndpointStatus::Error,
            _ => return Err(Error::new(EIO)),
        })
    }
    pub async fn on_req_reset_device(
        &self,
        port_num: PortId,
        endp_num: EndpNum,
        clear_feature: bool,
    ) -> Result<()> {
        if self.get_endp_status(port_num, endp_num)? != EndpointStatus::Halted {
            return Err(Error::new(EPROTO));
        }
        // Change the endpoint state from anything, but most likely HALTED (otherwise resetting
        // would be quite meaningless), to stopped.
        self.reset_endpoint(port_num, endp_num, false).await?;
        self.restart_endpoint(port_num, endp_num).await?;

        if clear_feature {
            self.device_req_no_data(
                port_num,
                usb::Setup {
                    kind: 0b0000_0010, // endpoint recipient
                    request: 0x01,     // CLEAR_FEATURE
                    value: 0x00,       // ENDPOINT_HALT
                    index: 0,          // TODO: interface num
                    length: 0,
                },
            )
            .await?;
        }
        Ok(())
    }
    pub async fn restart_endpoint(&self, port_num: PortId, endp_num: EndpNum) -> Result<()> {
        let mut port_state = self
            .port_states
            .get_mut(&port_num)
            .ok_or(Error::new(EBADFD))?;
        let slot = port_state.slot;

        let mut endpoint_state = port_state
            .endpoint_states
            .get_mut(&endp_num)
            .ok_or(Error::new(EBADFD))?;

        let (has_streams, ring) = match &mut endpoint_state.transfer {
            &mut RingOrStreams::Ring(ref mut ring) => (false, ring),
            &mut RingOrStreams::Streams(ref mut arr) => {
                (true, arr.rings.get_mut(&1).ok_or(Error::new(EBADFD))?)
            }
        };

        let (cmd, cycle) = ring.next();
        cmd.transfer_no_op(0, false, false, false, cycle);

        let deque_ptr_and_cycle = ring.register();

        let doorbell = if endp_num.get() != 0 {
            let endp_desc = port_state
                .dev_desc
                .as_ref()
                .unwrap()
                .config_descs
                .get(0)
                .ok_or(Error::new(EIO))?
                .interface_descs
                .get(0)
                .ok_or(Error::new(EIO))?
                .endpoints
                .get(endp_num.index().unwrap())
                .ok_or(Error::new(EBADFD))?;

            let stream_id = 1u16;

            Self::endp_doorbell(endp_num, endp_desc, if has_streams { stream_id } else { 0 })
        } else {
            Self::def_control_endp_doorbell()
        };

        self.dbs.lock().unwrap()[slot as usize].write(doorbell);

        self.set_tr_deque_ptr(port_num, endp_num, deque_ptr_and_cycle)
            .await?;

        Ok(())
    }
    pub fn endp_direction(&self, port_num: PortId, endp_num: EndpNum) -> Result<EndpDirection> {
        if endp_num.get() == 0 {
            return Ok(EndpDirection::Bidirectional);
        } else {
            Ok(self
                .port_states
                .get(&port_num)
                .ok_or(Error::new(EIO))?
                .dev_desc
                .as_ref()
                .unwrap()
                .config_descs
                .first()
                .ok_or(Error::new(EIO))?
                .interface_descs
                .first()
                .ok_or(Error::new(EIO))?
                .endpoints
                .get(endp_num.index().unwrap())
                .ok_or(Error::new(EIO))?
                .direction())
        }
    }
    pub async fn set_tr_deque_ptr(
        &self,
        port_num: PortId,
        endp_num: EndpNum,
        deque_ptr_and_cycle: u64,
    ) -> Result<()> {
        let port_state = self.port_states.get(&port_num).ok_or(Error::new(EBADFD))?;
        let slot = port_state.slot;

        let endp_desc = port_state
            .get_endp_desc(endp_num)
            .ok_or(Error::new(EBADFD))?;
        let endp_xhci_dci = endp_desc.xhci_dci();

        let (event_trb, command_trb) = self
            .execute_command(|trb, cycle| {
                trb.set_tr_deque_ptr(
                    deque_ptr_and_cycle,
                    cycle,
                    StreamContextType::PrimaryRing,
                    1,
                    endp_xhci_dci,
                    slot,
                )
            })
            .await;
        //self.event_handler_finished();

        handle_event_trb("SET_TR_DEQUEUE_PTR", &event_trb, &command_trb)
    }
    pub async fn on_write_endp_ctl(
        &self,
        port_num: PortId,
        endp_num: EndpNum,
        buf: &[u8],
    ) -> Result<usize> {
        let mut port_state = self
            .port_states
            .get_mut(&port_num)
            .ok_or(Error::new(EBADF))?;

        let ep_if_state = &mut port_state
            .endpoint_states
            .get_mut(&endp_num)
            .ok_or(Error::new(EBADF))?
            .driver_if_state;

        let req = serde_json::from_slice::<XhciEndpCtlReq>(buf).or(Err(Error::new(EBADMSG)))?;
        match req {
            XhciEndpCtlReq::Status => match ep_if_state {
                state @ EndpIfState::Init => *state = EndpIfState::WaitingForStatus,
                other => {
                    return Err(Error::new(EBADF));
                }
            },
            XhciEndpCtlReq::Reset { no_clear_feature } => match ep_if_state {
                EndpIfState::Init => {
                    self.on_req_reset_device(port_num, endp_num, !no_clear_feature)
                        .await?
                }
                other => {
                    return Err(Error::new(EBADF));
                }
            },
            XhciEndpCtlReq::Transfer { direction, count } => match ep_if_state {
                state @ EndpIfState::Init => {
                    if direction == XhciEndpCtlDirection::NoData {
                        // Yield the result directly because no bytes have to be sent or received
                        // beforehand.
                        let (completion_code, bytes_transferred, _) = self
                            .transfer(port_num, endp_num, None, PortReqDirection::DeviceToHost)
                            .await?;
                        if bytes_transferred > 0 {
                            return Err(Error::new(EIO));
                        }
                        let result = Self::transfer_result(completion_code, 0);

                        let mut port_state = self
                            .port_states
                            .get_mut(&port_num)
                            .ok_or(Error::new(EBADF))?;
                        let new_state = &mut port_state
                            .endpoint_states
                            .get_mut(&endp_num)
                            .ok_or(Error::new(EBADF))?
                            .driver_if_state;
                        *new_state = EndpIfState::WaitingForTransferResult(result)
                    } else {
                        *state = EndpIfState::WaitingForDataPipe {
                            direction,
                            bytes_to_transfer: count,
                            bytes_transferred: 0,
                        };
                    }
                }
                other => {
                    return Err(Error::new(EBADF));
                }
            },
            other => {
                return Err(Error::new(EBADF));
            }
        }
        Ok(buf.len())
    }
    fn transfer_result(completion_code: u8, bytes_transferred: u32) -> PortTransferStatus {
        let kind = if completion_code == TrbCompletionCode::Success as u8 {
            PortTransferStatusKind::Success
        } else if completion_code == TrbCompletionCode::ShortPacket as u8 {
            PortTransferStatusKind::ShortPacket
        } else if completion_code == TrbCompletionCode::Stall as u8 {
            PortTransferStatusKind::Stalled
        } else {
            PortTransferStatusKind::Unknown
        };
        PortTransferStatus {
            kind,
            bytes_transferred,
        }
    }
    pub async fn on_write_endp_data(
        &self,
        port_num: PortId,
        endp_num: EndpNum,
        buf: &[u8],
    ) -> Result<usize> {
        let mut port_state = self
            .port_states
            .get_mut(&port_num)
            .ok_or(Error::new(EBADFD))?;
        let mut endpoint_state = port_state
            .endpoint_states
            .get_mut(&endp_num)
            .ok_or(Error::new(EBADFD))?;

        let ep_if_state = &mut endpoint_state.driver_if_state;

        match ep_if_state {
            &mut EndpIfState::WaitingForDataPipe {
                direction: XhciEndpCtlDirection::Out,
                bytes_to_transfer: total_bytes_to_transfer,
                bytes_transferred,
            } => {
                if buf.len() > total_bytes_to_transfer as usize - bytes_transferred as usize {
                    return Err(Error::new(EINVAL));
                }
                drop(port_state);
                let (completion_code, some_bytes_transferred) =
                    self.transfer_write(port_num, endp_num, buf).await?;
                let result = Self::transfer_result(completion_code, some_bytes_transferred);

                // To avoid having to read from the Ctl interface file, the client should stop
                // invoking further data transfer calls if any single transfer returns fewer bytes
                // than requested.

                let mut port_state = self
                    .port_states
                    .get_mut(&port_num)
                    .ok_or(Error::new(EBADFD))?;
                let mut endpoint_state = port_state
                    .endpoint_states
                    .get_mut(&endp_num)
                    .ok_or(Error::new(EBADFD))?;
                let ep_if_state = &mut endpoint_state.driver_if_state;

                if let &mut EndpIfState::WaitingForDataPipe {
                    direction: XhciEndpCtlDirection::Out,
                    bytes_to_transfer,
                    ref mut bytes_transferred,
                } = ep_if_state
                {
                    if *bytes_transferred + some_bytes_transferred == bytes_to_transfer
                        || completion_code != TrbCompletionCode::Success as u8
                    {
                        *ep_if_state = EndpIfState::WaitingForTransferResult(result);
                    } else {
                        *bytes_transferred += some_bytes_transferred;
                    }
                } else {
                    unreachable!()
                }
                Ok(some_bytes_transferred as usize)
            }
            _ => return Err(Error::new(EBADF)),
        }
    }
    pub fn on_read_endp_ctl(
        &self,
        port_num: PortId,
        endp_num: EndpNum,
        buf: &mut [u8],
    ) -> Result<usize> {
        let port_state = &mut self
            .port_states
            .get_mut(&port_num)
            .ok_or(Error::new(EBADF))?;

        let ep_if_state = &mut port_state
            .endpoint_states
            .get_mut(&endp_num)
            .ok_or(Error::new(EBADF))?
            .driver_if_state;

        let res: XhciEndpCtlRes = match ep_if_state {
            &mut EndpIfState::Init => XhciEndpCtlRes::Idle,

            state @ &mut EndpIfState::WaitingForStatus => {
                *state = EndpIfState::Init;
                XhciEndpCtlRes::Status(self.get_endp_status(port_num, endp_num)?)
            }
            &mut EndpIfState::WaitingForDataPipe { .. } => XhciEndpCtlRes::Pending,
            &mut EndpIfState::WaitingForTransferResult(status) => {
                *ep_if_state = EndpIfState::Init;
                XhciEndpCtlRes::TransferResult(status)
            }
        };

        let mut cursor = io::Cursor::new(buf);
        serde_json::to_writer(&mut cursor, &res).or(Err(Error::new(EIO)))?;
        Ok(cursor.seek(io::SeekFrom::Current(0)).unwrap() as usize)
    }
    pub async fn on_read_endp_data(
        &self,
        port_num: PortId,
        endp_num: EndpNum,
        buf: &mut [u8],
    ) -> Result<usize> {
        let mut port_state = self
            .port_states
            .get_mut(&port_num)
            .ok_or(Error::new(EBADF))?;

        let mut ep_if_state = &mut port_state
            .endpoint_states
            .get_mut(&endp_num)
            .ok_or(Error::new(EBADF))?
            .driver_if_state;

        match ep_if_state {
            &mut EndpIfState::WaitingForDataPipe {
                direction: XhciEndpCtlDirection::In,
                bytes_transferred,
                bytes_to_transfer: total_bytes_to_transfer,
            } => {
                if buf.len() > total_bytes_to_transfer as usize - bytes_transferred as usize {
                    return Err(Error::new(EINVAL));
                }

                drop(port_state);
                let (completion_code, some_bytes_transferred) =
                    self.transfer_read(port_num, endp_num, buf).await?;

                // Just as with on_write_endp_data, a client issuing multiple reads must always
                // stop reading if one read returns fewer bytes than expected.

                let result = Self::transfer_result(completion_code, some_bytes_transferred);

                let mut port_state = self
                    .port_states
                    .get_mut(&port_num)
                    .ok_or(Error::new(EBADF))?;

                let mut ep_state = port_state
                    .endpoint_states
                    .get_mut(&endp_num)
                    .ok_or(Error::new(EBADF))?;

                let ep_if_state = &mut ep_state.driver_if_state;

                if let &mut EndpIfState::WaitingForDataPipe {
                    direction: XhciEndpCtlDirection::In,
                    bytes_to_transfer,
                    ref mut bytes_transferred,
                } = ep_if_state
                {
                    if *bytes_transferred + some_bytes_transferred == bytes_to_transfer
                        || completion_code != TrbCompletionCode::Success as u8
                    {
                        *ep_if_state = EndpIfState::WaitingForTransferResult(result);
                    } else {
                        *bytes_transferred += some_bytes_transferred;
                    }
                } else {
                    unreachable!()
                }
                Ok(some_bytes_transferred as usize)
            }
            _ => return Err(Error::new(EBADF)),
        }
    }
}
