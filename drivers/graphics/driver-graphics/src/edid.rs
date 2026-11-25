// SPDX-License-Identifier: GPL-2.0-only

pub const EDID_BLOCK_SIZE: usize = 128;
pub const MAX_EDID_EXTENSION_BLOCKS: usize = 8;
pub const MAX_EDID_TOTAL_BLOCKS: usize = 256;
pub const MAX_CTA_DATA_BLOCKS: usize = 32;
pub const MAX_CTA_VICS: usize = 31;
pub const MAX_CTA_DTDS: usize = 6;
pub const MAX_DISPLAYID_BLOCKS: usize = 32;
pub const MAX_DISPLAYID_TIMINGS: usize = 6;
pub const MAX_EDID_MODES: usize = 64;

const EDID_HEADER: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];
const EXT_TAG_CTA: u8 = 0x02;
const EXT_TAG_DISPLAYID: u8 = 0x70;
const CTA_TAG_VIDEO: u8 = 0x02;
const DISPLAYID_TAG_TYPE_I: u8 = 0x03;
const DISPLAYID_TAG_TYPE_VII: u8 = 0x22;

const CTA_PROGRESSIVE_VICS: &[(u8, u16, u16, u32, u16, u16, u16, u16, u16, u16, bool, bool)] = &[
    (
        1, 640, 480, 25175, 656, 752, 800, 490, 492, 525, false, false,
    ),
    (
        2, 720, 480, 27000, 736, 798, 858, 489, 495, 525, false, false,
    ),
    (
        3, 720, 480, 27000, 736, 798, 858, 489, 495, 525, false, false,
    ),
    (
        4, 1280, 720, 74250, 1390, 1430, 1650, 725, 730, 750, true, true,
    ),
    (
        8, 1440, 240, 27000, 1478, 1602, 1716, 244, 247, 262, false, false,
    ),
    (
        9, 1440, 240, 27000, 1478, 1602, 1716, 244, 247, 262, false, false,
    ),
    (
        12, 2880, 240, 54000, 2956, 3204, 3432, 244, 247, 262, false, false,
    ),
    (
        13, 2880, 240, 54000, 2956, 3204, 3432, 244, 247, 262, false, false,
    ),
    (
        14, 1440, 480, 54000, 1472, 1596, 1716, 489, 495, 525, false, false,
    ),
    (
        15, 1440, 480, 54000, 1472, 1596, 1716, 489, 495, 525, false, false,
    ),
    (
        16, 1920, 1080, 148500, 2008, 2052, 2200, 1084, 1089, 1125, true, true,
    ),
    (
        17, 720, 576, 27000, 732, 796, 864, 581, 586, 625, false, false,
    ),
    (
        18, 720, 576, 27000, 732, 796, 864, 581, 586, 625, false, false,
    ),
    (
        19, 1280, 720, 74250, 1720, 1760, 1980, 725, 730, 750, true, true,
    ),
    (
        23, 1440, 288, 27000, 1464, 1590, 1728, 290, 293, 312, false, false,
    ),
    (
        24, 1440, 288, 27000, 1464, 1590, 1728, 290, 293, 312, false, false,
    ),
    (
        27, 2880, 288, 54000, 2928, 3180, 3456, 290, 293, 312, false, false,
    ),
    (
        28, 2880, 288, 54000, 2928, 3180, 3456, 290, 293, 312, false, false,
    ),
    (
        29, 1440, 576, 54000, 1464, 1592, 1728, 581, 586, 625, false, false,
    ),
    (
        30, 1440, 576, 54000, 1464, 1592, 1728, 581, 586, 625, false, false,
    ),
    (
        31, 1920, 1080, 148500, 2448, 2492, 2640, 1084, 1089, 1125, true, true,
    ),
    (
        32, 1920, 1080, 74250, 2558, 2602, 2750, 1084, 1089, 1125, true, true,
    ),
    (
        33, 1920, 1080, 74250, 2448, 2492, 2640, 1084, 1089, 1125, true, true,
    ),
    (
        34, 1920, 1080, 74250, 2008, 2052, 2200, 1084, 1089, 1125, true, true,
    ),
    (
        35, 2880, 480, 108000, 2944, 3192, 3432, 489, 495, 525, false, false,
    ),
    (
        36, 2880, 480, 108000, 2944, 3192, 3432, 489, 495, 525, false, false,
    ),
    (
        37, 2880, 576, 108000, 2928, 3184, 3456, 581, 586, 625, false, false,
    ),
    (
        38, 2880, 576, 108000, 2928, 3184, 3456, 581, 586, 625, false, false,
    ),
    (
        41, 1280, 720, 148500, 1720, 1760, 1980, 725, 730, 750, true, true,
    ),
    (
        42, 720, 576, 54000, 732, 796, 864, 581, 586, 625, false, false,
    ),
    (
        43, 720, 576, 54000, 732, 796, 864, 581, 586, 625, false, false,
    ),
    (
        47, 1280, 720, 148500, 1390, 1430, 1650, 725, 730, 750, true, true,
    ),
    (
        48, 720, 480, 54000, 736, 798, 858, 489, 495, 525, false, false,
    ),
    (
        49, 720, 480, 54000, 736, 798, 858, 489, 495, 525, false, false,
    ),
    (
        52, 720, 576, 108000, 732, 796, 864, 581, 586, 625, false, false,
    ),
    (
        53, 720, 576, 108000, 732, 796, 864, 581, 586, 625, false, false,
    ),
    (
        56, 720, 480, 108000, 736, 798, 858, 489, 495, 525, false, false,
    ),
    (
        57, 720, 480, 108000, 736, 798, 858, 489, 495, 525, false, false,
    ),
    (
        60, 1280, 720, 59400, 3040, 3080, 3300, 725, 730, 750, true, true,
    ),
    (
        61, 1280, 720, 74250, 3700, 3740, 3960, 725, 730, 750, true, true,
    ),
    (
        62, 1280, 720, 74250, 3040, 3080, 3300, 725, 730, 750, true, true,
    ),
    (
        63, 1920, 1080, 297000, 2008, 2052, 2200, 1084, 1089, 1125, true, true,
    ),
    (
        64, 1920, 1080, 297000, 2448, 2492, 2640, 1084, 1089, 1125, true, true,
    ),
    (
        65, 1280, 720, 59400, 3040, 3080, 3300, 725, 730, 750, true, true,
    ),
    (
        66, 1280, 720, 74250, 3700, 3740, 3960, 725, 730, 750, true, true,
    ),
    (
        67, 1280, 720, 74250, 3040, 3080, 3300, 725, 730, 750, true, true,
    ),
    (
        68, 1280, 720, 74250, 1720, 1760, 1980, 725, 730, 750, true, true,
    ),
    (
        69, 1280, 720, 74250, 1390, 1430, 1650, 725, 730, 750, true, true,
    ),
    (
        70, 1280, 720, 148500, 1720, 1760, 1980, 725, 730, 750, true, true,
    ),
    (
        71, 1280, 720, 148500, 1390, 1430, 1650, 725, 730, 750, true, true,
    ),
    (
        72, 1920, 1080, 74250, 2558, 2602, 2750, 1084, 1089, 1125, true, true,
    ),
    (
        73, 1920, 1080, 74250, 2448, 2492, 2640, 1084, 1089, 1125, true, true,
    ),
    (
        74, 1920, 1080, 74250, 2008, 2052, 2200, 1084, 1089, 1125, true, true,
    ),
    (
        75, 1920, 1080, 148500, 2448, 2492, 2640, 1084, 1089, 1125, true, true,
    ),
    (
        76, 1920, 1080, 148500, 2008, 2052, 2200, 1084, 1089, 1125, true, true,
    ),
    (
        77, 1920, 1080, 297000, 2448, 2492, 2640, 1084, 1089, 1125, true, true,
    ),
    (
        78, 1920, 1080, 297000, 2008, 2052, 2200, 1084, 1089, 1125, true, true,
    ),
    (
        79, 1680, 720, 59400, 3040, 3080, 3300, 725, 730, 750, true, true,
    ),
    (
        80, 1680, 720, 59400, 2908, 2948, 3168, 725, 730, 750, true, true,
    ),
    (
        81, 1680, 720, 59400, 2380, 2420, 2640, 725, 730, 750, true, true,
    ),
    (
        82, 1680, 720, 82500, 1940, 1980, 2200, 725, 730, 750, true, true,
    ),
    (
        83, 1680, 720, 99000, 1940, 1980, 2200, 725, 730, 750, true, true,
    ),
    (
        84, 1680, 720, 165000, 1740, 1780, 2000, 725, 730, 825, true, true,
    ),
    (
        85, 1680, 720, 198000, 1740, 1780, 2000, 725, 730, 825, true, true,
    ),
    (
        86, 2560, 1080, 99000, 3558, 3602, 3750, 1084, 1089, 1100, true, true,
    ),
    (
        87, 2560, 1080, 90000, 3008, 3052, 3200, 1084, 1089, 1125, true, true,
    ),
    (
        88, 2560, 1080, 118800, 3328, 3372, 3520, 1084, 1089, 1125, true, true,
    ),
    (
        89, 2560, 1080, 185625, 3108, 3152, 3300, 1084, 1089, 1125, true, true,
    ),
    (
        90, 2560, 1080, 198000, 2808, 2852, 3000, 1084, 1089, 1100, true, true,
    ),
    (
        91, 2560, 1080, 371250, 2778, 2822, 2970, 1084, 1089, 1250, true, true,
    ),
    (
        92, 2560, 1080, 495000, 3108, 3152, 3300, 1084, 1089, 1250, true, true,
    ),
    (
        93, 3840, 2160, 297000, 5116, 5204, 5500, 2168, 2178, 2250, true, true,
    ),
    (
        94, 3840, 2160, 297000, 4896, 4984, 5280, 2168, 2178, 2250, true, true,
    ),
    (
        95, 3840, 2160, 297000, 4016, 4104, 4400, 2168, 2178, 2250, true, true,
    ),
    (
        96, 3840, 2160, 594000, 4896, 4984, 5280, 2168, 2178, 2250, true, true,
    ),
    (
        97, 3840, 2160, 594000, 4016, 4104, 4400, 2168, 2178, 2250, true, true,
    ),
    (
        98, 4096, 2160, 297000, 5116, 5204, 5500, 2168, 2178, 2250, true, true,
    ),
    (
        99, 4096, 2160, 297000, 5064, 5152, 5280, 2168, 2178, 2250, true, true,
    ),
    (
        100, 4096, 2160, 297000, 4184, 4272, 4400, 2168, 2178, 2250, true, true,
    ),
    (
        101, 4096, 2160, 594000, 5064, 5152, 5280, 2168, 2178, 2250, true, true,
    ),
    (
        102, 4096, 2160, 594000, 4184, 4272, 4400, 2168, 2178, 2250, true, true,
    ),
    (
        103, 3840, 2160, 297000, 5116, 5204, 5500, 2168, 2178, 2250, true, true,
    ),
    (
        104, 3840, 2160, 297000, 4896, 4984, 5280, 2168, 2178, 2250, true, true,
    ),
    (
        105, 3840, 2160, 297000, 4016, 4104, 4400, 2168, 2178, 2250, true, true,
    ),
    (
        106, 3840, 2160, 594000, 4896, 4984, 5280, 2168, 2178, 2250, true, true,
    ),
    (
        107, 3840, 2160, 594000, 4016, 4104, 4400, 2168, 2178, 2250, true, true,
    ),
    (
        108, 1280, 720, 90000, 2240, 2280, 2500, 725, 730, 750, true, true,
    ),
    (
        109, 1280, 720, 90000, 2240, 2280, 2500, 725, 730, 750, true, true,
    ),
    (
        110, 1680, 720, 99000, 2490, 2530, 2750, 725, 730, 750, true, true,
    ),
    (
        111, 1920, 1080, 148500, 2558, 2602, 2750, 1084, 1089, 1125, true, true,
    ),
    (
        112, 1920, 1080, 148500, 2558, 2602, 2750, 1084, 1089, 1125, true, true,
    ),
    (
        113, 2560, 1080, 198000, 3558, 3602, 3750, 1084, 1089, 1100, true, true,
    ),
    (
        114, 3840, 2160, 594000, 5116, 5204, 5500, 2168, 2178, 2250, true, true,
    ),
    (
        115, 4096, 2160, 594000, 5116, 5204, 5500, 2168, 2178, 2250, true, true,
    ),
    (
        116, 3840, 2160, 594000, 5116, 5204, 5500, 2168, 2178, 2250, true, true,
    ),
    (
        117, 3840, 2160, 1188000, 4896, 4984, 5280, 2168, 2178, 2250, true, true,
    ),
    (
        118, 3840, 2160, 1188000, 4016, 4104, 4400, 2168, 2178, 2250, true, true,
    ),
    (
        119, 3840, 2160, 1188000, 4896, 4984, 5280, 2168, 2178, 2250, true, true,
    ),
    (
        120, 3840, 2160, 1188000, 4016, 4104, 4400, 2168, 2178, 2250, true, true,
    ),
    (
        121, 5120, 2160, 396000, 7116, 7204, 7500, 2168, 2178, 2200, true, true,
    ),
    (
        122, 5120, 2160, 396000, 6816, 6904, 7200, 2168, 2178, 2200, true, true,
    ),
    (
        123, 5120, 2160, 396000, 5784, 5872, 6000, 2168, 2178, 2200, true, true,
    ),
    (
        124, 5120, 2160, 742500, 5866, 5954, 6250, 2168, 2178, 2475, true, true,
    ),
    (
        125, 5120, 2160, 742500, 6216, 6304, 6600, 2168, 2178, 2250, true, true,
    ),
    (
        126, 5120, 2160, 742500, 5284, 5372, 5500, 2168, 2178, 2250, true, true,
    ),
    (
        127, 5120, 2160, 1485000, 6216, 6304, 6600, 2168, 2178, 2250, true, true,
    ),
    (
        193, 5120, 2160, 1485000, 5284, 5372, 5500, 2168, 2178, 2250, true, true,
    ),
    (
        194, 7680, 4320, 1188000, 10232, 10408, 11000, 4336, 4356, 4500, true, true,
    ),
    (
        195, 7680, 4320, 1188000, 10032, 10208, 10800, 4336, 4356, 4400, true, true,
    ),
    (
        196, 7680, 4320, 1188000, 8232, 8408, 9000, 4336, 4356, 4400, true, true,
    ),
    (
        197, 7680, 4320, 2376000, 10232, 10408, 11000, 4336, 4356, 4500, true, true,
    ),
    (
        198, 7680, 4320, 2376000, 10032, 10208, 10800, 4336, 4356, 4400, true, true,
    ),
    (
        199, 7680, 4320, 2376000, 8232, 8408, 9000, 4336, 4356, 4400, true, true,
    ),
    (
        200, 7680, 4320, 4752000, 9792, 9968, 10560, 4336, 4356, 4500, true, true,
    ),
    (
        201, 7680, 4320, 4752000, 8032, 8208, 8800, 4336, 4356, 4500, true, true,
    ),
    (
        202, 7680, 4320, 1188000, 10232, 10408, 11000, 4336, 4356, 4500, true, true,
    ),
    (
        203, 7680, 4320, 1188000, 10032, 10208, 10800, 4336, 4356, 4400, true, true,
    ),
    (
        204, 7680, 4320, 1188000, 8232, 8408, 9000, 4336, 4356, 4400, true, true,
    ),
    (
        205, 7680, 4320, 2376000, 10232, 10408, 11000, 4336, 4356, 4500, true, true,
    ),
    (
        206, 7680, 4320, 2376000, 10032, 10208, 10800, 4336, 4356, 4400, true, true,
    ),
    (
        207, 7680, 4320, 2376000, 8232, 8408, 9000, 4336, 4356, 4400, true, true,
    ),
    (
        208, 7680, 4320, 4752000, 9792, 9968, 10560, 4336, 4356, 4500, true, true,
    ),
    (
        209, 7680, 4320, 4752000, 8032, 8208, 8800, 4336, 4356, 4500, true, true,
    ),
    (
        210, 10240, 4320, 1485000, 11732, 11908, 12500, 4336, 4356, 4950, true, true,
    ),
    (
        211, 10240, 4320, 1485000, 12732, 12908, 13500, 4336, 4356, 4400, true, true,
    ),
    (
        212, 10240, 4320, 1485000, 10528, 10704, 11000, 4336, 4356, 4500, true, true,
    ),
    (
        213, 10240, 4320, 2970000, 11732, 11908, 12500, 4336, 4356, 4950, true, true,
    ),
    (
        214, 10240, 4320, 2970000, 12732, 12908, 13500, 4336, 4356, 4400, true, true,
    ),
    (
        215, 10240, 4320, 2970000, 10528, 10704, 11000, 4336, 4356, 4500, true, true,
    ),
    (
        216, 10240, 4320, 5940000, 12432, 12608, 13200, 4336, 4356, 4500, true, true,
    ),
    (
        217, 10240, 4320, 5940000, 10528, 10704, 11000, 4336, 4356, 4500, true, true,
    ),
    (
        218, 4096, 2160, 1188000, 4896, 4984, 5280, 2168, 2178, 2250, true, true,
    ),
    (
        219, 4096, 2160, 1188000, 4184, 4272, 4400, 2168, 2178, 2250, true, true,
    ),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimingSource {
    BaseDtd,
    CtaDtd,
    CtaVic,
    DisplayIdType1,
    DisplayIdType7,
}

impl TimingSource {
    #[allow(dead_code)]
    pub fn label(self) -> &'static str {
        match self {
            TimingSource::BaseDtd => "base-dtd",
            TimingSource::CtaDtd => "cta-dtd",
            TimingSource::CtaVic => "cta-vic",
            TimingSource::DisplayIdType1 => "displayid-type1",
            TimingSource::DisplayIdType7 => "displayid-type7",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModeTiming {
    pub width: u32,
    pub height: u32,
    pub clock_khz: u32,
    pub hsync_start: u16,
    pub hsync_end: u16,
    pub htotal: u16,
    pub vsync_start: u16,
    pub vsync_end: u16,
    pub vtotal: u16,
    pub hsync_pos: Option<bool>,
    pub vsync_pos: Option<bool>,
    pub refresh_mhz: u32,
    pub preferred: bool,
    pub source: TimingSource,
}

impl ModeTiming {
    const ZERO: Self = Self {
        width: 0,
        height: 0,
        clock_khz: 0,
        hsync_start: 0,
        hsync_end: 0,
        htotal: 0,
        vsync_start: 0,
        vsync_end: 0,
        vtotal: 0,
        hsync_pos: None,
        vsync_pos: None,
        refresh_mhz: 0,
        preferred: false,
        source: TimingSource::BaseDtd,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EdidError {
    InvalidLength,
    BadHeader,
    BadBaseChecksum,
    NoTiming,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodedEdid {
    pub preferred: ModeTiming,
    pub modes: [ModeTiming; MAX_EDID_MODES],
    pub mode_count: usize,
    pub modes_truncated: bool,
    pub extensions_truncated: bool,
    pub bad_extension_blocks: usize,
    pub mm_width: u32,
    pub mm_height: u32,
}

struct ModeList {
    modes: [ModeTiming; MAX_EDID_MODES],
    count: usize,
    truncated: bool,
}

impl ModeList {
    fn new() -> Self {
        Self {
            modes: [ModeTiming::ZERO; MAX_EDID_MODES],
            count: 0,
            truncated: false,
        }
    }

    fn push(&mut self, mode: ModeTiming) {
        if self.count < MAX_EDID_MODES {
            self.modes[self.count] = mode;
            self.count += 1;
        } else {
            self.truncated = true;
        }
    }
}

fn valid_checksum(block: &[u8]) -> bool {
    if block.len() != EDID_BLOCK_SIZE {
        return false;
    }
    block.iter().fold(0u8, |acc, &byte| acc.wrapping_add(byte)) == 0
}

fn compute_refresh_mhz(pixel_clock_hz: u64, htotal: u64, vtotal: u64) -> Option<u32> {
    if pixel_clock_hz == 0 || htotal == 0 || vtotal == 0 {
        return None;
    }
    let denom = htotal.checked_mul(vtotal)?;
    let num = pixel_clock_hz.checked_mul(1000)?.checked_add(denom / 2)?;
    let mhz = num / denom;
    if mhz == 0 {
        return None;
    }
    u32::try_from(mhz).ok()
}

fn parse_dtd(desc: &[u8], preferred: bool, source: TimingSource) -> Option<(ModeTiming, u32, u32)> {
    if desc.len() < 18 {
        return None;
    }
    let pixel_clock_10khz = u16::from_le_bytes([desc[0], desc[1]]);
    if pixel_clock_10khz == 0 {
        return None;
    }
    if (desc[17] & 0x80) != 0 {
        return None;
    }
    let width = u32::from(desc[2]) | ((u32::from(desc[4]) & 0xF0) << 4);
    let hblank = u32::from(desc[3]) | ((u32::from(desc[4]) & 0x0F) << 8);
    let height = u32::from(desc[5]) | ((u32::from(desc[7]) & 0xF0) << 4);
    let vblank = u32::from(desc[6]) | ((u32::from(desc[7]) & 0x0F) << 8);
    if width == 0 || height == 0 {
        return None;
    }
    let hfp = u32::from(desc[8]) | ((u32::from(desc[11]) & 0xC0) << 2);
    let hsw = u32::from(desc[9]) | ((u32::from(desc[11]) & 0x30) << 4);
    let vfp = (u32::from(desc[10]) >> 4) | ((u32::from(desc[11]) & 0x0C) << 2);
    let vsw = (u32::from(desc[10]) & 0x0F) | ((u32::from(desc[11]) & 0x03) << 4);

    let htotal_u32 = width.checked_add(hblank)?;
    let vtotal_u32 = height.checked_add(vblank)?;
    let hsync_start_u32 = width.checked_add(hfp)?;
    let hsync_end_u32 = hsync_start_u32.checked_add(hsw)?;
    let vsync_start_u32 = height.checked_add(vfp)?;
    let vsync_end_u32 = vsync_start_u32.checked_add(vsw)?;

    let htotal = u16::try_from(htotal_u32).ok()?;
    let vtotal = u16::try_from(vtotal_u32).ok()?;
    let hsync_start = u16::try_from(hsync_start_u32.min(htotal_u32)).ok()?;
    let hsync_end = u16::try_from(hsync_end_u32.min(htotal_u32)).ok()?;
    let vsync_start = u16::try_from(vsync_start_u32.min(vtotal_u32)).ok()?;
    let vsync_end = u16::try_from(vsync_end_u32.min(vtotal_u32)).ok()?;

    let clock_khz = u32::from(pixel_clock_10khz) * 10;
    let pixel_clock_hz = u64::from(pixel_clock_10khz) * 10_000;
    let refresh_mhz = compute_refresh_mhz(pixel_clock_hz, u64::from(htotal), u64::from(vtotal))?;

    let (hsync_pos, vsync_pos) = if (desc[17] & 0x18) == 0x18 {
        (Some((desc[17] & 0x02) != 0), Some((desc[17] & 0x04) != 0))
    } else {
        (None, None)
    };

    let mm_width = u32::from(desc[12]) | ((u32::from(desc[14]) & 0xF0) << 4);
    let mm_height = u32::from(desc[13]) | ((u32::from(desc[14]) & 0x0F) << 8);

    Some((
        ModeTiming {
            width,
            height,
            clock_khz,
            hsync_start,
            hsync_end,
            htotal,
            vsync_start,
            vsync_end,
            vtotal,
            hsync_pos,
            vsync_pos,
            refresh_mhz,
            preferred,
            source,
        },
        mm_width,
        mm_height,
    ))
}

fn lookup_cta_vic(vic: u8, preferred: bool) -> Option<ModeTiming> {
    let idx = CTA_PROGRESSIVE_VICS
        .binary_search_by_key(&vic, |entry| entry.0)
        .ok()?;
    let (
        _vic,
        hdisplay,
        vdisplay,
        clock_khz,
        hsync_start,
        hsync_end,
        htotal,
        vsync_start,
        vsync_end,
        vtotal,
        hsync_pos,
        vsync_pos,
    ) = CTA_PROGRESSIVE_VICS[idx];
    let refresh_mhz = compute_refresh_mhz(
        u64::from(clock_khz) * 1000,
        u64::from(htotal),
        u64::from(vtotal),
    )?;
    Some(ModeTiming {
        width: u32::from(hdisplay),
        height: u32::from(vdisplay),
        clock_khz,
        hsync_start,
        hsync_end,
        htotal,
        vsync_start,
        vsync_end,
        vtotal,
        hsync_pos: Some(hsync_pos),
        vsync_pos: Some(vsync_pos),
        refresh_mhz,
        preferred,
        source: TimingSource::CtaVic,
    })
}

fn decode_svd(svd: u8) -> Option<(u8, bool)> {
    match svd {
        1..=64 => Some((svd, false)),
        129..=192 => Some((svd & 0x7F, true)),
        65..=127 | 193..=219 => Some((svd, false)),
        _ => None,
    }
}

fn parse_base_dtds(base: &[u8], list: &mut ModeList) -> (Option<ModeTiming>, u32, u32) {
    assert_eq!(base.len(), EDID_BLOCK_SIZE);
    let mut first = None;
    let mut mm_width = 0u32;
    let mut mm_height = 0u32;
    for slot in 0..4usize {
        let offset = 54 + slot * 18;
        let is_preferred = first.is_none();
        if let Some((timing, dtd_mm_w, dtd_mm_h)) = parse_dtd(
            &base[offset..offset + 18],
            is_preferred,
            TimingSource::BaseDtd,
        ) {
            if first.is_none() {
                first = Some(timing);
                if dtd_mm_w > 0 && dtd_mm_h > 0 {
                    mm_width = dtd_mm_w;
                    mm_height = dtd_mm_h;
                }
            }
            list.push(timing);
        }
    }
    (first, mm_width, mm_height)
}

fn parse_cta_extension(
    block: &[u8],
    list: &mut ModeList,
) -> (Option<ModeTiming>, Option<ModeTiming>, Option<ModeTiming>) {
    assert_eq!(block.len(), EDID_BLOCK_SIZE);
    let dtd_offset = usize::from(block[2]);
    let mut native_vic = None;
    let mut first_vic = None;
    let mut first_dtd = None;

    if dtd_offset > 4 && dtd_offset <= 127 {
        let mut pos = 4usize;
        for _ in 0..MAX_CTA_DATA_BLOCKS {
            if pos >= dtd_offset {
                break;
            }
            let header = block[pos];
            let tag = header >> 5;
            let len = usize::from(header & 0x1F);
            let data_start = pos + 1;
            let data_end = data_start + len;
            if data_end > dtd_offset {
                break;
            }
            if tag == CTA_TAG_VIDEO {
                let count = len.min(MAX_CTA_VICS);
                for i in 0..count {
                    if let Some((vic, native)) = decode_svd(block[data_start + i]) {
                        if let Some(timing) = lookup_cta_vic(vic, native) {
                            if native && native_vic.is_none() {
                                native_vic = Some(timing);
                            }
                            if first_vic.is_none() {
                                first_vic = Some(timing);
                            }
                            list.push(timing);
                        }
                    }
                }
            }
            pos = data_end;
        }
    }

    if (4..127).contains(&dtd_offset) {
        let mut offset = dtd_offset;
        for _ in 0..MAX_CTA_DTDS {
            if offset + 18 > 127 {
                break;
            }
            if let Some((timing, _, _)) =
                parse_dtd(&block[offset..offset + 18], false, TimingSource::CtaDtd)
            {
                if first_dtd.is_none() {
                    first_dtd = Some(timing);
                }
                list.push(timing);
            }
            offset += 18;
        }
    }

    (native_vic, first_dtd, first_vic)
}

fn parse_displayid_extension(
    block: &[u8],
    list: &mut ModeList,
) -> (Option<ModeTiming>, Option<ModeTiming>) {
    assert_eq!(block.len(), EDID_BLOCK_SIZE);
    let payload_len = usize::from(block[2]);
    let section_end = 5 + payload_len;
    if section_end > 126 {
        return (None, None);
    }
    let checksum = block[1..=section_end]
        .iter()
        .fold(0u8, |acc, &byte| acc.wrapping_add(byte));
    if checksum != 0 {
        return (None, None);
    }

    let mut preferred = None;
    let mut first_any = None;
    let mut pos = 5usize;
    for _ in 0..MAX_DISPLAYID_BLOCKS {
        if pos + 3 > section_end {
            break;
        }
        let tag = block[pos];
        let len = usize::from(block[pos + 2]);
        let data_start = pos + 3;
        let data_end = data_start + len;
        if data_end > section_end {
            break;
        }
        if tag == DISPLAYID_TAG_TYPE_I || tag == DISPLAYID_TAG_TYPE_VII {
            let step_hz: u64 = if tag == DISPLAYID_TAG_TYPE_I {
                10_000
            } else {
                1_000
            };
            let source = if tag == DISPLAYID_TAG_TYPE_I {
                TimingSource::DisplayIdType1
            } else {
                TimingSource::DisplayIdType7
            };
            let count = (len / 20).min(MAX_DISPLAYID_TIMINGS);
            for i in 0..count {
                let d = &block[data_start + i * 20..data_start + (i + 1) * 20];
                let flags = d[3];
                if (flags & 0x10) != 0 {
                    continue;
                }
                let raw_clk = u64::from(d[0]) | (u64::from(d[1]) << 8) | (u64::from(d[2]) << 16);
                let pixel_clock_hz = (raw_clk + 1) * step_hz;
                let Some(clock_khz) = u32::try_from((pixel_clock_hz + 500) / 1000).ok() else {
                    continue;
                };
                let width = u32::from(u16::from_le_bytes([d[4], d[5]])) + 1;
                let hblank = u32::from(u16::from_le_bytes([d[6], d[7]])) + 1;
                let hfp_raw = u16::from_le_bytes([d[8], d[9]]);
                let hfp = u32::from(hfp_raw & 0x7FFF) + 1;
                let hsync_pos = Some((hfp_raw & 0x8000) != 0);
                let hsw = u32::from(u16::from_le_bytes([d[10], d[11]])) + 1;

                let height = u32::from(u16::from_le_bytes([d[12], d[13]])) + 1;
                let vblank = u32::from(u16::from_le_bytes([d[14], d[15]])) + 1;
                let vfp_raw = u16::from_le_bytes([d[16], d[17]]);
                let vfp = u32::from(vfp_raw & 0x7FFF) + 1;
                let vsync_pos = Some((vfp_raw & 0x8000) != 0);
                let vsw = u32::from(u16::from_le_bytes([d[18], d[19]])) + 1;

                let Some(htotal_u32) = width.checked_add(hblank) else {
                    continue;
                };
                let Some(vtotal_u32) = height.checked_add(vblank) else {
                    continue;
                };
                let Some(hsync_start_u32) = width.checked_add(hfp) else {
                    continue;
                };
                let Some(hsync_end_u32) = hsync_start_u32.checked_add(hsw) else {
                    continue;
                };
                let Some(vsync_start_u32) = height.checked_add(vfp) else {
                    continue;
                };
                let Some(vsync_end_u32) = vsync_start_u32.checked_add(vsw) else {
                    continue;
                };
                let (
                    Ok(htotal),
                    Ok(vtotal),
                    Ok(hsync_start),
                    Ok(hsync_end),
                    Ok(vsync_start),
                    Ok(vsync_end),
                ) = (
                    u16::try_from(htotal_u32),
                    u16::try_from(vtotal_u32),
                    u16::try_from(hsync_start_u32.min(htotal_u32)),
                    u16::try_from(hsync_end_u32.min(htotal_u32)),
                    u16::try_from(vsync_start_u32.min(vtotal_u32)),
                    u16::try_from(vsync_end_u32.min(vtotal_u32)),
                )
                else {
                    continue;
                };

                let Some(refresh_mhz) =
                    compute_refresh_mhz(pixel_clock_hz, u64::from(htotal), u64::from(vtotal))
                else {
                    continue;
                };
                let is_pref = (flags & 0x80) != 0;
                let timing = ModeTiming {
                    width,
                    height,
                    clock_khz,
                    hsync_start,
                    hsync_end,
                    htotal,
                    vsync_start,
                    vsync_end,
                    vtotal,
                    hsync_pos,
                    vsync_pos,
                    refresh_mhz,
                    preferred: is_pref,
                    source,
                };
                if first_any.is_none() {
                    first_any = Some(timing);
                }
                if is_pref && preferred.is_none() {
                    preferred = Some(timing);
                }
                list.push(timing);
            }
        }
        pos = data_end;
    }
    (preferred, first_any)
}

pub fn decode_all(edid: &[u8]) -> Result<DecodedEdid, EdidError> {
    if edid.len() < EDID_BLOCK_SIZE
        || edid.len() % EDID_BLOCK_SIZE != 0
        || edid.len() > MAX_EDID_TOTAL_BLOCKS * EDID_BLOCK_SIZE
    {
        return Err(EdidError::InvalidLength);
    }
    let base = &edid[..EDID_BLOCK_SIZE];
    if base[..8] != EDID_HEADER {
        return Err(EdidError::BadHeader);
    }
    if !valid_checksum(base) {
        return Err(EdidError::BadBaseChecksum);
    }

    let declared_ext = usize::from(base[126]);
    let expected_len = match declared_ext
        .checked_add(1)
        .and_then(|blocks| blocks.checked_mul(EDID_BLOCK_SIZE))
    {
        Some(len) => len,
        None => return Err(EdidError::InvalidLength),
    };
    if edid.len() < expected_len
        || (edid.len() > expected_len && !edid[expected_len..].iter().all(|&b| b == 0))
    {
        return Err(EdidError::InvalidLength);
    }

    let extensions_truncated = declared_ext > MAX_EDID_EXTENSION_BLOCKS;
    let ext_to_walk = declared_ext.min(MAX_EDID_EXTENSION_BLOCKS);
    let mut list = ModeList::new();
    let (base_dtd, mut mm_width, mut mm_height) = parse_base_dtds(base, &mut list);
    if (mm_width == 0 || mm_height == 0) && base[21] > 0 && base[22] > 0 {
        mm_width = u32::from(base[21]) * 10;
        mm_height = u32::from(base[22]) * 10;
    }
    let mut displayid_preferred = None;
    let mut displayid_any = None;
    let mut cta_native_vic = None;
    let mut cta_dtd = None;
    let mut cta_first_vic = None;
    let mut bad_extension_blocks = 0usize;

    for ext_idx in 0..ext_to_walk {
        let start = (1 + ext_idx) * EDID_BLOCK_SIZE;
        let block = &edid[start..start + EDID_BLOCK_SIZE];
        if !valid_checksum(block) {
            bad_extension_blocks += 1;
            continue;
        }
        match block[0] {
            EXT_TAG_DISPLAYID => {
                let (pref, any) = parse_displayid_extension(block, &mut list);
                if displayid_preferred.is_none() {
                    displayid_preferred = pref;
                }
                if displayid_any.is_none() {
                    displayid_any = any;
                }
            }
            EXT_TAG_CTA => {
                let (n_vic, dtd, f_vic) = parse_cta_extension(block, &mut list);
                if cta_native_vic.is_none() {
                    cta_native_vic = n_vic;
                }
                if cta_dtd.is_none() {
                    cta_dtd = dtd;
                }
                if cta_first_vic.is_none() {
                    cta_first_vic = f_vic;
                }
            }
            _ => (),
        }
    }

    let preferred = displayid_preferred
        .or(base_dtd)
        .or(cta_native_vic)
        .or(displayid_any)
        .or(cta_dtd)
        .or(cta_first_vic)
        .ok_or(EdidError::NoTiming)?;

    Ok(DecodedEdid {
        preferred,
        modes: list.modes,
        mode_count: list.count,
        modes_truncated: list.truncated,
        extensions_truncated,
        bad_extension_blocks,
        mm_width,
        mm_height,
    })
}
