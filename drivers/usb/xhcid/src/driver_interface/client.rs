use std::convert::TryFrom;
use std::fs::File;
use std::io::prelude::*;
use std::os::fd::{FromRawFd, RawFd};
use std::string::FromUtf8Error;
use std::{fmt, io, result};

use thiserror::Error;

use super::protocol::{
    ConfigureEndpointsReq, DevDesc, EndpointStatus, Invalid, PortId, PortReq, PortReqDirection,
    PortReqRecipient, PortReqTy, PortState, PortTransferStatus, PortTransferStatusKind,
    XhciEndpCtlDirection, XhciEndpCtlReq, XhciEndpCtlRes,
};

pub struct XhciClientHandle {
    fd: libredox::Fd,
    scheme: String,
    port: PortId,
}
impl fmt::Debug for XhciClientHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("XhciClientHandle")
            .field("scheme", &self.scheme)
            .field("port", &self.port)
            .field("fd", &"libredox::Fd")
            .finish()
    }
}

pub enum DeviceReqData<'a> {
    In(&'a mut [u8]),
    Out(&'a [u8]),
    NoData,
}
impl DeviceReqData<'_> {
    pub fn len(&self) -> usize {
        match self {
            Self::In(buf) => buf.len(),
            Self::Out(buf) => buf.len(),
            Self::NoData => 0,
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn map_buf<T, F: Fn(&[u8]) -> T>(&self, f: F) -> Option<T> {
        match self {
            Self::In(sbuf) => Some(f(sbuf)),
            Self::Out(dbuf) => Some(f(dbuf)),
            _ => None,
        }
    }
    pub fn direction(&self) -> PortReqDirection {
        match self {
            DeviceReqData::Out(_) => PortReqDirection::HostToDevice,
            DeviceReqData::NoData => PortReqDirection::HostToDevice,
            DeviceReqData::In(_) => PortReqDirection::DeviceToHost,
        }
    }
}

impl XhciClientHandle {
    pub fn new(scheme: String, port: PortId) -> result::Result<Self, XhciClientHandleError> {
        let path = format!("/scheme/{}/port{}", scheme, port);
        let fd = libredox::Fd::open(&path, libredox::flag::O_DIRECTORY, 0)?;
        Ok(Self { fd, scheme, port })
    }
    fn read(&self, path: &str) -> result::Result<Vec<u8>, XhciClientHandleError> {
        let target_fd = self.fd.openat(path, libredox::flag::O_RDONLY, 0)?;
        let stat = target_fd.stat()?;
        let mut buf: Vec<u8> = vec![0u8; stat.st_size as usize];
        let count = target_fd.read(&mut buf)?;
        buf.truncate(count);
        Ok(buf)
    }
    fn read_to_string(&self, path: &str) -> result::Result<String, XhciClientHandleError> {
        let buf = self.read(path)?;
        Ok(String::from_utf8(buf)?)
    }
    pub fn attach(&self) -> result::Result<(), XhciClientHandleError> {
        let file = self.fd.openat("attach", libredox::flag::O_WRONLY, 0)?;
        let _bytes_written = file.write(&[])?;
        Ok(())
    }
    pub fn detach(&self) -> result::Result<(), XhciClientHandleError> {
        let file = self.fd.openat("detach", libredox::flag::O_WRONLY, 0)?;
        let _bytes_written = file.write(&[])?;
        Ok(())
    }
    pub fn get_standard_descs(&self) -> result::Result<DevDesc, XhciClientHandleError> {
        let json = self.read("descriptors")?;
        Ok(serde_json::from_slice(&json)?)
    }
    pub fn configure_endpoints(
        &self,
        req: &ConfigureEndpointsReq,
    ) -> result::Result<(), XhciClientHandleError> {
        let json = serde_json::to_vec(req)?;
        let file = self.fd.openat("configure", libredox::flag::O_WRONLY, 0)?;
        let json_bytes_written = file.write(&json)?;
        if json_bytes_written != json.len() {
            return Err(XhciClientHandleError::InvalidResponse(Invalid(
                "configure_endpoints didn't read as many bytes as were requested",
            )));
        }
        Ok(())
    }
    pub fn port_state(&self) -> result::Result<PortState, XhciClientHandleError> {
        let string = self.read_to_string("state")?;
        Ok(string.parse()?)
    }
    pub fn open_endpoint_ctl(&self, num: u8) -> result::Result<File, XhciClientHandleError> {
        let path = format!("endpoints/{}/ctl", num);
        let fd = self.fd.openat(&path, libredox::flag::O_RDWR, 0)?;
        Ok(unsafe { File::from_raw_fd(fd.into_raw() as RawFd) })
    }
    pub fn open_endpoint_data(&self, num: u8) -> result::Result<File, XhciClientHandleError> {
        let path = format!("endpoints/{}/data", num);
        let fd = self.fd.openat(&path, libredox::flag::O_RDWR, 0)?;
        Ok(unsafe { File::from_raw_fd(fd.into_raw() as RawFd) })
    }
    pub fn open_endpoint(&self, num: u8) -> result::Result<XhciEndpHandle, XhciClientHandleError> {
        Ok(XhciEndpHandle {
            ctl: self.open_endpoint_ctl(num)?,
            data: self.open_endpoint_data(num)?,
        })
    }
    pub fn device_request<'a>(
        &self,
        req_type: PortReqTy,
        req_recipient: PortReqRecipient,
        request: u8,
        value: u16,
        index: u16,
        data: DeviceReqData<'a>,
    ) -> result::Result<(), XhciClientHandleError> {
        let length = u16::try_from(data.len())
            .or(Err(XhciClientHandleError::TransferBufTooLarge(data.len())))?;

        let req = PortReq {
            direction: data.direction(),
            req_type,
            req_recipient,
            request,
            value,
            index,
            length,
            transfers_data: !matches!(data, DeviceReqData::NoData),
        };
        let json = serde_json::to_vec(&req)?;

        let file = self.fd.openat("request", libredox::flag::O_RDWR, 0)?;

        let json_bytes_written = file.write(&json)?;
        if json_bytes_written != json.len() {
            return Err(XhciClientHandleError::InvalidResponse(Invalid(
                "device_request didn't return the same number of bytes as were written",
            )));
        }

        match data {
            DeviceReqData::In(buf) => {
                let bytes_read = file.read(buf)?;

                if bytes_read != buf.len() {
                    return Err(XhciClientHandleError::InvalidResponse(Invalid(
                        "device_request didn't transfer (host2dev) all bytes",
                    )));
                }
            }
            DeviceReqData::Out(buf) => {
                let bytes_read = file.write(&buf)?;

                if bytes_read != buf.len() {
                    return Err(XhciClientHandleError::InvalidResponse(Invalid(
                        "device_request didn't transfer (dev2host) all bytes",
                    )));
                }
            }
            DeviceReqData::NoData => (),
        }
        Ok(())
    }
    pub fn get_descriptor(
        &self,
        recipient: PortReqRecipient,
        ty: u8,
        idx: u8,
        windex: u16,
        buffer: &mut [u8],
    ) -> result::Result<(), XhciClientHandleError> {
        self.device_request(
            PortReqTy::Standard,
            recipient,
            0x06,
            (u16::from(ty) << 8) | u16::from(idx),
            windex,
            DeviceReqData::In(buffer),
        )
    }
    pub fn clear_feature(
        &self,
        recipient: PortReqRecipient,
        index: u16,
        feature_sel: u16,
    ) -> result::Result<(), XhciClientHandleError> {
        self.device_request(
            PortReqTy::Standard,
            recipient,
            0x01,
            feature_sel,
            index,
            DeviceReqData::NoData,
        )
    }
}

#[derive(Debug)]
pub struct XhciEndpHandle {
    data: File,
    ctl: File,
}

impl XhciEndpHandle {
    fn ctl_req(&mut self, ctl_req: &XhciEndpCtlReq) -> result::Result<(), XhciClientHandleError> {
        let ctl_buffer = serde_json::to_vec(ctl_req)?;

        let ctl_bytes_written = self.ctl.write(&ctl_buffer)?;
        if ctl_bytes_written != ctl_buffer.len() {
            return Err(Invalid("xhcid didn't process all of the ctl bytes").into());
        }

        Ok(())
    }
    fn ctl_res(&mut self) -> result::Result<XhciEndpCtlRes, XhciClientHandleError> {
        // a response must never exceed 256 bytes
        let mut ctl_buffer = [0u8; 256];
        let ctl_bytes_read = self.ctl.read(&mut ctl_buffer)?;

        let ctl_res = serde_json::from_slice(&ctl_buffer[..ctl_bytes_read as usize])?;
        Ok(ctl_res)
    }
    pub fn reset(&mut self, no_clear_feature: bool) -> result::Result<(), XhciClientHandleError> {
        self.ctl_req(&XhciEndpCtlReq::Reset { no_clear_feature })
    }
    pub fn status(&mut self) -> result::Result<EndpointStatus, XhciClientHandleError> {
        self.ctl_req(&XhciEndpCtlReq::Status)?;
        match self.ctl_res()? {
            XhciEndpCtlRes::Status(s) => Ok(s),
            _ => Err(Invalid("expected status response").into()),
        }
    }
    fn generic_transfer<F: FnOnce(&mut File) -> io::Result<usize>>(
        &mut self,
        direction: XhciEndpCtlDirection,
        f: F,
        expected_len: u32,
    ) -> result::Result<PortTransferStatus, XhciClientHandleError> {
        let req = XhciEndpCtlReq::Transfer {
            direction,
            count: expected_len,
        };
        self.ctl_req(&req)?;

        let bytes_read = f(&mut self.data)?;
        let res = self.ctl_res()?;

        match res {
            XhciEndpCtlRes::TransferResult(PortTransferStatus {
                kind: PortTransferStatusKind::Success,
                ..
            }) if bytes_read != expected_len as usize => {
                Err(Invalid("no short packet, but fewer bytes were read/written").into())
            }
            XhciEndpCtlRes::TransferResult(r) => Ok(r),
            _ => Err(Invalid("expected transfer result").into()),
        }
    }
    pub fn transfer_write(
        &mut self,
        buf: &[u8],
    ) -> result::Result<PortTransferStatus, XhciClientHandleError> {
        self.generic_transfer(
            XhciEndpCtlDirection::Out,
            |data| data.write(buf),
            buf.len() as u32,
        )
    }
    pub fn transfer_read(
        &mut self,
        buf: &mut [u8],
    ) -> result::Result<PortTransferStatus, XhciClientHandleError> {
        let len = buf.len() as u32;
        self.generic_transfer(XhciEndpCtlDirection::In, |data| data.read(buf), len)
    }
    pub fn transfer_nodata(&mut self) -> result::Result<PortTransferStatus, XhciClientHandleError> {
        self.generic_transfer(XhciEndpCtlDirection::NoData, |_| Ok(0), 0)
    }
    fn transfer_stream(&mut self, total_len: u32) -> TransferStream<'_> {
        TransferStream {
            bytes_to_transfer: total_len,
            bytes_transferred: 0,
            bytes_per_transfer: 32768, // TODO
            endp_handle: self,
        }
    }
    pub fn transfer_write_stream(&mut self, total_len: u32) -> TransferWriteStream<'_> {
        TransferWriteStream {
            inner: self.transfer_stream(total_len),
        }
    }
    pub fn transfer_read_stream(&mut self, total_len: u32) -> TransferReadStream<'_> {
        TransferReadStream {
            inner: self.transfer_stream(total_len),
        }
    }
}

pub struct TransferWriteStream<'a> {
    inner: TransferStream<'a>,
}
pub struct TransferReadStream<'a> {
    inner: TransferStream<'a>,
}
struct TransferStream<'a> {
    bytes_to_transfer: u32,
    bytes_transferred: u32,
    bytes_per_transfer: u32,
    endp_handle: &'a mut XhciEndpHandle,
}

#[derive(Debug, Error)]
pub enum XhciClientHandleError {
    #[error("i/o error: {0}")]
    IoError(#[from] io::Error),

    #[error("serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("invalid response")]
    InvalidResponse(#[from] Invalid),

    #[error("transfer buffer too large ({0} > 65536)")]
    TransferBufTooLarge(usize),

    #[error("unexpected short packet of size {0}")]
    UnexpectedShortPacket(usize),

    #[error("utf8 error: {0}")]
    Utf8Error(#[from] FromUtf8Error),
}

impl From<libredox::error::Error> for XhciClientHandleError {
    fn from(error: libredox::error::Error) -> Self {
        Self::IoError(error.into())
    }
}
