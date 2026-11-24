// SPDX-License-Identifier: GPL-2.0-only AND MIT
use std::collections::HashMap;
use std::{env, time};

use anyhow::{Context, Result};
use inputd::ProducerHandle;
use rehid::report_desc::ReportTy;
use rehid::report_handler::ReportHandler;
use xhcid_interface::PortId;

use crate::device::profiles::xbox360;
use crate::device::{interface, report_reader};

mod device;
mod input;

fn main() -> Result<()> {
    let mut args = env::args().skip(1);

    const USAGE: &str = "usbhidd <scheme> <port> <interface>";

    let scheme = args.next().expect(USAGE);
    let port = args
        .next()
        .expect(USAGE)
        .parse::<PortId>()
        .expect("Expected port ID");
    let interface_num = args
        .next()
        .expect(USAGE)
        .parse::<u8>()
        .expect("Expected integer as input of interface");
    let mut report_desc_override = None;
    let override_name_opt = args.next();
    if let Some(override_name) = &override_name_opt {
        for (name, bytes) in xbox360::OVERRIDES.iter() {
            if name == override_name {
                report_desc_override = Some(bytes.to_vec());
                break;
            }
        }
    }

    let name = format!("{}_{}_{}_hid", scheme, port, interface_num);
    common::setup_logging(
        "usb",
        "usbhid",
        &name,
        common::output_level(),
        common::file_level(),
    );

    log::info!(
        "USB HID driver spawned with scheme `{}`, port {}, interface {}",
        scheme,
        port,
        interface_num
    );

    let (handle, desc) = interface::open(scheme, port)?;
    let (conf_desc, (if_desc, endp_int_in_opt, endp_int_out_opt, hid_desc_opt)) =
        interface::select(&desc, interface_num)?;
    interface::configure(&handle, &conf_desc, &if_desc, interface_num)?;

    let report_desc_bytes = if let Some(hid_desc) = hid_desc_opt {
        interface::get_report_descriptor(&handle, interface_num, hid_desc)?
    } else {
        report_desc_override.expect("failed to find report descriptor")
    };

    let mut handler =
        ReportHandler::new(&report_desc_bytes).expect("failed to parse report descriptor");

    let report_len = report_reader::report_len(endp_int_in_opt, &handler);

    let report_collections = input::collect_report_collections(&handler);

    let mut display = ProducerHandle::new().context("Failed to open input socket")?;

    xbox360::initialize_led(&handle, &override_name_opt, endp_int_out_opt);

    let mut endpoint_opt = match endp_int_in_opt {
        Some((endp_num, _endp_desc)) => match handle.open_endpoint(endp_num as u8) {
            Ok(ok) => Some(ok),
            Err(err) => {
                log::warn!("failed to open endpoint {endp_num}: {err}");
                None
            }
        },
        None => None,
    };

    let mut report_buffer = vec![0u8; report_len];
    let report_ty = ReportTy::Input;
    let report_id = 0;

    let mut left_shift = false;
    let mut right_shift = false;
    let mut last_mouse_pos = (0, 0);
    let mut last_buttons = [false, false, false];
    let mut gamepad_state = HashMap::new();
    libredox::call::setrens(0, 0).context("failed to enter null namespace")?;
    loop {
        let timer = time::Instant::now();
        report_reader::read(
            &timer,
            &mut endpoint_opt,
            &handle,
            report_ty,
            report_id,
            interface_num,
            &mut report_buffer,
        )?;

        input::process_report(
            &mut handler,
            &report_buffer,
            &report_collections,
            &mut display,
            &mut left_shift,
            &mut right_shift,
            &mut last_mouse_pos,
            &mut last_buttons,
            &mut gamepad_state,
        );

        // log::trace!("took {}ms", timer.elapsed().as_millis())
    }
}
