// SPDX-License-Identifier: GPL-2.0-only
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Arc;

use driver_graphics::kms::connector::{KmsConnectorDriver, KmsConnectorStatus};
use driver_graphics::kms::crtc::{KmsCrtc, KmsCrtcDriver, KmsCrtcState};
use driver_graphics::kms::framebuffer::KmsFramebuffer;
use driver_graphics::kms::objects::{KmsObjectId, KmsObjects};
use driver_graphics::kms::plane::{KmsPlane, KmsPlaneDriver, KmsPlaneState};
use driver_graphics::kms::rc_object::KmsRcObjectRef;
use driver_graphics::{Buffer, Damage, DumbBufferConfig, Framebuffer, GraphicsAdapter, PendingFrame};
use drm_sys::{
    drm_clip_rect, drm_mode_card_res, drm_mode_create_dumb, drm_mode_crtc, drm_mode_cursor2,
    drm_mode_destroy_dumb, drm_mode_fb_cmd2, drm_mode_fb_dirty_cmd, drm_mode_get_connector,
    drm_mode_modeinfo,
};
use syscall::{EINVAL, Error, Result};

const MODE_GETRESOURCES: u8 = 0xA0;
const MODE_SETCRTC: u8 = 0xA2;
const MODE_GETCONNECTOR: u8 = 0xA7;
const MODE_RMFB: u8 = 0xAF;
const MODE_DIRTYFB: u8 = 0xB1;
const MODE_CREATE_DUMB: u8 = 0xB2;
const MODE_DESTROY_DUMB: u8 = 0xB4;
const MODE_ADDFB2: u8 = 0xB8;
const MODE_CURSOR2: u8 = 0xBB;
const CURSOR_BO: u32 = 1;
const CURSOR_MOVE: u32 = 2;
const CONNECTED: u32 = 1;
const XRGB8888: u32 = u32::from_le_bytes(*b"XR24");
const OBJECTS_MAX: usize = 16;
const EDID_MAX: usize = 32 * 128;
const MODES_MAX: usize = 64;

unsafe extern "C" {
    fn ferrion_drm_ioctl(file: *mut c_void, cmd: u32, arg: *mut c_void) -> i32;
    fn ferrion_drm_vmap(file: *mut c_void, handle: u32, va: *mut *mut c_void, size: *mut usize) -> i32;
    fn ferrion_drm_vunmap(file: *mut c_void, handle: u32, va: *mut c_void);
    fn ferrion_drm_edid(file: *mut c_void, connector: u32, buf: *mut c_void, len: usize) -> isize;
}

#[derive(Debug)]
pub struct DrmFile(*mut c_void);

unsafe impl Send for DrmFile {}
unsafe impl Sync for DrmFile {}

impl DrmFile {
    pub fn new(file: *mut c_void) -> Arc<Self> {
        Arc::new(Self(file))
    }

    fn ioctl<T>(&self, nr: u8, arg: &mut T) -> Result<()> {
        let cmd = (3u32 << 30) | ((size_of::<T>() as u32) << 16) | (u32::from(b'd') << 8) | u32::from(nr);
        let ret = unsafe { ferrion_drm_ioctl(self.0, cmd, arg as *mut T as *mut c_void) };
        if ret < 0 { Err(Error::new(-ret)) } else { Ok(()) }
    }
}

#[derive(Debug)]
pub struct LinuxConnector {
    linux_id: u32,
}

impl KmsConnectorDriver for LinuxConnector {
    type State = ();
}

#[derive(Debug)]
pub struct LinuxCrtc {
    linux_id: u32,
}

impl KmsCrtcDriver for LinuxCrtc {
    type State = ();
}

#[derive(Debug)]
pub struct LinuxPlane {
    cursor: bool,
}

impl KmsPlaneDriver for LinuxPlane {
    type State = ();
}

#[derive(Debug)]
pub struct LinuxBuffer {
    file: Arc<DrmFile>,
    handle: u32,
    va: *mut u8,
    size: usize,
    width: u32,
    height: u32,
    pitch: u32,
}

unsafe impl Send for LinuxBuffer {}
unsafe impl Sync for LinuxBuffer {}

impl Buffer for LinuxBuffer {
    fn size(&self) -> usize {
        self.size
    }
}

impl Drop for LinuxBuffer {
    fn drop(&mut self) {
        unsafe { ferrion_drm_vunmap(self.file.0, self.handle, self.va as *mut c_void) };
        let mut arg = drm_mode_destroy_dumb { handle: self.handle };
        if let Err(error) = self.file.ioctl(MODE_DESTROY_DUMB, &mut arg) {
            log::warn!("ferrion-driver-host: destroying dumb buffer {}: {error}", self.handle);
        }
    }
}

#[derive(Debug)]
pub struct LinuxFramebuffer {
    file: Arc<DrmFile>,
    fb_id: u32,
}

impl Framebuffer for LinuxFramebuffer {}

impl Drop for LinuxFramebuffer {
    fn drop(&mut self) {
        if self.fb_id == 0 {
            return;
        }
        let mut id = self.fb_id;
        if let Err(error) = self.file.ioctl(MODE_RMFB, &mut id) {
            log::warn!("ferrion-driver-host: removing framebuffer {}: {error}", self.fb_id);
        }
    }
}

#[derive(Debug)]
pub struct LinuxAdapter {
    file: Arc<DrmFile>,
    unique: String,
    limits: (u32, u32, u32, u32),
    crtc_ids: Vec<u32>,
    connector_ids: Vec<u32>,
    scanout: HashMap<u32, (u32, drm_mode_modeinfo)>,
    cursor: HashMap<u32, u32>,
}

impl LinuxAdapter {
    pub fn new(file: Arc<DrmFile>, unique: String) -> std::result::Result<Self, String> {
        let mut crtcs = [0u32; OBJECTS_MAX];
        let mut connectors = [0u32; OBJECTS_MAX];
        let mut res = drm_mode_card_res::default();
        file.ioctl(MODE_GETRESOURCES, &mut res).map_err(|e| format!("GETRESOURCES: {e}"))?;
        let n_crtcs = (res.count_crtcs as usize).min(OBJECTS_MAX);
        let n_connectors = (res.count_connectors as usize).min(OBJECTS_MAX);
        let mut res = drm_mode_card_res {
            crtc_id_ptr: crtcs.as_mut_ptr() as u64,
            connector_id_ptr: connectors.as_mut_ptr() as u64,
            count_crtcs: n_crtcs as u32,
            count_connectors: n_connectors as u32,
            ..Default::default()
        };
        file.ioctl(MODE_GETRESOURCES, &mut res).map_err(|e| format!("GETRESOURCES: {e}"))?;
        Ok(Self {
            file,
            unique,
            limits: (res.min_width, res.max_width, res.min_height, res.max_height),
            crtc_ids: crtcs[..n_crtcs].to_vec(),
            connector_ids: connectors[..n_connectors].to_vec(),
            scanout: HashMap::new(),
            cursor: HashMap::new(),
        })
    }

    fn linux_crtc(objects: &KmsObjects<Self>, id: KmsObjectId) -> Result<(u32, Option<drm_mode_modeinfo>)> {
        let crtc = objects.get_crtc(id)?;
        let mode = crtc.state.lock().unwrap().mode;
        Ok((crtc.driver_data.linux_id, mode))
    }

    fn connectors_on(objects: &KmsObjects<Self>, crtc: KmsObjectId) -> Vec<u32> {
        objects
            .connectors()
            .filter_map(|c| {
                let c = c.lock().unwrap();
                (c.state.crtc_id == crtc).then_some(c.driver_data.linux_id)
            })
            .collect()
    }

    fn disable(&mut self, linux_crtc: u32) -> Result<()> {
        self.scanout.remove(&linux_crtc);
        let mut arg = drm_mode_crtc { crtc_id: linux_crtc, ..Default::default() };
        self.file.ioctl(MODE_SETCRTC, &mut arg)
    }

    fn dirty(&self, fb_id: u32, damage: Option<Damage>) -> Result<()> {
        let mut clip = damage.map(|d| drm_clip_rect {
            x1: d.x as u16,
            y1: d.y as u16,
            x2: (d.x + d.width) as u16,
            y2: (d.y + d.height) as u16,
        });
        let mut arg = drm_mode_fb_dirty_cmd {
            fb_id,
            num_clips: u32::from(clip.is_some()),
            clips_ptr: clip.as_mut().map_or(0, |c| c as *mut drm_clip_rect as u64),
            ..Default::default()
        };
        self.file.ioctl(MODE_DIRTYFB, &mut arg)
    }

    fn set_cursor(
        &mut self,
        linux_crtc: u32,
        old: Option<&KmsRcObjectRef<KmsFramebuffer<Self>>>,
        new: &KmsPlaneState<Self>,
        damage: Option<Damage>,
    ) -> Result<()> {
        let mut arg = drm_mode_cursor2 {
            crtc_id: linux_crtc,
            x: new.crtc_rect.x,
            y: new.crtc_rect.y,
            ..Default::default()
        };
        match &new.fb {
            Some(fb) => {
                let changed = old != Some(fb) || damage.is_none_or(|d| d.width != 0 && d.height != 0);
                arg.flags = CURSOR_MOVE;
                if changed {
                    let (hot_x, hot_y) = new.hotspot.unwrap_or((0, 0));
                    arg.flags |= CURSOR_BO;
                    arg.width = fb.width;
                    arg.height = fb.height;
                    arg.handle = fb.buffer.handle;
                    arg.hot_x = hot_x;
                    arg.hot_y = hot_y;
                    self.cursor.insert(linux_crtc, fb.buffer.handle);
                }
            }
            None => {
                if self.cursor.remove(&linux_crtc).is_none() {
                    return Ok(());
                }
                arg.flags = CURSOR_BO;
            }
        }
        self.file.ioctl(MODE_CURSOR2, &mut arg)
    }
}

impl GraphicsAdapter for LinuxAdapter {
    type Connector = LinuxConnector;
    type Crtc = LinuxCrtc;
    type Plane = LinuxPlane;

    type Buffer = LinuxBuffer;
    type Framebuffer = LinuxFramebuffer;

    fn name(&self) -> &'static [u8] {
        b"virtio_gpu"
    }

    fn desc(&self) -> &'static [u8] {
        b"virtio GPU, Linux driver in the FerrionOS driver host"
    }

    fn init(&mut self, objects: &mut KmsObjects<Self>) {
        for (crtc_id, connector_id) in self.crtc_ids.iter().zip(self.connector_ids.iter()) {
            let (crtc, _primary) = objects.add_crtc(
                LinuxCrtc { linux_id: *crtc_id },
                (),
                LinuxPlane { cursor: false },
                (),
                Some((LinuxPlane { cursor: true }, ())),
            );
            objects.add_connector(LinuxConnector { linux_id: *connector_id }, (), &[crtc]);
        }
    }

    fn get_unique(&self) -> String {
        self.unique.clone()
    }

    fn min_max_fb_size(&self) -> (u32, u32, u32, u32) {
        self.limits
    }

    fn dumb_buffer_config(&self) -> Option<DumbBufferConfig> {
        Some(DumbBufferConfig { preferred_depth: 24, prefer_shadow: false })
    }

    fn cursor_size(&self) -> Option<(u64, u64)> {
        Some((64, 64))
    }

    fn cursor_plane_needs_hotspot(&self) -> bool {
        true
    }

    fn probe_connector(&mut self, objects: &mut KmsObjects<Self>, id: KmsObjectId) -> Result<()> {
        let linux_id = objects.get_connector(id)?.lock().unwrap().driver_data.linux_id;
        let mut probe = drm_mode_get_connector { connector_id: linux_id, ..Default::default() };
        self.file.ioctl(MODE_GETCONNECTOR, &mut probe)?;
        if probe.count_modes as usize > MODES_MAX {
            return Err(Error::new(syscall::E2BIG));
        }
        let count = probe.count_modes as usize;
        let mut modes = vec![drm_mode_modeinfo::default(); count];
        if count > 0 {
            let mut all = drm_mode_get_connector {
                connector_id: linux_id,
                count_modes: count as u32,
                modes_ptr: modes.as_mut_ptr() as u64,
                ..Default::default()
            };
            self.file.ioctl(MODE_GETCONNECTOR, &mut all)?;
            modes.truncate(all.count_modes as usize);
        }
        let mut edid = vec![0u8; EDID_MAX];
        let len = unsafe { ferrion_drm_edid(self.file.0, linux_id, edid.as_mut_ptr() as *mut c_void, edid.len()) };
        if len < 0 {
            return Err(Error::new(-len as i32));
        }
        edid.truncate((len as usize).min(EDID_MAX));
        if !edid.is_empty() {
            objects.set_connector_edid(id, edid);
        }
        let mut connector = objects.get_connector(id)?.lock().unwrap();
        connector.connection = if probe.connection == CONNECTED {
            KmsConnectorStatus::Connected
        } else {
            KmsConnectorStatus::Disconnected
        };
        connector.mm_width = probe.mm_width;
        connector.mm_height = probe.mm_height;
        connector.modes = modes;
        Ok(())
    }

    fn create_dumb_buffer(&mut self, width: u32, height: u32) -> Result<(Self::Buffer, u32)> {
        let mut arg = drm_mode_create_dumb { width, height, bpp: 32, ..Default::default() };
        self.file.ioctl(MODE_CREATE_DUMB, &mut arg)?;
        let mut va = core::ptr::null_mut();
        let mut size = 0usize;
        let ret = unsafe { ferrion_drm_vmap(self.file.0, arg.handle, &mut va, &mut size) };
        if ret < 0 {
            let mut destroy = drm_mode_destroy_dumb { handle: arg.handle };
            let _ = self.file.ioctl(MODE_DESTROY_DUMB, &mut destroy);
            return Err(Error::new(-ret));
        }
        let buffer = LinuxBuffer {
            file: self.file.clone(),
            handle: arg.handle,
            va: va as *mut u8,
            size,
            width,
            height,
            pitch: arg.pitch,
        };
        Ok((buffer, arg.pitch))
    }

    fn map_dumb_buffer(&mut self, buffer: &Self::Buffer) -> *mut u8 {
        buffer.va
    }

    fn create_framebuffer(&mut self, buffer: &Self::Buffer) -> Self::Framebuffer {
        let mut handles = [0u32; 4];
        let mut pitches = [0u32; 4];
        handles[0] = buffer.handle;
        pitches[0] = buffer.pitch;
        let mut arg = drm_mode_fb_cmd2 {
            width: buffer.width,
            height: buffer.height,
            pixel_format: XRGB8888,
            handles,
            pitches,
            ..Default::default()
        };
        if let Err(error) = self.file.ioctl(MODE_ADDFB2, &mut arg) {
            log::error!("ferrion-driver-host: ADDFB2 for buffer {}: {error}", buffer.handle);
            arg.fb_id = 0;
        }
        LinuxFramebuffer { file: self.file.clone(), fb_id: arg.fb_id }
    }

    fn set_crtc(
        &mut self,
        _objects: &KmsObjects<Self>,
        crtc: &KmsCrtc<Self>,
        state: KmsCrtcState<Self>,
        _connector_ids: &[KmsObjectId],
    ) -> Result<()> {
        let disable = state.mode.is_none();
        *crtc.state.lock().unwrap() = state;
        if disable {
            self.disable(crtc.driver_data.linux_id)?;
        }
        Ok(())
    }

    fn set_plane(
        &mut self,
        objects: &KmsObjects<Self>,
        plane: &KmsPlane<Self>,
        new_plane_state: KmsPlaneState<Self>,
        damage: Option<Damage>,
    ) -> Result<()> {
        let Some(crtc_id) = new_plane_state.crtc_id else {
            *plane.state.lock().unwrap() = new_plane_state;
            return Ok(());
        };
        let (linux_crtc, mode) = Self::linux_crtc(objects, crtc_id)?;
        if plane.driver_data.cursor {
            let old = plane.state.lock().unwrap().fb.clone();
            self.set_cursor(linux_crtc, old.as_ref(), &new_plane_state, damage)?;
            *plane.state.lock().unwrap() = new_plane_state;
            return Ok(());
        }
        match (&new_plane_state.fb, mode) {
            (None, _) | (_, None) => self.disable(linux_crtc)?,
            (Some(fb), Some(mode)) => {
                let fb_id = fb.driver_data.fb_id;
                if fb_id == 0 {
                    return Err(Error::new(EINVAL));
                }
                if self.scanout.get(&linux_crtc) != Some(&(fb_id, mode)) {
                    let mut connectors = Self::connectors_on(objects, crtc_id);
                    let mut arg = drm_mode_crtc {
                        set_connectors_ptr: connectors.as_mut_ptr() as u64,
                        count_connectors: connectors.len() as u32,
                        crtc_id: linux_crtc,
                        fb_id,
                        x: new_plane_state.src_rect.x,
                        y: new_plane_state.src_rect.y,
                        mode_valid: 1,
                        mode,
                        ..Default::default()
                    };
                    self.file.ioctl(MODE_SETCRTC, &mut arg)?;
                    self.scanout.insert(linux_crtc, (fb_id, mode));
                } else {
                    self.dirty(fb_id, damage)?;
                }
            }
        }
        *plane.state.lock().unwrap() = new_plane_state;
        Ok(())
    }

    fn dirty_framebuffer(
        &mut self,
        objects: &KmsObjects<Self>,
        fb: &KmsRcObjectRef<KmsFramebuffer<Self>>,
        damage: Damage,
    ) -> Result<Option<PendingFrame>> {
        let mut scanned = false;
        for plane in objects.planes() {
            let state = plane.state.lock().unwrap().clone();
            if state.fb.as_ref() != Some(fb) {
                continue;
            }
            if plane.driver_data.cursor {
                self.set_plane(objects, plane, state, Some(damage))?;
            } else if state.crtc_id.is_some() {
                scanned = true;
            }
        }
        if scanned {
            self.dirty(fb.driver_data.fb_id, Some(damage))?;
        }
        Ok(None)
    }
}
