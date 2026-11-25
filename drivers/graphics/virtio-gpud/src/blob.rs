// SPDX-License-Identifier: GPL-2.0-only
use std::collections::HashMap;
use std::fmt;
use std::sync::mpsc::{self, Receiver, Sender};

use common::dma::Dma;
use common::{MemoryType, PhysBorrowed, Prot};
use driver_graphics::{BlobCreateParams, BlobInfo, SharedBlob};
use syscall::{EINVAL, ENOENT, ENOMEM, PAGE_SIZE};
use virtio_core::spec::{Buffer, ChainBuilder, DescriptorFlags};
use virtio_core::transport::{Error, Queue};
use virtio_core::SharedMemoryRegion;

use crate::render::{errno, send, ResponseHeader};
use crate::{CommandTy, ControlHeader, ResourceId, ResourceUnref};

pub const VIRTIO_GPU_SHM_ID_HOST_VISIBLE: u8 = 1;

const VIRTIO_GPU_BLOB_MEM_HOST3D: u32 = 0x0002;
const VIRTIO_GPU_BLOB_FLAG_USE_MAPPABLE: u32 = 0x0001;

const VIRTIO_GPU_MAP_CACHE_MASK: u32 = 0x0f;
const VIRTIO_GPU_MAP_CACHE_CACHED: u32 = 0x01;
const VIRTIO_GPU_MAP_CACHE_UNCACHED: u32 = 0x02;
const VIRTIO_GPU_MAP_CACHE_WC: u32 = 0x03;

#[repr(C)]
struct ResourceCreateBlob {
    header: ControlHeader,
    resource_id: u32,
    blob_mem: u32,
    blob_flags: u32,
    nr_entries: u32,
    blob_id: u64,
    size: u64,
}

const _: () = assert!(size_of::<ResourceCreateBlob>() == 56);

#[repr(C)]
struct ResourceMapBlob {
    header: ControlHeader,
    resource_id: u32,
    padding: u32,
    offset: u64,
}

const _: () = assert!(size_of::<ResourceMapBlob>() == 40);

#[derive(Default)]
#[repr(C)]
struct RespMapInfo {
    header: ResponseHeader,
    map_info: u32,
    padding: u32,
}

const _: () = assert!(size_of::<RespMapInfo>() == 32);

#[repr(C)]
struct ResourceUnmapBlob {
    header: ControlHeader,
    resource_id: u32,
    padding: u32,
}

const _: () = assert!(size_of::<ResourceUnmapBlob>() == 32);

#[repr(C)]
struct CmdSubmit3d {
    header: ControlHeader,
    size: u32,
    padding: u32,
}

const _: () = assert!(size_of::<CmdSubmit3d>() == 32);

#[derive(Debug)]
pub struct WindowAllocator {
    free: Vec<(u64, u64)>,
}

impl WindowAllocator {
    pub fn new(length: u64) -> Self {
        let aligned = length & !(PAGE_SIZE as u64 - 1);
        let free = if aligned > 0 {
            vec![(0, aligned)]
        } else {
            Vec::new()
        };
        Self { free }
    }

    pub fn alloc(&mut self, size: u64) -> Option<u64> {
        if size == 0 || size % PAGE_SIZE as u64 != 0 {
            return None;
        }
        let pos = self.free.iter().position(|&(_, len)| len >= size)?;
        let (start, len) = self.free[pos];
        if len == size {
            self.free.remove(pos);
        } else {
            self.free[pos] = (start + size, len - size);
        }
        Some(start)
    }

    pub fn free(&mut self, offset: u64, size: u64) {
        if size == 0 {
            return;
        }
        let idx = self.free.partition_point(|&(start, _)| start < offset);
        self.free.insert(idx, (offset, size));
        if idx + 1 < self.free.len() {
            let (cur_start, cur_len) = self.free[idx];
            let (next_start, next_len) = self.free[idx + 1];
            if cur_start.checked_add(cur_len) == Some(next_start) {
                self.free[idx] = (cur_start, cur_len + next_len);
                self.free.remove(idx + 1);
            }
        }
        if idx > 0 {
            let (prev_start, prev_len) = self.free[idx - 1];
            let (cur_start, cur_len) = self.free[idx];
            if prev_start.checked_add(prev_len) == Some(cur_start) {
                self.free[idx - 1] = (prev_start, prev_len + cur_len);
                self.free.remove(idx);
            }
        }
    }
}

struct HostVisibleWindow {
    region: SharedMemoryRegion,
    allocator: WindowAllocator,
}

struct MappedBlob {
    offset: u64,
    size: u64,
    mapping: PhysBorrowed,
}

struct VirtGpuBlob {
    size: u64,
    blob_mem: u32,
    blob_flags: u32,
    mapped: Option<MappedBlob>,
    mapped_pages: usize,
}

pub struct VirtGpuBlobs {
    window: Option<HostVisibleWindow>,
    blobs: HashMap<u32, VirtGpuBlob>,
    release_tx: Sender<u32>,
    release_rx: Receiver<u32>,
    logged_first_map: bool,
}

impl fmt::Debug for VirtGpuBlobs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VirtGpuBlobs")
            .field("window", &self.window.as_ref().map(|w| w.region))
            .field("blob_count", &self.blobs.len())
            .finish()
    }
}

fn device_errno(ty: u32) -> syscall::Error {
    syscall::Error::new(if ty == CommandTy::RespErrOutOfMemory as u32 {
        ENOMEM
    } else {
        EINVAL
    })
}

fn map_info_memory_type(map_info: u32) -> Option<MemoryType> {
    match map_info & VIRTIO_GPU_MAP_CACHE_MASK {
        VIRTIO_GPU_MAP_CACHE_CACHED => Some(MemoryType::Writeback),
        VIRTIO_GPU_MAP_CACHE_UNCACHED => Some(MemoryType::Uncacheable),
        VIRTIO_GPU_MAP_CACHE_WC => Some(MemoryType::WriteCombining),
        _ => None,
    }
}

async fn send_submit_3d(queue: &Queue, context_id: u32, cmd: &[u8]) -> Result<u32, Error> {
    let request = Dma::new(CmdSubmit3d {
        header: ControlHeader {
            ctx_id: context_id,
            ..ControlHeader::with_ty(CommandTy::Submit3d)
        },
        size: cmd.len() as u32,
        padding: 0,
    })?;
    let mut payload = unsafe { Dma::<[u8]>::zeroed_slice(cmd.len())?.assume_init() };
    payload.copy_from_slice(cmd);
    let header = Dma::new(ResponseHeader::default())?;
    let command = ChainBuilder::new()
        .chain(Buffer::new(request))
        .chain(Buffer::new(payload))
        .chain(Buffer::new(header).flags(DescriptorFlags::WRITE_ONLY))
        .build();
    let (_, [_, _, header]) = queue.send(command).await.into_parts();
    Ok(header.into_inner::<Dma<ResponseHeader>>().ty)
}

async fn send_map_blob(
    queue: &Queue,
    resource_id: u32,
    offset: u64,
) -> Result<(u32, RespMapInfo), Error> {
    let request = Dma::new(ResourceMapBlob {
        header: ControlHeader::with_ty(CommandTy::ResourceMapBlob),
        resource_id,
        padding: 0,
        offset,
    })?;
    let response = Dma::new(RespMapInfo::default())?;
    let command = ChainBuilder::new()
        .chain(Buffer::new(request))
        .chain(Buffer::new(response).flags(DescriptorFlags::WRITE_ONLY))
        .build();
    let (written, [_, response]) = queue.send(command).await.into_parts();
    let response: Dma<RespMapInfo> = response.into_inner();
    let copy = RespMapInfo {
        header: ResponseHeader {
            ty: response.header.ty,
            flags: response.header.flags,
            fence_id: response.header.fence_id,
            ctx_id: response.header.ctx_id,
            ring_index: response.header.ring_index,
            padding: response.header.padding,
        },
        map_info: response.map_info,
        padding: response.padding,
    };
    Ok((written, copy))
}

async fn unmap_on_device_async(queue: &Queue, resource_id: u32) -> bool {
    let command = ResourceUnmapBlob {
        header: ControlHeader::with_ty(CommandTy::ResourceUnmapBlob),
        resource_id,
        padding: 0,
    };
    match send(queue, command).await {
        Ok(ty) if ty == CommandTy::RespOkNodata as u32 => true,
        Ok(ty) => {
            log::warn!("virtio-gpu: blob {resource_id}: unmap answered {ty:#x}");
            false
        }
        Err(error) => {
            log::warn!("virtio-gpu: blob {resource_id}: unmap failed: {error:?}");
            false
        }
    }
}

fn unmap_on_device(queue: &Queue, resource_id: u32) -> bool {
    futures::executor::block_on(unmap_on_device_async(queue, resource_id))
}

impl VirtGpuBlobs {
    pub fn new(region: Option<SharedMemoryRegion>) -> Self {
        let window = region.map(|region| {
            let allocator = WindowAllocator::new(region.length);
            HostVisibleWindow { region, allocator }
        });
        let (release_tx, release_rx) = mpsc::channel();
        Self {
            window,
            blobs: HashMap::new(),
            release_tx,
            release_rx,
            logged_first_map: false,
        }
    }

    pub fn has_window(&self) -> bool {
        self.window.is_some()
    }

    pub async fn release_dropped_async(&mut self, queue: &Queue) {
        while let Ok(res_id) = self.release_rx.try_recv() {
            self.destroy_blob_async(queue, res_id).await;
        }
    }

    pub fn release_dropped(&mut self, queue: &Queue) {
        futures::executor::block_on(self.release_dropped_async(queue));
    }

    pub fn create_blob(
        &mut self,
        queue: &Queue,
        params: BlobCreateParams<'_>,
    ) -> syscall::Result<SharedBlob> {
        self.release_dropped(queue);
        if params.blob_mem != VIRTIO_GPU_BLOB_MEM_HOST3D {
            return Err(syscall::Error::new(EINVAL));
        }
        if params.blob_flags & VIRTIO_GPU_BLOB_FLAG_USE_MAPPABLE != 0 {
            let Some(window) = &self.window else {
                return Err(syscall::Error::new(EINVAL));
            };
            if params.size > window.region.length {
                return Err(syscall::Error::new(ENOMEM));
            }
        }
        if !params.cmd.is_empty() {
            let ty = futures::executor::block_on(send_submit_3d(
                queue,
                params.context_id,
                params.cmd,
            ))
            .map_err(errno)?;
            if ty != CommandTy::RespOkNodata as u32 {
                log::warn!(
                    "virtio-gpu: context {}: blob create cmd answered {ty:#x}",
                    params.context_id
                );
                return Err(device_errno(ty));
            }
        }
        let res_id = ResourceId::alloc();
        let command = ResourceCreateBlob {
            header: ControlHeader {
                ctx_id: params.context_id,
                ..ControlHeader::with_ty(CommandTy::ResourceCreateBlob)
            },
            resource_id: res_id.raw(),
            blob_mem: params.blob_mem,
            blob_flags: params.blob_flags,
            nr_entries: 0,
            blob_id: params.blob_id,
            size: params.size,
        };
        let ty = futures::executor::block_on(send(queue, command)).map_err(errno)?;
        if ty != CommandTy::RespOkNodata as u32 {
            log::warn!(
                "virtio-gpu: blob {} create answered {ty:#x}",
                res_id.raw()
            );
            return Err(device_errno(ty));
        }
        self.blobs.insert(
            res_id.raw(),
            VirtGpuBlob {
                size: params.size,
                blob_mem: params.blob_mem,
                blob_flags: params.blob_flags,
                mapped: None,
                mapped_pages: 0,
            },
        );
        let info = BlobInfo {
            res_id: res_id.raw(),
            size: params.size,
            blob_mem: params.blob_mem,
            blob_flags: params.blob_flags,
        };
        Ok(SharedBlob::new(info, self.release_tx.clone()))
    }

    pub fn map_blob(
        &mut self,
        queue: &Queue,
        res_id: u32,
        pages: usize,
    ) -> syscall::Result<usize> {
        self.release_dropped(queue);
        let Some(blob) = self.blobs.get_mut(&res_id) else {
            return Err(syscall::Error::new(ENOENT));
        };
        if let Some(mapped) = &blob.mapped {
            blob.mapped_pages = blob.mapped_pages.saturating_add(pages);
            return Ok(mapped.mapping.as_ptr() as usize);
        }
        if blob.blob_flags & VIRTIO_GPU_BLOB_FLAG_USE_MAPPABLE == 0 {
            return Err(syscall::Error::new(EINVAL));
        }
        let size = blob.size;
        let len = usize::try_from(size).map_err(|_| syscall::Error::new(EINVAL))?;
        let (phys_base, offset) = {
            let Some(window) = self.window.as_mut() else {
                return Err(syscall::Error::new(EINVAL));
            };
            let Some(offset) = window.allocator.alloc(size) else {
                return Err(syscall::Error::new(ENOMEM));
            };
            let Some(phys) = window.region.phys_addr.checked_add(offset) else {
                window.allocator.free(offset, size);
                return Err(syscall::Error::new(EINVAL));
            };
            (phys, offset)
        };

        let (written, resp) = match futures::executor::block_on(send_map_blob(queue, res_id, offset))
        {
            Ok(pair) => pair,
            Err(error) => {
                if let Some(window) = self.window.as_mut() {
                    window.allocator.free(offset, size);
                }
                return Err(errno(error));
            }
        };
        if (written as usize) < size_of::<RespMapInfo>()
            || resp.header.ty != CommandTy::RespOkMapInfo as u32
        {
            log::warn!(
                "virtio-gpu: blob {res_id} map at {offset:#x} answered {:#x}",
                resp.header.ty
            );
            if let Some(window) = self.window.as_mut() {
                window.allocator.free(offset, size);
            }
            return Err(device_errno(resp.header.ty));
        }
        let Some(mem_type) = map_info_memory_type(resp.map_info) else {
            log::warn!(
                "virtio-gpu: blob {res_id} map returned unsupported map_info {:#x}",
                resp.map_info
            );
            if unmap_on_device(queue, res_id) {
                if let Some(window) = self.window.as_mut() {
                    window.allocator.free(offset, size);
                }
            }
            return Err(syscall::Error::new(EINVAL));
        };
        let phys_usize = usize::try_from(phys_base).map_err(|_| syscall::Error::new(EINVAL))?;
        let mapping = match PhysBorrowed::map(phys_usize, len, Prot::RW, mem_type) {
            Ok(mapping) => mapping,
            Err(error) => {
                log::warn!(
                    "virtio-gpu: blob {res_id} physmap {phys_base:#x} +{len:#x} ({mem_type}) failed: {error:?}"
                );
                if unmap_on_device(queue, res_id) {
                    if let Some(window) = self.window.as_mut() {
                        window.allocator.free(offset, size);
                    }
                }
                return Err(syscall::Error::new(error.errno()));
            }
        };
        let addr = mapping.as_ptr() as usize;
        if !self.logged_first_map {
            self.logged_first_map = true;
            log::info!(
                "virtio-gpu: first map-blob res={res_id} offset={offset:#x} size={size:#x} map_info={:#x} ({mem_type})",
                resp.map_info
            );
        }
        if let Some(blob) = self.blobs.get_mut(&res_id) {
            blob.mapped = Some(MappedBlob {
                offset,
                size,
                mapping,
            });
            blob.mapped_pages = blob.mapped_pages.saturating_add(pages);
        }
        Ok(addr)
    }

    async fn force_unmap_blob_async(&mut self, queue: &Queue, res_id: u32) {
        let Some(blob) = self.blobs.get_mut(&res_id) else {
            return;
        };
        blob.mapped_pages = 0;
        let Some(mapped) = blob.mapped.take() else {
            return;
        };
        let (offset, size) = (mapped.offset, mapped.size);
        drop(mapped.mapping);
        if unmap_on_device_async(queue, res_id).await {
            if let Some(window) = self.window.as_mut() {
                window.allocator.free(offset, size);
            }
        }
    }

    fn force_unmap_blob(&mut self, queue: &Queue, res_id: u32) {
        futures::executor::block_on(self.force_unmap_blob_async(queue, res_id));
    }

    pub fn unmap_blob(&mut self, queue: &Queue, res_id: u32, pages: usize) {
        let should_unmap = match self.blobs.get_mut(&res_id) {
            Some(blob) => {
                blob.mapped_pages = blob.mapped_pages.saturating_sub(pages);
                blob.mapped_pages == 0
            }
            None => return,
        };
        if should_unmap {
            self.force_unmap_blob(queue, res_id);
        }
    }

    async fn destroy_blob_async(&mut self, queue: &Queue, res_id: u32) {
        self.force_unmap_blob_async(queue, res_id).await;
        if self.blobs.remove(&res_id).is_none() {
            return;
        }
        let command = ResourceUnref::new(ResourceId::from_raw(res_id));
        match send(queue, command).await {
            Ok(ty) if ty == CommandTy::RespOkNodata as u32 => {}
            Ok(ty) => log::warn!("virtio-gpu: blob {res_id}: unref answered {ty:#x}"),
            Err(error) => log::warn!("virtio-gpu: blob {res_id}: unref failed: {error:?}"),
        }
    }
}
