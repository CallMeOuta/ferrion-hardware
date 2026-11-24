// SPDX-License-Identifier: GPL-2.0-only AND MIT
use std::ffi::c_char;
use std::fmt::Debug;
use std::sync::Mutex;

use drm_sys::{
    DRM_MODE_CONNECTOR_Unknown, DRM_MODE_DPMS_OFF, DRM_MODE_DPMS_ON, DRM_MODE_DPMS_STANDBY,
    DRM_MODE_DPMS_SUSPEND, DRM_MODE_TYPE_PREFERRED, drm_mode_modeinfo,
};
use piaf::{ExtensionLibrary, VideoMode, capabilities_from_edid, parse_edid};
use syscall::Result;

use crate::GraphicsAdapter;
use crate::kms::objects::{KmsObjectId, KmsObjects};
use crate::kms::properties::{
    CRTC_ID, DPMS, EDID, KmsBlob, KmsPropertyData, TILE, define_object_props,
};
use crate::kms::rc_object::KmsRcObjectRef;

impl<T: GraphicsAdapter> KmsObjects<T> {
    pub fn add_connector(
        &mut self,
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

        let connector_index = self.connectors.len();
        let connector_id = self.add(Mutex::new(KmsConnector {
            connector_index,
            encoder_id,
            modes: vec![],
            connector_type: DRM_MODE_CONNECTOR_Unknown,
            connector_type_id: self.connectors.len() as u32, // FIXME maybe pick unique id within connector type?
            connection: KmsConnectorStatus::Unknown,
            mm_width: 0,
            mm_height: 0,
            subpixel: DrmSubpixelOrder::Unknown,
            properties: KmsConnector::base_properties(),
            edid: None,
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

    pub fn set_connector_edid(&mut self, id: KmsObjectId, edid: Vec<u8>) {
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
        let library = ExtensionLibrary::with_standard_handlers();
        let parsed = match parse_edid(edid, &library) {
            Ok(parsed) => parsed,
            Err(err) => {
                log::error!("unreadable EDID: {err:?}");
                self.modes.clear();
                return;
            }
        };
        let caps = capabilities_from_edid(&parsed, &library);

        if let Some(size) = caps.preferred_image_size_mm {
            self.mm_width = size.0.into();
            self.mm_height = size.1.into();
        }

        self.modes = caps
            .preferred_timing
            .iter()
            .chain(caps.supported_modes.iter())
            .filter_map(modeinfo_for_detailed_timing)
            .collect::<Vec<_>>();
        let mut seen = Vec::with_capacity(self.modes.len());
        self.modes.retain(|mode| {
            let key = (mode.hdisplay, mode.vdisplay, mode.vrefresh);
            let fresh = !seen.contains(&key);
            seen.push(key);
            fresh
        });

        for mode in self.modes.iter_mut().skip(1) {
            mode.type_ &= !DRM_MODE_TYPE_PREFERRED;
        }
    }
}

pub fn modeinfo_for_detailed_timing(timing: &VideoMode) -> Option<drm_mode_modeinfo> {
    let hdisplay = timing.width;
    let hblank = timing.h_blank?;
    let htotal = hdisplay.checked_add(hblank)?;
    let hsync_start = hdisplay.checked_add(timing.h_front_porch)?;
    let hsync_end = hsync_start.checked_add(timing.h_sync_width)?;
    let vdisplay = timing.height;
    let vsync_start = vdisplay.checked_add(timing.v_front_porch)?;
    let vsync_end = vsync_start.checked_add(timing.v_sync_width)?;

    let clock = timing.pixel_clock_khz?;
    let rate = timing.refresh_rate?;
    let line_rate = u64::from(htotal) * u64::from(rate.numer());
    if line_rate == 0 {
        return None;
    }
    let pixels_per_frame = u64::from(clock) * 1000 * u64::from(rate.denom());
    let vtotal = u16::try_from((pixels_per_frame + line_rate / 2) / line_rate).ok()?;
    if vtotal < vsync_end {
        return None;
    }
    let vrefresh = (clock * 1000) / (u32::from(htotal) * u32::from(vtotal));

    let mut flags = 0;
    match timing.sync {
        Some(piaf::SyncDefinition::DigitalSeparate {
            v_sync_positive,
            h_sync_positive,
        }) => {
            if v_sync_positive {
                flags |= drm_sys::DRM_MODE_FLAG_PVSYNC;
            } else {
                flags |= drm_sys::DRM_MODE_FLAG_NVSYNC;
            }
            if h_sync_positive {
                flags |= drm_sys::DRM_MODE_FLAG_PHSYNC;
            } else {
                flags |= drm_sys::DRM_MODE_FLAG_NHSYNC;
            }
        }
        Some(unsupported) => log::warn!("unsupported sync {:?}", unsupported),
        None => log::warn!("not a DTD mode"),
    }

    let mut modeinfo = drm_mode_modeinfo {
        // The actual visible display size
        hdisplay,
        vdisplay,

        // These are used to calculate the refresh rate
        clock,
        htotal,
        vtotal,
        vscan: 0,
        vrefresh,

        type_: drm_sys::DRM_MODE_TYPE_DRIVER | drm_sys::DRM_MODE_TYPE_PREFERRED,
        name: [0; 32],

        // These only matter when modesetting physical display adapters. For
        // those we should be able to parse the EDID blob.
        hsync_start,
        hsync_end,
        hskew: 0,
        vsync_start,
        vsync_end,
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
