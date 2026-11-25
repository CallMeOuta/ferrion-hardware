// SPDX-License-Identifier: GPL-2.0-only AND MIT
use std::ffi::c_char;
use std::fmt::Debug;
use std::fs;
use std::io::{self, Read};
use std::path::Path;
use std::sync::Mutex;

pub const FIRMWARE_EDID_DIR: &str = "/usr/lib/firmware/edid";

pub fn connector_type_name(connector_type: u32) -> &'static str {
    match connector_type {
        0 => "Unknown",
        1 => "VGA",
        2 => "DVI-I",
        3 => "DVI-D",
        4 => "DVI-A",
        5 => "Composite",
        6 => "SVIDEO",
        7 => "LVDS",
        8 => "Component",
        9 => "9PinDIN",
        10 => "DP",
        11 => "HDMI-A",
        12 => "HDMI-B",
        13 => "TV",
        14 => "eDP",
        15 => "Virtual",
        16 => "DSI",
        17 => "DPI",
        18 => "Writeback",
        19 => "SPI",
        20 => "USB",
        _ => "Unknown",
    }
}

#[derive(Debug, Clone)]
pub struct EdidOverride {
    pub path: String,
    pub data: Vec<u8>,
}

fn read_connector_edid_override(connector_name: &str) -> Option<EdidOverride> {
    let candidate = Path::new(FIRMWARE_EDID_DIR).join(format!("{connector_name}.edid"));
    match fs::symlink_metadata(&candidate) {
        Ok(_) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => return None,
        Err(err) => {
            log::warn!(
                "connector {connector_name}: ignoring EDID override {}: {err}",
                candidate.display()
            );
            return None;
        }
    }

    let canonical_dir = match fs::canonicalize(FIRMWARE_EDID_DIR) {
        Ok(dir) => dir,
        Err(err) => {
            log::warn!(
                "connector {connector_name}: ignoring EDID override {}: {err}",
                candidate.display()
            );
            return None;
        }
    };
    let canonical_target = match fs::canonicalize(&candidate) {
        Ok(path) => path,
        Err(err) => {
            log::warn!(
                "connector {connector_name}: ignoring EDID override {}: {err}",
                candidate.display()
            );
            return None;
        }
    };
    if !canonical_target.starts_with(&canonical_dir) {
        log::warn!(
            "connector {connector_name}: ignoring EDID override {} (resolves outside {FIRMWARE_EDID_DIR})",
            candidate.display()
        );
        return None;
    }

    let meta = match fs::metadata(&canonical_target) {
        Ok(meta) => meta,
        Err(err) => {
            log::warn!(
                "connector {connector_name}: ignoring EDID override {}: {err}",
                candidate.display()
            );
            return None;
        }
    };
    if !meta.is_file() {
        log::warn!(
            "connector {connector_name}: ignoring EDID override {} (not a regular file)",
            candidate.display()
        );
        return None;
    }
    let max_bytes = edid::MAX_EDID_OVERRIDE_BYTES as u64;
    if meta.len() == 0 || meta.len() > max_bytes {
        log::warn!(
            "connector {connector_name}: ignoring EDID override {} (size {} outside 1..={max_bytes})",
            candidate.display(),
            meta.len()
        );
        return None;
    }

    let file = match fs::File::open(&canonical_target) {
        Ok(file) => file,
        Err(err) => {
            log::warn!(
                "connector {connector_name}: ignoring EDID override {}: {err}",
                candidate.display()
            );
            return None;
        }
    };
    let mut data = Vec::with_capacity(meta.len() as usize);
    if let Err(err) = file.take(max_bytes + 1).read_to_end(&mut data) {
        log::warn!(
            "connector {connector_name}: ignoring EDID override {}: {err}",
            candidate.display()
        );
        return None;
    }
    if data.is_empty() || data.len() > edid::MAX_EDID_OVERRIDE_BYTES {
        log::warn!(
            "connector {connector_name}: ignoring EDID override {} (size {} outside 1..={})",
            candidate.display(),
            data.len(),
            edid::MAX_EDID_OVERRIDE_BYTES
        );
        return None;
    }
    if let Err(err) = edid::decode_all(&data) {
        log::warn!(
            "connector {connector_name}: ignoring invalid EDID override {}: {err:?}",
            candidate.display()
        );
        return None;
    }

    Some(EdidOverride {
        path: candidate.display().to_string(),
        data,
    })
}

pub use drm_sys::{DRM_MODE_CONNECTOR_Unknown, DRM_MODE_CONNECTOR_VIRTUAL};
use drm_sys::{
    DRM_MODE_DPMS_OFF, DRM_MODE_DPMS_ON, DRM_MODE_DPMS_STANDBY, DRM_MODE_DPMS_SUSPEND,
    DRM_MODE_TYPE_DRIVER, DRM_MODE_TYPE_PREFERRED, drm_mode_modeinfo,
};
use syscall::Result;

use crate::GraphicsAdapter;
use crate::edid::{self, ModeTiming};
use crate::kms::objects::{KmsObjectId, KmsObjects};
use crate::kms::properties::{
    CRTC_ID, DPMS, EDID, KmsBlob, KmsPropertyData, TILE, define_object_props,
};
use crate::kms::rc_object::KmsRcObjectRef;

impl<T: GraphicsAdapter> KmsObjects<T> {
    pub fn add_connector(
        &mut self,
        connector_type: u32,
        driver_data: T::Connector,
        driver_data_state: <T::Connector as KmsConnectorDriver>::State,
        crtcs: &[KmsObjectId],
    ) -> KmsObjectId {
        let mut possible_crtcs = 0;
        for &crtc in crtcs {
            possible_crtcs = 1 << self.get_crtc(crtc).unwrap().crtc_index;
        }

        let encoder_id = self.add(KmsEncoder {
            crtc_id: KmsObjectId::INVALID,
            possible_crtcs: possible_crtcs,
            possible_clones: 1 << self.encoders.len(),
        });
        self.encoders.push(encoder_id);

        let connector_type_id = self
            .connectors
            .iter()
            .filter(|&&id| {
                self.get_connector(id)
                    .unwrap()
                    .lock()
                    .unwrap()
                    .connector_type
                    == connector_type
            })
            .count()
            .checked_add(1)
            .and_then(|count| u32::try_from(count).ok())
            .expect("connector_type_id overflow");

        let connector_index = self.connectors.len();
        let connector_id = self.add(Mutex::new(KmsConnector {
            connector_index,
            encoder_id,
            modes: vec![],
            connector_type,
            connector_type_id,
            connection: KmsConnectorStatus::Unknown,
            mm_width: 0,
            mm_height: 0,
            subpixel: DrmSubpixelOrder::Unknown,
            properties: KmsConnector::base_properties(),
            edid: None,
            edid_override: None,
            state: KmsConnectorState {
                dpms: KmsDpms::On,
                crtc_id: KmsObjectId::INVALID,
                driver_data: driver_data_state,
            },
            driver_data,
        }));
        self.connectors.push(connector_id);

        connector_id
    }

    pub fn connector_ids(&self) -> &[KmsObjectId] {
        &self.connectors
    }

    pub fn connectors(&self) -> impl Iterator<Item = &Mutex<KmsConnector<T>>> + use<'_, T> {
        self.connectors
            .iter()
            .map(|&id| self.get_connector(id).unwrap())
    }

    pub fn get_connector(&self, id: KmsObjectId) -> Result<&Mutex<KmsConnector<T>>> {
        self.get(id)
    }

    pub fn load_edid_overrides(&mut self) {
        let ids = self.connectors.clone();
        for id in ids {
            let name = self.get_connector(id).unwrap().lock().unwrap().name();
            let loaded = read_connector_edid_override(&name);
            self.get_connector(id).unwrap().lock().unwrap().edid_override = loaded;
        }
    }

    pub fn apply_connector_edid_override(&mut self, id: KmsObjectId) -> bool {
        let (name, path, data, should_log) = {
            let connector = self.get_connector(id).unwrap().lock().unwrap();
            let Some(ov) = connector.edid_override.as_ref() else {
                return false;
            };
            let should_log = connector.modes.is_empty()
                || connector.edid.as_ref().map(|b| b.data()) != Some(ov.data.as_slice());
            (
                connector.name(),
                ov.path.clone(),
                ov.data.clone(),
                should_log,
            )
        };
        if should_log {
            log::info!(
                "connector {name}: applying EDID override {path} ({} bytes)",
                data.len()
            );
        }
        self.set_connector_edid_raw(id, data);
        true
    }

    pub fn set_connector_edid(&mut self, id: KmsObjectId, edid: Vec<u8>) {
        if self.apply_connector_edid_override(id) {
            return;
        }
        self.set_connector_edid_raw(id, edid);
    }

    fn set_connector_edid_raw(&mut self, id: KmsObjectId, edid: Vec<u8>) {
        let mut connector = self.get_connector(id).unwrap().lock().unwrap();
        connector.update_from_edid(&edid);

        let old_edid = &connector.edid;
        if let Some(old_edid) = old_edid {
            if old_edid.data() == edid {
                return; // EDID is unchanged; nothing to do
            }
        }
        drop(connector);

        let blob = self.add_blob(edid);
        self.get_connector(id).unwrap().lock().unwrap().edid = Some(blob);
    }

    pub fn encoder_ids(&self) -> &[KmsObjectId] {
        &self.encoders
    }

    pub fn get_encoder(&self, id: KmsObjectId) -> Result<&KmsEncoder> {
        self.get(id)
    }
}

pub trait KmsConnectorDriver: Debug {
    type State: Clone + Debug;
}

impl KmsConnectorDriver for () {
    type State = ();
}

#[derive(Debug)]
pub struct KmsConnector<T: GraphicsAdapter> {
    pub connector_index: usize,
    pub encoder_id: KmsObjectId,
    pub modes: Vec<drm_mode_modeinfo>,
    pub connector_type: u32,
    pub connector_type_id: u32,
    pub connection: KmsConnectorStatus,
    pub mm_width: u32,
    pub mm_height: u32,
    pub subpixel: DrmSubpixelOrder,
    pub properties: Vec<KmsPropertyData<Self>>,
    pub edid: Option<KmsRcObjectRef<KmsBlob>>,
    pub edid_override: Option<EdidOverride>,
    pub state: KmsConnectorState<T>,
    pub driver_data: T::Connector,
}

#[derive(Debug)]
pub struct KmsConnectorState<T: GraphicsAdapter> {
    pub dpms: KmsDpms,
    pub crtc_id: KmsObjectId,
    pub driver_data: <T::Connector as KmsConnectorDriver>::State,
}

impl<T: GraphicsAdapter> Clone for KmsConnectorState<T> {
    fn clone(&self) -> Self {
        Self {
            dpms: self.dpms.clone(),
            crtc_id: self.crtc_id.clone(),
            driver_data: self.driver_data.clone(),
        }
    }
}

define_object_props!(object, KmsConnector<T: GraphicsAdapter> {
    EDID {
        get => u64::from(object.edid.as_ref().map_or(0, |blob| blob.id().0)),
    }
    DPMS {
        get => object.state.dpms as u64,
    }
    TILE {
        get => {
            // FIXME set this property once we support DP MST
            let _ = object;
            0
        },
    }
    CRTC_ID {
        get => u64::from(object.state.crtc_id.0),
    }
});

impl<T: GraphicsAdapter> KmsConnector<T> {
    pub fn name(&self) -> String {
        format!(
            "{}-{}",
            connector_type_name(self.connector_type),
            self.connector_type_id
        )
    }

    pub fn has_edid_override(&self) -> bool {
        self.edid_override.is_some()
    }

    pub fn update_from_size(&mut self, width: u32, height: u32) {
        self.modes = vec![modeinfo_for_size(width, height)];
    }

    pub fn prefer_size(&mut self, width: u32, height: u32) {
        let matches = |mode: &drm_mode_modeinfo| {
            u32::from(mode.hdisplay) == width && u32::from(mode.vdisplay) == height
        };
        match self.modes.iter().position(matches) {
            Some(index) => {
                let mode = self.modes.remove(index);
                self.modes.insert(0, mode);
            }
            None => self.modes.insert(0, modeinfo_for_size(width, height)),
        }
        for (index, mode) in self.modes.iter_mut().enumerate() {
            if index == 0 {
                mode.type_ |= DRM_MODE_TYPE_PREFERRED;
            } else {
                mode.type_ &= !DRM_MODE_TYPE_PREFERRED;
            }
        }
    }

    fn update_from_edid(&mut self, edid: &[u8]) {
        let decoded = match edid::decode_all(edid) {
            Ok(decoded) => decoded,
            Err(err) => {
                log::error!("unreadable EDID: {err:?}");
                self.modes.clear();
                return;
            }
        };
        if decoded.extensions_truncated {
            log::warn!(
                "EDID extension blocks exceed {}; truncating",
                edid::MAX_EDID_EXTENSION_BLOCKS
            );
        }
        if decoded.bad_extension_blocks > 0 {
            log::warn!(
                "skipped {} EDID extension block(s) with bad checksum",
                decoded.bad_extension_blocks
            );
        }
        if decoded.modes_truncated {
            log::warn!(
                "EDID mode list exceeded {}; truncating",
                edid::MAX_EDID_MODES
            );
        }
        if decoded.displayid_timings_truncated {
            log::warn!(
                "DisplayID timing block exceeded {}; truncating",
                edid::MAX_DISPLAYID_TIMINGS
            );
        }

        if decoded.mm_width > 0 && decoded.mm_height > 0 {
            self.mm_width = decoded.mm_width;
            self.mm_height = decoded.mm_height;
        }

        let mut modes = Vec::with_capacity(decoded.mode_count + 1);
        let mut seen = Vec::with_capacity(decoded.mode_count + 1);
        for timing in
            std::iter::once(&decoded.preferred).chain(decoded.modes[..decoded.mode_count].iter())
        {
            let key = (timing.width, timing.height, timing.refresh_mhz);
            if seen.contains(&key) {
                continue;
            }
            if let Some(modeinfo) = modeinfo_for_timing(timing, modes.is_empty()) {
                seen.push(key);
                modes.push(modeinfo);
            }
        }
        self.modes = modes;
    }
}

pub fn modeinfo_for_timing(timing: &ModeTiming, is_preferred: bool) -> Option<drm_mode_modeinfo> {
    let hdisplay = u16::try_from(timing.width).ok()?;
    let vdisplay = u16::try_from(timing.height).ok()?;
    if hdisplay == 0 || vdisplay == 0 || timing.htotal < hdisplay || timing.vtotal < vdisplay {
        return None;
    }
    let vrefresh = timing.refresh_mhz.saturating_add(500) / 1000;
    let mut flags = 0;
    if let Some(pos) = timing.hsync_pos {
        flags |= if pos {
            drm_sys::DRM_MODE_FLAG_PHSYNC
        } else {
            drm_sys::DRM_MODE_FLAG_NHSYNC
        };
    }
    if let Some(pos) = timing.vsync_pos {
        flags |= if pos {
            drm_sys::DRM_MODE_FLAG_PVSYNC
        } else {
            drm_sys::DRM_MODE_FLAG_NVSYNC
        };
    }

    let type_ = if is_preferred {
        DRM_MODE_TYPE_DRIVER | DRM_MODE_TYPE_PREFERRED
    } else {
        DRM_MODE_TYPE_DRIVER
    };

    let mut modeinfo = drm_mode_modeinfo {
        hdisplay,
        vdisplay,
        clock: timing.clock_khz,
        htotal: timing.htotal,
        vtotal: timing.vtotal,
        vscan: 0,
        vrefresh,
        type_,
        name: [0; 32],
        hsync_start: timing.hsync_start,
        hsync_end: timing.hsync_end,
        hskew: 0,
        vsync_start: timing.vsync_start,
        vsync_end: timing.vsync_end,
        flags,
    };

    let name = format!("{hdisplay}x{vdisplay}@{vrefresh}").into_bytes();
    for (to, from) in modeinfo.name.iter_mut().zip(name) {
        *to = from as c_char;
    }

    Some(modeinfo)
}

pub fn modeinfo_for_size(width: u32, height: u32) -> drm_mode_modeinfo {
    let mut modeinfo = drm_mode_modeinfo {
        // The actual visible display size
        hdisplay: width as u16,
        vdisplay: height as u16,

        // These are used to calculate the refresh rate
        clock: 60 * width * height / 1000,
        htotal: width as u16,
        vtotal: height as u16,
        vscan: 0,
        vrefresh: 60,

        type_: drm_sys::DRM_MODE_TYPE_PREFERRED | drm_sys::DRM_MODE_TYPE_DRIVER,
        name: [0; 32],

        // These only matter when modesetting physical display adapters. For
        // those we should be able to parse the EDID blob.
        hsync_start: width as u16,
        hsync_end: width as u16,
        hskew: 0,
        vsync_start: height as u16,
        vsync_end: height as u16,
        flags: 0,
    };

    let name = format!("{width}x{height}").into_bytes();
    for (to, from) in modeinfo.name.iter_mut().zip(name) {
        *to = from as c_char;
    }

    modeinfo
}

#[derive(Debug, Copy, Clone)]
#[repr(u32)]
pub enum KmsConnectorStatus {
    Disconnected = 0,
    Connected = 1,
    Unknown = 2,
}

#[derive(Debug, Copy, Clone)]
#[repr(u32)]
pub enum DrmSubpixelOrder {
    Unknown = 0,
    HorizontalRGB,
    HorizontalBGR,
    VerticalRGB,
    VerticalBGR,
    None,
}

#[derive(Debug, Copy, Clone)]
#[repr(u64)]
pub enum KmsDpms {
    On = DRM_MODE_DPMS_ON as u64,
    Standby = DRM_MODE_DPMS_STANDBY as u64,
    Suspend = DRM_MODE_DPMS_SUSPEND as u64,
    Off = DRM_MODE_DPMS_OFF as u64,
}

// FIXME can we represent connector and encoder using a single struct?
#[derive(Debug)]
pub struct KmsEncoder {
    pub crtc_id: KmsObjectId,
    pub possible_crtcs: u32,
    pub possible_clones: u32,
}
