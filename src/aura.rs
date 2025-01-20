use constants::{AURA_INPUT_REPORT_SIZE, AURA_OUTPUT_REPORT_SIZE};
use int_enum::IntEnum;
#[cfg(feature = "rgb-crate")]
pub use rgb::RGB8;

pub mod constants {
    /// The HID Report ID that uses Asus for the HID output reports.
    pub const AURA_HID_REPORT_ID: u8 = 0xec;

    /// The maximum LED count that can be sent for change in a single direct LED update report.
    pub const AURA_MAX_DIRECT_LED_COUNT: u8 = 20;

    /// The length of an Aura firmware length string.
    pub const AURA_FIRMWARE_VERSION_LEN: u8 = 15;

    pub const AURA_OUTPUT_REPORT_SIZE: usize = 65;

    // The original ASUS ROG Terminal firmware specifies IN transfer size
    // of 65 bytes. However, usbd-hid and synopsys-usb-otg (for STM32 with
    // OTG support) have limited the transfer size to wMaxPacketSize,
    // which for full-speed devices is 64b. Therefore, reducing it to 64.
    pub const AURA_INPUT_REPORT_SIZE: usize = 64;
}

#[cfg(not(feature = "rgb-crate"))]
#[repr(packed, C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct RGB8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

pub fn rgb_from_raw_slice(slice: &[u8]) -> &[RGB8] {
    const _: () = assert!(size_of::<RGB8>() == 3);
    unsafe {
        // SAFETY: every triple of u8 is valid represented as a RGB
        // color. RGB8 size of 3 is ensured with a constant time
        // assertion.
        let ptr: *const RGB8 = core::mem::transmute(slice.as_ptr());
        core::slice::from_raw_parts(ptr, slice.len() / size_of::<RGB8>())
    }
}

#[repr(u8)]
#[derive(Clone, Copy, IntEnum)]
pub enum AuraEffect {
    Off = 0,
    Static = 1,
    Breathing = 2,
    Flashing = 3,
    SpectrumCycle = 4,
    Rainbow = 5,
    SpectrumCycleBreathing = 6,
    ChaseFade = 7,
    SpectrumCycleChaseFade = 8,
    Chase = 9,
    SpectrumCycleChase = 10,
    SpectrumCycleWave = 11,
    ChaseRainbowPulse = 12,
    RandomFlicker = 13,
    Music = 14,
    Direct = 0xff,
}

/// The possible report types that the host can send to the device.
#[repr(u8)]
#[derive(Clone, Copy, IntEnum)]
pub enum AuraOutputReportType {
    /// The host is requesting the firmware version of the device.
    FirmwareVersionRequest = 0x82,

    /// The host is requesting the config table of the device.
    ConfigTableRequest = 0xB0,

    /// The host is requesting the device to set a preset effect in the LEDs.
    SetEffect = 0x3B,

    /// The host is requesting the device to set the LEDs of the
    /// device to a specific colors.
    SetDirectLeds = 0x40,
}

/// The possible report types that the device can send to the host.
#[repr(u8)]
#[derive(Clone, Copy, IntEnum)]
pub enum AuraInputReportType {
    /// The firmware request was successfully completed.
    FirmwareVersionRequestOk = 0x02,

    /// The config table request was successfully completed.
    ConfigTableRequestOk = 0x30,
}

pub type AuraOutputReport = [u8; AURA_OUTPUT_REPORT_SIZE];
pub type AuraInputReport = [u8; AURA_INPUT_REPORT_SIZE];

pub enum InvalidReportError {
    InvalidReportId,
    InvalidReportType
}
