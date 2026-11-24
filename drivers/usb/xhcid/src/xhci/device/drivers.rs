// SPDX-License-Identifier: GPL-2.0-only AND MIT
use std::collections::BTreeMap;
use std::convert::TryFrom;
use std::process;

use log::{debug, info, trace};
use serde::Deserialize;
use syscall::error::{Error, Result, EBADF, EBADMSG, ENOENT};

use crate::xhci::{PortId, Xhci};

#[derive(Deserialize)]
pub(in crate::xhci) struct DriverConfig {
    name: String,
    class: u8,
    subclass: i16, // The subclass may be meaningless for some drivers, hence negative values (and values above 255) mean "undefined".
    protocol: Option<u8>,
    ids: Option<BTreeMap<String, Vec<u16>>>,
    command: Vec<String>,
}
impl DriverConfig {
    fn subclass(&self) -> Option<u8> {
        u8::try_from(self.subclass).ok()
    }
}
#[derive(Deserialize)]
pub(in crate::xhci) struct DriversConfig {
    pub(in crate::xhci) drivers: Vec<DriverConfig>,
}

impl<const N: usize> Xhci<N> {
    pub(super) fn spawn_drivers(&self, port: PortId) -> Result<()> {
        // TODO: There should probably be a way to select alternate interfaces, and not just the
        // first one.
        // TODO: Now that there are some good error crates, I don't think errno.h error codes are
        // suitable here.

        let ps = self.port_states.get(&port).unwrap();
        trace!("Spawning driver on port: {}", port);

        //TODO: support choosing config?
        let dev_desc = ps.dev_desc.as_ref().ok_or_else(|| {
            log::warn!("Missing device descriptor");
            Error::new(EBADF)
        })?;
        let config_desc = dev_desc.config_descs.first().ok_or_else(|| {
            log::warn!("Missing config descriptor");
            Error::new(EBADF)
        })?;

        trace!("Got config and device descriptors on port {}", port);

        for ifdesc in config_desc.interface_descs.iter() {
            //TODO: support alternate settings
            // This is difficult because the device driver must know which alternate
            // to use, but if alternates can have different classes, then a different
            // device driver may be required for each alternate. For now, we will use
            // only the default alternate setting (0)
            if ifdesc.alternate_setting != 0 {
                debug!(
                    "ignoring port {} iface {} alternate {} class {}.{} proto {}",
                    port,
                    ifdesc.number,
                    ifdesc.alternate_setting,
                    ifdesc.class,
                    ifdesc.sub_class,
                    ifdesc.protocol
                );
                continue;
            }

            if let Some(driver) = self.drivers_config.drivers.iter().find(|driver| {
                if let Some(ref ids) = driver.ids {
                    let mut device_found = false;
                    for (vendor, devices) in ids {
                        let vendor_without_prefix = vendor.trim_start_matches("0x");
                        let vendor = i64::from_str_radix(vendor_without_prefix, 16).unwrap() as u16;

                        if vendor != dev_desc.vendor {
                            continue;
                        }

                        for device in devices {
                            if *device == dev_desc.product {
                                device_found = true;
                                break;
                            }
                        }
                    }
                    if !device_found {
                        return false;
                    }
                }
                driver.class == ifdesc.class
                    && driver
                        .subclass()
                        .map(|subclass| subclass == ifdesc.sub_class)
                        .unwrap_or(true)
                    && driver
                        .protocol
                        .map(|protocol| protocol == ifdesc.protocol)
                        .unwrap_or(true)
            }) {
                info!(
                    "Loading subdriver \"{}\" for port {} iface {} alternate {} class {}.{} proto {}",
                    driver.name,
                    port,
                    ifdesc.number,
                    ifdesc.alternate_setting,
                    ifdesc.class,
                    ifdesc.sub_class,
                    ifdesc.protocol,
                );
                let (command, args) = driver.command.split_first().ok_or(Error::new(EBADMSG))?;

                let command = if command.starts_with('/') {
                    command.to_owned()
                } else {
                    "/usr/lib/drivers/".to_owned() + command
                };
                let mut cmd = process::Command::new(command);
                cmd.args(
                    args.into_iter()
                        .map(|arg| {
                            arg.replace("$SCHEME", &self.scheme_name)
                                .replace("$PORT", &format!("{}", port))
                                .replace("$IF_NUM", &format!("{}", ifdesc.number))
                                .replace("$IF_PROTO", &format!("{}", ifdesc.protocol))
                        })
                        .collect::<Vec<_>>(),
                )
                .stdin(process::Stdio::null());
                // FerrionOS lockdown (S13): class drivers talk to this driver's scheme, not to
                // hardware, so they start without the hardware right.
                daemon::without_hardware(&mut cmd);
                let process = cmd.spawn().or(Err(Error::new(ENOENT)))?;
                self.drivers.alter(port, |children_opt| {
                    let mut children = children_opt.unwrap_or_else(|| Vec::new());
                    children.push(process);
                    Some(children)
                });
            } else {
                debug!(
                    "No driver for port {} iface {} alternate {} class {}.{} proto {}",
                    port,
                    ifdesc.number,
                    ifdesc.alternate_setting,
                    ifdesc.class,
                    ifdesc.sub_class,
                    ifdesc.protocol,
                );
            }
        }

        Ok(())
    }
}
