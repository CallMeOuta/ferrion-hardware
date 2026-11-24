// SPDX-License-Identifier: GPL-2.0-only
//! The boot splash fbbootlogd draws instead of the log when the bootloader passes `QUIET=1`: the
//! background color, the FerrionOS mark where the login screen shows it next, and a progress bar
//! that moves only when a boot event shows up in the log.

use std::io;

use console_draw::V2DisplayMap;

/// Boot events that move the progress bar: text that init and the drivers write to the log on
/// every boot, roughly in this order. Each counts once, in whatever order it arrives.
const PROGRESS_EVENTS: &[&str] = &[
    // The kernel started init, which runs from the initfs; in the log before fbbootlogd starts
    "init: switchroot to /scheme/initfs",
    // pcid-spawner started the first driver from the initfs, usually the boot disk's
    "pcid-spawner: spawn \"/scheme/initfs/",
    // The root filesystem is mounted and init starts the services on it
    "init: switchroot to /usr",
    // pcid-spawner started the first driver from the root filesystem: USB, audio or network
    "pcid-spawner: spawn \"/usr/",
    // The text console on VT 2 took its display, among the last services before the display
    // server
    "fbcond: Performing handoff",
];

/// The steps of the bar: the events, then the display server taking the screen, which ends the
/// splash.
const PROGRESS_STEPS: usize = PROGRESS_EVENTS.len() + 1;

/// The ground, the ink and the empty part of a progress bar, as 0xRRGGBB.
const GROUND: u32 = 0x0f0e0c;
const INK: u32 = 0xedebe7;
const TRACK: u32 = 0x33312f;

/// The mark on its grid of 4 by 7 units: one byte per row from the top, with the left
/// column in bit 3.
const MARK_COLUMNS: usize = 4;
const MARK: [u8; 7] = [0b1111, 0b1000, 0b1000, 0b0000, 0b1110, 0b1000, 0b1000];

/// The login background is 1280 by 800 pixels, and orblogin scales it to cover the screen.
/// The lengths below are in its pixels: on a screen a length is the larger of
/// `length * width / LOGIN_WIDTH` and `length * height / LOGIN_HEIGHT`.
const LOGIN_WIDTH: usize = 1280;
const LOGIN_HEIGHT: usize = 800;

/// One unit of the mark's grid, and the width of the box of the mark and the name.
const MARK_UNIT: usize = 16;
const LOCKUP_WIDTH: usize = 160;

/// Heights above the centre of the screen: the top of the mark, then the cap line and the
/// baseline of the name that the login screen sets under it.
const MARK_TOP: usize = 314;
const NAME_CAP_LINE: usize = 170;
const NAME_BASELINE: usize = 146;

pub struct Splash {
    /// 0xRRGGBB, from the bootloader's `BOOT_BACKGROUND`.
    background: u32,
    seen: [bool; PROGRESS_EVENTS.len()],
    /// The log line read so far.
    line: Vec<u8>,
}

#[derive(Clone, Copy)]
struct Rect {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

impl Splash {
    /// The splash, when the kernel environment the bootloader wrote asks for a quiet boot.
    pub fn from_env(env: &str) -> Option<Splash> {
        let mut quiet = false;
        // The bootloader writes BOOT_BACKGROUND with QUIET=1; black if it is missing
        let mut background = GROUND;
        for line in env.lines() {
            match line.split_once('=') {
                Some(("QUIET", value)) => quiet = value == "1",
                Some(("BOOT_BACKGROUND", value)) => {
                    background = u32::from_str_radix(value, 16).unwrap_or(0) & 0xFFFFFF;
                }
                _ => {}
            }
        }
        quiet.then(|| Splash {
            background,
            seen: [false; PROGRESS_EVENTS.len()],
            line: Vec::new(),
        })
    }

    /// Takes log output; returns whether a boot event in it moved the progress bar.
    pub fn take_logs(&mut self, buf: &[u8]) -> bool {
        let mut moved = false;
        for &b in buf {
            if b != b'\n' {
                self.line.push(b);
                continue;
            }
            for (seen, event) in self.seen.iter_mut().zip(PROGRESS_EVENTS) {
                if !*seen
                    && self
                        .line
                        .windows(event.len())
                        .any(|window| window == event.as_bytes())
                {
                    *seen = true;
                    moved = true;
                }
            }
            self.line.clear();
        }
        moved
    }

    pub fn draw(&self, map: &mut V2DisplayMap) -> io::Result<()> {
        let done = self.seen.iter().filter(|seen| **seen).count();
        map.draw(|pixels, stride, width, height| {
            let screen = Rect {
                x: 0,
                y: 0,
                width,
                height,
            };
            fill(pixels, stride, screen, self.background);

            // A length of the login background on this screen. orblogin scales that background
            // to cover the screen, so the mark drawn here is where the login screen shows it next
            let scaled =
                |length: usize| (length * width / LOGIN_WIDTH).max(length * height / LOGIN_HEIGHT);
            let unit = scaled(MARK_UNIT).max(1);

            let mark_x = (width / 2).saturating_sub(MARK_COLUMNS * unit / 2);
            let mark_y = (height / 2).saturating_sub(scaled(MARK_TOP));
            for (row, bits) in MARK.iter().enumerate() {
                for column in 0..MARK_COLUMNS {
                    if bits >> (MARK_COLUMNS - 1 - column) & 1 == 1 {
                        let cell = Rect {
                            x: mark_x + column * unit,
                            y: mark_y + row * unit,
                            width: unit,
                            height: unit,
                        };
                        fill(pixels, stride, cell, INK);
                    }
                }
            }

            // The bar takes the place of the name that the login screen sets under the mark: as
            // wide as the name's box, a quarter of a unit high, halfway up its capitals
            let bar_width = scaled(LOCKUP_WIDTH);
            let bar_height = (unit / 4).max(2);
            let bar = Rect {
                x: (width / 2).saturating_sub(bar_width / 2),
                y: (height / 2)
                    .saturating_sub(scaled((NAME_CAP_LINE + NAME_BASELINE) / 2))
                    .saturating_sub(bar_height / 2),
                width: bar_width,
                height: bar_height,
            };
            fill(pixels, stride, bar, TRACK);
            fill(
                pixels,
                stride,
                Rect {
                    width: bar.width * done / PROGRESS_STEPS,
                    ..bar
                },
                INK,
            );
        })
    }
}

fn fill(pixels: &mut [u32], stride: usize, rect: Rect, rgb: u32) {
    for y in rect.y..rect.y + rect.height {
        let start = y * stride + rect.x;
        if let Some(row) = pixels.get_mut(start..start + rect.width) {
            row.fill(0xFF000000 | rgb);
        }
    }
}
