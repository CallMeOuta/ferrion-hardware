//! The 3D side of virtio-gpu that render handles use (FerrionOS): the virgl, blob and
//! context-init features, the capsets read from the device at start, and contexts. Linux's
//! `drivers/gpu/drm/virtio/virtgpu_vq.c` (MIT) is the reference for the commands.
//!
//! The host-visible window, blobs and submissions come with slice GP4 of the GPU plan. Until
//! then the device's shared memory region is not mapped: virtio-core skips that capability.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

use common::dma::Dma;
use driver_graphics::{RenderDevice, RenderFeatures};
use syscall::{EINVAL, EIO, ENOMEM};
use virtio_core::spec::{Buffer, ChainBuilder, DescriptorFlags};
use virtio_core::transport::{Error, Queue};

use crate::{CommandTy, ControlHeader};

/// The features the driver acknowledged.
#[derive(Clone, Copy, Debug)]
pub struct Features {
    pub virgl: bool,
    pub resource_blob: bool,
    pub context_init: bool,
}

/// Bounds on what the device reports, for robustness: Linux's `MAX_CAPSET_ID` gives 64
/// capsets at most; no capset today is near 64 KiB (Venus's has 160 bytes, virgl's second
/// 1408) or has more than three versions.
const MAX_CAPSETS: u32 = 64;
const MAX_CAPSET_ID: u32 = 63;
const MAX_CAPSET_SIZE: u32 = 64 * 1024;
const MAX_CAPSET_VERSIONS: u32 = 8;

/// Contexts alive at once. Each Venus context is a worker on the host, so a bound keeps one
/// client from making the host start them without end; slice GP12 sets quotas per handle.
const MAX_CONTEXTS: usize = 256;

/// The header of a response, read with its type as a number, so that a type the driver does
/// not know is never taken for a `CommandTy`.
#[derive(Default)]
#[repr(C)]
struct ResponseHeader {
    ty: u32,
    flags: u32,
    fence_id: u64,
    ctx_id: u32,
    ring_index: u8,
    padding: [u8; 3],
}

const _: () = assert!(size_of::<ResponseHeader>() == size_of::<ControlHeader>());

#[repr(C)]
struct GetCapsetInfo {
    header: ControlHeader,
    capset_index: u32,
    padding: u32,
}

#[derive(Default)]
#[repr(C)]
struct CapsetInfo {
    header: ResponseHeader,
    capset_id: u32,
    capset_max_version: u32,
    capset_max_size: u32,
    padding: u32,
}

#[repr(C)]
struct GetCapset {
    header: ControlHeader,
    capset_id: u32,
    capset_version: u32,
}

#[repr(C)]
struct CtxCreate {
    header: ControlHeader,
    nlen: u32,
    context_init: u32,
    debug_name: [u8; 64],
}

struct Capset {
    id: u32,
    /// The capset's bytes at each version from 0 on.
    versions: Vec<Vec<u8>>,
}

pub struct VirtGpuRender {
    control_queue: Arc<Queue>,
    features: Features,
    capsets: Vec<Capset>,
    contexts: BTreeSet<u32>,
    next_context: u32,
}

impl fmt::Debug for VirtGpuRender {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VirtGpuRender")
            .field("features", &self.features)
            .field("capset_ids", &self.capset_ids())
            .field("contexts", &self.contexts)
            .finish()
    }
}

fn errno(error: Error) -> syscall::Error {
    syscall::Error::new(match error {
        Error::SyscallError(error) => error.errno(),
        _ => EIO,
    })
}

/// Sends one command whose answer is a bare header, and returns the answer's type.
async fn send<T: Send + 'static>(queue: &Queue, request: T) -> Result<u32, Error> {
    let request = Dma::new(request)?;
    let header = Dma::new(ResponseHeader::default())?;
    let command = ChainBuilder::new()
        .chain(Buffer::new(request))
        .chain(Buffer::new(header).flags(DescriptorFlags::WRITE_ONLY))
        .build();
    let (_, [_, header]) = queue.send(command).await.into_parts();
    Ok(header.into_inner::<Dma<ResponseHeader>>().ty)
}

/// Capset `index` of the device: its number and its bytes at each version, or None when the
/// device's answer is out of bounds.
async fn read_capset(queue: &Queue, index: u32) -> Result<Option<Capset>, Error> {
    let request = Dma::new(GetCapsetInfo {
        header: ControlHeader::with_ty(CommandTy::GetCapsetInfo),
        capset_index: index,
        padding: 0,
    })?;
    let info = Dma::new(CapsetInfo::default())?;
    let command = ChainBuilder::new()
        .chain(Buffer::new(request))
        .chain(Buffer::new(info).flags(DescriptorFlags::WRITE_ONLY))
        .build();
    let (written, [_, info]) = queue.send(command).await.into_parts();
    let info: Dma<CapsetInfo> = info.into_inner();
    if (written as usize) < size_of::<CapsetInfo>()
        || info.header.ty != CommandTy::RespOkCapsetInfo as u32
    {
        log::warn!(
            "virtio-gpu: capset {index}: the device answered type {:#x}",
            info.header.ty
        );
        return Ok(None);
    }
    let (id, max_size) = (info.capset_id, info.capset_max_size);
    if id > MAX_CAPSET_ID || max_size == 0 || max_size > MAX_CAPSET_SIZE {
        log::warn!("virtio-gpu: capset {index}: id {id}, {max_size} bytes, out of bounds");
        return Ok(None);
    }
    let mut versions = Vec::new();
    for version in 0..=info.capset_max_version.min(MAX_CAPSET_VERSIONS - 1) {
        let request = Dma::new(GetCapset {
            header: ControlHeader::with_ty(CommandTy::GetCapset),
            capset_id: id,
            capset_version: version,
        })?;
        let header_len = size_of::<ResponseHeader>();
        let response =
            unsafe { Dma::<[u8]>::zeroed_slice(header_len + max_size as usize)?.assume_init() };
        let command = ChainBuilder::new()
            .chain(Buffer::new(request))
            .chain(Buffer::new(response).flags(DescriptorFlags::WRITE_ONLY))
            .build();
        let (written, [_, response]) = queue.send(command).await.into_parts();
        let response: Dma<[u8]> = response.into_inner();
        let ty = u32::from_ne_bytes(response[..4].try_into().unwrap());
        if ty != CommandTy::RespOkCapset as u32 || (written as usize) < header_len {
            log::warn!(
                "virtio-gpu: capset {id} version {version}: the device answered type {ty:#x}"
            );
            break;
        }
        let len = (written as usize - header_len).min(max_size as usize);
        versions.push(response[header_len..header_len + len].to_vec());
    }
    if versions.is_empty() {
        return Ok(None);
    }
    log::info!(
        "virtio-gpu: capset {id}, versions 0 to {}, {max_size} bytes",
        versions.len() - 1
    );
    Ok(Some(Capset { id, versions }))
}

impl VirtGpuRender {
    /// The render side of a device that offers 3D, with the capsets it reports; None for a 2D
    /// device.
    pub fn new(control_queue: Arc<Queue>, features: Features, num_capsets: u32) -> Option<Self> {
        if !features.virgl {
            return None;
        }
        if num_capsets > MAX_CAPSETS {
            log::warn!("virtio-gpu: {num_capsets} capsets reported, reading {MAX_CAPSETS}");
        }
        let mut capsets: Vec<Capset> = Vec::new();
        futures::executor::block_on(async {
            for index in 0..num_capsets.min(MAX_CAPSETS) {
                match read_capset(&control_queue, index).await {
                    Ok(Some(capset)) if capsets.iter().all(|known| known.id != capset.id) => {
                        capsets.push(capset)
                    }
                    Ok(_) => {}
                    Err(error) => log::warn!("virtio-gpu: capset {index}: {error:?}"),
                }
            }
        });
        Some(Self {
            control_queue,
            features,
            capsets,
            contexts: BTreeSet::new(),
            next_context: 1,
        })
    }

    /// A context number no live context has; 0 is never one.
    fn allocate_context(&mut self) -> u32 {
        loop {
            let id = self.next_context;
            self.next_context = self.next_context.checked_add(1).unwrap_or(1);
            if id != 0 && !self.contexts.contains(&id) {
                return id;
            }
        }
    }
}

impl RenderDevice for VirtGpuRender {
    fn driver_info(&self) -> (&'static [u8], &'static [u8], [i32; 3]) {
        // Linux's virtio-gpu driver: Mesa's virtio drivers accept no other name, and version 0.
        (b"virtio_gpu", b"virtio GPU", [0, 1, 0])
    }

    fn features(&self) -> RenderFeatures {
        RenderFeatures {
            three_d: self.features.virgl,
            resource_blob: self.features.resource_blob,
            context_init: self.features.context_init,
            host_visible: false,
        }
    }

    fn capset_ids(&self) -> u64 {
        self.capsets
            .iter()
            .fold(0, |ids, capset| ids | (1 << capset.id))
    }

    fn capset(&self, id: u32, version: u32) -> Option<&[u8]> {
        let capset = self.capsets.iter().find(|capset| capset.id == id)?;
        capset.versions.get(version as usize).map(Vec::as_slice)
    }

    fn create_context(&mut self, capset_id: u32, name: &[u8]) -> syscall::Result<u32> {
        if self.contexts.len() >= MAX_CONTEXTS {
            return Err(syscall::Error::new(ENOMEM));
        }
        let id = self.allocate_context();
        let mut command = CtxCreate {
            header: ControlHeader {
                ctx_id: id,
                ..ControlHeader::with_ty(CommandTy::CtxCreate)
            },
            nlen: 0,
            context_init: capset_id,
            debug_name: [0; 64],
        };
        // The name field keeps a terminating zero, as Linux's strscpy leaves one.
        let len = name.len().min(command.debug_name.len() - 1);
        command.debug_name[..len].copy_from_slice(&name[..len]);
        command.nlen = len as u32;
        let ty = futures::executor::block_on(send(&self.control_queue, command)).map_err(errno)?;
        if ty != CommandTy::RespOkNodata as u32 {
            log::warn!("virtio-gpu: context {id}: the device answered type {ty:#x}");
            return Err(syscall::Error::new(
                if ty == CommandTy::RespErrOutOfMemory as u32 {
                    ENOMEM
                } else {
                    EINVAL
                },
            ));
        }
        self.contexts.insert(id);
        Ok(id)
    }

    fn destroy_context(&mut self, context_id: u32) {
        if !self.contexts.remove(&context_id) {
            return;
        }
        let command = ControlHeader {
            ctx_id: context_id,
            ..ControlHeader::with_ty(CommandTy::CtxDestroy)
        };
        match futures::executor::block_on(send(&self.control_queue, command)) {
            Ok(ty) if ty == CommandTy::RespOkNodata as u32 => {}
            Ok(ty) => log::warn!("virtio-gpu: context {context_id}: destroy answered {ty:#x}"),
            Err(error) => log::warn!("virtio-gpu: context {context_id}: destroy failed: {error:?}"),
        }
    }
}
