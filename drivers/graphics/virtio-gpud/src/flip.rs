use common::dma::Dma;
use driver_graphics::kms::framebuffer::KmsFramebuffer;
use driver_graphics::kms::objects::{KmsObjectId, KmsObjects};
use driver_graphics::kms::rc_object::KmsRcObjectRef;
use driver_graphics::FlipCompletion;
use syscall::{EIO, EWOULDBLOCK};
use virtio_core::transport::Error;

use crate::scheme::VirtGpuAdapter;
use crate::{CommandTy, GpuRect, ResourceFlush, ResourceId, SetScanout, XferToHost2d};

pub(crate) enum ScanoutBacking {
    Dumb2d {
        resource_id: ResourceId,
        width: u32,
        height: u32,
    },
}

impl ScanoutBacking {
    fn resource_id(&self) -> ResourceId {
        match *self {
            Self::Dumb2d { resource_id, .. } => resource_id,
        }
    }
}

fn scanout_backing(fb: &KmsFramebuffer<VirtGpuAdapter<'_>>) -> ScanoutBacking {
    ScanoutBacking::Dumb2d {
        resource_id: fb.buffer.id,
        width: fb.width,
        height: fb.height,
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

    let backing = scanout_backing(fb);
    let resource_id = backing.resource_id();
    for &scanout_id in &scanout_ids {
        if let Some(display) = adapter.displays.get_mut(scanout_id as usize) {
            display.active_resource = Some(resource_id);
        }
    }

    let queue = adapter.control_queue.clone();
    let _buffer = fb.buffer.clone();
    adapter.pending_frames.push_back(Box::pin(async move {
        let result = async {
            let _keep_alive = &_buffer;
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
