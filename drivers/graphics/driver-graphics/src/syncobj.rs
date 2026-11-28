// SPDX-License-Identifier: GPL-2.0-only
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, HashMap};
use std::ffi::c_char;
use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::{Arc, Condvar, Mutex};
use std::task::Waker;
use std::thread;
use std::time::{Duration, Instant};

use redox_ioctl::sync::{
    self, DRM_SYNCOBJ_CREATE_SIGNALED, DRM_SYNCOBJ_FD_TO_HANDLE_FLAGS_IMPORT_SYNC_FILE,
    DRM_SYNCOBJ_FD_TO_HANDLE_FLAGS_TIMELINE, DRM_SYNCOBJ_HANDLE_TO_FD_FLAGS_EXPORT_SYNC_FILE,
    DRM_SYNCOBJ_HANDLE_TO_FD_FLAGS_TIMELINE, DRM_SYNCOBJ_QUERY_FLAGS_LAST_SUBMITTED,
    DRM_SYNCOBJ_WAIT_FLAGS_WAIT_ALL, DRM_SYNCOBJ_WAIT_FLAGS_WAIT_AVAILABLE,
    DRM_SYNCOBJ_WAIT_FLAGS_WAIT_DEADLINE, DRM_SYNCOBJ_WAIT_FLAGS_WAIT_FOR_SUBMIT,
    DrmSyncobjArray, DrmSyncobjCreate, DrmSyncobjDestroy, DrmSyncobjEventfd, DrmSyncobjHandle,
    DrmSyncobjQuery, DrmSyncobjTimelineSignal, DrmSyncobjTimelineWait, DrmSyncobjTransfer,
    DrmSyncobjWait, SYNC_MAX_FENCES, SYNCOBJ_MAX_HANDLES, SyncFileInfo, SyncSetDeadline,
};
use redox_scheme::{CallerCtx, Id};
use scheme_utils::{ResourceOpenResult, ResourceSync};
use syscall::schemev2::NewFdFlags;
use syscall::{EINVAL, ENOENT, ENOMEM, EOPNOTSUPP, ETIME, EWOULDBLOCK, Error, EventFlags, Result};

use crate::render::RenderHandle;
use crate::{GraphicsAdapter, GraphicsResource, GraphicsSchemeData};

const MAX_SYNCOBJS_PER_HANDLE: usize = 4096;
const MAX_TIMELINE_POINTS: usize = 1024;
const MAX_MERGED_FENCES: usize = 64;
const MAX_PENDING_MERGES: usize = 16;
const MAX_PENDING_WAITS_PER_HANDLE: usize = 64;

fn monotonic_now_ns() -> u64 {
    let mut ts = syscall::TimeSpec::default();
    if syscall::clock_gettime(syscall::CLOCK_MONOTONIC, &mut ts).is_ok() {
        (ts.tv_sec as u64)
            .saturating_mul(1_000_000_000)
            .saturating_add(ts.tv_nsec as u64)
    } else {
        0
    }
}

fn copy_name_32(src: &[u8]) -> [u8; 32] {
    let mut dst = [0u8; 32];
    let len = src.len().min(31);
    for (i, &b) in src[..len].iter().enumerate() {
        if b == 0 {
            break;
        }
        dst[i] = b;
    }
    dst
}

fn bytes_to_c_char_32(src: &[u8; 32]) -> [c_char; 32] {
    let mut dst = [0 as c_char; 32];
    for i in 0..32 {
        dst[i] = src[i] as c_char;
    }
    dst
}

#[derive(Debug)]
struct FenceLeafInner {
    signaled: bool,
    status: i32,
    timestamp_ns: u64,
}

#[derive(Debug)]
pub struct FenceLeaf {
    inner: Mutex<FenceLeafInner>,
    obj_name: [u8; 32],
    waker: Waker,
}

impl FenceLeaf {
    pub fn new_unsignaled(obj_name: &[u8], waker: Waker) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(FenceLeafInner {
                signaled: false,
                status: 0,
                timestamp_ns: 0,
            }),
            obj_name: copy_name_32(obj_name),
            waker,
        })
    }

    pub fn new_signaled(obj_name: &[u8], waker: Waker) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(FenceLeafInner {
                signaled: true,
                status: 1,
                timestamp_ns: monotonic_now_ns(),
            }),
            obj_name: copy_name_32(obj_name),
            waker,
        })
    }

    pub fn signal(&self) {
        let woke = {
            let mut inner = self.inner.lock().unwrap();
            if inner.signaled {
                false
            } else {
                inner.signaled = true;
                inner.status = 1;
                inner.timestamp_ns = monotonic_now_ns();
                true
            }
        };
        if woke {
            self.waker.wake_by_ref();
        }
    }

    pub fn signal_error(&self, errno: i32) {
        let err = if errno > 0 { -errno } else { -1 };
        let woke = {
            let mut inner = self.inner.lock().unwrap();
            if inner.signaled {
                false
            } else {
                inner.signaled = true;
                inner.status = err;
                inner.timestamp_ns = monotonic_now_ns();
                true
            }
        };
        if woke {
            self.waker.wake_by_ref();
        }
    }

    pub fn is_signaled(&self) -> bool {
        self.inner.lock().unwrap().signaled
    }

    fn snapshot(&self) -> (i32, u64) {
        let inner = self.inner.lock().unwrap();
        (inner.status, inner.timestamp_ns)
    }
}

#[derive(Clone, Debug)]
pub struct SyncFence {
    leaves: Vec<Arc<FenceLeaf>>,
}

impl SyncFence {
    pub fn from_leaf(leaf: Arc<FenceLeaf>) -> Self {
        Self { leaves: vec![leaf] }
    }

    pub fn new_unsignaled(obj_name: &[u8], waker: Waker) -> Self {
        Self::from_leaf(FenceLeaf::new_unsignaled(obj_name, waker))
    }

    pub fn new_signaled(obj_name: &[u8], waker: Waker) -> Self {
        Self::from_leaf(FenceLeaf::new_signaled(obj_name, waker))
    }

    pub fn signal(&self) {
        for leaf in &self.leaves {
            leaf.signal();
        }
    }

    pub fn is_signaled(&self) -> bool {
        self.leaves.iter().all(|leaf| leaf.is_signaled())
    }

    pub fn status(&self) -> i32 {
        let mut all_signaled = true;
        for leaf in &self.leaves {
            let (st, _) = leaf.snapshot();
            if st < 0 {
                return st;
            }
            if st == 0 {
                all_signaled = false;
            }
        }
        if all_signaled { 1 } else { 0 }
    }

    pub fn merge(&self, other: &Self) -> Result<Self> {
        let mut leaves = Vec::with_capacity(self.leaves.len() + other.leaves.len());
        for leaf in self.leaves.iter().chain(other.leaves.iter()) {
            if !leaves.iter().any(|existing| Arc::ptr_eq(existing, leaf)) {
                if leaves.len() >= MAX_MERGED_FENCES {
                    return Err(Error::new(ENOMEM));
                }
                leaves.push(leaf.clone());
            }
        }
        Ok(Self { leaves })
    }
}

#[derive(Debug, Default)]
struct SyncobjState {
    binary_fence: Option<SyncFence>,
    binary_pending: Option<SyncFence>,
    timeline: BTreeMap<u64, SyncFence>,
    signaled_point: u64,
}

impl SyncobjState {
    fn refresh(&mut self) {
        let mut highest = self.signaled_point;
        for (&point, fence) in &self.timeline {
            if fence.is_signaled() && point > highest {
                highest = point;
            }
        }
        if highest > self.signaled_point {
            self.signaled_point = highest;
        }
        let mut to_signal = Vec::new();
        for (&point, fence) in &self.timeline {
            if point <= self.signaled_point {
                to_signal.push(fence.clone());
            } else {
                break;
            }
        }
        for fence in to_signal {
            fence.signal();
        }
        self.timeline.retain(|&point, _| point > self.signaled_point);
    }
}

#[derive(Debug)]
pub struct Syncobj {
    state: Mutex<SyncobjState>,
    waker: Waker,
}

impl Syncobj {
    pub fn new(signaled: bool, waker: Waker) -> Arc<Self> {
        let binary_fence = if signaled {
            Some(SyncFence::new_signaled(b"syncobj", waker.clone()))
        } else {
            None
        };
        Arc::new(Self {
            state: Mutex::new(SyncobjState {
                binary_fence,
                binary_pending: None,
                timeline: BTreeMap::new(),
                signaled_point: 0,
            }),
            waker,
        })
    }

    pub fn reset(&self) {
        let mut state = self.state.lock().unwrap();
        state.binary_fence = None;
        state.binary_pending = None;
        state.timeline.clear();
        state.signaled_point = 0;
    }

    pub fn signal_binary(&self) {
        let pending = {
            let mut state = self.state.lock().unwrap();
            if let Some(p) = state.binary_pending.take() {
                state.binary_fence = Some(p.clone());
                Some(p)
            } else {
                state.binary_fence = Some(SyncFence::new_signaled(b"syncobj", self.waker.clone()));
                None
            }
        };
        if let Some(p) = pending {
            p.signal();
        }
        self.waker.wake_by_ref();
    }

    pub fn signal_point(&self, point: u64) {
        if point == 0 {
            self.signal_binary();
            return;
        }
        let to_signal = {
            let mut state = self.state.lock().unwrap();
            if point > state.signaled_point {
                state.signaled_point = point;
            }
            let mut list = Vec::new();
            for (&p, fence) in &state.timeline {
                if p <= state.signaled_point {
                    list.push(fence.clone());
                } else {
                    break;
                }
            }
            let signaled = state.signaled_point;
            state.timeline.retain(|&p, _| p > signaled);
            list
        };
        for fence in to_signal {
            fence.signal();
        }
        self.waker.wake_by_ref();
    }

    pub fn attach_fence(&self, point: u64, fence: SyncFence) -> Result<()> {
        if point == 0 {
            let pending = {
                let mut state = self.state.lock().unwrap();
                let p = if fence.is_signaled() {
                    state.binary_pending.take()
                } else {
                    state.binary_pending = None;
                    None
                };
                state.binary_fence = Some(fence);
                p
            };
            if let Some(p) = pending {
                p.signal();
            }
            self.waker.wake_by_ref();
            return Ok(());
        }
        if fence.is_signaled() {
            self.signal_point(point);
            return Ok(());
        }
        let mut state = self.state.lock().unwrap();
        state.refresh();
        if state.signaled_point >= point {
            drop(state);
            fence.signal();
            return Ok(());
        }
        if !state.timeline.contains_key(&point) && state.timeline.len() >= MAX_TIMELINE_POINTS {
            return Err(Error::new(ENOMEM));
        }
        state.timeline.insert(point, fence);
        drop(state);
        self.waker.wake_by_ref();
        Ok(())
    }

    pub fn is_point_signaled(&self, point: u64) -> bool {
        let mut state = self.state.lock().unwrap();
        state.refresh();
        if point == 0 {
            state
                .binary_fence
                .as_ref()
                .is_some_and(SyncFence::is_signaled)
                || state.signaled_point > 0
        } else {
            state.signaled_point >= point
        }
    }

    pub fn is_point_available(&self, point: u64) -> bool {
        let mut state = self.state.lock().unwrap();
        state.refresh();
        if point == 0 {
            state.binary_fence.is_some()
                || !state.timeline.is_empty()
                || state.signaled_point > 0
        } else {
            state.signaled_point >= point || state.timeline.keys().any(|&p| p >= point)
        }
    }

    pub fn query(&self, last_submitted: bool) -> u64 {
        let mut state = self.state.lock().unwrap();
        state.refresh();
        if last_submitted {
            state
                .timeline
                .keys()
                .next_back()
                .copied()
                .unwrap_or(0)
                .max(state.signaled_point)
        } else {
            state.signaled_point
        }
    }

    pub fn export_fence(&self, point: u64) -> Result<SyncFence> {
        let mut state = self.state.lock().unwrap();
        state.refresh();
        if point == 0 {
            if let Some(fence) = &state.binary_fence {
                return Ok(fence.clone());
            }
            if state.signaled_point > 0 {
                return Ok(SyncFence::new_signaled(b"syncobj", self.waker.clone()));
            }
            if let Some(pending) = &state.binary_pending {
                return Ok(pending.clone());
            }
            let pending = SyncFence::new_unsignaled(b"syncobj", self.waker.clone());
            state.binary_pending = Some(pending.clone());
            return Ok(pending);
        }
        if state.signaled_point >= point {
            return Ok(SyncFence::new_signaled(b"timeline", self.waker.clone()));
        }
        if let Some(fence) = state.timeline.get(&point) {
            return Ok(fence.clone());
        }
        if state.timeline.len() >= MAX_TIMELINE_POINTS {
            return Err(Error::new(ENOMEM));
        }
        let fence = SyncFence::new_unsignaled(b"timeline", self.waker.clone());
        state.timeline.insert(point, fence.clone());
        Ok(fence)
    }

    pub(crate) fn can_attach(&self, point: u64) -> bool {
        if point == 0 {
            return true;
        }
        let mut state = self.state.lock().unwrap();
        state.refresh();
        state.signaled_point >= point
            || state.timeline.contains_key(&point)
            || state.timeline.len() < MAX_TIMELINE_POINTS
    }

    pub(crate) fn find_fence(&self, point: u64) -> Result<SyncFence> {
        self.find_transfer_fence(point)
    }

    fn find_transfer_fence(&self, point: u64) -> Result<SyncFence> {
        let mut state = self.state.lock().unwrap();
        state.refresh();
        if point == 0 {
            if let Some(fence) = &state.binary_fence {
                return Ok(fence.clone());
            }
            if state.signaled_point > 0 {
                return Ok(SyncFence::new_signaled(b"syncobj", self.waker.clone()));
            }
            return Err(Error::new(EINVAL));
        }
        if state.signaled_point >= point {
            return Ok(SyncFence::new_signaled(b"timeline", self.waker.clone()));
        }
        if let Some((_, fence)) = state.timeline.range(point..).next() {
            return Ok(fence.clone());
        }
        Err(Error::new(EINVAL))
    }
}

#[derive(Debug, Default)]
pub(crate) struct SyncobjTable {
    next_id: u32,
    entries: HashMap<u32, Arc<Syncobj>>,
}

impl SyncobjTable {
    fn alloc_id(&mut self) -> Result<u32> {
        if self.entries.len() >= MAX_SYNCOBJS_PER_HANDLE {
            return Err(Error::new(ENOMEM));
        }
        for _ in 0..=MAX_SYNCOBJS_PER_HANDLE {
            self.next_id = self.next_id.wrapping_add(1);
            if self.next_id == 0 {
                self.next_id = 1;
            }
            if !self.entries.contains_key(&self.next_id) {
                return Ok(self.next_id);
            }
        }
        Err(Error::new(ENOMEM))
    }

    pub(crate) fn insert(&mut self, obj: Arc<Syncobj>) -> Result<u32> {
        let id = self.alloc_id()?;
        self.entries.insert(id, obj);
        Ok(id)
    }

    pub(crate) fn get(&self, id: u32) -> Result<Arc<Syncobj>> {
        self.entries
            .get(&id)
            .cloned()
            .ok_or_else(|| Error::new(ENOENT))
    }

    pub(crate) fn remove(&mut self, id: u32) -> Result<()> {
        self.entries
            .remove(&id)
            .map(|_| ())
            .ok_or_else(|| Error::new(ENOENT))
    }
}

struct TimerState {
    deadlines: BinaryHeap<Reverse<Instant>>,
    shutdown: bool,
}

pub(crate) struct WaitTimer {
    shared: Option<Arc<(Mutex<TimerState>, Condvar)>>,
}

impl WaitTimer {
    pub(crate) fn disabled() -> Self {
        Self { shared: None }
    }

    pub(crate) fn new(waker: Waker) -> Self {
        let shared = Arc::new((
            Mutex::new(TimerState {
                deadlines: BinaryHeap::new(),
                shutdown: false,
            }),
            Condvar::new(),
        ));
        let worker = shared.clone();
        let spawned = thread::Builder::new()
            .name("gpu-sync-timer".to_owned())
            .spawn(move || {
                let (lock, cvar) = &*worker;
                let mut guard = lock.lock().unwrap();
                loop {
                    if guard.shutdown {
                        break;
                    }
                    let now = Instant::now();
                    let mut fired = false;
                    while let Some(&Reverse(ts)) = guard.deadlines.peek() {
                        if ts <= now {
                            guard.deadlines.pop();
                            fired = true;
                        } else {
                            break;
                        }
                    }
                    if fired {
                        drop(guard);
                        waker.wake_by_ref();
                        guard = lock.lock().unwrap();
                        continue;
                    }
                    if let Some(&Reverse(next)) = guard.deadlines.peek() {
                        let wait_for = next.saturating_duration_since(Instant::now());
                        if wait_for.is_zero() {
                            continue;
                        }
                        let (next_guard, _) = cvar.wait_timeout(guard, wait_for).unwrap();
                        guard = next_guard;
                    } else {
                        guard = cvar.wait(guard).unwrap();
                    }
                }
            });
        match spawned {
            Ok(_) => Self {
                shared: Some(shared),
            },
            Err(err) => {
                log::warn!("driver-graphics: failed to spawn sync timer thread: {err}");
                Self { shared: None }
            }
        }
    }

    pub(crate) fn schedule(&self, deadline: Instant) {
        let Some(shared) = &self.shared else {
            return;
        };
        let (lock, cvar) = &**shared;
        let mut guard = lock.lock().unwrap();
        if guard.deadlines.len() < 4096 {
            guard.deadlines.push(Reverse(deadline));
            cvar.notify_one();
        }
    }
}

impl Drop for WaitTimer {
    fn drop(&mut self) {
        if let Some(shared) = &self.shared {
            let (lock, cvar) = &**shared;
            let mut guard = lock.lock().unwrap();
            guard.shutdown = true;
            cvar.notify_one();
        }
    }
}

#[derive(Debug)]
pub(crate) struct PendingWait {
    targets: Vec<(Arc<Syncobj>, u64)>,
    wait_all: bool,
    wait_available: bool,
    is_timeline: bool,
    deadline: Option<Instant>,
}

impl PendingWait {
    fn evaluate(&self) -> Option<u32> {
        let check = |obj: &Syncobj, pt: u64| {
            if self.wait_available {
                obj.is_point_available(pt)
            } else {
                obj.is_point_signaled(pt)
            }
        };
        if self.wait_all {
            if self.targets.iter().all(|(obj, pt)| check(obj, *pt)) {
                Some(0)
            } else {
                None
            }
        } else {
            self.targets
                .iter()
                .position(|(obj, pt)| check(obj, *pt))
                .map(|idx| idx as u32)
        }
    }
}

pub(crate) fn poll_pending_wait<T>(
    handle: &mut RenderHandle<T>,
    req_id: Id,
    cmd: u64,
    payload: &mut [u8],
) -> Option<Result<usize>> {
    let pending = handle.sync_waits.get(&req_id)?;
    if let Some(first_signaled) = pending.evaluate() {
        let is_timeline = pending.is_timeline;
        handle.sync_waits.remove(&req_id);
        let res = if is_timeline && cmd == sync::SYNCOBJ_TIMELINE_WAIT {
            DrmSyncobjTimelineWait::with(payload, |mut data| {
                data.set_first_signaled(first_signaled);
                Ok(0)
            })
        } else if !is_timeline && cmd == sync::SYNCOBJ_WAIT {
            DrmSyncobjWait::with(payload, |mut data| {
                data.set_first_signaled(first_signaled);
                Ok(0)
            })
        } else {
            Err(Error::new(EINVAL))
        };
        return Some(res);
    }
    if pending.deadline.is_some_and(|d| Instant::now() >= d) {
        handle.sync_waits.remove(&req_id);
        return Some(Err(Error::new(ETIME)));
    }
    Some(Err(Error::new(EWOULDBLOCK)))
}

pub(crate) fn call_syncobj_ioctl<T>(
    handle: &mut RenderHandle<T>,
    waker: &Waker,
    timer: &WaitTimer,
    req_id: Id,
    cmd: u64,
    payload: &mut [u8],
) -> Result<usize> {
    match cmd {
        sync::SYNCOBJ_CREATE => DrmSyncobjCreate::with(payload, |mut data| {
            let flags = data.flags();
            if flags & !DRM_SYNCOBJ_CREATE_SIGNALED != 0 {
                return Err(Error::new(EINVAL));
            }
            let obj = Syncobj::new(flags & DRM_SYNCOBJ_CREATE_SIGNALED != 0, waker.clone());
            let id = handle.syncobjs.insert(obj)?;
            data.set_handle(id);
            Ok(0)
        }),
        sync::SYNCOBJ_DESTROY => DrmSyncobjDestroy::with(payload, |data| {
            if data.pad() != 0 {
                return Err(Error::new(EINVAL));
            }
            handle.syncobjs.remove(data.handle())?;
            Ok(0)
        }),
        sync::SYNCOBJ_RESET => DrmSyncobjArray::with(payload, |data| {
            if data.pad() != 0 || data.count_handles() > SYNCOBJ_MAX_HANDLES {
                return Err(Error::new(EINVAL));
            }
            let objs: Vec<Arc<Syncobj>> = data
                .handles()
                .iter()
                .map(|&id| handle.syncobjs.get(id))
                .collect::<Result<_>>()?;
            for obj in objs {
                obj.reset();
            }
            Ok(0)
        }),
        sync::SYNCOBJ_SIGNAL => DrmSyncobjArray::with(payload, |data| {
            if data.pad() != 0 || data.count_handles() > SYNCOBJ_MAX_HANDLES {
                return Err(Error::new(EINVAL));
            }
            let objs: Vec<Arc<Syncobj>> = data
                .handles()
                .iter()
                .map(|&id| handle.syncobjs.get(id))
                .collect::<Result<_>>()?;
            for obj in objs {
                obj.signal_binary();
            }
            Ok(0)
        }),
        sync::SYNCOBJ_QUERY => DrmSyncobjQuery::with(payload, |mut data| {
            let flags = data.flags();
            if flags & !DRM_SYNCOBJ_QUERY_FLAGS_LAST_SUBMITTED != 0
                || data.count_handles() > SYNCOBJ_MAX_HANDLES
            {
                return Err(Error::new(EINVAL));
            }
            let last_submitted = flags & DRM_SYNCOBJ_QUERY_FLAGS_LAST_SUBMITTED != 0;
            let objs: Vec<Arc<Syncobj>> = data
                .handles()
                .iter()
                .map(|&id| handle.syncobjs.get(id))
                .collect::<Result<_>>()?;
            let out = data.points_mut();
            for (slot, obj) in out.iter_mut().zip(objs.iter()) {
                *slot = obj.query(last_submitted);
            }
            Ok(0)
        }),
        sync::SYNCOBJ_TIMELINE_SIGNAL => DrmSyncobjTimelineSignal::with(payload, |data| {
            if data.flags() != 0 || data.count_handles() > SYNCOBJ_MAX_HANDLES {
                return Err(Error::new(EINVAL));
            }
            let objs: Vec<Arc<Syncobj>> = data
                .handles()
                .iter()
                .map(|&id| handle.syncobjs.get(id))
                .collect::<Result<_>>()?;
            for (obj, &point) in objs.iter().zip(data.points().iter()) {
                obj.signal_point(point);
            }
            Ok(0)
        }),
        sync::SYNCOBJ_TRANSFER => DrmSyncobjTransfer::with(payload, |data| {
            let allowed = DRM_SYNCOBJ_WAIT_FLAGS_WAIT_FOR_SUBMIT
                | DRM_SYNCOBJ_WAIT_FLAGS_WAIT_AVAILABLE;
            if data.pad() != 0 || data.flags() & !allowed != 0 {
                return Err(Error::new(EINVAL));
            }
            let src = handle.syncobjs.get(data.src_handle())?;
            let dst = handle.syncobjs.get(data.dst_handle())?;
            let fence = src.find_transfer_fence(data.src_point())?;
            dst.attach_fence(data.dst_point(), fence)?;
            Ok(0)
        }),
        sync::SYNCOBJ_WAIT => DrmSyncobjWait::with(payload, |mut data| {
            let allowed = DRM_SYNCOBJ_WAIT_FLAGS_WAIT_ALL
                | DRM_SYNCOBJ_WAIT_FLAGS_WAIT_FOR_SUBMIT
                | DRM_SYNCOBJ_WAIT_FLAGS_WAIT_AVAILABLE
                | DRM_SYNCOBJ_WAIT_FLAGS_WAIT_DEADLINE;
            if data.pad() != 0
                || data.flags() & !allowed != 0
                || data.count_handles() > SYNCOBJ_MAX_HANDLES
            {
                return Err(Error::new(EINVAL));
            }
            if data.count_handles() == 0 {
                return Ok(0);
            }
            let targets: Vec<(Arc<Syncobj>, u64)> = data
                .handles()
                .iter()
                .map(|&id| Ok((handle.syncobjs.get(id)?, 0)))
                .collect::<Result<_>>()?;
            match start_wait(handle, timer, req_id, targets, data.timeout_nsec(), data.flags(), false)? {
                Some(first) => {
                    data.set_first_signaled(first);
                    Ok(0)
                }
                None => Err(Error::new(EWOULDBLOCK)),
            }
        }),
        sync::SYNCOBJ_TIMELINE_WAIT => DrmSyncobjTimelineWait::with(payload, |mut data| {
            let allowed = DRM_SYNCOBJ_WAIT_FLAGS_WAIT_ALL
                | DRM_SYNCOBJ_WAIT_FLAGS_WAIT_FOR_SUBMIT
                | DRM_SYNCOBJ_WAIT_FLAGS_WAIT_AVAILABLE
                | DRM_SYNCOBJ_WAIT_FLAGS_WAIT_DEADLINE;
            if data.pad() != 0
                || data.flags() & !allowed != 0
                || data.count_handles() > SYNCOBJ_MAX_HANDLES
            {
                return Err(Error::new(EINVAL));
            }
            if data.count_handles() == 0 {
                return Ok(0);
            }
            let targets: Vec<(Arc<Syncobj>, u64)> = data
                .handles()
                .iter()
                .zip(data.points().iter())
                .map(|(&id, &pt)| Ok((handle.syncobjs.get(id)?, pt)))
                .collect::<Result<_>>()?;
            match start_wait(handle, timer, req_id, targets, data.timeout_nsec(), data.flags(), true)? {
                Some(first) => {
                    data.set_first_signaled(first);
                    Ok(0)
                }
                None => Err(Error::new(EWOULDBLOCK)),
            }
        }),
        sync::SYNCOBJ_EVENTFD => DrmSyncobjEventfd::with(payload, |data| {
            let allowed = DRM_SYNCOBJ_WAIT_FLAGS_WAIT_AVAILABLE;
            if data.pad() != 0 || data.flags() & !allowed != 0 {
                return Err(Error::new(EINVAL));
            }
            let _ = handle.syncobjs.get(data.handle())?;
            Err(Error::new(EOPNOTSUPP))
        }),
        _ => Err(Error::new(EINVAL)),
    }
}

fn start_wait<T>(
    handle: &mut RenderHandle<T>,
    timer: &WaitTimer,
    req_id: Id,
    targets: Vec<(Arc<Syncobj>, u64)>,
    remaining_ns: i64,
    flags: u32,
    is_timeline: bool,
) -> Result<Option<u32>> {
    let wait_all = flags & DRM_SYNCOBJ_WAIT_FLAGS_WAIT_ALL != 0;
    let wait_for_submit = flags & DRM_SYNCOBJ_WAIT_FLAGS_WAIT_FOR_SUBMIT != 0;
    let wait_available = flags & DRM_SYNCOBJ_WAIT_FLAGS_WAIT_AVAILABLE != 0;

    let deadline = if remaining_ns <= 0 {
        None
    } else if remaining_ns == i64::MAX {
        None
    } else {
        Some(Instant::now() + Duration::from_nanos(remaining_ns as u64))
    };

    let pending = PendingWait {
        targets,
        wait_all,
        wait_available,
        is_timeline,
        deadline,
    };

    if let Some(first) = pending.evaluate() {
        return Ok(Some(first));
    }
    if remaining_ns <= 0 {
        return Err(Error::new(ETIME));
    }
    if !wait_for_submit && !wait_available {
        let missing = if wait_all {
            pending
                .targets
                .iter()
                .any(|(obj, pt)| !obj.is_point_available(*pt))
        } else {
            pending
                .targets
                .iter()
                .all(|(obj, pt)| !obj.is_point_available(*pt))
        };
        if missing {
            return Err(Error::new(EINVAL));
        }
    }
    if handle.sync_waits.len() >= MAX_PENDING_WAITS_PER_HANDLE {
        return Err(Error::new(ENOMEM));
    }
    if let Some(d) = pending.deadline {
        timer.schedule(d);
    }
    handle.sync_waits.insert(req_id, pending);
    Ok(None)
}

pub(crate) fn render_handle_dup<T: GraphicsAdapter>(
    handle: &mut RenderHandle<T>,
    buf: &[u8],
) -> Result<ResourceOpenResult<GraphicsResource<T>>> {
    let Some((obj_handle, flags, point)) = sync::decode_handle_to_fd_dup(buf) else {
        return Err(Error::new(EINVAL));
    };
    let allowed =
        DRM_SYNCOBJ_HANDLE_TO_FD_FLAGS_EXPORT_SYNC_FILE | DRM_SYNCOBJ_HANDLE_TO_FD_FLAGS_TIMELINE;
    if flags & !allowed != 0 {
        return Err(Error::new(EINVAL));
    }
    let export_sync_file = flags & DRM_SYNCOBJ_HANDLE_TO_FD_FLAGS_EXPORT_SYNC_FILE != 0;
    let is_timeline = flags & DRM_SYNCOBJ_HANDLE_TO_FD_FLAGS_TIMELINE != 0;
    if is_timeline && !export_sync_file {
        return Err(Error::new(EINVAL));
    }
    if !is_timeline && point != 0 {
        return Err(Error::new(EINVAL));
    }
    let obj = handle.syncobjs.get(obj_handle)?;
    let resource = if export_sync_file {
        let fence = obj.export_fence(if is_timeline { point } else { 0 })?;
        GraphicsResource::SyncFile(SyncFile::new(b"syncobj", fence))
    } else {
        GraphicsResource::SyncobjFd(SyncobjFd {
            syncobj: obj,
            _adapter: PhantomData,
        })
    };
    Ok(ResourceOpenResult::ThisScheme {
        data: resource,
        flags: NewFdFlags::empty(),
    })
}

pub(crate) fn render_handle_call_with_handles<T: GraphicsAdapter>(
    handle: &mut RenderHandle<T>,
    cmd: u64,
    handles: &mut [GraphicsResource<T>],
    payload: &mut [u8],
) -> Result<usize> {
    if cmd != sync::SYNCOBJ_FD_TO_HANDLE {
        return Err(Error::new(EINVAL));
    }
    DrmSyncobjHandle::with(payload, |mut data| {
        let flags = data.flags();
        let allowed =
            DRM_SYNCOBJ_FD_TO_HANDLE_FLAGS_IMPORT_SYNC_FILE | DRM_SYNCOBJ_FD_TO_HANDLE_FLAGS_TIMELINE;
        if data.pad() != 0 || flags & !allowed != 0 {
            return Err(Error::new(EINVAL));
        }
        let import_sync_file = flags & DRM_SYNCOBJ_FD_TO_HANDLE_FLAGS_IMPORT_SYNC_FILE != 0;
        let is_timeline = flags & DRM_SYNCOBJ_FD_TO_HANDLE_FLAGS_TIMELINE != 0;
        if is_timeline && !import_sync_file {
            return Err(Error::new(EINVAL));
        }
        if !is_timeline && data.point() != 0 {
            return Err(Error::new(EINVAL));
        }
        if import_sync_file {
            let sf = match handles {
                [GraphicsResource::SyncFile(sf)] => sf,
                _ => return Err(Error::new(EINVAL)),
            };
            let dst = handle.syncobjs.get(data.handle())?;
            dst.attach_fence(if is_timeline { data.point() } else { 0 }, sf.fence.clone())?;
            Ok(0)
        } else {
            let obj = match handles {
                [GraphicsResource::SyncobjFd(fd)] => fd.syncobj.clone(),
                _ => return Err(Error::new(EINVAL)),
            };
            let id = handle.syncobjs.insert(obj)?;
            data.set_handle(id);
            Ok(0)
        }
    })
}

pub(crate) struct SyncobjFd<T> {
    pub(crate) syncobj: Arc<Syncobj>,
    _adapter: PhantomData<T>,
}

impl<T> Debug for SyncobjFd<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncobjFd").finish_non_exhaustive()
    }
}

impl<T: GraphicsAdapter> ResourceSync for SyncobjFd<T> {
    type SchemeData = GraphicsSchemeData<T>;
    type ResourceEnum = GraphicsResource<T>;
}

pub(crate) struct SyncFile<T> {
    name: [u8; 32],
    fence: SyncFence,
    requested_read: bool,
    notified_read: bool,
    next_merge_token: u64,
    pending_merges: HashMap<u64, ([u8; 32], SyncFence)>,
    _adapter: PhantomData<T>,
}

impl<T> Debug for SyncFile<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncFile")
            .field("signaled", &self.fence.is_signaled())
            .finish_non_exhaustive()
    }
}

impl<T> SyncFile<T> {
    pub(crate) fn new(name: &[u8], fence: SyncFence) -> Self {
        Self {
            name: copy_name_32(name),
            fence,
            requested_read: false,
            notified_read: false,
            next_merge_token: 0,
            pending_merges: HashMap::new(),
            _adapter: PhantomData,
        }
    }

    pub(crate) fn fence(&self) -> &SyncFence {
        &self.fence
    }

    pub(crate) fn should_post_event(&mut self) -> bool {
        if self.requested_read && !self.notified_read && self.fence.is_signaled() {
            self.notified_read = true;
            true
        } else {
            false
        }
    }
}

impl<T: GraphicsAdapter> ResourceSync for SyncFile<T> {
    type SchemeData = GraphicsSchemeData<T>;
    type ResourceEnum = GraphicsResource<T>;

    fn fevent(
        &mut self,
        _scheme_data: &mut Self::SchemeData,
        flags: EventFlags,
    ) -> Result<EventFlags> {
        self.requested_read = flags.contains(EventFlags::EVENT_READ);
        if !self.requested_read {
            self.notified_read = false;
            return Ok(EventFlags::empty());
        }
        if self.fence.is_signaled() {
            Ok(EventFlags::EVENT_READ)
        } else {
            self.notified_read = false;
            Ok(EventFlags::empty())
        }
    }

    fn call(
        &mut self,
        _scheme_data: &mut Self::SchemeData,
        payload: &mut [u8],
        metadata: &[u64],
        _ctx: &CallerCtx,
    ) -> Result<usize> {
        match metadata[0] {
            sync::SYNC_FILE_INFO => SyncFileInfo::with(payload, |mut data| {
                if data.flags() != 0 || data.pad() != 0 || data.num_fences() > SYNC_MAX_FENCES {
                    return Err(Error::new(EINVAL));
                }
                let actual = self.fence.leaves.len() as u32;
                let requested = data.num_fences();
                if requested > 0 && requested < actual {
                    return Err(Error::new(EINVAL));
                }
                data.set_name(bytes_to_c_char_32(&self.name));
                data.set_status(self.fence.status());
                data.set_flags(0);
                if requested > 0 {
                    let driver_name = bytes_to_c_char_32(&copy_name_32(b"virtio_gpu"));
                    let out = data.sync_fence_info_mut();
                    for (dst, leaf) in out.iter_mut().zip(self.fence.leaves.iter()) {
                        let (status, timestamp_ns) = leaf.snapshot();
                        dst.obj_name = bytes_to_c_char_32(&leaf.obj_name);
                        dst.driver_name = driver_name;
                        dst.status = status;
                        dst.flags = 0;
                        dst.timestamp_ns = timestamp_ns;
                    }
                }
                data.set_num_fences(actual);
                Ok(0)
            }),
            sync::SYNC_SET_DEADLINE => SyncSetDeadline::with(payload, |data| {
                if data.pad() != 0 {
                    return Err(Error::new(EINVAL));
                }
                Ok(0)
            }),
            _ => Err(Error::new(EINVAL)),
        }
    }

    fn call_with_handles(
        &mut self,
        _scheme_data: &mut Self::SchemeData,
        handles: &mut [Self::ResourceEnum],
        payload: &mut [u8],
        metadata: &[u64],
        _ctx: &CallerCtx,
    ) -> Result<usize> {
        if metadata[0] != sync::SYNC_MERGE {
            return Err(Error::new(EINVAL));
        }
        let Some((name, flags, pad)) = sync::decode_sync_merge_call(payload) else {
            return Err(Error::new(EINVAL));
        };
        if flags != 0 || pad != 0 {
            return Err(Error::new(EINVAL));
        }
        let other = match handles {
            [GraphicsResource::SyncFile(other)] => other,
            _ => return Err(Error::new(EINVAL)),
        };
        if self.pending_merges.len() >= MAX_PENDING_MERGES {
            return Err(Error::new(ENOMEM));
        }
        let merged = self.fence.merge(&other.fence)?;
        self.next_merge_token = self.next_merge_token.wrapping_add(1).max(1);
        let token = self.next_merge_token;
        self.pending_merges.insert(token, (name, merged));
        Ok(token as usize)
    }

    fn dup(
        &mut self,
        _scheme_data: &mut Self::SchemeData,
        buf: &[u8],
        _ctx: &CallerCtx,
    ) -> Result<ResourceOpenResult<Self::ResourceEnum>> {
        let Some(token) = sync::decode_sync_merge_dup(buf) else {
            return Err(Error::new(EINVAL));
        };
        let Some((name, fence)) = self.pending_merges.remove(&token) else {
            return Err(Error::new(ENOENT));
        };
        Ok(ResourceOpenResult::ThisScheme {
            data: GraphicsResource::SyncFile(SyncFile::new(&name, fence)),
            flags: NewFdFlags::empty(),
        })
    }
}
