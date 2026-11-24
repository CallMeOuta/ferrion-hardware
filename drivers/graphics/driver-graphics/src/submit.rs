// SPDX-License-Identifier: GPL-2.0-only
use std::sync::Arc;

use redox_ioctl::virtgpu;
use scheme_utils::ResourceOpenResult;
use syscall::schemev2::NewFdFlags;
use syscall::{EBUSY, EINVAL, ENOENT, ENOMEM, ENOSYS, Error, Result};

use crate::render::{RenderDevice, RenderHandle};
use crate::syncobj::{SyncFence, SyncFile, Syncobj};
use crate::{GraphicsAdapter, GraphicsResource};

pub const MAX_PENDING_FENCES_PER_HANDLE: usize = 64;

pub(crate) enum PendingWait {
    Wait(SyncFence),
    Execbuffer {
        fences: Vec<SyncFence>,
        in_fence: Option<SyncFence>,
    },
}

impl PendingWait {
    pub(crate) fn is_ready(&self) -> bool {
        match self {
            PendingWait::Wait(fence) => fence.is_signaled(),
            PendingWait::Execbuffer { fences, .. } => fences.iter().all(SyncFence::is_signaled),
        }
    }
}

struct InSyncobj {
    syncobj: Arc<Syncobj>,
    reset: bool,
}

struct OutSyncobj {
    syncobj: Arc<Syncobj>,
    point: u64,
}

pub(crate) fn execbuffer<T>(
    device: &mut dyn RenderDevice,
    handle: &mut RenderHandle<T>,
    mut data: virtgpu::VirtgpuExecbuffer<'_>,
    in_fence: Option<SyncFence>,
    pending_wait: &mut Option<PendingWait>,
) -> Result<usize> {
    if !device.features().three_d {
        return Err(Error::new(ENOSYS));
    }
    let ctx_id = handle.context.ok_or(Error::new(EINVAL))?;
    let flags = data.flags();
    if flags & !virtgpu::VIRTGPU_EXECBUF_FLAGS != 0 {
        return Err(Error::new(EINVAL));
    }
    if ((flags & virtgpu::VIRTGPU_EXECBUF_FENCE_FD_IN) != 0) != in_fence.is_some() {
        return Err(Error::new(EINVAL));
    }
    let ring_idx = if (flags & virtgpu::VIRTGPU_EXECBUF_RING_IDX) != 0 {
        if handle.num_rings == 0 || data.ring_idx() >= handle.num_rings {
            return Err(Error::new(EINVAL));
        }
        Some(data.ring_idx() as u8)
    } else {
        None
    };
    let mut waits = Vec::new();
    let mut in_objs = Vec::new();
    let mut out_objs = Vec::new();
    if data.num_in_syncobjs() != 0 || data.num_out_syncobjs() != 0 {
        if data.syncobj_stride() != size_of::<virtgpu::drm_virtgpu_execbuffer_syncobj>() as u32
            || data.num_in_syncobjs() > virtgpu::EXECBUF_MAX_SYNCOBJS
            || data.num_out_syncobjs() > virtgpu::EXECBUF_MAX_SYNCOBJS
        {
            return Err(Error::new(EINVAL));
        }
        for desc in data.in_syncobjs() {
            if desc.flags & !virtgpu::VIRTGPU_EXECBUF_SYNCOBJ_FLAGS != 0 {
                return Err(Error::new(EINVAL));
            }
            let syncobj = handle.syncobjs.get(desc.handle)?;
            waits.push(syncobj.find_fence(desc.point)?);
            in_objs.push(InSyncobj {
                syncobj,
                reset: desc.flags & virtgpu::VIRTGPU_EXECBUF_SYNCOBJ_RESET != 0,
            });
        }
        for desc in data.out_syncobjs() {
            if desc.flags != 0 {
                return Err(Error::new(EINVAL));
            }
            let syncobj = handle.syncobjs.get(desc.handle)?;
            if !syncobj.can_attach(desc.point) {
                return Err(Error::new(ENOMEM));
            }
            out_objs.push(OutSyncobj {
                syncobj,
                point: desc.point,
            });
        }
    }
    if data.size() > virtgpu::EXECBUF_MAX_CMD_SIZE
        || data.num_bo_handles() > virtgpu::EXECBUF_MAX_BO_HANDLES
    {
        return Err(Error::new(EINVAL));
    }
    for &bo_handle in data.bo_handles() {
        if bo_handle == 0 || !handle.known_resources.contains(&bo_handle) {
            return Err(Error::new(ENOENT));
        }
    }
    let want_fence_fd = (flags & virtgpu::VIRTGPU_EXECBUF_FENCE_FD_OUT) != 0;
    if want_fence_fd && handle.pending_fences.len() >= MAX_PENDING_FENCES_PER_HANDLE {
        return Err(Error::new(ENOMEM));
    }
    waits.extend(in_fence.iter().cloned());
    if !waits.iter().all(SyncFence::is_signaled) {
        *pending_wait = Some(PendingWait::Execbuffer {
            fences: waits,
            in_fence,
        });
        return Ok(0);
    }

    let fence = SyncFence::from_leaf(device.submit_3d(ctx_id, ring_idx, data.command())?);
    for in_obj in in_objs.iter().filter(|in_obj| in_obj.reset) {
        in_obj.syncobj.reset();
    }
    for out_obj in &out_objs {
        out_obj.syncobj.attach_fence(out_obj.point, fence.clone())?;
    }
    for &bo_handle in data.bo_handles() {
        handle.resource_fences.insert(bo_handle, fence.clone());
    }
    handle.last_fence = Some(fence.clone());
    if want_fence_fd {
        let fence_handle = handle.alloc_pending_fence(fence);
        data.set_fence_fd(fence_handle);
    }
    Ok(0)
}

pub(crate) fn wait<T>(
    handle: &RenderHandle<T>,
    data: virtgpu::VirtgpuWait<'_>,
    pending_wait: &mut Option<PendingWait>,
) -> Result<usize> {
    if data.flags() & !virtgpu::VIRTGPU_WAIT_NOWAIT != 0 {
        return Err(Error::new(EINVAL));
    }
    let res_handle = data.handle();
    if res_handle == 0 || !handle.known_resources.contains(&res_handle) {
        return Err(Error::new(ENOENT));
    }
    let Some(fence) = handle.resource_fences.get(&res_handle) else {
        return Ok(0);
    };
    if fence.is_signaled() {
        return Ok(0);
    }
    if (data.flags() & virtgpu::VIRTGPU_WAIT_NOWAIT) != 0 {
        return Err(Error::new(EBUSY));
    }
    *pending_wait = Some(PendingWait::Wait(fence.clone()));
    Ok(0)
}

pub(crate) fn dup_fence<T: GraphicsAdapter>(
    handle: &mut RenderHandle<T>,
    buf: &[u8],
) -> Result<ResourceOpenResult<GraphicsResource<T>>> {
    if buf.len() != 16 {
        return Err(Error::new(EINVAL));
    }
    let cmd = u64::from_ne_bytes(buf[0..8].try_into().unwrap());
    let fence_handle = u32::from_ne_bytes(buf[8..12].try_into().unwrap());
    let flags = u32::from_ne_bytes(buf[12..16].try_into().unwrap());
    if cmd != virtgpu::FENCE_DUP_EXECBUFFER || flags != 0 {
        return Err(Error::new(EINVAL));
    }
    let fence = i32::try_from(fence_handle)
        .ok()
        .and_then(|id| handle.pending_fences.remove(&id))
        .ok_or(Error::new(ENOENT))?;
    Ok(ResourceOpenResult::ThisScheme {
        data: GraphicsResource::SyncFile(SyncFile::new(b"virtio_gpu", fence)),
        flags: NewFdFlags::empty(),
    })
}
