// SPDX-License-Identifier: GPL-2.0-only AND MIT
use std::cell::RefCell;
use std::fs::File;
use std::rc::Rc;
use std::sync::Arc;

use executor::{Hardware, LocalExecutor};

use super::{CmdId, CqId, Nvme, NvmeCmd, NvmeComp, SqId};

pub struct NvmeHw;

impl Hardware for NvmeHw {
    type Iv = u16;
    type Sqe = NvmeCmd;
    type Cqe = NvmeComp;
    type CmdId = CmdId;
    type CqId = CqId;
    type SqId = SqId;
    type GlobalCtxt = Arc<Nvme>;

    fn mask_vector(ctxt: &Arc<Nvme>, iv: Self::Iv) {
        ctxt.set_vector_masked(iv, true)
    }
    fn unmask_vector(ctxt: &Arc<Nvme>, iv: Self::Iv) {
        ctxt.set_vector_masked(iv, false)
    }
    fn set_sqe_cmdid(sqe: &mut NvmeCmd, id: CmdId) {
        sqe.cid = id;
    }
    fn get_cqe_cmdid(cqe: &Self::Cqe) -> Self::CmdId {
        cqe.cid
    }
    fn vtable() -> &'static std::task::RawWakerVTable {
        &VTABLE
    }
    fn current() -> std::rc::Rc<executor::LocalExecutor<Self>> {
        THE_EXECUTOR.with(|exec| Rc::clone(exec.borrow().as_ref().unwrap()))
    }
    fn push_sqe(
        nvme: &Arc<Nvme>,
        sq_id: Self::SqId,
        success: impl FnOnce(Self::CmdId) -> Self::Sqe,
        fail: impl FnOnce(),
    ) -> Option<(Self::CqId, Self::CmdId)> {
        let ctxt = nvme.cur_thread_ctxt();
        let ctxt = ctxt.lock();

        ctxt.push_sqe(sq_id, success, fail)
    }
    fn submit(nvme: &Arc<Nvme>, sq_id: Self::SqId) {
        let ctxt = nvme.cur_thread_ctxt();
        let ctxt = ctxt.lock();

        ctxt.submit(sq_id)
    }
    fn poll_cqes(nvme: &Arc<Nvme>, mut handle: impl FnMut(Self::CqId, Self::Cqe)) {
        let ctxt = nvme.cur_thread_ctxt();
        let ctxt = ctxt.lock();

        for (sq_cq_id, (sq, cq)) in ctxt.queues.borrow_mut().iter_mut() {
            while let Some(cqe) = cq.complete() {
                let sq_head = cqe.sq_head;
                let cid = cqe.cid;
                if usize::from(sq_head) < sq.data.len() {
                    sq.head = sq_head;
                } else {
                    log::error!(
                        "nvmed: ignoring out-of-range sq_head {} on queue {} (len {})",
                        sq_head,
                        sq_cq_id,
                        sq.data.len()
                    );
                }
                if !sq.has_cmd_id(cid) {
                    log::error!(
                        "nvmed: ignoring completion for unallocated cid {} on queue {}",
                        cid,
                        sq_cq_id
                    );
                    continue;
                }
                handle(*sq_cq_id, cqe);
                let released = sq.release_cmd_id(cid);
                assert!(released);
            }

            cq.kick();
        }
    }
    fn sq_cq(_ctxt: &Arc<Nvme>, id: Self::CqId) -> Self::SqId {
        id
    }
}

static VTABLE: std::task::RawWakerVTable = executor::vtable::<NvmeHw>();

thread_local! {
    static THE_EXECUTOR: RefCell<Option<Rc<LocalExecutor<NvmeHw>>>> = const { RefCell::new(None) };
}

pub type NvmeExecutor = LocalExecutor<NvmeHw>;

pub fn init(nvme: Arc<Nvme>, iv: u16, intx: bool, irq_handle: File) -> Rc<LocalExecutor<NvmeHw>> {
    let this = Rc::new(executor::init_raw(nvme, iv, intx, irq_handle));
    THE_EXECUTOR.with(|exec| *exec.borrow_mut() = Some(Rc::clone(&this)));
    this
}
