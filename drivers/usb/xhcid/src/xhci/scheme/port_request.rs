// SPDX-License-Identifier: GPL-2.0-only AND MIT
use std::fmt;

use common::dma::Dma;
use syscall::{Error, Result, EBADF, EBADMSG, EINVAL, ENOTDIR, O_DIRECTORY, O_STAT};

use super::{usb, Handle, PortId, Xhci};
use crate::driver_interface::PortReq;
use crate::xhci::transfer::trb::TransferKind;
use crate::xhci::transfer::{ControlFlow, DmaSliceDbg};

pub enum PortReqState {
    Init,
    WaitingForDeviceBytes(Dma<[u8]>, usb::Setup), // buffer, setup params
    WaitingForHostBytes(Dma<[u8]>, usb::Setup),   // buffer, setup params
    TmpSetup(usb::Setup),
    Tmp,
}

impl fmt::Debug for PortReqState {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Init => f.debug_struct("PortReqState::Init").finish(),
            Self::WaitingForDeviceBytes(ref dma, setup) => f
                .debug_tuple("PortReqState::WaitingForDeviceBytes")
                .field(&DmaSliceDbg(dma))
                .field(&setup)
                .finish(),
            Self::WaitingForHostBytes(ref dma, setup) => f
                .debug_tuple("PortReqState::WaitingForHostBytes")
                .field(&DmaSliceDbg(dma))
                .field(&setup)
                .finish(),
            Self::TmpSetup(setup) => f
                .debug_tuple("PortReqState::TmpSetup")
                .field(&setup)
                .finish(),
            Self::Tmp => f.debug_struct("PortReqState::Init").finish(),
        }
    }
}

impl<const N: usize> Xhci<N> {
    async fn port_req_transfer(
        &self,
        port_num: PortId,
        data_buffer: Option<&mut Dma<[u8]>>,
        setup: usb::Setup,
        transfer_kind: TransferKind,
    ) -> Result<()> {
        self.execute_control_transfer(
            port_num,
            setup,
            transfer_kind,
            "CUSTOM_DEVICE_REQ",
            |trb, cycle| {
                trb.data(
                    data_buffer.as_ref().map(|dma| dma.physical()).unwrap_or(0),
                    setup.length,
                    transfer_kind == TransferKind::In,
                    cycle,
                );
                ControlFlow::Break
            },
        )
        .await?;
        Ok(())
    }
    fn port_req_init_st(&self, port_num: PortId, req: &PortReq) -> Result<PortReqState> {
        use usb::setup::*;

        let direction = ReqDirection::from(req.direction);
        let ty = ReqType::from(req.req_type) as u8;
        let recipient = ReqRecipient::from(req.req_recipient) as u8;

        let transfer_kind = match direction {
            _ if !req.transfers_data => TransferKind::NoData,
            ReqDirection::DeviceToHost => TransferKind::In,
            ReqDirection::HostToDevice => TransferKind::Out,
        };

        let setup = Setup {
            kind: ((direction as u8) << USB_SETUP_DIR_SHIFT)
                | (ty << USB_SETUP_REQ_TY_SHIFT)
                | (recipient << USB_SETUP_RECIPIENT_SHIFT),
            request: req.request,
            value: req.value,
            index: req.index,
            length: req.length,
        };
        // TODO: Reuse buffers, or something.
        // TODO: Validate the size.
        // TODO: Sizes above 65536, *perhaps*.
        let data_buffer_opt = if req.transfers_data {
            let data_buffer = unsafe { self.alloc_dma_zeroed_unsized(req.length as usize)? };
            assert_eq!(data_buffer.len(), req.length as usize);
            Some(data_buffer)
        } else {
            None
        };

        Ok(match transfer_kind {
            TransferKind::In => PortReqState::WaitingForDeviceBytes(
                data_buffer_opt.ok_or(Error::new(EINVAL))?,
                setup,
            ),
            TransferKind::Out => {
                PortReqState::WaitingForHostBytes(data_buffer_opt.ok_or(Error::new(EINVAL))?, setup)
            }
            TransferKind::NoData => PortReqState::TmpSetup(setup),
            _ => unreachable!(),
        })
        // FIXME: Make sure there aren't any other PortReq handles, perhaps by storing the state in
        // PortState?
    }
    pub(super) async fn handle_port_req_write(
        &self,
        fd: usize,
        port_num: PortId,
        mut st: PortReqState,
        buf: &[u8],
    ) -> Result<usize> {
        let bytes_written = match st {
            PortReqState::Init => {
                let req = serde_json::from_slice::<PortReq>(buf).or(Err(Error::new(EBADMSG)))?;

                st = self.port_req_init_st(port_num, &req)?;

                if let PortReqState::TmpSetup(setup) = st {
                    // No need for any additional reads or writes, before completing.
                    self.port_req_transfer(port_num, None, setup, TransferKind::NoData)
                        .await?;
                    st = PortReqState::Init;
                }

                buf.len()
            }
            PortReqState::WaitingForHostBytes(mut dma_buffer, setup) => {
                if buf.len() != dma_buffer.len() {
                    return Err(Error::new(EINVAL));
                }
                dma_buffer.copy_from_slice(buf);

                self.port_req_transfer(port_num, Some(&mut dma_buffer), setup, TransferKind::Out)
                    .await?;
                st = PortReqState::Init;

                buf.len()
            }
            PortReqState::WaitingForDeviceBytes(_, _) => return Err(Error::new(EBADF)),
            PortReqState::Tmp | PortReqState::TmpSetup(_) => unreachable!(),
        };
        let mut guard = self.handles.get_mut(&fd).ok_or(Error::new(EBADF))?;
        match &mut *guard {
            Handle::PortReq(_, ref mut state) => *state = st,
            _ => unreachable!(),
        }
        Ok(bytes_written)
    }
    pub(super) async fn handle_port_req_read(
        &self,
        fd: usize,
        port_num: PortId,
        mut st: PortReqState,
        buf: &mut [u8],
    ) -> Result<usize> {
        let bytes_read = match st {
            PortReqState::WaitingForDeviceBytes(mut dma_buffer, setup) => {
                if buf.len() != dma_buffer.len() {
                    return Err(Error::new(EINVAL));
                }
                self.port_req_transfer(port_num, Some(&mut dma_buffer), setup, TransferKind::In)
                    .await?;
                buf.copy_from_slice(&dma_buffer);

                st = PortReqState::Init;

                buf.len()
            }
            PortReqState::Init | PortReqState::WaitingForHostBytes(_, _) => {
                return Err(Error::new(EBADF))
            }
            PortReqState::Tmp | PortReqState::TmpSetup(_) => unreachable!(),
        };

        let mut guard = self.handles.get_mut(&fd).ok_or(Error::new(EBADF))?;
        match &mut *guard {
            Handle::PortReq(_, ref mut state) => *state = st,
            _ => unreachable!(),
        }
        Ok(bytes_read)
    }

    /// implements open() for /port<n>/request
    ///
    /// # Arguments
    /// - 'port_num: [PortId]' - The port number specified in the scheme path
    /// - 'flags: [usize]'    - The flags parameter passed to open()
    ///
    /// # Returns
    /// This function returns a [Result] containing either:
    ///
    /// - [Handle::Port]     - The handle was opened successfully
    /// - [ENOTDIR]          - open() was called on this scheme endpoint, but directory-specific flags were passed to open
    pub(super) fn open_handle_port_request(
        &self,
        port_num: PortId,
        flags: usize,
    ) -> Result<Handle> {
        if flags & O_DIRECTORY != 0 && flags & O_STAT == 0 {
            return Err(Error::new(ENOTDIR));
        }

        Ok(Handle::PortReq(port_num, PortReqState::Init))
    }
}
