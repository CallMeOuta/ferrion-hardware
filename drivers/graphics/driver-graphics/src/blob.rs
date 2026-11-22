use std::collections::HashMap;

use redox_ioctl::virtgpu;
use syscall::{EINVAL, ENOENT, ENOMEM, Error, MapFlags, PAGE_SIZE, Result};

use crate::render::RenderDevice;

pub const BLOB_OFFSET_SHIFT: u32 = 40;
pub const BLOB_OFFSET_MASK: u64 = (1u64 << BLOB_OFFSET_SHIFT) - 1;
pub const MAX_BLOB_SLOT: u32 = (1u32 << 23) - 1;
pub const MAX_BLOBS_PER_HANDLE: usize = 4096;
pub const MAX_BLOB_CMD_SIZE: usize = 64 * 1024;

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

#[derive(Debug)]
struct SlotEntry {
    info: BlobInfo,
    mapped_pages: usize,
    closed: bool,
}

#[derive(Debug, Default)]
pub(crate) struct RenderBlobs {
    bo_to_slot: HashMap<u32, u32>,
    slots: HashMap<u32, SlotEntry>,
    next_slot: u32,
}

impl RenderBlobs {
    pub(crate) fn new() -> Self {
        Self {
            bo_to_slot: HashMap::new(),
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
        let info = device.create_blob(BlobCreateParams {
            context_id,
            blob_mem,
            blob_flags,
            size: aligned_size,
            blob_id: data.blob_id(),
            blob_hints,
            cmd,
        })?;
        self.slots.insert(
            slot,
            SlotEntry {
                info,
                mapped_pages: 0,
                closed: false,
            },
        );
        self.bo_to_slot.insert(slot, slot);
        data.set_bo_handle(slot);
        data.set_res_handle(info.res_id);
        Ok(slot)
    }

    pub(crate) fn map_ioctl(&self, mut data: virtgpu::VirtgpuMap<'_>) -> Result<usize> {
        let Some(&slot) = self.bo_to_slot.get(&data.handle()) else {
            return Err(Error::new(ENOENT));
        };
        let Some(entry) = self.slots.get(&slot) else {
            return Err(Error::new(ENOENT));
        };
        if entry.info.blob_flags & virtgpu::VIRTGPU_BLOB_FLAG_USE_MAPPABLE == 0 {
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
        data.set_res_handle(entry.info.res_id);
        data.set_size(entry.info.size as u32);
        data.set_blob_mem(entry.info.blob_mem);
        Ok(0)
    }

    pub(crate) fn gem_close(
        &mut self,
        device: &mut dyn RenderDevice,
        data: virtgpu::GemClose<'_>,
    ) -> Result<usize> {
        if data.pad() != 0 {
            return Err(Error::new(EINVAL));
        }
        let Some(slot) = self.bo_to_slot.remove(&data.handle()) else {
            return Err(Error::new(EINVAL));
        };
        let mut destroy_res = None;
        if let Some(entry) = self.slots.get_mut(&slot) {
            entry.closed = true;
            if entry.mapped_pages == 0 {
                destroy_res = Some(entry.info.res_id);
            }
        }
        if let Some(res_id) = destroy_res {
            self.slots.remove(&slot);
            device.destroy_blob(res_id);
        }
        Ok(0)
    }

    pub(crate) fn mmap_prep(
        &mut self,
        device: &mut dyn RenderDevice,
        offset: u64,
        size: usize,
        _flags: MapFlags,
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
        let (res_id, blob_size, mappable) = {
            let entry = self.slots.get(&slot).ok_or(Error::new(ENOENT))?;
            if entry.closed {
                return Err(Error::new(ENOENT));
            }
            (
                entry.info.res_id,
                entry.info.size,
                entry.info.blob_flags & virtgpu::VIRTGPU_BLOB_FLAG_USE_MAPPABLE != 0,
            )
        };
        if !mappable || end > blob_size {
            return Err(Error::new(EINVAL));
        }
        let base = device.map_blob(res_id)?;
        let addr = base
            .checked_add(usize::try_from(in_blob).map_err(|_| Error::new(EINVAL))?)
            .ok_or(Error::new(EINVAL))?;
        if let Some(entry) = self.slots.get_mut(&slot) {
            entry.mapped_pages = entry.mapped_pages.saturating_add(aligned_len / PAGE_SIZE);
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
        let pages = size.div_ceil(PAGE_SIZE);
        let mut unmap_res = None;
        let mut destroy_res = None;
        if let Some(entry) = self.slots.get_mut(&slot) {
            entry.mapped_pages = entry.mapped_pages.saturating_sub(pages);
            if entry.mapped_pages == 0 {
                unmap_res = Some(entry.info.res_id);
                if entry.closed {
                    destroy_res = Some(entry.info.res_id);
                }
            }
        }
        if let Some(res_id) = destroy_res {
            self.slots.remove(&slot);
            device.destroy_blob(res_id);
        } else if let Some(res_id) = unmap_res {
            device.unmap_blob(res_id);
        }
        Ok(())
    }

    pub(crate) fn destroy_all(&mut self, device: &mut dyn RenderDevice) {
        self.bo_to_slot.clear();
        for (_, entry) in self.slots.drain() {
            device.destroy_blob(entry.info.res_id);
        }
    }
}
