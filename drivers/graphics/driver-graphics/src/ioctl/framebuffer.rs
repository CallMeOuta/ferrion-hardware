// SPDX-License-Identifier: GPL-2.0-only AND MIT
use std::collections::HashMap;

use drm_fourcc::DrmFourcc;
use syscall::{EINVAL, ENOENT, Error};

use crate::handle::{DumbEntry, ImportedBlob};
use crate::kms::framebuffer::{FramebufferBacking, KmsFramebuffer, disable_planes_with_fb};
use crate::kms::objects::{KmsObjectId, KmsObjects};
use crate::{Damage, DrmHandle, GraphicsAdapter, VtState};

const DRM_MODE_FB_MODIFIERS: u32 = 1 << 1;
const DRM_FORMAT_MOD_LINEAR: u64 = 0;

fn is_supported_blob_format(format: DrmFourcc) -> bool {
    matches!(
        format,
        DrmFourcc::Xrgb8888 | DrmFourcc::Argb8888 | DrmFourcc::Xbgr8888 | DrmFourcc::Abgr8888
    )
}

pub(super) fn mode_get_fb<T: GraphicsAdapter>(
    objects: &mut KmsObjects<T>,
    handle: &mut DrmHandle<T>,
    mut data: redox_ioctl::drm::DrmModeFbCmd<'_>,
) -> Result<usize, Error> {
    let fb = objects.get_framebuffer(KmsObjectId(data.fb_id()))?;
    let buffer = fb.dumb_buffer().ok_or(Error::new(EINVAL))?;

    let (bpp, depth) = match fb.pixel_format {
        DrmFourcc::Xrgb8888 => (32, 24),
        DrmFourcc::Argb8888 => (32, 32),
        _ => return Err(Error::new(EINVAL)),
    };

    let id = handle.alloc_buffer_id()?;
    handle.buffers.insert(
        id,
        DumbEntry {
            buffer: buffer.clone(),
            writable: true,
        },
    );

    data.set_width(fb.width);
    data.set_height(fb.height);
    data.set_pitch(fb.pitch);
    data.set_bpp(bpp);
    data.set_depth(depth);
    data.set_handle(id);
    Ok(0)
}

pub(super) fn mode_add_fb<T: GraphicsAdapter>(
    adapter: &mut T,
    objects: &mut KmsObjects<T>,
    handle: &mut DrmHandle<T>,
    mut data: redox_ioctl::drm::DrmModeFbCmd<'_>,
) -> Result<usize, Error> {
    let buffer = &handle
        .buffers
        .get(&data.handle())
        .ok_or(Error::new(EINVAL))?
        .buffer;

    if data.bpp() != 32 {
        return Err(Error::new(EINVAL));
    }
    let pixel_format = match data.depth() {
        24 => DrmFourcc::Xrgb8888,
        32 => DrmFourcc::Argb8888,
        _ => return Err(Error::new(EINVAL)),
    };

    let driver_data = adapter.create_framebuffer(buffer);
    let fb = objects.add_framebuffer(KmsFramebuffer {
        width: data.width(),
        height: data.height(),
        pixel_format,
        pitch: data.pitch(),
        backing: FramebufferBacking::Dumb(buffer.clone()),
        driver_data,
    });
    let fb_id = fb.id();
    handle.fbs.insert(fb_id, fb);

    data.set_fb_id(fb_id.0);

    Ok(0)
}

pub(super) fn mode_rm_fb<T: GraphicsAdapter>(
    adapter: &mut T,
    objects: &mut KmsObjects<T>,
    active_vt: usize,
    vts: &mut HashMap<usize, VtState<T>>,
    handle: &mut DrmHandle<T>,
    data: redox_ioctl::drm::StandinForUint<'_>,
) -> Result<usize, Error> {
    let fb_id = KmsObjectId(data.inner());

    if handle.fbs.remove(&fb_id).is_none() {
        return Err(Error::new(ENOENT));
    }

    disable_planes_with_fb(adapter, objects, active_vt, vts, fb_id);

    Ok(0)
}

pub(super) fn mode_dirtyfb<T: GraphicsAdapter>(
    adapter: &mut T,
    objects: &mut KmsObjects<T>,
    active_vt: usize,
    handle: &mut DrmHandle<T>,
    data: redox_ioctl::drm::DrmModeFbDirtyCmd<'_>,
    pending: &mut Option<crate::PendingFrame>,
) -> Result<usize, Error> {
    let fb_id = KmsObjectId(data.fb_id());
    let fb = objects.get_framebuffer(fb_id)?;

    let damage = data
        .clips_ptr()
        .iter()
        .map(|rect| Damage {
            x: u32::from(rect.x1),
            y: u32::from(rect.y1),
            width: u32::from(rect.x2 - rect.x1),
            height: u32::from(rect.y2 - rect.y1),
        })
        .reduce(Damage::merge)
        .unwrap_or(Damage {
            x: 0,
            y: 0,
            width: fb.width,
            height: fb.height,
        })
        .clip(fb.width, fb.height);

    if handle.vt == active_vt {
        *pending = adapter.dirty_framebuffer(objects, &fb, damage)?;
    }

    Ok(0)
}

pub(super) fn mode_add_fb2<T: GraphicsAdapter>(
    adapter: &mut T,
    objects: &mut KmsObjects<T>,
    handle: &mut DrmHandle<T>,
    mut data: redox_ioctl::drm::DrmModeFbCmd2<'_>,
) -> Result<usize, Error> {
    let flags = data.flags();
    if flags & !DRM_MODE_FB_MODIFIERS != 0 {
        return Err(Error::new(EINVAL));
    }
    let width = data.width();
    let height = data.height();
    let (min_w, max_w, min_h, max_h) = adapter.min_max_fb_size();
    if width == 0
        || height == 0
        || width < min_w
        || width > max_w
        || height < min_h
        || height > max_h
    {
        return Err(Error::new(EINVAL));
    }

    let handles = data.handles();
    let pitches = data.pitches();
    let offsets = data.offsets();
    let modifiers = data.modifier();
    if handles[1..].iter().any(|&h| h != 0)
        || pitches[1..].iter().any(|&p| p != 0)
        || offsets[1..].iter().any(|&o| o != 0)
    {
        return Err(Error::new(EINVAL));
    }
    let has_modifiers = (flags & DRM_MODE_FB_MODIFIERS) != 0;
    if has_modifiers && modifiers[0] != DRM_FORMAT_MOD_LINEAR {
        return Err(Error::new(EINVAL));
    }

    let pixel_format = DrmFourcc::try_from(data.pixel_format()).map_err(|_| Error::new(EINVAL))?;
    let pitch = pitches[0];
    let offset = offsets[0];

    let (backing, driver_data) = if let Some(entry) = handle.buffers.get(&handles[0]) {
        if offset != 0 {
            return Err(Error::new(EINVAL));
        }
        let driver_data = adapter.create_framebuffer(&entry.buffer);
        (FramebufferBacking::Dumb(entry.buffer.clone()), driver_data)
    } else if let Some(entry) = handle.imported_blobs.get(&handles[0]) {
        if !adapter.supports_blob_scanout() || !is_supported_blob_format(pixel_format) {
            return Err(Error::new(EINVAL));
        }
        let min_row = u64::from(width).checked_mul(4).ok_or(Error::new(EINVAL))?;
        if u64::from(pitch) < min_row || pitch % 4 != 0 {
            return Err(Error::new(EINVAL));
        }
        let image_bytes = u64::from(pitch)
            .checked_mul(u64::from(height))
            .ok_or(Error::new(EINVAL))?;
        let end = u64::from(offset)
            .checked_add(image_bytes)
            .ok_or(Error::new(EINVAL))?;
        if end > entry.blob.info().size {
            return Err(Error::new(EINVAL));
        }
        let driver_data = adapter.create_blob_framebuffer(&entry.blob)?;
        (
            FramebufferBacking::Blob {
                blob: entry.blob.clone(),
                offset,
                modifier: DRM_FORMAT_MOD_LINEAR,
            },
            driver_data,
        )
    } else {
        return Err(Error::new(EINVAL));
    };

    let fb = objects.add_framebuffer(KmsFramebuffer {
        width,
        height,
        pixel_format,
        pitch,
        backing,
        driver_data,
    });
    let fb_id = fb.id();
    handle.fbs.insert(fb_id, fb);

    data.set_fb_id(fb_id.0);

    Ok(0)
}

pub(super) fn mode_get_fb2<T: GraphicsAdapter>(
    objects: &mut KmsObjects<T>,
    handle: &mut DrmHandle<T>,
    mut data: redox_ioctl::drm::DrmModeFbCmd2<'_>,
) -> Result<usize, Error> {
    let fb = objects.get_framebuffer(KmsObjectId(data.fb_id()))?;
    let (id, offset, modifier) = match &fb.backing {
        FramebufferBacking::Dumb(buffer) => {
            let id = handle.alloc_buffer_id()?;
            handle.buffers.insert(
                id,
                DumbEntry {
                    buffer: buffer.clone(),
                    writable: true,
                },
            );
            (id, 0u32, 0u64)
        }
        FramebufferBacking::Blob {
            blob,
            offset,
            modifier,
        } => {
            let id = handle.alloc_buffer_id()?;
            handle.imported_blobs.insert(
                id,
                ImportedBlob {
                    blob: blob.clone(),
                    writable: false,
                },
            );
            (id, *offset, *modifier)
        }
    };

    data.set_width(fb.width);
    data.set_height(fb.height);
    data.set_pixel_format(fb.pixel_format as u32);
    data.set_handles([id, 0, 0, 0]);
    data.set_pitches([fb.pitch, 0, 0, 0]);
    data.set_offsets([offset, 0, 0, 0]);
    data.set_modifier([modifier, 0, 0, 0]);
    Ok(0)
}

pub(super) fn mode_close_fb<T: GraphicsAdapter>(
    handle: &mut DrmHandle<T>,
    data: redox_ioctl::drm::DrmModeClosefb<'_>,
) -> Result<usize, Error> {
    let fb_id = KmsObjectId(data.fb_id());

    if handle.fbs.remove(&fb_id).is_none() {
        return Err(Error::new(ENOENT));
    }

    Ok(0)
}
