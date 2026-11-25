// SPDX-License-Identifier: GPL-2.0-only AND MIT
use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::Arc;

use redox_ioctl::virtgpu;
use scheme_utils::ResourceSync;
use syscall::{EACCES, EINVAL, ENODEV, Error, MapFlags, MunmapFlags, PAGE_SIZE, Result};

use crate::blob::{ExportGuard, SharedBlob};
use crate::{Buffer, GraphicsAdapter, GraphicsResource, GraphicsSchemeData};

#[derive(Debug)]
pub(crate) struct DrmBuffer<T: GraphicsAdapter> {
    pub buffer: Arc<T::Buffer>,
    pub writable: bool,
    pub(crate) _export_guard: ExportGuard,
}

impl<T: GraphicsAdapter> ResourceSync for DrmBuffer<T> {
    type SchemeData = GraphicsSchemeData<T>;
    type ResourceEnum = GraphicsResource<T>;

    fn fsize(&mut self, _scheme_data: &mut Self::SchemeData) -> Result<u64> {
        Ok(self.buffer.size() as u64)
    }

    fn mmap_prep(
        &mut self,
        scheme_data: &mut Self::SchemeData,
        offset: u64,
        size: usize,
        flags: MapFlags,
    ) -> Result<usize> {
        if !self.writable && flags.contains(MapFlags::PROT_WRITE) {
            return Err(Error::new(EACCES));
        }
        if offset != 0 {
            return Err(Error::new(EINVAL));
        }
        if size > self.buffer.size() {
            return Err(Error::new(EINVAL));
        }
        let ptr = scheme_data.adapter.map_dumb_buffer(&self.buffer);
        Ok(ptr as usize)
    }
}

#[derive(Debug)]
pub(crate) struct BlobBuffer<T: GraphicsAdapter> {
    pub(crate) blob: SharedBlob,
    pub(crate) writable: bool,
    mapped_pages: usize,
    _export_guard: ExportGuard,
    _adapter: PhantomData<T>,
}

impl<T: GraphicsAdapter> BlobBuffer<T> {
    pub(crate) fn new(blob: SharedBlob, writable: bool, export_guard: ExportGuard) -> Self {
        Self {
            blob,
            writable,
            mapped_pages: 0,
            _export_guard: export_guard,
            _adapter: PhantomData,
        }
    }
}

impl<T: GraphicsAdapter> ResourceSync for BlobBuffer<T> {
    type SchemeData = GraphicsSchemeData<T>;
    type ResourceEnum = GraphicsResource<T>;

    fn fsize(&mut self, _scheme_data: &mut Self::SchemeData) -> Result<u64> {
        Ok(self.blob.info().size)
    }

    fn mmap_prep(
        &mut self,
        scheme_data: &mut Self::SchemeData,
        offset: u64,
        size: usize,
        flags: MapFlags,
    ) -> Result<usize> {
        if !self.writable && flags.contains(MapFlags::PROT_WRITE) {
            return Err(Error::new(EACCES));
        }
        if size == 0 || offset % PAGE_SIZE as u64 != 0 {
            return Err(Error::new(EINVAL));
        }
        let info = self.blob.info();
        if info.blob_flags & virtgpu::VIRTGPU_BLOB_FLAG_USE_MAPPABLE == 0 {
            return Err(Error::new(EINVAL));
        }
        let aligned_len = size
            .checked_next_multiple_of(PAGE_SIZE)
            .ok_or(Error::new(EINVAL))?;
        let end = offset
            .checked_add(aligned_len as u64)
            .ok_or(Error::new(EINVAL))?;
        if end > info.size {
            return Err(Error::new(EINVAL));
        }
        let pages = aligned_len / PAGE_SIZE;
        let device = scheme_data
            .adapter
            .render_device()
            .ok_or(Error::new(ENODEV))?;
        let base = device.map_blob(info.res_id, pages)?;
        let addr = base
            .checked_add(usize::try_from(offset).map_err(|_| Error::new(EINVAL))?)
            .ok_or(Error::new(EINVAL))?;
        self.mapped_pages = self.mapped_pages.saturating_add(pages);
        Ok(addr)
    }

    fn munmap(
        &mut self,
        scheme_data: &mut Self::SchemeData,
        _offset: u64,
        size: usize,
        _flags: MunmapFlags,
    ) -> Result<()> {
        let pages = size.div_ceil(PAGE_SIZE).min(self.mapped_pages);
        if pages > 0 {
            self.mapped_pages -= pages;
            if let Some(device) = scheme_data.adapter.render_device() {
                device.unmap_blob(self.blob.res_id(), pages);
            }
        }
        Ok(())
    }

    fn on_close(mut self, scheme_data: &mut Self::SchemeData) {
        if self.mapped_pages > 0 {
            if let Some(device) = scheme_data.adapter.render_device() {
                device.unmap_blob(self.blob.res_id(), self.mapped_pages);
            }
            self.mapped_pages = 0;
        }
    }
}
