//! Provides the File Descriptor Scheme Interface to the XHCI.
//!
//! This file implements the basic unix file operations that are used to interface with the XHCI
//! driver. While an external program could interact with the driver using this interface, a
//! higher-level abstraction can be found in driver_interface.rs. It is recommended that you use
//! the functions in that module to interact with the driver.
//!
//! The XHCI driver has the following set of schemes:
//!
//! port<n>
//! port<n>/configure
//! port<n>/request
//! port<n>/endpoints
//! port<n>/descriptors
//! port<n>/state
//! port<n>/endpoints/<n>
//! port<n>/endpoints/<n>/ctl
//! port<n>/endpoints/<n>/data
use std::convert::TryFrom;
use std::io::prelude::*;
use std::sync::atomic;
use std::{cmp, str};

use common::io::Io;
use futures::executor::block_on;
use log::{info, trace};
use redox_scheme::scheme::SchemeSync;
use redox_scheme::{CallerCtx, OpenResult};
use scheme_utils::FpathWriter;
use syscall::schemev2::NewFdFlags;
use syscall::{
    Error, Result, Stat, EACCES, EBADF, EINVAL, EIO, EISDIR, ENOENT, ENOTDIR, EOPNOTSUPP, ESPIPE,
    MODE_CHR, MODE_DIR, MODE_FILE, O_DIRECTORY, O_RDWR, O_STAT, O_WRONLY, SEEK_CUR, SEEK_END,
    SEEK_SET,
};

pub use self::endpoint::{EndpIfState, EndpointHandleTy, PortTransferState};
use self::path::SchemeParameters;
pub use self::port_request::PortReqState;
use super::context::{
    SlotState, StreamContextType, CONTEXT_32, CONTEXT_64, SLOT_CONTEXT_STATE_MASK,
    SLOT_CONTEXT_STATE_SHIFT,
};
pub use super::device::descriptors::AnyDescriptor;
pub use super::transfer::{div_round_up, handle_event_trb, handle_transfer_event_trb, ControlFlow};
use super::{port, usb, EndpNum, PortId, Xhci};
use crate::driver_interface::*;

mod endpoint;
mod path;
mod port_request;

/// The Handle to a specific scheme that is returned by an open() operation.
///
/// Contains some information about the data requested via the handle.
#[derive(Debug)]
pub enum Handle {
    TopLevel(Vec<u8>),                           // contents (ports)
    Port(PortId, Vec<u8>),                       // port, contents
    PortDesc(PortId, Vec<u8>),                   // port, contents
    PortState(PortId),                           // port
    PortReq(PortId, PortReqState),               // port, state
    Endpoints(PortId, Vec<u8>),                  // port, contents
    Endpoint(PortId, EndpNum, EndpointHandleTy), // port, endpoint, state
    ConfigureEndpoints(PortId),                  // port
    AttachDevice(PortId),                        // port
    DetachDevice(PortId),                        // port
    SchemeRoot,
}

/// The type of handle.
///
/// This is used by fstat() to determine whether to return a:
///     - MODE_DIR
///     - MODE_FILE
///     - MODE_CHR
pub(crate) enum HandleType {
    Directory,
    File,
    Character,
}

impl Handle {
    /// Converts a handle back into the scheme that generated it.
    ///
    /// This is useful for implementing fpath, as the input parameters for our existing schemes
    /// are generally static for the lifetime of the driver and can easily be retrieved.
    ///
    /// # Returns
    /// - A [String] containing the scheme path that the handle is associated with.
    pub(crate) fn to_path(&self) -> String {
        match self {
            Handle::TopLevel(_) => String::from(""),
            Handle::Port(port_num, _) => {
                format!("port{}", port_num)
            }
            Handle::PortDesc(port_num, _) => {
                format!("port{}/descriptors", port_num)
            }
            Handle::PortState(port_num) => {
                format!("port{}/state", port_num)
            }
            Handle::PortReq(port_num, _) => {
                format!("port{}/request", port_num)
            }
            Handle::Endpoints(port_num, _) => {
                format!("port{}/endpoints", port_num)
            }
            Handle::Endpoint(port_num, endpoint_num, handle_type) => match handle_type {
                EndpointHandleTy::Data => {
                    format!("port{}/endpoints/{}/data", port_num, endpoint_num.get())
                }
                EndpointHandleTy::Ctl => {
                    format!("port{}/endpoints/{}/ctl", port_num, endpoint_num.get())
                }
                EndpointHandleTy::Root(_) => {
                    format!("port{}/endpoints/{}", port_num, endpoint_num.get())
                }
            },
            Handle::ConfigureEndpoints(port_num) => {
                format!("port{}/configure", port_num)
            }
            Handle::AttachDevice(port_num) => {
                format!("port{}/attach", port_num)
            }
            Handle::DetachDevice(port_num) => {
                format!("port{}/detach", port_num)
            }
            Handle::SchemeRoot => String::from(""),
        }
    }

    /// Gets the access mode for this handle
    ///
    /// Handles can be a file, a directory, or a character interface. The mode that we use is
    /// entirely dependent upon the functionality of the scheme endpoint, so this returns the value
    /// that should be associated with that endpoint.
    ///
    /// # Returns
    /// - [HandleType] - The access mode associated with the handle.
    pub(crate) fn get_handle_type(&self) -> HandleType {
        match self {
            &Handle::TopLevel(_) => HandleType::Directory,
            &Handle::Port(_, _) => HandleType::Directory,
            &Handle::Endpoints(_, _) => HandleType::Directory,
            &Handle::PortDesc(_, _) => HandleType::File,
            &Handle::PortReq(_, PortReqState::WaitingForDeviceBytes(_, _)) => HandleType::Character,
            &Handle::PortReq(_, PortReqState::WaitingForHostBytes(_, _)) => HandleType::Character,
            &Handle::PortReq(_, PortReqState::Tmp) => unreachable!(),
            &Handle::PortReq(_, PortReqState::TmpSetup(_)) => unreachable!(),
            &Handle::PortState(_) => HandleType::Character,
            &Handle::PortReq(_, _) => HandleType::Character,
            &Handle::ConfigureEndpoints(_) => HandleType::Character,
            &Handle::AttachDevice(_) => HandleType::Character,
            &Handle::DetachDevice(_) => HandleType::Character,
            &Handle::Endpoint(_, _, ref st) => match st {
                EndpointHandleTy::Data => HandleType::Character,
                EndpointHandleTy::Ctl => HandleType::Character,
                EndpointHandleTy::Root(_) => HandleType::Directory,
            },
            &Handle::SchemeRoot => HandleType::Directory,
        }
    }

    /// Gets the length of the file buffer as returned by fstat in Stat.st_size
    ///
    /// As some of these endpoints did not return a length in the origin code, this
    /// provides an Option<usize>
    ///
    /// # Returns
    /// Either the size of the buffer, or [Option::None] if the buffer does not exist.
    pub(crate) fn get_buf_len(&self) -> Option<usize> {
        match self {
            &Handle::TopLevel(ref buf) => Some(buf.len()),
            &Handle::Port(_, ref buf) => Some(buf.len()),
            &Handle::Endpoints(_, ref buf) => Some(buf.len()),
            &Handle::PortDesc(_, ref buf) => Some(buf.len()),
            &Handle::PortReq(_, PortReqState::WaitingForDeviceBytes(ref buf, _)) => Some(buf.len()),
            &Handle::PortReq(_, PortReqState::WaitingForHostBytes(ref buf, _)) => Some(buf.len()),
            &Handle::PortReq(_, PortReqState::Tmp) => None,
            &Handle::PortReq(_, PortReqState::TmpSetup(_)) => None,
            &Handle::PortState(_) => None,
            &Handle::PortReq(_, _) => None,
            &Handle::ConfigureEndpoints(_) => None,
            &Handle::AttachDevice(_) => None,
            &Handle::DetachDevice(_) => None,
            &Handle::Endpoint(_, _, ref st) => match st {
                EndpointHandleTy::Data => None,
                EndpointHandleTy::Ctl => None,
                EndpointHandleTy::Root(ref buf) => Some(buf.len()),
            },
            &Handle::SchemeRoot => None,
        }
    }
}

impl<const N: usize> Xhci<N> {
    fn port_desc_json(&self, port_id: PortId) -> Result<Vec<u8>> {
        let dev_desc = &self
            .port_states
            .get(&port_id)
            .ok_or(Error::new(ENOENT))?
            .dev_desc;
        serde_json::to_vec(dev_desc).or(Err(Error::new(EIO)))
    }
    fn write_dyn_string(string: &[u8], buf: &mut [u8], offset: usize) -> usize {
        let max_bytes_to_read = cmp::min(string.len(), buf.len());
        let bytes_to_read = cmp::max(offset, max_bytes_to_read) - offset;
        buf[..bytes_to_read].copy_from_slice(&string[..bytes_to_read]);

        bytes_to_read
    }
    /// Implements open() for the root level scheme
    ///
    /// # Arguments
    /// - 'flags: [usize]' - The flags parameter passed to open()
    ///
    /// # Returns
    /// This function returns a [Result] containing either
    ///
    /// - Handle::TopLevel - The file was opened.
    /// - EISDIR           - This is a directory endpoint, but neither O_DIRECTORY nor O_STAT were passed.
    ///
    fn open_handle_top_level(&self, flags: usize) -> Result<Handle> {
        if flags & O_DIRECTORY != 0 || flags & O_STAT != 0 {
            let mut contents = Vec::new();

            let ports_guard = self.ports.lock().unwrap();

            for (index, _) in ports_guard
                .iter()
                .enumerate()
                .filter(|(_, port)| port.flags().contains(port::PortFlags::CCS))
            {
                write!(contents, "port{}\n", index).unwrap();
            }

            Ok(Handle::TopLevel(contents))
        } else {
            Err(Error::new(EISDIR))
        }
    }

    /// implements open() for /port<n>/descriptors
    ///
    /// # Arguments
    /// - 'port_num: [PortId]' - The port number specified in the scheme path
    /// - 'flags: [usize]'    - The flags parameter passed to open()
    ///
    /// # Returns
    /// This function returns a [Result] containing either:
    ///
    /// - [Handle::PortDesc] - The handle was opened successfully
    /// - [ENOTDIR]          - Directory-specific flags were passed to open(), but this endpoint is not a directory.
    fn open_handle_port_descriptors(&self, port_num: PortId, flags: usize) -> Result<Handle> {
        if flags & O_DIRECTORY != 0 && flags & O_STAT == 0 {
            return Err(Error::new(ENOTDIR));
        }

        let contents = self.port_desc_json(port_num)?;
        Ok(Handle::PortDesc(port_num, contents))
    }

    /// implements open() for /port<n>
    ///
    /// # Arguments
    /// - 'port_num: [PortId]' - The port number specified in the scheme path
    /// - 'flags: [usize]'    - The flags parameter passed to open()
    ///
    /// # Returns
    /// This function returns a [Result] containing either:
    ///
    /// - [Handle::Port]     - The handle was opened successfully
    /// - [ENOENT]           - The scheme is valid, but there is no port associated with the given port_num
    /// - [EISDIR]           - open() was called on this scheme endpoint, but no directory-specific flags were passed to open
    fn open_handle_port(&self, port_num: PortId, flags: usize) -> Result<Handle> {
        // The != here is unintuitive. You would assume that you could do
        // flags & O_DIRECTORY || flags & O_STAT, but rust doesn't allow
        // you to cast integers to booleans.
        if (flags & O_DIRECTORY != 0) || (flags & O_STAT != 0) {
            let mut contents = Vec::new();

            write!(contents, "descriptors\nendpoints\n").unwrap();

            if self.slot_state(
                self.port_states
                    .get(&port_num)
                    .ok_or(Error::new(ENOENT))?
                    .slot as usize,
            ) != SlotState::Configured as u8
            {
                write!(contents, "configure\n").unwrap();
            }

            Ok(Handle::Port(port_num, contents))
        } else {
            Err(Error::new(EISDIR))
        }
    }

    /// implements open() for /port<n>/state
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
    fn open_handle_port_state(&self, port_num: PortId, flags: usize) -> Result<Handle> {
        if flags & O_DIRECTORY != 0 && flags & O_STAT == 0 {
            return Err(Error::new(ENOTDIR));
        }

        Ok(Handle::PortState(port_num))
    }

    /// implements open() for /port<n>/endpoints
    ///
    /// # Arguments
    /// - 'port_num: [PortId]' - The port number specified in the scheme path
    /// - 'flags: [usize]'    - The flags parameter passed to open()
    ///
    /// # Returns
    /// This function returns a [Result] containing either:
    ///
    /// - [Handle::Port]     - The handle was opened successfully
    /// - [EISDIR]          - open() was called on this scheme endpoint, but no directory-specific flags were passed to open
    fn open_handle_port_endpoints(&self, port_num: PortId, flags: usize) -> Result<Handle> {
        if flags & O_DIRECTORY == 0 && flags & O_STAT == 0 {
            return Err(Error::new(EISDIR));
        };
        let mut contents = Vec::new();
        let ps = self.port_states.get(&port_num).ok_or(Error::new(ENOENT))?;

        /*for (ep_num, _) in self.dev_ctx.contexts[ps.slot as usize].endpoints.iter().enumerate().filter(|(_, ep)| ep.a.read() & 0b111 == 1) {
            write!(contents, "{}\n", ep_num).unwrap();
        }*/

        for ep_num in ps.endpoint_states.keys() {
            write!(contents, "{}\n", ep_num.get()).unwrap();
        }

        Ok(Handle::Endpoints(port_num, contents))
    }

    /// implements open() for /port<n>/configure
    ///
    /// # Arguments
    /// - 'port_num: [PortId]' - The port number specified in the scheme path
    /// - 'flags: [usize]'    - The flags parameter passed to open()
    ///
    /// # Returns
    /// This function returns a [Result] containing either:
    ///
    /// - [Handle::Port]     - The handle was opened successfully
    /// - [EISDIR]          - open() was called on this scheme endpoint, but no directory-specific flags were passed to open
    /// - [ENOENT]           - The scheme is valid, but there is no port associated with the given port_num, or no endpoint with the given endpoint_num
    fn open_handle_configure_endpoints(&self, port_num: PortId, flags: usize) -> Result<Handle> {
        if flags & O_DIRECTORY != 0 && flags & O_STAT == 0 {
            return Err(Error::new(ENOTDIR));
        }

        if flags & O_RDWR != O_WRONLY && flags & O_STAT == 0 {
            return Err(Error::new(EACCES));
        }

        Ok(Handle::ConfigureEndpoints(port_num))
    }

    /// implements open() for /port<n>/attach
    ///
    /// # Arguments
    /// - 'port_num: [PortId]' - The port number specified in the scheme path
    /// - 'flags: [usize]'    - The flags parameter passed to open()
    ///
    /// # Returns
    /// This function returns a [Result] containing either:
    ///
    /// - [Handle::Port]     - The handle was opened successfully
    /// - [EISDIR]          - open() was called on this scheme endpoint, but no directory-specific flags were passed to open
    /// - [ENOENT]           - The scheme is valid, but there is no port associated with the given port_num, or no endpoint with the given endpoint_num
    fn open_handle_attach_device(&self, port_num: PortId, flags: usize) -> Result<Handle> {
        if flags & O_DIRECTORY != 0 && flags & O_STAT == 0 {
            return Err(Error::new(ENOTDIR));
        }

        if flags & O_RDWR != O_WRONLY && flags & O_STAT == 0 {
            return Err(Error::new(EACCES));
        }

        Ok(Handle::AttachDevice(port_num))
    }

    /// implements open() for /port<n>/detach
    ///
    /// # Arguments
    /// - 'port_num: [PortId]' - The port number specified in the scheme path
    /// - 'flags: [usize]'    - The flags parameter passed to open()
    ///
    /// # Returns
    /// This function returns a [Result] containing either:
    ///
    /// - [Handle::Port]     - The handle was opened successfully
    /// - [EISDIR]          - open() was called on this scheme endpoint, but no directory-specific flags were passed to open
    /// - [ENOENT]           - The scheme is valid, but there is no port associated with the given port_num, or no endpoint with the given endpoint_num
    fn open_handle_detach_device(&self, port_num: PortId, flags: usize) -> Result<Handle> {
        if flags & O_DIRECTORY != 0 && flags & O_STAT == 0 {
            return Err(Error::new(ENOTDIR));
        }

        if flags & O_RDWR != O_WRONLY && flags & O_STAT == 0 {
            return Err(Error::new(EACCES));
        }

        Ok(Handle::DetachDevice(port_num))
    }
}

impl<const N: usize> SchemeSync for &Xhci<N> {
    fn scheme_root(&mut self) -> Result<usize> {
        let fd = self.next_handle.fetch_add(1, atomic::Ordering::Relaxed);
        self.handles.insert(fd, Handle::SchemeRoot);
        Ok(fd)
    }
    fn openat(
        &mut self,
        dirfd: usize,
        path_str: &str,
        mut flags: usize,
        fcntl_flags: u32,
        ctx: &CallerCtx,
    ) -> Result<OpenResult> {
        let full_path = match *self.handles.get(&dirfd).ok_or(Error::new(EBADF))? {
            Handle::SchemeRoot => path_str.trim_start_matches('/').to_string(),
            Handle::Port(port_num, _) => {
                let clean_path = path_str.trim_start_matches('/');
                if clean_path.is_empty() {
                    format!("port{}", port_num)
                } else {
                    format!("port{}/{}", port_num, clean_path)
                }
            }
            _ => return Err(Error::new(EACCES)),
        };
        if ctx.uid != 0 {
            return Err(Error::new(EACCES));
        }

        //Parse the scheme, determine if it's in the valid format, return an error if not.
        //This doesn't guarantee that the parameters themselves are valid (i.e. bounded correctly)
        //only that the scheme itself was parseable.
        let scheme_parameters = SchemeParameters::from_scheme(&full_path)?;

        flags |= fcntl_flags as usize;

        //Once we have our scheme parsed into parameters, we can match on those parameters to
        //find the correct routine to open a handle
        let handle = match scheme_parameters {
            SchemeParameters::TopLevel => self.open_handle_top_level(flags)?,
            SchemeParameters::Port(port_number) => self.open_handle_port(port_number, flags)?,
            SchemeParameters::PortDesc(port_number) => {
                self.open_handle_port_descriptors(port_number, flags)?
            }
            SchemeParameters::PortState(port_number) => {
                self.open_handle_port_state(port_number, flags)?
            }
            SchemeParameters::PortReq(port_number) => {
                self.open_handle_port_request(port_number, flags)?
            }
            SchemeParameters::Endpoints(port_number) => {
                self.open_handle_port_endpoints(port_number, flags)?
            }
            SchemeParameters::Endpoint(port_number, endpoint_number, handle_type) => {
                self.open_handle_single_endpoint(port_number, endpoint_number, handle_type, flags)?
            }
            SchemeParameters::ConfigureEndpoints(port_number) => {
                self.open_handle_configure_endpoints(port_number, flags)?
            }
            SchemeParameters::AttachDevice(port_number) => {
                self.open_handle_attach_device(port_number, flags)?
            }
            SchemeParameters::DetachDevice(port_number) => {
                self.open_handle_detach_device(port_number, flags)?
            }
        };

        let fd = self.next_handle.fetch_add(1, atomic::Ordering::Relaxed);

        trace!("OPENED {} to FD={}, handle: {:?}", full_path, fd, handle);

        self.handles.insert(fd, handle);

        Ok(OpenResult::ThisScheme {
            number: fd,
            flags: NewFdFlags::POSITIONED,
        })
    }

    fn fstat(&mut self, id: usize, stat: &mut Stat, _ctx: &CallerCtx) -> Result<()> {
        let guard = self.handles.get(&id).ok_or(Error::new(EBADF))?;

        stat.st_mode = match (&*guard).get_handle_type() {
            HandleType::Directory => MODE_DIR,
            HandleType::File => MODE_FILE,
            HandleType::Character => MODE_CHR,
        };

        stat.st_size = match (&*guard).get_buf_len() {
            None => stat.st_size,
            Some(size) => size as u64,
        };

        //If we have a handle to the configure scheme, we need to mark it as write only.
        match &*guard {
            Handle::ConfigureEndpoints(_) | Handle::AttachDevice(_) | Handle::DetachDevice(_) => {
                stat.st_mode = stat.st_mode | 0o200;
            }
            _ => {}
        }

        Ok(())
    }

    fn fpath(&mut self, fd: usize, buf: &mut [u8], _ctx: &CallerCtx) -> Result<usize> {
        FpathWriter::with(buf, "xhci", |w| {
            let handle = self.handles.get(&fd).ok_or(Error::new(EBADF))?;
            write!(w, "{}", handle.to_path()).unwrap();
            Ok(())
        })
    }

    fn read(
        &mut self,
        fd: usize,
        buf: &mut [u8],
        offset: u64,
        _fcntl_flags: u32,
        _ctx: &CallerCtx,
    ) -> Result<usize> {
        let offset = offset as usize;
        let mut guard = self.handles.get_mut(&fd).ok_or(Error::new(EBADF))?;
        trace!(
            "READ fd={}, handle={:?}, buf=(addr {:p}, length {})",
            fd,
            guard,
            buf.as_ptr(),
            buf.len()
        );
        match &mut *guard {
            Handle::TopLevel(ref src_buf)
            | Handle::Port(_, ref src_buf)
            | Handle::PortDesc(_, ref src_buf)
            | Handle::Endpoints(_, ref src_buf)
            | Handle::Endpoint(_, _, EndpointHandleTy::Root(ref src_buf)) => {
                let max_bytes_to_read = cmp::min(src_buf.len(), buf.len());
                let bytes_to_read = cmp::max(max_bytes_to_read, offset) - offset;

                buf[..bytes_to_read].copy_from_slice(&src_buf[..bytes_to_read]);

                Ok(bytes_to_read)
            }
            Handle::ConfigureEndpoints(_) => Err(Error::new(EBADF)),
            Handle::AttachDevice(_) => Err(Error::new(EBADF)),
            Handle::DetachDevice(_) => Err(Error::new(EBADF)),
            Handle::SchemeRoot => Err(Error::new(EBADF)),

            &mut Handle::Endpoint(port_num, endp_num, ref mut st) => match st {
                EndpointHandleTy::Ctl => self.on_read_endp_ctl(port_num, endp_num, buf),
                EndpointHandleTy::Data => block_on(self.on_read_endp_data(port_num, endp_num, buf)),
                EndpointHandleTy::Root(_) => Err(Error::new(EBADF)),
            },
            &mut Handle::PortState(port_num) => {
                let ps = self.port_states.get(&port_num).ok_or(Error::new(EBADF))?;
                let ctx = self
                    .dev_ctx
                    .contexts
                    .get(ps.slot as usize)
                    .ok_or(Error::new(EBADF))?;
                let state = ((ctx.slot.d.read() & SLOT_CONTEXT_STATE_MASK)
                    >> SLOT_CONTEXT_STATE_SHIFT) as u8;

                let string = match state {
                    0 => Some(PortState::EnabledOrDisabled),
                    1 => Some(PortState::Default),
                    2 => Some(PortState::Addressed),
                    3 => Some(PortState::Configured),
                    _ => None,
                }
                .as_ref()
                .map(PortState::as_str)
                .unwrap_or("unknown")
                .as_bytes();

                Ok(Xhci::<N>::write_dyn_string(string, buf, offset))
            }
            &mut Handle::PortReq(port_num, ref mut st) => {
                let state = std::mem::replace(st, PortReqState::Tmp);
                drop(guard); // release the lock
                block_on(self.handle_port_req_read(fd, port_num, state, buf))
            }
        }
    }
    fn write(
        &mut self,
        fd: usize,
        buf: &[u8],
        _offset: u64,
        _fcntl_flags: u32,
        _ctx: &CallerCtx,
    ) -> Result<usize> {
        let mut guard = self.handles.get_mut(&fd).ok_or(Error::new(EBADF))?;
        trace!(
            "WRITE fd={}, handle={:?}, buf=(addr {:p}, length {})",
            fd,
            guard,
            buf.as_ptr(),
            buf.len()
        );

        match &mut *guard {
            &mut Handle::ConfigureEndpoints(port_num) => {
                block_on(self.configure_endpoints(port_num, buf))?;
                Ok(buf.len())
            }
            &mut Handle::AttachDevice(port_num) => {
                //TODO: accept some arguments in buffer?
                block_on(self.attach_device(port_num))?;
                Ok(buf.len())
            }
            &mut Handle::DetachDevice(port_num) => {
                //TODO: accept some arguments in buffer?
                block_on(self.detach_device(port_num))?;
                Ok(buf.len())
            }
            &mut Handle::Endpoint(port_num, endp_num, ref ep_file_ty) => match ep_file_ty {
                EndpointHandleTy::Ctl => block_on(self.on_write_endp_ctl(port_num, endp_num, buf)),
                EndpointHandleTy::Data => {
                    block_on(self.on_write_endp_data(port_num, endp_num, buf))
                }
                EndpointHandleTy::Root(_) => return Err(Error::new(EBADF)),
            },
            &mut Handle::PortReq(port_num, ref mut st) => {
                let state = std::mem::replace(st, PortReqState::Tmp);
                drop(guard); // release the lock
                block_on(self.handle_port_req_write(fd, port_num, state, buf))
            }
            // TODO: Introduce PortReqState::Waiting, which this write call changes to
            // PortReqState::ReadyToWrite when all bytes are written.
            _ => Err(Error::new(EBADF)),
        }
    }

    fn on_close(&mut self, fd: usize) {
        self.handles.remove(&fd);
    }
}
