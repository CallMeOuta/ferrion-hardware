// SPDX-License-Identifier: GPL-2.0-only
use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use redox_ioctl::virtgpu;
use syscall::{EACCES, EINVAL, ENOENT, ENOMEM, Error, MapFlags, PAGE_SIZE, Result};

use crate::render::RenderDevice;

pub const BLOB_OFFSET_SHIFT: u32 = 40;
pub const BLOB_OFFSET_MASK: u64 = (1u64 << BLOB_OFFSET_SHIFT) - 1;
pub const MAX_BLOB_SLOT: u32 = (1u32 << 23) - 1;
pub const MAX_BLOBS_PER_HANDLE: usize = 4096;
pub const MAX_PRIME_EXPORTS_PER_HANDLE: usize = 4096;
pub const MAX_BLOB_CMD_SIZE: usize = 64 * 1024;

pub(crate) const PRIME_WIRE_RDWR: u32 = 1 << 0;
pub(crate) const PRIME_WIRE_CLOEXEC: u32 = 1 << 1;
pub(crate) const PRIME_WIRE_ALLOWED: u32 = PRIME_WIRE_RDWR | PRIME_WIRE_CLOEXEC;

const ALLOWED_BLOB_FLAGS: u32 = virtgpu::VIRTGPU_BLOB_FLAG_USE_MAPPABLE
    | virtgpu::VIRTGPU_BLOB_FLAG_USE_SHAREABLE
    | virtgpu::VIRTGPU_BLOB_FLAG_USE_CROSS_DEVICE;

#[derive(Clone, Copy, Debug)]
pub struct BlobCreateParams<'a> {
    pub context_id: u32,
    pub blob_mem: u32,
    pub blob_flags: u32,
    pub size: u64,
    pub blob_id: u64,
    pub blob_hints: u32,
    pub cmd: &'a [u8],
}

#[derive(Clone, Copy, Debug)]
pub struct BlobInfo {
    pub res_id: u32,
    pub size: u64,
    pub blob_mem: u32,
    pub blob_flags: u32,
}

struct SharedBlobInner {
    info: BlobInfo,
    release: Mutex<Option<Sender<u32>>>,
}

impl Drop for SharedBlobInner {
    fn drop(&mut self) {
        if let Ok(slot) = self.release.get_mut() {
            if let Some(tx) = slot.take() {
                let _ = tx.send(self.info.res_id);
            }
        }
    }
}

#[derive(Clone)]
pub struct SharedBlob {
    inner: Arc<SharedBlobInner>,
}

impl SharedBlob {
    pub fn new(info: BlobInfo, release_tx: Sender<u32>) -> Self {
        assert!(info.res_id != 0);
        assert!(info.size > 0);
        Self {
            inner: Arc::new(SharedBlobInner {
                info,
                release: Mutex::new(Some(release_tx)),
            }),
        }
    }

    pub fn info(&self) -> BlobInfo {
        self.inner.info
    }

    pub fn res_id(&self) -> u32 {
        self.inner.info.res_id
    }
}

impl fmt::Debug for SharedBlob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SharedBlob")
            .field("info", &self.inner.info)
            .finish()
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ExportCounter {
    active: Arc<AtomicUsize>,
}

impl ExportCounter {
    pub(crate) fn acquire(&self) -> Result<ExportGuard> {
        let mut current = self.active.load(Ordering::Relaxed);
        for _ in 0..MAX_PRIME_EXPORTS_PER_HANDLE {
            if current >= MAX_PRIME_EXPORTS_PER_HANDLE {
                return Err(Error::new(ENOMEM));
            }
            match self.active.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    return Ok(ExportGuard {
                        active: self.active.clone(),
                    });
                }
                Err(observed) => current = observed,
            }
        }
        Err(Error::new(ENOMEM))
    }
}

#[derive(Debug)]
pub(crate) struct ExportGuard {
    active: Arc<AtomicUsize>,
}

impl Drop for ExportGuard {
    fn drop(&mut self) {
        let prev = self.active.fetch_sub(1, Ordering::AcqRel);
        assert!(prev > 0);
    }
}

#[derive(Debug)]
struct SlotEntry {
    blob: SharedBlob,
    mapped_pages: usize,
    writable: bool,
    closed: bool,
}

#[derive(Debug, Default)]
pub(crate) struct RenderBlobs {
    bo_to_slot: HashMap<u32, u32>,
    res_to_bo: HashMap<u32, u32>,
    slots: HashMap<u32, SlotEntry>,
    next_slot: u32,
}

impl RenderBlobs {
    pub(crate) fn new() -> Self {
        Self {
            bo_to_slot: HashMap::new(),
            res_to_bo: HashMap::new(),
            slots: HashMap::new(),
            next_slot: 1,
        }
    }

    fn allocate_slot(&mut self) -> Result<u32> {
        if self.slots.len() >= MAX_BLOBS_PER_HANDLE {
            return Err(Error::new(ENOMEM));
        }
        for _ in 0..MAX_BLOB_SLOT {
            let slot = self.next_slot;
            self.next_slot = if slot >= MAX_BLOB_SLOT { 1 } else { slot + 1 };
            if slot != 0 && !self.slots.contains_key(&slot) && !self.bo_to_slot.contains_key(&slot) {
                return Ok(slot);
            }
        }
        Err(Error::new(ENOMEM))
    }

    pub(crate) fn create_blob(
        &mut self,
        device: &mut dyn RenderDevice,
        context_id: Option<u32>,
        mut data: virtgpu::VirtgpuResourceCreateBlob<'_>,
    ) -> Result<u32> {
        let features = device.features();
        if !features.resource_blob {
            return Err(Error::new(EINVAL));
        }
        if data.pad() != 0 || data.pad2() != 0 {
            return Err(Error::new(EINVAL));
        }
        let blob_mem = data.blob_mem();
        let blob_flags = data.blob_flags();
        let blob_hints = data.blob_hints();
        if blob_flags & !ALLOWED_BLOB_FLAGS != 0 {
            return Err(Error::new(EINVAL));
        }
        if blob_flags & virtgpu::VIRTGPU_BLOB_FLAG_USE_CROSS_DEVICE != 0 {
            return Err(Error::new(EINVAL));
        }
        if blob_hints & !virtgpu::DRM_VIRTGPU_BLOB_FLAG_HINT_DEFER_MAPPING != 0 {
            return Err(Error::new(EINVAL));
        }
        if blob_mem != virtgpu::VIRTGPU_BLOB_MEM_HOST3D {
            return Err(Error::new(EINVAL));
        }
        if !features.three_d {
            return Err(Error::new(EINVAL));
        }
        if blob_flags & virtgpu::VIRTGPU_BLOB_FLAG_USE_MAPPABLE != 0 && !features.host_visible {
            return Err(Error::new(EINVAL));
        }
        let Some(context_id) = context_id else {
            return Err(Error::new(EINVAL));
        };
        let req_size = data.size();
        if req_size == 0 {
            return Err(Error::new(EINVAL));
        }
        let aligned_size = req_size
            .checked_next_multiple_of(PAGE_SIZE as u64)
            .ok_or(Error::new(EINVAL))?;
        if aligned_size > BLOB_OFFSET_MASK + 1 || usize::try_from(aligned_size).is_err() {
            return Err(Error::new(EINVAL));
        }
        let cmd = data.cmd();
        if cmd.len() > MAX_BLOB_CMD_SIZE {
            return Err(Error::new(EINVAL));
        }

        let slot = self.allocate_slot()?;
        let blob = device.create_blob(BlobCreateParams {
            context_id,
            blob_mem,
            blob_flags,
            size: aligned_size,
            blob_id: data.blob_id(),
            blob_hints,
            cmd,
        })?;
        let res_id = blob.res_id();
        self.slots.insert(
            slot,
            SlotEntry {
                blob,
                mapped_pages: 0,
                writable: true,
                closed: false,
            },
        );
        self.bo_to_slot.insert(slot, slot);
        self.res_to_bo.insert(res_id, slot);
        data.set_bo_handle(slot);
        data.set_res_handle(res_id);
        Ok(slot)
    }

    pub(crate) fn import_blob(&mut self, blob: SharedBlob, writable: bool) -> Result<u32> {
        let res_id = blob.res_id();
        if let Some(&bo_handle) = self.res_to_bo.get(&res_id) {
            if let Some(&slot) = self.bo_to_slot.get(&bo_handle) {
                if let Some(entry) = self.slots.get_mut(&slot) {
                    assert!(!entry.closed);
                    if writable {
                        entry.writable = true;
                    }
                    return Ok(bo_handle);
                }
            }
        }
        let slot = self.allocate_slot()?;
        self.slots.insert(
            slot,
            SlotEntry {
                blob,
                mapped_pages: 0,
                writable,
                closed: false,
            },
        );
        self.bo_to_slot.insert(slot, slot);
        self.res_to_bo.insert(res_id, slot);
        Ok(slot)
    }

    pub(crate) fn export_blob(&self, bo_handle: u32, want_writable: bool) -> Result<SharedBlob> {
        let Some(&slot) = self.bo_to_slot.get(&bo_handle) else {
            return Err(Error::new(ENOENT));
        };
        let Some(entry) = self.slots.get(&slot) else {
            return Err(Error::new(ENOENT));
        };
        if entry.closed {
            return Err(Error::new(ENOENT));
        }
        if entry.blob.info().blob_flags & virtgpu::VIRTGPU_BLOB_FLAG_USE_SHAREABLE == 0 {
            return Err(Error::new(EINVAL));
        }
        if want_writable && !entry.writable {
            return Err(Error::new(EACCES));
        }
        Ok(entry.blob.clone())
    }

    pub(crate) fn map_ioctl(&self, mut data: virtgpu::VirtgpuMap<'_>) -> Result<usize> {
        let Some(&slot) = self.bo_to_slot.get(&data.handle()) else {
            return Err(Error::new(ENOENT));
        };
        let Some(entry) = self.slots.get(&slot) else {
            return Err(Error::new(ENOENT));
        };
        if entry.blob.info().blob_flags & virtgpu::VIRTGPU_BLOB_FLAG_USE_MAPPABLE == 0 {
            return Err(Error::new(EINVAL));
        }
        data.set_offset(u64::from(slot) << BLOB_OFFSET_SHIFT);
        Ok(0)
    }

    pub(crate) fn resource_info(
        &self,
        mut data: virtgpu::VirtgpuResourceInfo<'_>,
    ) -> Result<usize> {
        let Some(&slot) = self.bo_to_slot.get(&data.bo_handle()) else {
            return Err(Error::new(ENOENT));
        };
        let Some(entry) = self.slots.get(&slot) else {
            return Err(Error::new(ENOENT));
        };
        let info = entry.blob.info();
        data.set_res_handle(info.res_id);
        data.set_size(info.size as u32);
        data.set_blob_mem(info.blob_mem);
        Ok(0)
    }

    pub(crate) fn gem_close(&mut self, data: virtgpu::GemClose<'_>) -> Result<usize> {
        if data.pad() != 0 {
            return Err(Error::new(EINVAL));
        }
        let Some(slot) = self.bo_to_slot.remove(&data.handle()) else {
            return Err(Error::new(EINVAL));
        };
        let mut remove_slot = false;
        if let Some(entry) = self.slots.get_mut(&slot) {
            self.res_to_bo.remove(&entry.blob.res_id());
            entry.closed = true;
            if entry.mapped_pages == 0 {
                remove_slot = true;
            }
        }
        if remove_slot {
            self.slots.remove(&slot);
        }
        Ok(0)
    }

    pub(crate) fn mmap_prep(
        &mut self,
        device: &mut dyn RenderDevice,
        offset: u64,
        size: usize,
        flags: MapFlags,
    ) -> Result<usize> {
        if offset & (1u64 << 63) != 0 || size == 0 {
            return Err(Error::new(EINVAL));
        }
        let slot = (offset >> BLOB_OFFSET_SHIFT) as u32;
        let in_blob = offset & BLOB_OFFSET_MASK;
        if in_blob % PAGE_SIZE as u64 != 0 {
            return Err(Error::new(EINVAL));
        }
        let aligned_len = size
            .checked_next_multiple_of(PAGE_SIZE)
            .ok_or(Error::new(EINVAL))?;
        let end = in_blob
            .checked_add(aligned_len as u64)
            .ok_or(Error::new(EINVAL))?;
        let (res_id, blob_size, mappable, writable) = {
            let entry = self.slots.get(&slot).ok_or(Error::new(ENOENT))?;
            if entry.closed {
                return Err(Error::new(ENOENT));
            }
            let info = entry.blob.info();
            (
                info.res_id,
                info.size,
                info.blob_flags & virtgpu::VIRTGPU_BLOB_FLAG_USE_MAPPABLE != 0,
                entry.writable,
            )
        };
        if !writable && flags.contains(MapFlags::PROT_WRITE) {
            return Err(Error::new(EACCES));
        }
        if !mappable || end > blob_size {
            return Err(Error::new(EINVAL));
        }
        let pages = aligned_len / PAGE_SIZE;
        let base = device.map_blob(res_id, pages)?;
        let addr = base
            .checked_add(usize::try_from(in_blob).map_err(|_| Error::new(EINVAL))?)
            .ok_or(Error::new(EINVAL))?;
        if let Some(entry) = self.slots.get_mut(&slot) {
            entry.mapped_pages = entry.mapped_pages.saturating_add(pages);
        }
        Ok(addr)
    }

    pub(crate) fn munmap(
        &mut self,
        device: &mut dyn RenderDevice,
        offset: u64,
        size: usize,
    ) -> Result<()> {
        if offset & (1u64 << 63) != 0 {
            return Ok(());
        }
        let slot = (offset >> BLOB_OFFSET_SHIFT) as u32;
        let req_pages = size.div_ceil(PAGE_SIZE);
        let mut unmap_info = None;
        let mut remove_slot = false;
        if let Some(entry) = self.slots.get_mut(&slot) {
            let actual_pages = req_pages.min(entry.mapped_pages);
            entry.mapped_pages -= actual_pages;
            if actual_pages > 0 {
                unmap_info = Some((entry.blob.res_id(), actual_pages));
            }
            if entry.mapped_pages == 0 && entry.closed {
                remove_slot = true;
            }
        }
        if let Some((res_id, pages)) = unmap_info {
            device.unmap_blob(res_id, pages);
        }
        if remove_slot {
            self.slots.remove(&slot);
        }
        Ok(())
    }

    pub(crate) fn destroy_all(&mut self, device: &mut dyn RenderDevice) {
        self.bo_to_slot.clear();
        self.res_to_bo.clear();
        for (_, entry) in self.slots.drain() {
            if entry.mapped_pages > 0 {
                device.unmap_blob(entry.blob.res_id(), entry.mapped_pages);
            }
        }
    }
}
