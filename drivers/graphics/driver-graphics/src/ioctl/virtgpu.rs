// SPDX-License-Identifier: GPL-2.0-only
//! The requests a render handle serves (FerrionOS): those of Linux's virtio-gpu driver that
//! Mesa's Venus driver makes, with the checks and errors of Linux v7.3's
//! `drivers/gpu/drm/virtio/virtgpu_ioctl.c`. Every count, number and name a client sends is
//! checked here before the device sees it; the wire formats are `redox_ioctl::virtgpu`'s.

use std::ffi::c_char;
use std::task::Waker;
use std::{cmp, slice};

use drm_sys::{
    DRM_CAP_DUMB_BUFFER, DRM_CAP_PRIME, DRM_CAP_SYNCOBJ, DRM_CAP_SYNCOBJ_TIMELINE,
    DRM_CAP_TIMESTAMP_MONOTONIC, DRM_PRIME_CAP_EXPORT, DRM_PRIME_CAP_IMPORT,
};
use redox_ioctl::{drm, sync, virtgpu};
use redox_scheme::Id;
use syscall::{EACCES, EEXIST, EINVAL, ENOENT, ENOSYS, Error, Result};

use crate::render::{RenderDevice, RenderHandle};
use crate::syncobj::WaitTimer;

/// Linux's `MAX_CAPSET_ID` and `MAX_RINGS` (`virtgpu_drv.h`).
const MAX_CAPSET_ID: u64 = 63;
const MAX_RINGS: u64 = 64;

pub(crate) fn call_render_ioctl<T>(
    device: &mut dyn RenderDevice,
    handle: &mut RenderHandle<T>,
    waker: &Waker,
    sync_timer: &WaitTimer,
    req_id: Id,
    cmd: u64,
    payload: &mut [u8],
    in_fence: Option<crate::syncobj::SyncFence>,
    pending_wait: &mut Option<crate::submit::PendingWait>,
) -> Result<usize> {
    match cmd {
        drm::VERSION => drm::DrmVersion::with(payload, |data| version(device, data)),
        drm::GET_CAP => drm::DrmGetCap::with(payload, |data| render_get_cap(data)),
        virtgpu::GEM_CLOSE => virtgpu::GemClose::with(payload, |data| {
            let bo_handle = data.handle();
            handle.blobs.gem_close(data)?;
            handle.known_resources.remove(&bo_handle);
            handle.resource_fences.remove(&bo_handle);
            Ok(0)
        }),
        virtgpu::MAP => virtgpu::VirtgpuMap::with(payload, |data| handle.blobs.map_ioctl(data)),
        virtgpu::EXECBUFFER => virtgpu::VirtgpuExecbuffer::with(payload, |data| {
            crate::submit::execbuffer(device, handle, data, in_fence, pending_wait)
        }),
        virtgpu::GETPARAM => virtgpu::VirtgpuGetparam::with(payload, |mut data| {
            data.set_value(getparam(device, data.param())?);
            Ok(0)
        }),
        virtgpu::RESOURCE_INFO => {
            virtgpu::VirtgpuResourceInfo::with(payload, |data| handle.blobs.resource_info(data))
        }
        virtgpu::WAIT => {
            virtgpu::VirtgpuWait::with(payload, |data| crate::submit::wait(handle, data, pending_wait))
        }
        virtgpu::GET_CAPS => virtgpu::VirtgpuGetCaps::with(payload, |data| get_caps(device, data)),
        virtgpu::RESOURCE_CREATE_BLOB => virtgpu::VirtgpuResourceCreateBlob::with(payload, |data| {
            let bo_handle = handle.blobs.create_blob(device, handle.context, data)?;
            handle.known_resources.insert(bo_handle);
            Ok(0)
        }),
        virtgpu::CONTEXT_INIT => {
            virtgpu::VirtgpuContextInit::with(payload, |data| context_init(device, handle, data))
        }
        sync::SYNCOBJ_CREATE..=sync::SYNCOBJ_SIGNAL
        | sync::SYNCOBJ_TIMELINE_WAIT..=sync::SYNCOBJ_TIMELINE_SIGNAL
        | sync::SYNCOBJ_EVENTFD => {
            crate::syncobj::call_syncobj_ioctl(handle, waker, sync_timer, req_id, cmd, payload)
        }
        // DRM's own requests and mode-setting: a render handle shows nothing on a display, and
        // Linux answers these on a render node with EACCES.
        0x00..=0x3F | 0xA0..=0xFF => Err(Error::new(EACCES)),
        _ => Err(Error::new(EINVAL)),
    }
}

fn render_get_cap(mut data: drm::DrmGetCap<'_>) -> Result<usize> {
    let cap: u32 = data
        .capability()
        .try_into()
        .map_err(|_| Error::new(EINVAL))?;
    let value = match cap {
        DRM_CAP_DUMB_BUFFER => 0,
        DRM_CAP_PRIME => u64::from(DRM_PRIME_CAP_IMPORT | DRM_PRIME_CAP_EXPORT),
        DRM_CAP_TIMESTAMP_MONOTONIC => 1,
        DRM_CAP_SYNCOBJ => 1,
        DRM_CAP_SYNCOBJ_TIMELINE => 1,
        _ => return Err(Error::new(EINVAL)),
    };
    data.set_value(value);
    Ok(0)
}

fn c_chars(bytes: &'static [u8]) -> &'static [c_char] {
    // c_char is one byte on every target, signed or not.
    unsafe { slice::from_raw_parts(bytes.as_ptr().cast::<c_char>(), bytes.len()) }
}

fn version(device: &dyn RenderDevice, mut data: drm::DrmVersion<'_>) -> Result<usize> {
    let (name, desc, [major, minor, patchlevel]) = device.driver_info();
    data.set_version_major(major);
    data.set_version_minor(minor);
    data.set_version_patchlevel(patchlevel);
    data.set_name(c_chars(name));
    data.set_date(c_chars(b"0"));
    data.set_desc(c_chars(desc));
    Ok(0)
}

fn getparam(device: &dyn RenderDevice, param: u64) -> Result<i32> {
    let features = device.features();
    Ok(match param {
        virtgpu::VIRTGPU_PARAM_3D_FEATURES => features.three_d.into(),
        virtgpu::VIRTGPU_PARAM_CAPSET_QUERY_FIX => 1,
        virtgpu::VIRTGPU_PARAM_RESOURCE_BLOB => features.resource_blob.into(),
        virtgpu::VIRTGPU_PARAM_HOST_VISIBLE => features.host_visible.into(),
        // Resource identifiers (the UUID feature) are not acknowledged.
        virtgpu::VIRTGPU_PARAM_CROSS_DEVICE => 0,
        virtgpu::VIRTGPU_PARAM_CONTEXT_INIT => features.context_init.into(),
        // Linux answers its 64-bit mask in an int, so capsets above 31 do not show.
        virtgpu::VIRTGPU_PARAM_SUPPORTED_CAPSET_IDs => device.capset_ids() as i32,
        // A context can be named whenever it can be created.
        virtgpu::VIRTGPU_PARAM_EXPLICIT_DEBUG_NAME => features.context_init.into(),
        // The device's blob alignment feature is not acknowledged, and Linux answers ENOENT then.
        virtgpu::VIRTGPU_PARAM_BLOB_ALIGNMENT => return Err(Error::new(ENOENT)),
        _ => return Err(Error::new(EINVAL)),
    })
}

fn get_caps(device: &dyn RenderDevice, mut data: virtgpu::VirtgpuGetCaps<'_>) -> Result<usize> {
    // Linux's order: no capsets at all, a size of 0, then a capset or version the device lacks.
    if device.capset_ids() == 0 {
        return Err(Error::new(ENOSYS));
    }
    let size = data.size();
    if size == 0 {
        return Err(Error::new(EINVAL));
    }
    let capset = device
        .capset(data.cap_set_id(), data.cap_set_ver())
        .ok_or(Error::new(EINVAL))?;
    // The smaller of the two sizes; addr_mut() leaves the caller's size as it asked, as on Linux.
    let len = cmp::min(size as usize, capset.len());
    data.addr_mut()[..len].copy_from_slice(&capset[..len]);
    Ok(0)
}

fn context_init<T>(
    device: &mut dyn RenderDevice,
    handle: &mut RenderHandle<T>,
    data: virtgpu::VirtgpuContextInit<'_>,
) -> Result<usize> {
    let invalid = || Error::new(EINVAL);
    let features = device.features();
    if !(features.three_d && features.context_init)
        || data.num_params() > virtgpu::CONTEXT_INIT_MAX_PARAMS
    {
        return Err(invalid());
    }
    if handle.context.is_some() {
        return Err(Error::new(EEXIST));
    }
    // Every parameter is checked before anything changes, and each may come once.
    let (mut capset_id, mut num_rings, mut ring_mask, mut named) = (None, None, None, false);
    for param in data.params() {
        match param.param {
            virtgpu::VIRTGPU_CONTEXT_PARAM_CAPSET_ID => {
                if capset_id.is_some()
                    || param.value > MAX_CAPSET_ID
                    || device.capset_ids() & (1 << param.value) == 0
                {
                    return Err(invalid());
                }
                capset_id = Some(param.value as u32);
            }
            virtgpu::VIRTGPU_CONTEXT_PARAM_NUM_RINGS => {
                if num_rings.is_some() || param.value > MAX_RINGS {
                    return Err(invalid());
                }
                num_rings = Some(param.value);
            }
            // No reader of ring events exists yet, so the handle sends none: only an empty mask.
            virtgpu::VIRTGPU_CONTEXT_PARAM_POLL_RINGS_MASK => {
                if ring_mask.is_some() || param.value != 0 {
                    return Err(invalid());
                }
                ring_mask = Some(param.value);
            }
            // Its value is the caller's address; relibc sent the name itself after the
            // parameters.
            virtgpu::VIRTGPU_CONTEXT_PARAM_DEBUG_NAME => {
                if named {
                    return Err(invalid());
                }
                named = true;
            }
            _ => return Err(invalid()),
        }
    }
    // The label reaches the host's logs and traces: up to its first zero, printable ASCII only.
    let name: Vec<u8> = data
        .name()
        .iter()
        .take_while(|&&byte| byte != 0)
        .map(|&byte| {
            if byte.is_ascii_graphic() || byte == b' ' {
                byte
            } else {
                b'?'
            }
        })
        .collect();
    handle.context = Some(device.create_context(capset_id.unwrap_or(0), &name)?);
    handle.num_rings = num_rings.unwrap_or(0) as u32;
    Ok(0)
}
