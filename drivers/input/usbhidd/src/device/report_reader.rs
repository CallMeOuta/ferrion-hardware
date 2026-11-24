use std::{thread, time};

use anyhow::{Context, Result};
use rehid::report_desc::ReportTy;
use rehid::report_handler::ReportHandler;
use xhcid_interface::{EndpDesc, XhciClientHandle, XhciEndpHandle};

use super::requests;

pub(crate) fn report_len(
    endp_int_in_opt: Option<(usize, EndpDesc)>,
    handler: &ReportHandler,
) -> usize {
    match endp_int_in_opt {
        Some((_endp_num, endp_desc)) => endp_desc.max_packet_size as usize,
        None => handler.total_byte_length as usize,
    }
}

pub(crate) fn read(
    timer: &time::Instant,
    endpoint_opt: &mut Option<XhciEndpHandle>,
    handle: &XhciClientHandle,
    report_ty: ReportTy,
    report_id: u8,
    interface_num: u8,
    report_buffer: &mut [u8],
) -> Result<()> {
    //TODO: get frequency from device
    //TODO: use sleeps when accuracy is better: thread::sleep(time::Duration::from_millis(10));
    while timer.elapsed() < time::Duration::from_millis(1) {
        thread::yield_now();
    }

    if let Some(endpoint) = endpoint_opt {
        // interrupt transfer
        endpoint
            .transfer_read(report_buffer)
            .context("failed to get report")?;
    } else {
        // control transfer
        requests::get_report(
            handle,
            report_ty,
            report_id,
            //TODO: should this be an index into interface_descs?
            u16::from(interface_num),
            report_buffer,
        )
        .context("failed to get report")?;
    }

    Ok(())
}
