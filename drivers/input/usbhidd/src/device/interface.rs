// SPDX-License-Identifier: GPL-2.0-only AND MIT
use anyhow::{Context, Result};
use rehid::report_desc::REPORT_DESC_TY;
use xhcid_interface::{
    ConfDesc, ConfigureEndpointsReq, DevDesc, EndpDesc, EndpDirection, EndpointTy, IfDesc, PortId,
    PortReqRecipient, XhciClientHandle,
};

use super::descriptors::HidDescriptor;
use super::requests;

type SelectedInterface = (
    IfDesc,
    Option<(usize, EndpDesc)>,
    Option<(usize, EndpDesc)>,
    Option<HidDescriptor>,
);

pub(crate) fn open(scheme: String, port: PortId) -> Result<(XhciClientHandle, DevDesc)> {
    let handle = XhciClientHandle::new(scheme, port).context("Failed to open XhciClientHandle")?;
    let desc: DevDesc = handle
        .get_standard_descs()
        .context("Failed to get standard descriptors")?;

    log::info!(
        "USB HID driver: {:?} serial {:?}",
        desc.product_str.as_ref().map(|s| s.as_str()).unwrap_or(""),
        desc.serial_str.as_ref().map(|s| s.as_str()).unwrap_or(""),
    );

    log::debug!("{:X?}", desc);

    Ok((handle, desc))
}

pub(crate) fn select(desc: &DevDesc, interface_num: u8) -> Result<(ConfDesc, SelectedInterface)> {
    let mut endp_count = 0;
    desc.config_descs
        .iter()
        .find_map(|conf_desc| {
            let if_desc = conf_desc.interface_descs.iter().find_map(|if_desc| {
                if if_desc.number == interface_num {
                    let mut endp_int_in_opt = None;
                    let mut endp_int_out_opt = None;
                    for endp_desc in if_desc.endpoints.iter() {
                        endp_count += 1;
                        match (endp_desc.ty(), endp_desc.direction()) {
                            (EndpointTy::Interrupt, EndpDirection::In)
                                if endp_int_in_opt.is_none() =>
                            {
                                log::warn!(
                                    "using endpoint 0x{:x} {:?} {:?}",
                                    endp_desc.address,
                                    endp_desc.ty(),
                                    endp_desc.direction()
                                );
                                endp_int_in_opt = Some((endp_count, endp_desc.clone()));
                            }
                            (EndpointTy::Interrupt, EndpDirection::Out)
                                if endp_int_out_opt.is_none() =>
                            {
                                log::warn!(
                                    "using endpoint 0x{:x} {:?} {:?}",
                                    endp_desc.address,
                                    endp_desc.ty(),
                                    endp_desc.direction()
                                );
                                endp_int_out_opt = Some((endp_count, endp_desc.clone()));
                            }
                            _ => {
                                log::warn!(
                                    "ignoring endpoint 0x{:x} {:?} {:?}",
                                    endp_desc.address,
                                    endp_desc.ty(),
                                    endp_desc.direction()
                                );
                            }
                        }
                    }
                    let hid_desc_opt = if_desc.unknown_descs.iter().find_map(|unknown_desc| {
                        //TODO: should we do any filtering?
                        HidDescriptor::from_bytes(&unknown_desc.all_bytes).ok()
                    });
                    Some((
                        if_desc.clone(),
                        endp_int_in_opt,
                        endp_int_out_opt,
                        hid_desc_opt,
                    ))
                } else {
                    endp_count += if_desc.endpoints.len();
                    None
                }
            })?;
            Some((conf_desc.clone(), if_desc))
        })
        .context("Failed to find suitable configuration")
}

pub(crate) fn configure(
    handle: &XhciClientHandle,
    conf_desc: &ConfDesc,
    if_desc: &IfDesc,
    interface_num: u8,
) -> Result<()> {
    handle
        .configure_endpoints(&ConfigureEndpointsReq {
            config_desc: conf_desc.configuration_value,
            interface_desc: Some(interface_num),
            alternate_setting: Some(if_desc.alternate_setting),
            hub_ports: None,
        })
        .context("Failed to configure endpoints")?;

    //TODO: do we need to set protocol to report? It fails for mice.

    // Only set idle on actual HID devices (this driver also supports mapping some non-HID devices)
    if if_desc.class == 3 {
        //TODO: dynamically create good values, fix xhcid so it does not block on each request
        // This sets all reports to a duration of 4ms
        if let Err(err) = requests::set_idle(handle, 1, 0, u16::from(interface_num)) {
            log::warn!("failed to set idle: {}", err);
        }
    }

    Ok(())
}

pub(crate) fn get_report_descriptor(
    handle: &XhciClientHandle,
    interface_num: u8,
    hid_desc: HidDescriptor,
) -> Result<Vec<u8>> {
    let report_desc_len = hid_desc.get_report_desc()?.desc_len;

    let mut report_desc_bytes = vec![0u8; report_desc_len as usize];
    handle
        .get_descriptor(
            PortReqRecipient::Interface,
            REPORT_DESC_TY,
            0,
            //TODO: should this be an index into interface_descs?
            u16::from(interface_num),
            &mut report_desc_bytes,
        )
        .context("Failed to retrieve report descriptor")?;

    Ok(report_desc_bytes)
}
