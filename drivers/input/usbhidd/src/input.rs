use std::collections::HashMap;

use inputd::ProducerHandle;
use rehid::hidreport::{Collection, Report};
use rehid::report_handler::{ReportEvent, ReportHandler};
use rehid::usage_tables::{GenericDesktopUsage, UsagePage};

use self::keyboard::send_key_event;

mod gamepad;
mod keyboard;
mod pointer;

type ReportCollections = HashMap<(Option<u8>, u32), Vec<Collection>>;

pub(super) fn collect_report_collections(handler: &ReportHandler) -> ReportCollections {
    let mut report_collections = HashMap::new();
    for report in handler.descriptor.input_reports() {
        for field in report.fields() {
            let key = (report.report_id().map(u8::from), u32::from(field.id()));
            if let Some(old) = report_collections.get(&key) {
                log::warn!(
                    "key {:?} has old collections {:?} and new collections {:?}",
                    key,
                    old,
                    field.collections()
                );
                continue;
            }
            report_collections.insert(key, field.collections().to_vec());
        }
    }
    report_collections
}

pub(super) fn process_report(
    handler: &mut ReportHandler,
    report_buffer: &[u8],
    report_collections: &ReportCollections,
    display: &mut ProducerHandle,
    left_shift: &mut bool,
    right_shift: &mut bool,
    last_mouse_pos: &mut (i32, i32),
    last_buttons: &mut [bool; 3],
    gamepad_state: &mut HashMap<(u16, u16), (i64, i64)>,
) {
    let mut mouse_pos = *last_mouse_pos;
    let mut mouse_dx = 0i32;
    let mut mouse_dy = 0i32;
    let mut scroll_y = 0i32;
    let mut buttons = *last_buttons;
    for event in handler
        .handle(report_buffer)
        .expect("failed to parse report")
    {
        let mut gamepad = false;
        if let Some(collections) = report_collections.get(&(event.report_id, event.field_id)) {
            for collection in collections {
                for usage in collection.usages() {
                    match UsagePage::from_repr(usage.usage_page.into()) {
                        Some(UsagePage::GenericDesktop) => {
                            match GenericDesktopUsage::from_repr(usage.usage_id.into()) {
                                Some(GenericDesktopUsage::GamePad) => {
                                    gamepad = true;
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                }
            }
        } else {
            log::warn!(
                "collections missing for report {:?} field {}",
                event.report_id,
                event.field_id
            );
        }

        // Special handling for gamepad
        //TODO: make this more generic
        if gamepad {
            let ReportEvent {
                usage_page,
                usage,
                value,
                ..
            } = event;
            let last = gamepad_state
                .get(&(usage_page, usage))
                .map_or(0, |x: &(i64, i64)| x.0);
            gamepad_state.insert((usage_page, usage), (value, last));
            if value == last {
                continue;
            }
            gamepad::send_event(display, usage_page, usage, value);
            continue;
        }

        match UsagePage::from_repr(event.usage_page) {
            Some(UsagePage::GenericDesktop) => {
                pointer::accumulate_motion(
                    &event,
                    &mut mouse_pos,
                    &mut mouse_dx,
                    &mut mouse_dy,
                    &mut scroll_y,
                );
            }
            Some(UsagePage::KeyboardOrKeypad) => {
                let (pressed, _shift_opt) = if event.value != 0 {
                    (true, Some(*left_shift | *right_shift))
                } else {
                    (false, None)
                };
                if event.usage == 0xE1 {
                    *left_shift = pressed;
                } else if event.usage == 0xE5 {
                    *right_shift = pressed;
                }
                send_key_event(display, event.usage_page, event.usage, pressed);
            }
            Some(UsagePage::Button) => {
                pointer::accumulate_button(&event, &mut buttons);
            }
            _ => {
                if event.usage_page >= 0xFF00 {
                    // Ignore vendor defined event
                } else {
                    log::info!(
                        "unsupported usage 0x{:X}:0x{:X} value {}",
                        event.usage_page,
                        event.usage,
                        event.value
                    );
                }
            }
        }
    }

    pointer::send_position(display, mouse_pos, last_mouse_pos);
    pointer::send_relative_motion(display, mouse_dx, mouse_dy);
    pointer::send_scroll(display, scroll_y);
    pointer::send_buttons(display, buttons, last_buttons);
}
