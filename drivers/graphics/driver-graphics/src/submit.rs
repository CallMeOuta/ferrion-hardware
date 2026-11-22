use std::fmt;
use std::marker::PhantomData;
use std::slice;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicUsize, Ordering};

use redox_ioctl::virtgpu;
use scheme_utils::{ResourceOpenResult, ResourceSync};
use syscall::schemev2::NewFdFlags;
use syscall::{EBUSY, EINVAL, EMFILE, ENOENT, ENOMEM, ENOSYS, EOPNOTSUPP, Error, EventFlags, MODE_CHR, Result};

use crate::render::{RenderDevice, RenderHandle};
use crate::{GraphicsAdapter, GraphicsResource, GraphicsSchemeData};

pub const MAX_PENDING_FENCES_PER_HANDLE: usize = 64;
pub const MAX_LIVE_FENCE_DESCRIPTORS: usize = 1024;

#[derive(Debug)]
pub struct Fence {
    fence_id: AtomicU64,
    signaled: AtomicBool,
    status: AtomicI32,
}

impl Fence {
    pub fn new(fence_id: u64) -> Arc<Self> {
        Arc::new(Self {
            fence_id: AtomicU64::new(fence_id),
            signaled: AtomicBool::new(false),
            status: AtomicI32::new(0),
        })
    }

    pub fn new_unsignaled() -> Arc<Self> {
        Self::new(0)
    }

    pub fn signal(&self, status: i32) {
        self.status.store(status, Ordering::Release);
        self.signaled.store(true, Ordering::Release);
    }

    pub fn is_signaled(&self) -> bool {
        self.signaled.load(Ordering::Acquire)
    }

    pub fn state(&self) -> virtgpu::VirtgpuFenceState {
        let signaled = self.signaled.load(Ordering::Acquire);
        let status = if signaled {
            self.status.load(Ordering::Acquire)
        } else {
            0
        };
        virtgpu::VirtgpuFenceState {
            signaled: u32::from(signaled),
            status,
            fence_id: self.fence_id.load(Ordering::Relaxed),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PendingWait {
    fence: Arc<Fence>,
}

impl PendingWait {
    pub(crate) fn poll_result(&self) -> Option<Result<usize>> {
        if self.fence.is_signaled() {
            Some(Ok(0))
        } else {
            None
        }
    }
}

pub(crate) struct FenceResource<T> {
    fence: Arc<Fence>,
    live_count: Arc<AtomicUsize>,
    requested_events: EventFlags,
    notified_read: bool,
    _adapter: PhantomData<T>,
}

impl<T> fmt::Debug for FenceResource<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FenceResource")
            .field("fence", &self.fence)
            .finish()
    }
}

impl<T> FenceResource<T> {
    fn new(fence: Arc<Fence>, live_count: Arc<AtomicUsize>) -> Self {
        Self {
            fence,
            live_count,
            requested_events: EventFlags::empty(),
            notified_read: false,
            _adapter: PhantomData,
        }
    }

    pub(crate) fn should_post_event(&mut self) -> bool {
        if self.requested_events.contains(EventFlags::EVENT_READ)
            && self.fence.is_signaled()
            && !self.notified_read
        {
            self.notified_read = true;
            true
        } else {
            false
        }
    }
}

impl<T> Drop for FenceResource<T> {
    fn drop(&mut self) {
        self.live_count.fetch_sub(1, Ordering::AcqRel);
    }
}

impl<T: GraphicsAdapter> ResourceSync for FenceResource<T> {
    type SchemeData = GraphicsSchemeData<T>;
    type ResourceEnum = GraphicsResource<T>;

    fn read(
        &mut self,
        _scheme_data: &mut Self::SchemeData,
        buf: &mut [u8],
        _offset: u64,
        _fcntl_flags: u32,
    ) -> Result<usize> {
        let state = self.fence.state();
        let bytes = unsafe {
            slice::from_raw_parts(
                (&raw const state).cast::<u8>(),
                size_of::<virtgpu::VirtgpuFenceState>(),
            )
        };
        if buf.len() < bytes.len() {
            return Err(Error::new(EINVAL));
        }
        buf[..bytes.len()].copy_from_slice(bytes);
        Ok(bytes.len())
    }

    fn fevent(
        &mut self,
        _scheme_data: &mut Self::SchemeData,
        flags: EventFlags,
    ) -> Result<EventFlags> {
        self.requested_events = flags;
        if !flags.contains(EventFlags::EVENT_READ) {
            return Ok(EventFlags::empty());
        }
        if self.fence.is_signaled() {
            self.notified_read = true;
            Ok(EventFlags::EVENT_READ)
        } else {
            self.notified_read = false;
            Ok(EventFlags::empty())
        }
    }

    fn fstat(
        &mut self,
        _scheme_data: &mut Self::SchemeData,
        stat: &mut syscall::Stat,
    ) -> Result<()> {
        stat.st_mode = MODE_CHR;
        Ok(())
    }
}

pub(crate) fn execbuffer<T>(
    device: &mut dyn RenderDevice,
    handle: &mut RenderHandle<T>,
    mut data: virtgpu::VirtgpuExecbuffer<'_>,
) -> Result<usize> {
    if !device.features().three_d {
        return Err(Error::new(ENOSYS));
    }
    let ctx_id = handle.context.ok_or(Error::new(EINVAL))?;
    let flags = data.flags();
    if flags & !virtgpu::VIRTGPU_EXECBUF_FLAGS != 0 {
        return Err(Error::new(EINVAL));
    }
    if (flags & virtgpu::VIRTGPU_EXECBUF_FENCE_FD_IN) != 0 {
        return Err(Error::new(EOPNOTSUPP));
    }
    let ring_idx = if (flags & virtgpu::VIRTGPU_EXECBUF_RING_IDX) != 0 {
        if handle.num_rings == 0 || data.ring_idx() >= handle.num_rings {
            return Err(Error::new(EINVAL));
        }
        Some(data.ring_idx() as u8)
    } else {
        None
    };
    if data.num_in_syncobjs() != 0 || data.num_out_syncobjs() != 0 {
        if data.syncobj_stride() != size_of::<virtgpu::drm_virtgpu_execbuffer_syncobj>() as u32
            || data.num_in_syncobjs() > virtgpu::EXECBUF_MAX_SYNCOBJS
            || data.num_out_syncobjs() > virtgpu::EXECBUF_MAX_SYNCOBJS
        {
            return Err(Error::new(EINVAL));
        }
        return Err(Error::new(EOPNOTSUPP));
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

    let fence = device.submit_3d(ctx_id, ring_idx, data.command())?;
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
    *pending_wait = Some(PendingWait {
        fence: fence.clone(),
    });
    Ok(0)
}

pub(crate) fn dup_fence<T: GraphicsAdapter>(
    handle: &mut RenderHandle<T>,
    live_fences: &Arc<AtomicUsize>,
    buf: &[u8],
) -> Result<ResourceOpenResult<GraphicsResource<T>>> {
    if buf.len() != 16 {
        return Err(Error::new(EINVAL));
    }
    let cmd = u64::from_ne_bytes(buf[0..8].try_into().unwrap());
    let fence_handle = u32::from_ne_bytes(buf[8..12].try_into().unwrap());
    let flags = u32::from_ne_bytes(buf[12..16].try_into().unwrap());
    if flags != 0 {
        return Err(Error::new(EINVAL));
    }
    let prev = live_fences.fetch_add(1, Ordering::AcqRel);
    if prev >= MAX_LIVE_FENCE_DESCRIPTORS {
        live_fences.fetch_sub(1, Ordering::AcqRel);
        return Err(Error::new(EMFILE));
    }
    let fence = match cmd {
        virtgpu::FENCE_DUP_EXECBUFFER => {
            match i32::try_from(fence_handle)
                .ok()
                .and_then(|id| handle.pending_fences.remove(&id))
            {
                Some(fence) => fence,
                None => {
                    live_fences.fetch_sub(1, Ordering::AcqRel);
                    return Err(Error::new(ENOENT));
                }
            }
        }
        virtgpu::FENCE_DUP_UNSIGNALED => {
            if fence_handle != 0 {
                live_fences.fetch_sub(1, Ordering::AcqRel);
                return Err(Error::new(EINVAL));
            }
            Fence::new_unsignaled()
        }
        _ => {
            live_fences.fetch_sub(1, Ordering::AcqRel);
            return Err(Error::new(EINVAL));
        }
    };
    Ok(ResourceOpenResult::ThisScheme {
        data: GraphicsResource::Fence(FenceResource::new(fence, live_fences.clone())),
        flags: NewFdFlags::empty(),
    })
}
