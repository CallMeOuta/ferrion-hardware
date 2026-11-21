use std::sync::Arc;

use common::dma::Dma;
use driver_network::NetworkAdapter;
use virtio_core::spec::{Buffer, ChainBuilder, DescriptorFlags};
use virtio_core::transport::Queue;

use crate::{VirtHeader, MAX_BUFFER_LEN};

pub struct VirtioNet {
    mac_address: [u8; 6],

    /// Reciever Queue.
    rx: Arc<Queue>,

    /// Transmiter Queue.
    tx: Arc<Queue>,

    /// Received frames dropped because they did not fit the reader's buffer.
    oversized_frames: u64,
}

impl VirtioNet {
    pub fn new(mac_address: [u8; 6], rx: Arc<Queue>, tx: Arc<Queue>) -> Self {
        // Populate all of the `rx_queue` with buffers to maximize performence.
        for _ in 0..rx.descriptor_len() {
            let buffer = unsafe {
                Dma::<[u8]>::zeroed_slice(MAX_BUFFER_LEN)
                    .unwrap()
                    .assume_init()
            };

            let chain = ChainBuilder::new()
                .chain(Buffer::new(buffer).flags(DescriptorFlags::WRITE_ONLY))
                .build();

            rx.post(chain);
        }

        Self {
            mac_address,
            rx,
            tx,
            oversized_frames: 0,
        }
    }

    /// Returns the number of bytes read. Returns `0` if the operation would block.
    ///
    /// A frame larger than `target` is dropped and counted, and the next one is read.
    fn try_recv(&mut self, target: &mut [u8]) -> usize {
        let header_size = size_of::<VirtHeader>();

        while let Some(used) = self.rx.pop_completed() {
            let copied = {
                // XXX: The header and packet are added as one output descriptor to the transmit queue,
                //      and the device is notified of the new entry (see 5.1.5 Device Initialization).
                let buffer = used.buffers[0].owner::<Dma<[u8]>>().unwrap();
                // TODO: Check the header.
                let written = used.written as usize;
                match buffer.get(header_size..written) {
                    Some(packet) if !packet.is_empty() && packet.len() <= target.len() => {
                        target[..packet.len()].copy_from_slice(packet);
                        packet.len()
                    }
                    Some(packet) if packet.len() > target.len() => {
                        self.oversized_frames += 1;
                        if self.oversized_frames.is_power_of_two() {
                            log::warn!(
                                "virtio-net: dropped {} received frames larger than the {}-byte read buffer",
                                self.oversized_frames,
                                target.len()
                            );
                        }
                        0
                    }
                    _ => {
                        log::warn!("virtio-net: dropped a received buffer of {written} bytes");
                        0
                    }
                }
            };

            // Hand the buffer back to the device for the next packet.
            self.rx.post(used.buffers);
            if copied != 0 {
                return copied;
            }
        }

        // The read would block.
        0
    }
}

impl NetworkAdapter for VirtioNet {
    fn mac_address(&mut self) -> [u8; 6] {
        self.mac_address
    }

    fn available_for_read(&mut self) -> usize {
        self.rx.completed_len()
    }

    fn read_packet(&mut self, buf: &mut [u8]) -> syscall::Result<Option<usize>> {
        let bytes = self.try_recv(buf);

        if bytes != 0 {
            // We read some bytes.
            Ok(Some(bytes))
        } else {
            Ok(None)
        }
    }

    fn write_packet(&mut self, buffer: &[u8]) -> syscall::Result<usize> {
        let header = unsafe { Dma::<VirtHeader>::zeroed()?.assume_init() };

        let mut payload = unsafe { Dma::<[u8]>::zeroed_slice(buffer.len())?.assume_init() };
        payload.copy_from_slice(buffer);

        let chain = ChainBuilder::new()
            .chain(Buffer::new(header))
            .chain(Buffer::new(payload))
            .build();

        futures::executor::block_on(self.tx.send(chain));
        Ok(buffer.len())
    }
}
