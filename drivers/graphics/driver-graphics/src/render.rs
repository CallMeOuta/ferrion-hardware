//! The render side of a GPU (FerrionOS).
//!
//! A GPU's scheme has two names: `display.<driver>`, whose root opens display handles for a
//! virtual terminal, and `gpu.<driver>`, whose root opens render handles. A render handle is a
//! client of the GPU: it holds one context on the device and, in later steps, buffers and
//! fences. It can neither set a mode, show a buffer, move the cursor nor read a display. The
//! render name is the grant to use the GPU, so it is registered beside the display name, and
//! a namespace holds it only when given it.
//!
//! The requests a render handle serves are those of Linux's virtio-gpu driver that Mesa's Venus
//! driver makes, in `ioctl::virtgpu`; everything a client sends is checked there before the
//! device sees it.

use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::Arc;

use redox_scheme::{CallerCtx, Id};
use scheme_utils::{ResourceOpenResult, ResourceSync};
use syscall::schemev2::NewFdFlags;
use syscall::{EINVAL, ENODEV, ENOENT, ENOSYS, Error, EventFlags, MODE_CHR, MapFlags, MunmapFlags, Result};

use crate::blob::{BlobCreateParams, BlobInfo, RenderBlobs};
use crate::{Fence, GraphicsAdapter, GraphicsResource, GraphicsSchemeData, ioctl};

/// The device features render handles report, as Linux's virtio-gpu driver reports them.
#[derive(Clone, Copy, Debug, Default)]
pub struct RenderFeatures {
    /// 3D: virtio-gpu's virgl feature, which every 3D capset needs.
    pub three_d: bool,
    pub resource_blob: bool,
    pub context_init: bool,
    /// The host-visible memory region is mapped, so host blobs can be.
    pub host_visible: bool,
}

/// What a GPU's driver gives render handles. Every value it receives has been checked against
/// Linux's rules by the request code; it still trusts nothing it reads from the device.
pub trait RenderDevice: Debug {
    /// The name, description and version (major, minor, patch level) a render handle's version
    /// request answers. Mesa picks its driver by the name and the major version.
    fn driver_info(&self) -> (&'static [u8], &'static [u8], [i32; 3]);

    fn features(&self) -> RenderFeatures;

    /// The capset numbers the device offers, one bit each.
    fn capset_ids(&self) -> u64;

    /// Capset `id` at `version`, as the device answered it at start.
    fn capset(&self, id: u32, version: u32) -> Option<&[u8]>;

    /// Creates a context on the device that speaks capset `capset_id` (0: none named), labelled
    /// `name`, and returns its number.
    fn create_context(&mut self, capset_id: u32, name: &[u8]) -> Result<u32>;

    /// Destroys a context `create_context` made, which stops its work on the host.
    fn destroy_context(&mut self, context_id: u32);

    fn create_blob(&mut self, _params: BlobCreateParams<'_>) -> Result<BlobInfo> {
        Err(Error::new(ENOSYS))
    }

    fn map_blob(&mut self, _res_id: u32) -> Result<usize> {
        Err(Error::new(ENOSYS))
    }

    fn unmap_blob(&mut self, _res_id: u32) {}

    fn destroy_blob(&mut self, _res_id: u32) {}

    fn submit_3d(
        &mut self,
        _context_id: u32,
        _ring_idx: Option<u8>,
        _command: &[u8],
    ) -> Result<Arc<Fence>> {
        Err(Error::new(EINVAL))
    }
}

/// The root registered under the render name.
pub(crate) struct RenderRoot<T>(pub(crate) PhantomData<T>);

impl<T> Debug for RenderRoot<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("RenderRoot").finish()
    }
}

impl<T: GraphicsAdapter> ResourceSync for RenderRoot<T> {
    type SchemeData = GraphicsSchemeData<T>;
    type ResourceEnum = GraphicsResource<T>;

    fn openat(
        &mut self,
        scheme_data: &mut Self::SchemeData,
        path: &str,
        _flags: usize,
        _fcntl_flags: u32,
        _ctx: &CallerCtx,
    ) -> Result<ResourceOpenResult<Self::ResourceEnum>> {
        // The root is the device: each open of it is a new client, and nothing lies below it.
        if !path.trim_matches('/').is_empty() {
            return Err(Error::new(ENOENT));
        }
        if scheme_data.adapter.render_device().is_none() {
            return Err(Error::new(ENODEV));
        }
        Ok(ResourceOpenResult::ThisScheme {
            data: GraphicsResource::RenderHandle(RenderHandle::new()),
            flags: NewFdFlags::empty(),
        })
    }
}

/// One client of the GPU, opened from the render root. Its context, the device's number for
/// it, is set once by context-init and destroyed with the handle; no request names it, so no
/// other handle can reach it.
pub(crate) struct RenderHandle<T> {
    pub(crate) context: Option<u32>,
    pub(crate) blobs: RenderBlobs,
    pub(crate) num_rings: u32,
    pub(crate) known_resources: HashSet<u32>,
    pub(crate) resource_fences: HashMap<u32, Arc<Fence>>,
    pub(crate) last_fence: Option<Arc<Fence>>,
    pub(crate) pending_fences: HashMap<i32, Arc<Fence>>,
    next_fence_handle: i32,
    pub(crate) pending_waits: HashMap<Id, crate::submit::PendingWait>,
    _adapter: PhantomData<T>,
}

impl<T> Debug for RenderHandle<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderHandle")
            .field("context", &self.context)
            .field("blobs", &self.blobs)
            .field("num_rings", &self.num_rings)
            .finish()
    }
}

impl<T> RenderHandle<T> {
    fn new() -> Self {
        Self {
            context: None,
            blobs: RenderBlobs::new(),
            num_rings: 0,
            known_resources: HashSet::new(),
            resource_fences: HashMap::new(),
            last_fence: None,
            pending_fences: HashMap::new(),
            next_fence_handle: 1,
            pending_waits: HashMap::new(),
            _adapter: PhantomData,
        }
    }

    pub(crate) fn alloc_pending_fence(&mut self, fence: Arc<Fence>) -> i32 {
        loop {
            let id = self.next_fence_handle;
            self.next_fence_handle = self.next_fence_handle.checked_add(1).unwrap_or(1);
            if id > 0 && !self.pending_fences.contains_key(&id) {
                self.pending_fences.insert(id, fence);
                return id;
            }
        }
    }
}

impl<T: GraphicsAdapter> ResourceSync for RenderHandle<T> {
    type SchemeData = GraphicsSchemeData<T>;
    type ResourceEnum = GraphicsResource<T>;

    fn fevent(
        &mut self,
        _scheme_data: &mut Self::SchemeData,
        _flags: EventFlags,
    ) -> Result<EventFlags> {
        Ok(EventFlags::empty())
    }

    fn fstat(
        &mut self,
        _scheme_data: &mut Self::SchemeData,
        stat: &mut syscall::Stat,
    ) -> Result<()> {
        // A DRM render node: major 226, minors from 128.
        stat.st_mode = MODE_CHR;
        stat.st_dev = (226 << 8) | 128;
        Ok(())
    }

    fn call(
        &mut self,
        scheme_data: &mut Self::SchemeData,
        payload: &mut [u8],
        metadata: &[u64],
        ctx: &CallerCtx,
    ) -> Result<usize> {
        if let Some(wait) = self.pending_waits.get(&ctx.id) {
            let Some(result) = wait.poll_result() else {
                return Err(Error::new(syscall::EWOULDBLOCK));
            };
            self.pending_waits.remove(&ctx.id);
            return result;
        }
        let device = scheme_data
            .adapter
            .render_device()
            .ok_or(Error::new(ENODEV))?;
        let mut pending_wait = None;
        let res = ioctl::virtgpu::call_render_ioctl(
            device,
            self,
            metadata[0],
            payload,
            &mut pending_wait,
        );
        if let Some(wait) = pending_wait {
            self.pending_waits.insert(ctx.id, wait);
            Err(Error::new(syscall::EWOULDBLOCK))
        } else {
            res
        }
    }

    fn dup(
        &mut self,
        scheme_data: &mut Self::SchemeData,
        buf: &[u8],
        _ctx: &CallerCtx,
    ) -> Result<ResourceOpenResult<Self::ResourceEnum>> {
        crate::submit::dup_fence(self, &scheme_data.live_fences, buf)
    }

    fn mmap_prep(
        &mut self,
        scheme_data: &mut Self::SchemeData,
        offset: u64,
        size: usize,
        flags: MapFlags,
    ) -> Result<usize> {
        let device = scheme_data
            .adapter
            .render_device()
            .ok_or(Error::new(ENODEV))?;
        self.blobs.mmap_prep(device, offset, size, flags)
    }

    fn munmap(
        &mut self,
        scheme_data: &mut Self::SchemeData,
        offset: u64,
        size: usize,
        _flags: MunmapFlags,
    ) -> Result<()> {
        if let Some(device) = scheme_data.adapter.render_device() {
            self.blobs.munmap(device, offset, size)?;
        }
        Ok(())
    }

    fn on_close(mut self, scheme_data: &mut Self::SchemeData) {
        if let Some(device) = scheme_data.adapter.render_device() {
            if let Some(context) = self.context {
                device.destroy_context(context);
            }
            self.blobs.destroy_all(device);
        }
    }
}

/// Registers `gpu.<driver>` for the scheme registered as `display.<driver>`: a second root on
/// the same socket, which opens render handles.
pub(crate) fn register_render_root<T: GraphicsAdapter>(
    inner: &mut crate::GraphicsSchemeImpl<T>,
    socket: &redox_scheme::Socket,
    display_name: &str,
) {
    let render_name = format!(
        "gpu.{}",
        display_name
            .strip_prefix("display.")
            .unwrap_or(display_name)
    );
    let root = inner
        .new_handle_fd(
            socket,
            GraphicsResource::RenderRoot(RenderRoot(PhantomData)),
            0,
        )
        .expect("failed to create the graphics render root");
    // As redox_scheme's register_scheme_inner does for a root number.
    let namespace = libredox::call::getns().expect("failed to get the namespace");
    libredox::call::register_scheme_to_ns(namespace, &render_name, root.into_raw())
        .expect("failed to register the graphics render root");
    log::info!("{render_name}: render handles for {display_name}");
}
