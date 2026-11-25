// SPDX-License-Identifier: GPL-2.0-only
use common::dma::Dma;
use driver_graphics::kms::framebuffer::{FramebufferBacking, KmsFramebuffer};
use driver_graphics::kms::objects::{KmsObjectId, KmsObjects};
use driver_graphics::kms::rc_object::KmsRcObjectRef;
use driver_graphics::{DrmFourcc, FlipCompletion};
use syscall::{EINVAL, EIO, EWOULDBLOCK};
use virtio_core::transport::Error;

use crate::scheme::{ActiveBacking, VirtGpuAdapter};
use crate::{
    CommandTy, GpuRect, ResourceFlush, ResourceId, SetScanout, SetScanoutBlob, XferToHost2d,
};

pub(crate) const VIRTIO_GPU_FORMAT_B8G8R8A8_UNORM: u32 = 1;
pub(crate) const VIRTIO_GPU_FORMAT_B8G8R8X8_UNORM: u32 = 2;
pub(crate) const VIRTIO_GPU_FORMAT_R8G8B8A8_UNORM: u32 = 67;
pub(crate) const VIRTIO_GPU_FORMAT_R8G8B8X8_UNORM: u32 = 134;

pub(crate) fn virtio_gpu_format(fourcc: DrmFourcc) -> Option<u32> {
    match fourcc {
        DrmFourcc::Argb8888 => Some(VIRTIO_GPU_FORMAT_B8G8R8A8_UNORM),
        DrmFourcc::Xrgb8888 => Some(VIRTIO_GPU_FORMAT_B8G8R8X8_UNORM),
        DrmFourcc::Abgr8888 => Some(VIRTIO_GPU_FORMAT_R8G8B8A8_UNORM),
        DrmFourcc::Xbgr8888 => Some(VIRTIO_GPU_FORMAT_R8G8B8X8_UNORM),
        _ => None,
    }
}

pub(crate) enum ScanoutBacking {
    Dumb2d {
        resource_id: ResourceId,
        width: u32,
        height: u32,
    },
    Blob {
        resource_id: ResourceId,
        width: u32,
        height: u32,
        format: u32,
        stride: u32,
        offset: u32,
    },
}

impl ScanoutBacking {
    fn resource_id(&self) -> ResourceId {
        match *self {
            Self::Dumb2d { resource_id, .. } | Self::Blob { resource_id, .. } => resource_id,
        }
    }
}

fn scanout_backing(fb: &KmsFramebuffer<VirtGpuAdapter<'_>>) -> syscall::Result<ScanoutBacking> {
    match &fb.backing {
        FramebufferBacking::Dumb(buf) => Ok(ScanoutBacking::Dumb2d {
            resource_id: buf.id,
            width: fb.width,
            height: fb.height,
        }),
        FramebufferBacking::Blob { blob, offset, .. } => {
            let format = virtio_gpu_format(fb.pixel_format).ok_or(syscall::Error::new(EINVAL))?;
            Ok(ScanoutBacking::Blob {
                resource_id: ResourceId::from_raw(blob.res_id()),
                width: fb.width,
                height: fb.height,
                format,
                stride: fb.pitch,
                offset: *offset,
            })
        }
    }
}

pub(crate) fn queue_page_flip(
    adapter: &mut VirtGpuAdapter<'_>,
    objects: &KmsObjects<VirtGpuAdapter<'_>>,
    crtc_id: KmsObjectId,
    fb: &KmsRcObjectRef<KmsFramebuffer<VirtGpuAdapter<'_>>>,
    completion: FlipCompletion,
) -> syscall::Result<()> {
    if adapter.pending_frames.len() >= 16 {
        return Err(syscall::Error::new(EWOULDBLOCK));
    }

    let mut scanout_ids = Vec::new();
    for connector in objects.connectors() {
        let connector = connector.lock().unwrap();
        if connector.state.crtc_id == crtc_id {
            scanout_ids.push(connector.driver_data.scanout_id);
        }
    }

    let backing = scanout_backing(fb)?;
    let resource_id = backing.resource_id();
    let new_active = match &fb.backing {
        FramebufferBacking::Dumb(buf) => ActiveBacking::Dumb(buf.clone()),
        FramebufferBacking::Blob { blob, .. } => {
            if let Some(render) = adapter.render.as_mut() {
                render.flush_submits();
            }
            ActiveBacking::Blob(blob.clone())
        }
    };

    let mut replaced_backings = Vec::with_capacity(scanout_ids.len());
    for &scanout_id in &scanout_ids {
        if let Some(display) = adapter.displays.get_mut(scanout_id as usize) {
            display.active_resource = Some(resource_id);
            if let Some(prev) = display.active_backing.replace(new_active.clone()) {
                replaced_backings.push(prev);
            }
        }
    }

    let queue = adapter.control_queue.clone();
    let keep_active = new_active;
    adapter.pending_frames.push_back(Box::pin(async move {
        let result = async {
            let _keep_new = &keep_active;
            let _keep_replaced = &replaced_backings;
            match backing {
                ScanoutBacking::Dumb2d {
                    resource_id,
                    width,
                    height,
                } => {
                    let rect = GpuRect::new(0, 0, width, height);
                    let transfer = Dma::new(XferToHost2d::new(resource_id, rect, 0))?;
                    let header = VirtGpuAdapter::send_control(&queue, transfer).await?;
                    if header.ty != CommandTy::RespOkNodata {
                        return Err(crate::scheme::refused(header.ty));
                    }
                    for &scanout_id in &scanout_ids {
                        let scanout = Dma::new(SetScanout::new(scanout_id, resource_id, rect))?;
                        let header = VirtGpuAdapter::send_control(&queue, scanout).await?;
                        if header.ty != CommandTy::RespOkNodata {
                            return Err(crate::scheme::refused(header.ty));
                        }
                    }
                    if !scanout_ids.is_empty() {
                        let flush = Dma::new(ResourceFlush::new(resource_id, rect))?;
                        let header = VirtGpuAdapter::send_control(&queue, flush).await?;
                        if header.ty != CommandTy::RespOkNodata {
                            return Err(crate::scheme::refused(header.ty));
                        }
                    }
                }
                ScanoutBacking::Blob {
                    resource_id,
                    width,
                    height,
                    format,
                    stride,
                    offset,
                } => {
                    let rect = GpuRect::new(0, 0, width, height);
                    for &scanout_id in &scanout_ids {
                        let scanout = Dma::new(SetScanoutBlob::new(
                            scanout_id,
                            resource_id,
                            rect,
                            width,
                            height,
                            format,
                            stride,
                            offset,
                        ))?;
                        let header = VirtGpuAdapter::send_control(&queue, scanout).await?;
                        if header.ty != CommandTy::RespOkNodata {
                            return Err(crate::scheme::refused(header.ty));
                        }
                    }
                    if !scanout_ids.is_empty() {
                        let flush = Dma::new(ResourceFlush::new(resource_id, rect))?;
                        let header = VirtGpuAdapter::send_control(&queue, flush).await?;
                        if header.ty != CommandTy::RespOkNodata {
                            return Err(crate::scheme::refused(header.ty));
                        }
                    }
                }
            }
            Ok(())
        }
        .await
        .map_err(|error| {
            syscall::Error::new(match error {
                Error::SyscallError(error) => error.errno(),
                _ => EIO,
            })
        });
        completion.complete(result);
    }));

    Ok(())
}
