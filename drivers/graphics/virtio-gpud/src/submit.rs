// SPDX-License-Identifier: GPL-2.0-only
use std::collections::VecDeque;
use std::future::{Future, poll_fn};
use std::pin::Pin;
use std::slice;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use common::dma::Dma;
use driver_graphics::FenceLeaf;
use syscall::{EBUSY, EINVAL, EIO, ENOMEM};
use virtio_core::spec::{Buffer, ChainBuilder, DescriptorFlags};
use virtio_core::transport::{Error, PendingRequest, Queue};

use crate::render::ResponseHeader;
use crate::{CommandTy, ControlHeader, VIRTIO_GPU_FLAG_FENCE, VIRTIO_GPU_FLAG_INFO_RING_IDX};

const MAX_IN_FLIGHT_SUBMITS: usize = 256;

#[repr(C)]
struct CmdSubmit {
    header: ControlHeader,
    size: u32,
    padding: u32,
}

const _: () = assert!(size_of::<CmdSubmit>() == 32);

struct Unplaced {
    chain: Vec<Buffer>,
    fence: Arc<FenceLeaf>,
    fence_id: u64,
    ctx_id: u32,
}

pub struct SubmitQueue {
    control_queue: Arc<Queue>,
    waker: Waker,
    next_fence_id: u64,
    unplaced: VecDeque<Unplaced>,
    pending: Vec<Pin<Box<dyn Future<Output = ()>>>>,
}

fn errno(error: Error) -> syscall::Error {
    syscall::Error::new(match error {
        Error::SyscallError(error) => error.errno(),
        _ => EIO,
    })
}

impl SubmitQueue {
    pub fn new(control_queue: Arc<Queue>, waker: Waker) -> Self {
        Self {
            control_queue,
            waker,
            next_fence_id: 1,
            unplaced: VecDeque::new(),
            pending: Vec::new(),
        }
    }

    fn alloc_fence_id(&mut self) -> u64 {
        let id = self.next_fence_id;
        self.next_fence_id = self.next_fence_id.checked_add(1).unwrap_or(1);
        id
    }

    pub fn submit(
        &mut self,
        ctx_id: u32,
        ring_idx: Option<u8>,
        command: &[u8],
    ) -> syscall::Result<Arc<FenceLeaf>> {
        if self.pending.len() + self.unplaced.len() >= MAX_IN_FLIGHT_SUBMITS {
            return Err(syscall::Error::new(EBUSY));
        }
        let cmd_size = u32::try_from(command.len()).map_err(|_| syscall::Error::new(EINVAL))?;
        let fence_id = self.alloc_fence_id();
        let mut flags = VIRTIO_GPU_FLAG_FENCE;
        let ring_index = match ring_idx {
            Some(idx) => {
                flags |= VIRTIO_GPU_FLAG_INFO_RING_IDX;
                idx
            }
            None => 0,
        };
        let header = CmdSubmit {
            header: ControlHeader {
                ty: CommandTy::Submit3d,
                flags,
                fence_id,
                ctx_id,
                ring_index,
                ..Default::default()
            },
            size: cmd_size,
            padding: 0,
        };
        let total = size_of::<CmdSubmit>()
            .checked_add(command.len())
            .ok_or_else(|| syscall::Error::new(EINVAL))?;
        let mut request = unsafe {
            Dma::<[u8]>::zeroed_slice(total)
                .map_err(Error::from)
                .map_err(errno)?
                .assume_init()
        };
        let header_bytes = unsafe {
            slice::from_raw_parts((&raw const header).cast::<u8>(), size_of::<CmdSubmit>())
        };
        request[..size_of::<CmdSubmit>()].copy_from_slice(header_bytes);
        request[size_of::<CmdSubmit>()..].copy_from_slice(command);
        let response = Dma::new(ResponseHeader::default())
            .map_err(Error::from)
            .map_err(errno)?;
        let chain = ChainBuilder::new()
            .chain(Buffer::new(request))
            .chain(Buffer::new(response).flags(DescriptorFlags::WRITE_ONLY))
            .build();

        let fence = FenceLeaf::new_unsignaled(b"controlq", self.waker.clone());
        self.unplaced.push_back(Unplaced {
            chain,
            fence: fence.clone(),
            fence_id,
            ctx_id,
        });
        let waker = self.waker.clone();
        self.place(&waker);
        Ok(fence)
    }

    fn place(&mut self, waker: &Waker) {
        while let Some(next) = self.unplaced.pop_front() {
            match self.control_queue.send_now(next.chain, waker) {
                Ok(request) => {
                    let mut fut = Box::pin(complete(request, next.fence, next.fence_id, next.ctx_id));
                    let mut cx = Context::from_waker(&self.waker);
                    if fut.as_mut().poll(&mut cx).is_pending() {
                        self.pending.push(fut);
                    }
                }
                Err(chain) => {
                    self.unplaced.push_front(Unplaced { chain, ..next });
                    break;
                }
            }
        }
    }

    pub fn flush(&mut self) {
        if self.unplaced.is_empty() {
            return;
        }
        futures::executor::block_on(poll_fn(|cx| {
            self.place(cx.waker());
            if self.unplaced.is_empty() {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }));
    }

    pub fn poll_pending(&mut self, cx: &mut Context<'_>) {
        self.place(cx.waker());
        self.pending.retain_mut(|fut| fut.as_mut().poll(cx).is_pending());
    }
}

async fn complete(request: PendingRequest, done: Arc<FenceLeaf>, fence_id: u64, ctx_id: u32) {
    let (_, [_, response]) = request.await.into_parts();
    let response: Dma<ResponseHeader> = response.into_inner();
    if response.ty == CommandTy::RespOkNodata as u32 {
        if (response.flags & VIRTIO_GPU_FLAG_FENCE) != 0 && response.fence_id != fence_id {
            log::warn!(
                "virtio-gpu: submit ctx {ctx_id}: expected fence {fence_id}, got {}",
                response.fence_id
            );
        }
        done.signal();
    } else {
        log::warn!(
            "virtio-gpu: submit ctx {ctx_id} fence {fence_id}: device answered {:#x}",
            response.ty
        );
        done.signal_error(if response.ty == CommandTy::RespErrOutOfMemory as u32 {
            ENOMEM as i32
        } else {
            EIO as i32
        });
    }
}
