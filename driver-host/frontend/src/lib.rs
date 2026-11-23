// SPDX-License-Identifier: GPL-2.0-only
mod adapter;
mod backend;

use std::fs::File;
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd};
use std::time::Instant;

use driver_graphics::{GraphicsAdapter, GraphicsScheme};
use event::{EventQueue, user_data};
use pcid_interface::PciFunctionHandle;

use crate::adapter::{DrmFile, LinuxAdapter};

const SCHEME_NAME: &str = "display.virtio-gpu";
const HOTPLUG_DRAIN: usize = 64;

unsafe extern "C" {
    fn ferrion_linux_start() -> i32;
    fn ferrion_pci_attach() -> i32;
    fn ferrion_drm_open() -> *mut core::ffi::c_void;
}

#[unsafe(no_mangle)]
pub extern "C" fn ferrion_driver_host_main() -> ! {
    pcid_interface::pci_daemon(run)
}

fn run(daemon: daemon::Daemon, handle: PciFunctionHandle) -> ! {
    let started = Instant::now();
    common::setup_logging(
        "graphics",
        "pci",
        "ferrion-driver-host",
        common::output_level(),
        common::file_level(),
    );
    match serve(daemon, handle, started) {
        Ok(()) => std::process::exit(0),
        Err(error) => {
            log::error!("ferrion-driver-host: {error}");
            std::process::exit(1);
        }
    }
}

fn serve(daemon: daemon::Daemon, handle: PciFunctionHandle, started: Instant) -> Result<(), String> {
    let unique = format!("pci:{}", handle.config().func.addr);
    let hotplug = backend::init(handle)?;
    if unsafe { ferrion_linux_start() } != 0 {
        return Err("Linux's initcalls failed".into());
    }
    let ret = unsafe { ferrion_pci_attach() };
    if ret != 0 {
        return Err(format!("no Linux driver attached to the device: {ret}"));
    }
    let file = unsafe { ferrion_drm_open() };
    if file.is_null() {
        return Err("the DRM device did not open".into());
    }
    let adapter = LinuxAdapter::new(DrmFile::new(file), unique)?;

    let event_queue: EventQueue<Source> =
        EventQueue::new().map_err(|e| format!("event queue: {e}"))?;
    let mut scheme = GraphicsScheme::new(adapter, SCHEME_NAME.to_owned(), false);
    daemon.ready();
    log::info!(
        "ferrion-driver-host: {SCHEME_NAME} ready {} ms after start",
        started.elapsed().as_millis()
    );

    user_data! {
        enum Source {
            Scheme,
            Hotplug,
        }
    }

    let mut hotplug = unsafe { File::from_raw_fd(hotplug) };
    event_queue
        .subscribe(scheme.event_handle().raw(), Source::Scheme, event::EventFlags::READ)
        .map_err(|e| format!("subscribe: {e}"))?;
    event_queue
        .subscribe(hotplug.as_raw_fd() as usize, Source::Hotplug, event::EventFlags::READ)
        .map_err(|e| format!("subscribe: {e}"))?;

    for event in [Source::Scheme].into_iter().chain(
        event_queue.map(|e| e.expect("ferrion-driver-host: event queue failed").user_data),
    ) {
        match event {
            Source::Scheme => scheme
                .tick()
                .map_err(|e| format!("scheme: {e}"))?,
            Source::Hotplug => {
                let mut bytes = [0u8; HOTPLUG_DRAIN];
                while let Ok(n) = hotplug.read(&mut bytes) {
                    if n < bytes.len() {
                        break;
                    }
                }
                let (adapter, objects) = scheme.adapter_and_kms_objects_mut();
                let ids = objects.connector_ids().to_vec();
                for id in &ids {
                    if let Err(error) = adapter.probe_connector(objects, *id) {
                        log::warn!("ferrion-driver-host: connector probe failed: {error}");
                    }
                }
                for id in ids {
                    scheme.notify_connector_hotplug(id);
                }
            }
        }
    }
    Ok(())
}
