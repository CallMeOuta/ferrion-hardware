// SPDX-License-Identifier: GPL-2.0-only AND MIT
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fmt::Debug;
use std::slice;
use std::sync::Arc;

use drm_sys::DRM_CLIENT_NAME_MAX_LEN;
use graphics_ipc::redox_uapi_exts::RedoxDrmEventConnectorHotplug;
use redox_scheme::{CallerCtx, Id};
use scheme_utils::{FpathWriter, ResourceSync};
use syscall::{EACCES, EFAULT, EINVAL, ENOENT, ENOMEM, Error, EventFlags, MapFlags, Result};

use crate::blob::{ExportCounter, PRIME_WIRE_ALLOWED, PRIME_WIRE_RDWR, SharedBlob};
use crate::buffer::{BlobBuffer, DrmBuffer};
use crate::kms::framebuffer::{KmsFramebuffer, disable_planes_with_fb};
use crate::kms::objects::KmsObjectId;
use crate::kms::rc_object::KmsRcObjectRef;
use crate::{
    Buffer, GraphicsAdapter, GraphicsResource, GraphicsSchemeData, MAP_FAKE_OFFSET_MULTIPLIER,
    ioctl,
};

pub const MAX_DISPLAY_BUFFERS_PER_HANDLE: usize = 4096;

#[derive(Debug)]
pub(crate) struct DumbEntry<T: GraphicsAdapter> {
    pub(crate) buffer: Arc<T::Buffer>,
    pub(crate) writable: bool,
}

#[derive(Debug)]
pub(crate) struct ImportedBlob {
    pub(crate) blob: SharedBlob,
    pub(crate) writable: bool,
}

/// # Safety
///
/// Type must not have any padding or contain any references.
pub(crate) unsafe trait DrmEvent {}
unsafe impl DrmEvent for RedoxDrmEventConnectorHotplug {}

#[derive(Debug)]
pub(crate) struct DrmHandle<T: GraphicsAdapter> {
    pub(crate) vt: usize,
    pub(crate) client_name: [u8; DRM_CLIENT_NAME_MAX_LEN as usize],
    pub(crate) unique: Option<String>,
    pub(crate) supports_universal_planes: bool,
    pub(crate) supports_cursor_hotspot: bool,
    pub(crate) supports_redox_hotplug_events: bool,
    pub(crate) fbs: BTreeMap<KmsObjectId, KmsRcObjectRef<KmsFramebuffer<T>>>,
    pub(crate) next_buffer_id: u32,
    pub(crate) buffers: HashMap<u32, DumbEntry<T>>,
    pub(crate) imported_blobs: HashMap<u32, ImportedBlob>,
    pub(crate) exports: ExportCounter,
    pub(crate) pending_frames: HashMap<Id, crate::PendingFrame>,
    pub(crate) pending_flips: Vec<crate::flip::PendingFlip>,
    events: VecDeque<Box<[u8]>>,
    requested_events: EventFlags,
    notified_read: bool,
}

impl<T: GraphicsAdapter> DrmHandle<T> {
    pub(crate) fn new(vt: usize) -> Self {
        DrmHandle {
            vt,
            client_name: [0; _],
            unique: None,
            supports_universal_planes: false,
            supports_cursor_hotspot: false,
            supports_redox_hotplug_events: false,
            fbs: BTreeMap::new(),
            next_buffer_id: 0,
            buffers: HashMap::new(),
            imported_blobs: HashMap::new(),
            exports: ExportCounter::default(),
            pending_frames: HashMap::new(),
            pending_flips: Vec::new(),
            events: VecDeque::new(),
            requested_events: EventFlags::empty(),
            notified_read: true,
        }
    }

    pub(crate) fn alloc_buffer_id(&mut self) -> Result<u32> {
        if self.buffers.len() + self.imported_blobs.len() >= MAX_DISPLAY_BUFFERS_PER_HANDLE {
            return Err(Error::new(ENOMEM));
        }
        for _ in 0..=MAX_DISPLAY_BUFFERS_PER_HANDLE {
            self.next_buffer_id = self.next_buffer_id.wrapping_add(1);
            let id = self.next_buffer_id;
            if id != 0
                && !self.buffers.contains_key(&id)
                && !self.imported_blobs.contains_key(&id)
            {
                return Ok(id);
            }
        }
        Err(Error::new(ENOMEM))
    }

    pub(crate) fn should_post_event(&self) -> bool {
        self.requested_events.contains(EventFlags::EVENT_READ)
            && !self.events.is_empty()
            && !self.notified_read
    }

    pub(crate) fn mark_read_notified(&mut self) {
        self.notified_read = true;
    }

    pub(crate) fn can_queue_event(&self) -> bool {
        self.events.len() < 1024
    }

    pub(crate) fn push_event<U: DrmEvent>(&mut self, event: U) {
        self.events.push_back(
            unsafe { slice::from_raw_parts((&raw const event).cast::<u8>(), size_of::<U>()) }
                .to_vec()
                .into_boxed_slice(),
        );
        self.notified_read = false;
    }
}

impl<T: GraphicsAdapter> ResourceSync for DrmHandle<T> {
    type SchemeData = GraphicsSchemeData<T>;
    type ResourceEnum = GraphicsResource<T>;

    fn read(
        &mut self,
        _scheme_data: &mut Self::SchemeData,
        buf: &mut [u8],
        _offset: u64,
        _fcntl_flags: u32,
    ) -> Result<usize> {
        if self.events.is_empty() {
            return Err(Error::new(syscall::EWOULDBLOCK));
        }
        let mut written = 0;
        while let Some(event) = self.events.pop_front() {
            if event.len() > buf.len() - written {
                self.events.push_front(event);
                if written == 0 {
                    return Err(Error::new(EFAULT));
                }
                break;
            }
            buf[written..written + event.len()].copy_from_slice(&event);
            written += event.len();
        }
        self.notified_read = false;
        Ok(written)
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
        if self.events.is_empty() {
            self.notified_read = false;
            Ok(EventFlags::empty())
        } else {
            self.notified_read = true;
            Ok(EventFlags::EVENT_READ)
        }
    }

    fn fstat(
        &mut self,
        _scheme_data: &mut Self::SchemeData,
        stat: &mut syscall::Stat,
    ) -> Result<()> {
        stat.st_dev = 226 /*DRM_MAJOR*/ << 8;
        Ok(())
    }

    fn fpath(
        &mut self,
        _scheme_data: &mut Self::SchemeData,
        w: &mut FpathWriter,
    ) -> syscall::Result<()> {
        write!(w, "{}", self.vt).unwrap();
        Ok(())
    }

    fn call(
        &mut self,
        scheme_data: &mut Self::SchemeData,
        payload: &mut [u8],
        metadata: &[u64],
        ctx: &CallerCtx,
    ) -> Result<usize> {
        if let Some(frame) = self.pending_frames.get(&ctx.id) {
            let Some(result) = frame.take() else {
                return Err(Error::new(syscall::EWOULDBLOCK));
            };
            self.pending_frames.remove(&ctx.id);
            return result;
        }
        let mut pending = None;
        let res = ioctl::call_ioctl(
            &mut scheme_data.adapter,
            &mut scheme_data.objects,
            scheme_data.active_vt,
            &mut scheme_data.vts,
            &scheme_data.waker,
            &mut scheme_data.crtc_flips,
            self,
            metadata[0],
            payload,
            &mut pending,
        );
        scheme_data.objects.remove_all_deferred();
        if let Some(frame) = pending {
            self.pending_frames.insert(ctx.id, frame);
            Err(Error::new(syscall::EWOULDBLOCK))
        } else {
            res
        }
    }

    fn call_with_handles(
        &mut self,
        scheme_data: &mut Self::SchemeData,
        handles: &mut [Self::ResourceEnum],
        payload: &mut [u8],
        metadata: &[u64],
        _ctx: &CallerCtx,
    ) -> Result<usize> {
        let res = ioctl::call_ioctl_handles(
            &mut scheme_data.adapter,
            &mut scheme_data.objects,
            scheme_data.active_vt,
            &mut scheme_data.vts,
            self,
            metadata[0],
            handles,
            payload,
        );
        scheme_data.objects.remove_all_deferred();
        res
    }

    fn mmap_prep(
        &mut self,
        scheme_data: &mut Self::SchemeData,
        offset: u64,
        size: usize,
        flags: MapFlags,
    ) -> syscall::Result<usize> {
        // The offset comes from MODE_MAP_DUMB: the buffer number above MAP_FAKE_OFFSET_MULTIPLIER,
        // the position inside that buffer below it. The mapping has to stay inside the buffer.
        let entry = u32::try_from(offset / MAP_FAKE_OFFSET_MULTIPLIER as u64)
            .ok()
            .and_then(|buffer_id| self.buffers.get(&buffer_id))
            .ok_or(Error::new(ENOENT))?;
        if !entry.writable && flags.contains(MapFlags::PROT_WRITE) {
            return Err(Error::new(EACCES));
        }
        let framebuffer = &entry.buffer;
        let offset = (offset % MAP_FAKE_OFFSET_MULTIPLIER as u64) as usize;
        let end = offset.checked_add(size).ok_or(Error::new(EINVAL))?;
        if size == 0 || end > framebuffer.size() {
            return Err(Error::new(EINVAL));
        }
        let ptr = T::map_dumb_buffer(&mut scheme_data.adapter, framebuffer);
        Ok(unsafe { ptr.add(offset) } as usize)
    }

    fn on_close(self, scheme_data: &mut Self::SchemeData) {
        for &fb_id in self.fbs.keys() {
            disable_planes_with_fb(
                &mut scheme_data.adapter,
                &mut scheme_data.objects,
                scheme_data.active_vt,
                &mut scheme_data.vts,
                fb_id,
            );
        }
        scheme_data.objects.remove_all_deferred();
    }

    fn dup(
        &mut self,
        _scheme_data: &mut Self::SchemeData,
        buf: &[u8],
        _ctx: &CallerCtx,
    ) -> Result<scheme_utils::ResourceOpenResult<Self::ResourceEnum>> {
        use redox_ioctl::{drm as ipc, virtgpu};
        assert!(!buf.is_empty());
        if buf.len() != 16 {
            return Err(Error::new(EINVAL));
        }
        let cmd = u64::from_ne_bytes(buf[0..8].try_into().unwrap());
        let handle = u32::from_ne_bytes(buf[8..12].try_into().unwrap());
        let wire_flags = u32::from_ne_bytes(buf[12..16].try_into().unwrap());
        if cmd != ipc::PRIME_HANDLE_TO_FD {
            return Err(Error::new(EINVAL));
        }
        if wire_flags & !PRIME_WIRE_ALLOWED != 0 {
            return Err(Error::new(EINVAL));
        }
        let writable = (wire_flags & PRIME_WIRE_RDWR) != 0;
        if let Some(entry) = self.buffers.get(&handle) {
            if writable && !entry.writable {
                return Err(Error::new(EACCES));
            }
            let guard = self.exports.acquire()?;
            return Ok(scheme_utils::ResourceOpenResult::ThisScheme {
                data: GraphicsResource::DrmBuffer(DrmBuffer {
                    buffer: entry.buffer.clone(),
                    writable,
                    _export_guard: guard,
                }),
                flags: syscall::schemev2::NewFdFlags::empty(),
            });
        }
        if let Some(entry) = self.imported_blobs.get(&handle) {
            if entry.blob.info().blob_flags & virtgpu::VIRTGPU_BLOB_FLAG_USE_SHAREABLE == 0 {
                return Err(Error::new(EINVAL));
            }
            if writable && !entry.writable {
                return Err(Error::new(EACCES));
            }
            let guard = self.exports.acquire()?;
            return Ok(scheme_utils::ResourceOpenResult::ThisScheme {
                data: GraphicsResource::BlobBuffer(BlobBuffer::new(
                    entry.blob.clone(),
                    writable,
                    guard,
                )),
                flags: syscall::schemev2::NewFdFlags::empty(),
            });
        }
        Err(Error::new(ENOENT))
    }
}
