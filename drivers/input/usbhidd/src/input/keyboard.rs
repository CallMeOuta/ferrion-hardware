use inputd::ProducerHandle;
use orbclient::KeyEvent as OrbKeyEvent;

pub(crate) fn send_key_event(
    display: &mut ProducerHandle,
    usage_page: u16,
    usage: u16,
    pressed: bool,
) {
    let scancode = match usage_page {
        0x07 => match usage {
            0x04 => orbclient::K_A,
            0x05 => orbclient::K_B,
            0x06 => orbclient::K_C,
            0x07 => orbclient::K_D,
            0x08 => orbclient::K_E,
            0x09 => orbclient::K_F,
            0x0A => orbclient::K_G,
            0x0B => orbclient::K_H,
            0x0C => orbclient::K_I,
            0x0D => orbclient::K_J,
            0x0E => orbclient::K_K,
            0x0F => orbclient::K_L,
            0x10 => orbclient::K_M,
            0x11 => orbclient::K_N,
            0x12 => orbclient::K_O,
            0x13 => orbclient::K_P,
            0x14 => orbclient::K_Q,
            0x15 => orbclient::K_R,
            0x16 => orbclient::K_S,
            0x17 => orbclient::K_T,
            0x18 => orbclient::K_U,
            0x19 => orbclient::K_V,
            0x1A => orbclient::K_W,
            0x1B => orbclient::K_X,
            0x1C => orbclient::K_Y,
            0x1D => orbclient::K_Z,
            0x1E => orbclient::K_1,
            0x1F => orbclient::K_2,
            0x20 => orbclient::K_3,
            0x21 => orbclient::K_4,
            0x22 => orbclient::K_5,
            0x23 => orbclient::K_6,
            0x24 => orbclient::K_7,
            0x25 => orbclient::K_8,
            0x26 => orbclient::K_9,
            0x27 => orbclient::K_0,
            0x28 => orbclient::K_ENTER,
            0x29 => orbclient::K_ESC,
            0x2A => orbclient::K_BKSP,
            0x2B => orbclient::K_TAB,
            0x2C => orbclient::K_SPACE,
            0x2D => orbclient::K_MINUS,
            0x2E => orbclient::K_EQUALS,
            0x2F => orbclient::K_BRACE_OPEN,
            0x30 => orbclient::K_BRACE_CLOSE,
            0x31 => orbclient::K_BACKSLASH,
            // 0x32 non-us # and ~
            0x32 => 0x56,
            0x33 => orbclient::K_SEMICOLON,
            0x34 => orbclient::K_QUOTE,
            0x35 => orbclient::K_TICK,
            0x36 => orbclient::K_COMMA,
            0x37 => orbclient::K_PERIOD,
            0x38 => orbclient::K_SLASH,
            0x39 => orbclient::K_CAPS,
            0x3A => orbclient::K_F1,
            0x3B => orbclient::K_F2,
            0x3C => orbclient::K_F3,
            0x3D => orbclient::K_F4,
            0x3E => orbclient::K_F5,
            0x3F => orbclient::K_F6,
            0x40 => orbclient::K_F7,
            0x41 => orbclient::K_F8,
            0x42 => orbclient::K_F9,
            0x43 => orbclient::K_F10,
            0x44 => orbclient::K_F11,
            0x45 => orbclient::K_F12,
            0x46 => orbclient::K_PRTSC,
            0x47 => orbclient::K_SCROLL,
            // 0x48 pause
            0x49 => orbclient::K_INS,
            0x4A => orbclient::K_HOME,
            0x4B => orbclient::K_PGUP,
            0x4C => orbclient::K_DEL,
            0x4D => orbclient::K_END,
            0x4E => orbclient::K_PGDN,
            0x4F => orbclient::K_RIGHT,
            0x50 => orbclient::K_LEFT,
            0x51 => orbclient::K_DOWN,
            0x52 => orbclient::K_UP,
            0x53 => orbclient::K_NUM,
            0x54 => orbclient::K_NUM_SLASH,
            0x55 => orbclient::K_NUM_ASTERISK,
            0x56 => orbclient::K_NUM_MINUS,
            0x57 => orbclient::K_NUM_PLUS,
            0x58 => orbclient::K_NUM_ENTER,
            0x59 => orbclient::K_NUM_1,
            0x5A => orbclient::K_NUM_2,
            0x5B => orbclient::K_NUM_3,
            0x5C => orbclient::K_NUM_4,
            0x5D => orbclient::K_NUM_5,
            0x5E => orbclient::K_NUM_6,
            0x5F => orbclient::K_NUM_7,
            0x60 => orbclient::K_NUM_8,
            0x61 => orbclient::K_NUM_9,
            0x62 => orbclient::K_NUM_0,
            // 0x62 num .
            // 0x64 non-us \ and |
            0x64 => orbclient::K_APP,
            0x66 => orbclient::K_POWER,
            // 0x67 num =
            // unmapped values
            0xE0 => orbclient::K_LEFT_CTRL,
            0xE1 => orbclient::K_LEFT_SHIFT,
            0xE2 => orbclient::K_ALT,
            0xE3 => orbclient::K_LEFT_SUPER,
            0xE4 => orbclient::K_RIGHT_CTRL,
            0xE5 => orbclient::K_RIGHT_SHIFT,
            0xE6 => orbclient::K_ALT_GR,
            0xE7 => orbclient::K_RIGHT_SUPER,
            // reserved values
            _ => {
                log::warn!("unknown usage_page {:#x} usage {:#x}", usage_page, usage);
                return;
            }
        },
        _ => {
            log::warn!("unknown usage_page {:#x}", usage_page);
            return;
        }
    };

    let key_event = OrbKeyEvent {
        character: '\0',
        scancode,
        pressed,
    };

    match display.write_event(key_event.to_event()) {
        Ok(_) => (),
        Err(err) => {
            log::warn!("failed to send key event to orbital: {}", err);
        }
    }
}
