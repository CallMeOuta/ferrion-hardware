// SPDX-License-Identifier: GPL-2.0-only
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::task::Waker;

use drm_sys::{
    DRM_EVENT_FLIP_COMPLETE, DRM_EVENT_VBLANK, DRM_MODE_PAGE_FLIP_ASYNC, DRM_MODE_PAGE_FLIP_EVENT,
    drm_event, drm_event_vblank, drm_vblank_seq_type,
};
use redox_ioctl::drm::{DrmModeCrtcPageFlipTarget, DrmWaitVblank};
use syscall::{EBUSY, EINVAL, ENOMEM, Error, Result};

use crate::handle::{DrmEvent, DrmHandle};
use crate::kms::objects::{KmsObjectId, KmsObjects};
use crate::{GraphicsAdapter, GraphicsResource, GraphicsSchemeImpl, VtState};

unsafe impl DrmEvent for drm_event_vblank {}

const _: () = assert!(size_of::<drm_event_vblank>() == 32);

const DEFAULT_REFRESH_HZ: u64 = 60;
const ALLOWED_FLIP_FLAGS: u32 = DRM_MODE_PAGE_FLIP_EVENT | DRM_MODE_PAGE_FLIP_ASYNC;
const VBLANK_HIGH_CRTC_SHIFT: u32 = 1;
const VBLANK_TYPES_MASK: u32 =
    drm_vblank_seq_type::_DRM_VBLANK_ABSOLUTE | drm_vblank_seq_type::_DRM_VBLANK_RELATIVE;
const VBLANK_ALLOWED_MASK: u32 = VBLANK_TYPES_MASK
    | drm_vblank_seq_type::_DRM_VBLANK_HIGH_CRTC_MASK
    | drm_vblank_seq_type::_DRM_VBLANK_EVENT
    | drm_vblank_seq_type::_DRM_VBLANK_NEXTONMISS
    | drm_vblank_seq_type::_DRM_VBLANK_SECONDARY;

#[derive(Clone, Debug)]
pub struct FlipCompletion {
    result: Rc<Cell<Option<Result<()>>>>,
    waker: Waker,
}

impl FlipCompletion {
    pub fn new(waker: Waker) -> Self {
        Self {
            result: Rc::default(),
            waker,
        }
    }

    pub fn complete(&self, result: Result<()>) {
        self.result.set(Some(result));
        self.waker.wake_by_ref();
    }

    pub(crate) fn take(&self) -> Option<Result<()>> {
        self.result.take()
    }
}

#[derive(Debug)]
pub(crate) struct PendingFlip {
    completion: FlipCompletion,
    crtc_id: KmsObjectId,
    crtc_index: usize,
    user_data: u64,
    send_event: bool,
    busy: Rc<Cell<bool>>,
}

impl Drop for PendingFlip {
    fn drop(&mut self) {
        self.busy.set(false);
    }
}

#[derive(Debug, Default)]
pub(crate) struct CrtcFlipState {
    last_sequence: u32,
    busy: Rc<Cell<bool>>,
}

fn monotonic_now() -> (u32, u32, u64) {
    match libredox::call::clock_gettime(libredox::flag::CLOCK_MONOTONIC) {
        Ok(ts) => {
            let secs = ts.tv_sec.max(0) as u64;
            let nsecs = ts.tv_nsec.max(0) as u64;
            let total_ns = secs.saturating_mul(1_000_000_000).saturating_add(nsecs);
            (secs as u32, (nsecs / 1_000) as u32, total_ns)
        }
        Err(_) => (0, 0, 0),
    }
}

impl CrtcFlipState {
    fn sample(&mut self, refresh_hz: u64, advance: bool) -> (u32, u32, u32) {
        let (sec, usec, total_ns) = monotonic_now();
        let period_ns = 1_000_000_000 / refresh_hz.max(1);
        let clock_seq = (total_ns / period_ns) as u32;
        let seq = if advance {
            if clock_seq > self.last_sequence {
                clock_seq
            } else {
                self.last_sequence.wrapping_add(1)
            }
        } else {
            clock_seq.max(self.last_sequence)
        };
        self.last_sequence = seq;
        (seq, sec, usec)
    }
}

fn crtc_refresh_hz<T: GraphicsAdapter>(objects: &KmsObjects<T>, crtc_id: KmsObjectId) -> u64 {
    objects
        .get_crtc(crtc_id)
        .ok()
        .and_then(|crtc| crtc.state.lock().unwrap().mode)
        .map(|mode| u64::from(mode.vrefresh))
        .filter(|&hz| hz > 0)
        .unwrap_or(DEFAULT_REFRESH_HZ)
}

fn ensure_crtc_state(states: &mut Vec<CrtcFlipState>, crtc_index: usize) -> &mut CrtcFlipState {
    if states.len() <= crtc_index {
        states.resize_with(crtc_index + 1, CrtcFlipState::default);
    }
    &mut states[crtc_index]
}

pub(crate) fn mode_page_flip<T: GraphicsAdapter>(
    adapter: &mut T,
    objects: &KmsObjects<T>,
    active_vt: usize,
    vts: &mut HashMap<usize, VtState<T>>,
    waker: &Waker,
    crtc_flips: &mut Vec<CrtcFlipState>,
    handle: &mut DrmHandle<T>,
    data: DrmModeCrtcPageFlipTarget<'_>,
) -> Result<usize> {
    if !adapter.supports_page_flip() {
        return Err(Error::new(EINVAL));
    }

    let flags = data.flags();
    if (flags & !ALLOWED_FLIP_FLAGS) != 0 || data.sequence() != 0 {
        return Err(Error::new(EINVAL));
    }

    let crtc_id = KmsObjectId(data.crtc_id());
    let crtc = objects.get_crtc(crtc_id)?;
    let crtc_index = crtc.crtc_index as usize;
    let fb = objects.get_framebuffer(KmsObjectId(data.fb_id()))?;
    let plane = objects.get_plane(crtc.primary_plane)?;
    let plane_index = plane.plane_index as usize;

    let vt_state = vts.get_mut(&handle.vt).ok_or(Error::new(EINVAL))?;
    if vt_state.plane_state[plane_index].fb.is_none() {
        return Err(Error::new(EINVAL));
    }

    let send_event = (flags & DRM_MODE_PAGE_FLIP_EVENT) != 0;
    if send_event && !handle.can_queue_event() {
        return Err(Error::new(ENOMEM));
    }

    let crtc_state = ensure_crtc_state(crtc_flips, crtc_index);
    if crtc_state.busy.get() {
        return Err(Error::new(EBUSY));
    }

    let mut new_plane_state = vt_state.plane_state[plane_index].clone();
    new_plane_state.fb = Some(fb.clone());
    new_plane_state.crtc_id = Some(crtc_id);

    let completion = FlipCompletion::new(waker.clone());
    if handle.vt == active_vt {
        adapter.page_flip(objects, crtc_id, plane, &fb, completion.clone())?;
        *plane.state.lock().unwrap() = new_plane_state.clone();
    } else {
        completion.complete(Ok(()));
    }
    vt_state.plane_state[plane_index] = new_plane_state;

    let busy = crtc_state.busy.clone();
    busy.set(true);
    handle.pending_flips.push(PendingFlip {
        completion,
        crtc_id,
        crtc_index,
        user_data: data.user_data(),
        send_event,
        busy,
    });

    Ok(0)
}

pub(crate) fn wait_vblank<T: GraphicsAdapter>(
    objects: &KmsObjects<T>,
    crtc_flips: &mut Vec<CrtcFlipState>,
    handle: &mut DrmHandle<T>,
    mut data: DrmWaitVblank<'_>,
) -> Result<usize> {
    let req_type = data.request_type();
    if (req_type & !VBLANK_ALLOWED_MASK) != 0 {
        return Err(Error::new(EINVAL));
    }

    let high = (req_type & drm_vblank_seq_type::_DRM_VBLANK_HIGH_CRTC_MASK) >> VBLANK_HIGH_CRTC_SHIFT;
    let crtc_index = if high != 0 {
        high as usize
    } else if (req_type & drm_vblank_seq_type::_DRM_VBLANK_SECONDARY) != 0 {
        1
    } else {
        0
    };
    let crtc_id = *objects
        .crtc_ids()
        .get(crtc_index)
        .ok_or(Error::new(EINVAL))?;

    let refresh_hz = crtc_refresh_hz(objects, crtc_id);
    let crtc_state = ensure_crtc_state(crtc_flips, crtc_index);
    let advance = data.request_sequence() != 0;
    let (seq, sec, usec) = crtc_state.sample(refresh_hz, advance);

    if (req_type & drm_vblank_seq_type::_DRM_VBLANK_EVENT) != 0 {
        if !handle.can_queue_event() {
            return Err(Error::new(ENOMEM));
        }
        handle.push_event(drm_event_vblank {
            base: drm_event {
                type_: DRM_EVENT_VBLANK,
                length: size_of::<drm_event_vblank>() as u32,
            },
            user_data: data.request_signal(),
            tv_sec: sec,
            tv_usec: usec,
            sequence: seq,
            crtc_id: crtc_id.0,
        });
    }

    data.set_reply_type(req_type & VBLANK_TYPES_MASK);
    data.set_reply_sequence(seq);
    data.set_reply_tval_sec(i64::from(sec));
    data.set_reply_tval_usec(i64::from(usec));
    Ok(0)
}

pub(crate) fn collect_completed_flips<T: GraphicsAdapter>(inner: &mut GraphicsSchemeImpl<T>) {
    let (handles, scheme_data) = inner.handles_mut_and_scheme_data();
    for (_, resource) in handles {
        let GraphicsResource::DrmHandle(handle) = resource else {
            continue;
        };
        let mut idx = 0;
        while idx < handle.pending_flips.len() {
            let Some(result) = handle.pending_flips[idx].completion.take() else {
                idx += 1;
                continue;
            };
            let flip = handle.pending_flips.remove(idx);
            let refresh_hz = crtc_refresh_hz(&scheme_data.objects, flip.crtc_id);
            let crtc_state = ensure_crtc_state(&mut scheme_data.crtc_flips, flip.crtc_index);
            let (seq, sec, usec) = crtc_state.sample(refresh_hz, true);
            if flip.send_event && result.is_ok() && handle.can_queue_event() {
                handle.push_event(drm_event_vblank {
                    base: drm_event {
                        type_: DRM_EVENT_FLIP_COMPLETE,
                        length: size_of::<drm_event_vblank>() as u32,
                    },
                    user_data: flip.user_data,
                    tv_sec: sec,
                    tv_usec: usec,
                    sequence: seq,
                    crtc_id: flip.crtc_id.0,
                });
            }
        }
    }
}
