// SPDX-License-Identifier: GPL-2.0-only AND MIT
use inputd::ProducerHandle;
use rehid::report_handler::ReportEvent;
use rehid::usage_tables::GenericDesktopUsage;

pub(crate) fn accumulate_motion(
    event: &ReportEvent,
    mouse_pos: &mut (i32, i32),
    mouse_dx: &mut i32,
    mouse_dy: &mut i32,
    scroll_y: &mut i32,
) {
    match GenericDesktopUsage::from_repr(event.usage) {
        Some(GenericDesktopUsage::X) => {
            if event.relative {
                *mouse_dx += event.value as i32;
            } else {
                mouse_pos.0 = event.value as i32;
            }
        }
        Some(GenericDesktopUsage::Y) => {
            if event.relative {
                *mouse_dy += event.value as i32;
            } else {
                mouse_pos.1 = event.value as i32;
            }
        }
        Some(GenericDesktopUsage::Wheel) => {
            //TODO: what is X scroll?
            if event.relative {
                *scroll_y += event.value as i32;
            } else {
                log::warn!("absolute mouse wheel not supported");
            }
        }
        unsupported => {
            log::info!(
                "unsupported generic desktop usage 0x{:X}:0x{:X} ({:?}) value {}",
                event.usage_page,
                event.usage,
                unsupported,
                event.value
            );
        }
    }
}

pub(crate) fn accumulate_button(event: &ReportEvent, buttons: &mut [bool; 3]) {
    if event.usage > 0 && event.usage as usize <= buttons.len() {
        buttons[event.usage as usize - 1] = event.value != 0;
    } else {
        log::info!(
            "unsupported buttons usage 0x{:X}:0x{:X} value {}",
            event.usage_page,
            event.usage,
            event.value
        );
    }
}

pub(crate) fn send_position(
    display: &mut ProducerHandle,
    mouse_pos: (i32, i32),
    last_mouse_pos: &mut (i32, i32),
) {
    if mouse_pos != *last_mouse_pos {
        *last_mouse_pos = mouse_pos;

        // TODO
        // ps2d uses 0..=65535 as range, while usb uses 0..=32767. orbital
        // expects the former range, so multiply by two here to temporarily
        // align with orbital expectation. This workaround will make cursor
        //  looks out of sync in QEMU using virtio-vga with usb-tablet.
        let mouse_event = orbclient::event::MouseEvent {
            x: mouse_pos.0 * 2,
            y: mouse_pos.1 * 2,
        };

        match display.write_event(mouse_event.to_event()) {
            Ok(_) => (),
            Err(err) => {
                log::warn!("failed to send mouse event to orbital: {}", err);
            }
        }
    }
}

pub(crate) fn send_relative_motion(display: &mut ProducerHandle, mouse_dx: i32, mouse_dy: i32) {
    if mouse_dx != 0 || mouse_dy != 0 {
        // TODO: This is a filter to prevent random mouse jumps
        if mouse_dx > -127 && mouse_dx < 127 {
            let mouse_event = orbclient::event::MouseRelativeEvent {
                dx: mouse_dx,
                dy: mouse_dy,
            };

            match display.write_event(mouse_event.to_event()) {
                Ok(_) => (),
                Err(err) => {
                    log::warn!("failed to send mouse event to orbital: {}", err);
                }
            }
        }
    }
}

pub(crate) fn send_scroll(display: &mut ProducerHandle, scroll_y: i32) {
    if scroll_y != 0 {
        let scroll_event = orbclient::event::ScrollEvent { x: 0, y: scroll_y };

        match display.write_event(scroll_event.to_event()) {
            Ok(_) => (),
            Err(err) => {
                log::warn!("failed to send scroll event to orbital: {}", err);
            }
        }
    }
}

pub(crate) fn send_buttons(
    display: &mut ProducerHandle,
    buttons: [bool; 3],
    last_buttons: &mut [bool; 3],
) {
    if buttons != *last_buttons {
        *last_buttons = buttons;

        let button_event = orbclient::event::ButtonEvent {
            left: buttons[0],
            right: buttons[1],
            middle: buttons[2],
        };

        match display.write_event(button_event.to_event()) {
            Ok(_) => (),
            Err(err) => {
                log::warn!("failed to send button event to orbital: {}", err);
            }
        }
    }
}
