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

use std::fmt::Debug;
use std::marker::PhantomData;

use redox_scheme::CallerCtx;
use scheme_utils::{ResourceOpenResult, ResourceSync};
use syscall::schemev2::NewFdFlags;
use syscall::{ENODEV, ENOENT, Error, MODE_CHR, Result};

use crate::{GraphicsAdapter, GraphicsResource, GraphicsSchemeData, ioctl};

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
    _adapter: PhantomData<T>,
}

impl<T> Debug for RenderHandle<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderHandle")
            .field("context", &self.context)
            .finish()
    }
}

impl<T> RenderHandle<T> {
    fn new() -> Self {
        Self {
            context: None,
            _adapter: PhantomData,
        }
    }
}

impl<T: GraphicsAdapter> ResourceSync for RenderHandle<T> {
    type SchemeData = GraphicsSchemeData<T>;
    type ResourceEnum = GraphicsResource<T>;

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
        _ctx: &CallerCtx,
    ) -> Result<usize> {
        let device = scheme_data
            .adapter
            .render_device()
            .ok_or(Error::new(ENODEV))?;
        ioctl::virtgpu::call_render_ioctl(device, self, metadata[0], payload)
    }

    fn on_close(self, scheme_data: &mut Self::SchemeData) {
        if let Some(context) = self.context {
            if let Some(device) = scheme_data.adapter.render_device() {
                device.destroy_context(context);
            }
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
