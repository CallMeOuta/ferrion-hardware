use inputd::ProducerHandle;
use rehid::usage_tables::{GenericDesktopUsage, UsagePage};

pub(crate) fn send_event(display: &mut ProducerHandle, usage_page: u16, usage: u16, value: i64) {
    match UsagePage::from_repr(usage_page) {
        Some(UsagePage::GenericDesktop) => match GenericDesktopUsage::from_repr(usage) {
            Some(GenericDesktopUsage::X) => {
                send_gamepad_axis(display, orbclient::CONTROLLER_AXIS_X, value);
            }
            Some(GenericDesktopUsage::Y) => {
                send_gamepad_axis(display, orbclient::CONTROLLER_AXIS_Y, value);
            }
            Some(GenericDesktopUsage::Z) => {
                send_gamepad_axis(display, orbclient::CONTROLLER_AXIS_Z, value);
            }
            Some(GenericDesktopUsage::Rx) => {
                send_gamepad_axis(display, orbclient::CONTROLLER_AXIS_RX, value);
            }
            Some(GenericDesktopUsage::Ry) => {
                send_gamepad_axis(display, orbclient::CONTROLLER_AXIS_RY, value);
            }
            Some(GenericDesktopUsage::Rz) => {
                send_gamepad_axis(display, orbclient::CONTROLLER_AXIS_RZ, value);
            }
            Some(GenericDesktopUsage::DpadLeft) => {
                send_gamepad_button(display, orbclient::CONTROLLER_DPAD_LEFT, value != 0);
            }
            Some(GenericDesktopUsage::DpadRight) => {
                send_gamepad_button(display, orbclient::CONTROLLER_DPAD_RIGHT, value != 0);
            }
            Some(GenericDesktopUsage::DpadUp) => {
                send_gamepad_button(display, orbclient::CONTROLLER_DPAD_UP, value != 0);
            }
            Some(GenericDesktopUsage::DpadDown) => {
                send_gamepad_button(display, orbclient::CONTROLLER_DPAD_DOWN, value != 0);
            }
            _ => {}
        },
        Some(UsagePage::Button) => {
            send_gamepad_button(display, usage, value != 0);
        }
        _ => {}
    }
}

fn send_gamepad_axis(display: &mut ProducerHandle, axis: u16, value: i64) {
    let event = orbclient::ControllerAxisEvent {
        // Filled in by inputd
        id: 0,
        axis,
        value: value as i32,
    };

    match display.write_event(event.to_event()) {
        Ok(_) => (),
        Err(err) => {
            log::warn!("failed to send axis event to orbital: {}", err);
        }
    }
}

fn send_gamepad_button(display: &mut ProducerHandle, button: u16, pressed: bool) {
    let event = orbclient::ControllerButtonEvent {
        // Filled in by inputd
        id: 0,
        button,
        pressed,
    };

    match display.write_event(event.to_event()) {
        Ok(_) => (),
        Err(err) => {
            log::warn!("failed to send button event to orbital: {}", err);
        }
    }
}
