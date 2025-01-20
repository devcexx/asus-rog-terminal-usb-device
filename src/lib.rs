#![no_std]

pub mod aura;

use aura::constants::{AURA_INPUT_REPORT_SIZE, AURA_OUTPUT_REPORT_SIZE};
use aura::RGB8;
use aura::{
    constants::{AURA_FIRMWARE_VERSION_LEN, AURA_HID_REPORT_ID, AURA_MAX_DIRECT_LED_COUNT},
    rgb_from_raw_slice, AuraEffect, AuraInputReport, AuraInputReportType, AuraOutputReport,
    AuraOutputReportType,
};
use ringbuffer::{ConstGenericRingBuffer, RingBuffer};
use tinyvec::ArrayVec;
use usb_device::{
    bus::{UsbBus, UsbBusAllocator},
    class::UsbClass,
    device::{UsbDeviceBuilder, UsbVidPid},
};
use usbd_hid::{hid_class::HIDClass, UsbError};

macro_rules! dev_error {
    () => {};
    ($($arg:tt)*) => {
        #[cfg(feature = "log")]
        log::error!($($arg)*);
    }
}

macro_rules! dev_info {
    () => {};
    ($($arg:tt)*) => {
        #[cfg(feature = "log")]
        log::info!($($arg)*);
    }
}

macro_rules! const_data {
    (@internal $buf:ident $idx:ident ..)  => {
        $idx = $buf.len()
    };


    (@internal $buf:ident $idx:ident $expr:expr)  => {
        let slice = &($expr);
        let mut cur = 0;
        while cur < slice.len() {
            $buf[$idx] = slice[cur];
            $idx += 1;
            cur += 1;
        }
    };

    ($len:literal, { $($gen:tt),* }) => {
        {
            let mut buf: [u8; $len] = [0; $len];
            let mut idx: usize = 0;

            $(
            const_data!(@internal buf idx $gen);
            )*

            if idx < buf.len() {
                panic!("Not enough bytes!!")
            }

            buf
        }
    };
}
/// The HID descriptor used by an ROG Aura Terminal.
pub const ROG_AURA_TERMINAL_HID_DESCRIPTOR: [u8; 36] = [
    0x06, 0x72, 0xff, // Usage Page (Vendor Usage Page 0xff72)
    0x09, 0xa1, // Usage (Vendor Usage 0xa1)
    0xa1, 0x01, // Collection (Application)
    0x85, 0xec, //  Report ID (236)
    0x09, 0x10, //  Usage (Vendor Usage 0x10)
    0x15, 0x00, //  Logical Minimum (0)
    0x26, 0xff, 0x00, //  Logical Maximum (255)
    0x75, 0x08, //  Report Size (8)
    0x95, 0x3F, //  Report Count (63) [In the original device, this is 64]
    0x81, 0x02, //  Input (Data,Var,Abs)
    0x09, 0x11, //  Usage (Vendor Usage 0x11)
    0x15, 0x00, //  Logical Minimum (0)
    0x26, 0xff, 0x00, //  Logical Maximum (255)
    0x75, 0x08, //  Report Size (8)
    0x95, 0x40, //  Report Count (64)
    0x91, 0x02, //  Output (Data,Var,Abs)
    0xc0, // End Collection
];

pub const ROG_AURA_DEFAULT_FIRMWARE_VERSION: &[u8; 15] = b"AUTA0-S072-0101";

// From my own tests, Armoury Crate doesn't give a fluff about any of
// this data, except for the header (first 2 bytes) and, for whatever
// reason, the byte at index 8. For now just sending all the data
// possible to try to honor the original device behavior, just in case
const CONFIG_TABLE_RESPONSE: [u8; 64] = const_data!(64, {
    [
        AURA_HID_REPORT_ID, AuraInputReportType::ConfigTableRequestOk as u8,
        0x00, 0x00, 0x1f, 0xff,             // The fuck
        0x04,                               // Number of channels
        0x1f, 0x01, 0x01,                   // The fuck #2

        0x00, 0x5a, 0x01, 0x64, 0x01, 0x01, // Channel 1 data: 0x5a refers to the number of leds in channel, although Armoury crate seems to be ignoring this value.
        0x00, 0x5a, 0x01, 0x64, 0x01, 0x01, // Channel 2 data
        0x00, 0x5a, 0x01, 0x64, 0x01, 0x01, // Channel 3 data
        0x00, 0x5a, 0x01, 0x64, 0x01, 0x03, // Channel 4 data
    ],
    ..
});

pub enum RogTerminalMessage {
    UpdateLeds {
        channel: u8,
        offset: u8,
        apply: bool,
        led_data: ArrayVec<[RGB8; AURA_MAX_DIRECT_LED_COUNT as usize]>,
    },

    SetEffect {
        channel: u8,
        effect: AuraEffect,
    },
}

enum RogTerminalReadyData {
    FirmwareVersion,
    ConfigTable,
}

pub struct AsusRogTerminalHidClass<'a, B: UsbBus> {
    inner: HIDClass<'a, B>,
    data_rdy: ConstGenericRingBuffer<RogTerminalReadyData, 4>,
    next_message: Option<RogTerminalMessage>,
    firmware_version: &'static [u8; AURA_FIRMWARE_VERSION_LEN as usize],
}

impl<'a, B: UsbBus> AsusRogTerminalHidClass<'a, B> {
    pub fn build_default_hid_class(alloc: &'a UsbBusAllocator<B>) -> HIDClass<B> {
        HIDClass::new_ep_in(alloc, &ROG_AURA_TERMINAL_HID_DESCRIPTOR, 4)
    }

    pub fn new_with_defaults(alloc: &'a UsbBusAllocator<B>) -> Self {
        Self::new(
            Self::build_default_hid_class(alloc),
            &ROG_AURA_DEFAULT_FIRMWARE_VERSION,
        )
    }

    pub fn new(
        hid: HIDClass<'a, B>,
        firmware_version: &'static [u8; AURA_FIRMWARE_VERSION_LEN as usize],
    ) -> Self {
        Self {
            inner: hid,
            data_rdy: ConstGenericRingBuffer::new(),
            next_message: None,
            firmware_version: &firmware_version,
        }
    }

    pub fn hid_class(&self) -> &HIDClass<'a, B> {
        &self.inner
    }

    pub fn hid_class_mut(&mut self) -> &mut HIDClass<'a, B> {
        &mut self.inner
    }

    fn push_ready_data(&mut self) -> Result<(), UsbError> {
        while let Some(elem) = self.data_rdy.peek() {
            match elem {
                RogTerminalReadyData::FirmwareVersion => {
                    let mut fw_report: AuraInputReport = [0u8; AURA_INPUT_REPORT_SIZE];
                    fw_report[0] = AURA_HID_REPORT_ID;
                    fw_report[1] = AuraInputReportType::FirmwareVersionRequestOk as u8;
                    fw_report[2..17].copy_from_slice(self.firmware_version);
                    self.inner.push_raw_input(&fw_report)?;
                }
                RogTerminalReadyData::ConfigTable => {
                    self.inner.push_raw_input(&CONFIG_TABLE_RESPONSE)?;
                }
            }
            self.data_rdy.dequeue();
        }

        Ok(())
    }

    fn handle_report(&mut self, report: &AuraOutputReport) {
        let report_id = report[0];
        let report_type = report[1];

        if report_id != AURA_HID_REPORT_ID {
            dev_error!("Unrecognized report ID: {}", report_id);
            return
        }

        let Ok(report_type) = AuraOutputReportType::try_from(report_type) else {
            dev_error!("Received unrecognized request type: {}", report_type);
            return;
        };

        match report_type {
            AuraOutputReportType::FirmwareVersionRequest => {
                dev_info!("Host requested firmware version");
                self.data_rdy.push(RogTerminalReadyData::FirmwareVersion)
            }
            AuraOutputReportType::ConfigTableRequest => {
                dev_info!("Host requested device configuration table");
                self.data_rdy.push(RogTerminalReadyData::ConfigTable)
            }
            AuraOutputReportType::SetEffect => {
                let channel = report[2];
                let effect_code = report[4];
                let Ok(effect) = AuraEffect::try_from(effect_code) else {
                    dev_error!("Unknown effect code received: {:02x}", effect_code);
                    return;
                };

                dev_info!(
                    "Host requested set effect for ch {} to {:02x}",
                    channel,
                    effect_code
                );
                self.next_message = Some(RogTerminalMessage::SetEffect { channel, effect })
            }
            AuraOutputReportType::SetDirectLeds => {
                let apply = (report[2] & 0x80) > 0;
                let channel = report[2] & 0x7f;

                let offset = report[3];
                let mut num_leds = report[4];
                if num_leds > AURA_MAX_DIRECT_LED_COUNT {
                    dev_error!("Host sent a led count greater than maximum ({})", num_leds);
                    num_leds = AURA_MAX_DIRECT_LED_COUNT;
                }

                let mut led_data = [RGB8 { r: 0, g: 0, b: 0 }; AURA_MAX_DIRECT_LED_COUNT as usize];
                led_data[0..num_leds as usize]
                    .copy_from_slice(rgb_from_raw_slice(&report[5..5 + num_leds as usize * 3]));

                self.next_message = Some(RogTerminalMessage::UpdateLeds {
                    channel,
                    apply,
                    offset,
                    led_data: ArrayVec::from_array_len(led_data, num_leds as usize),
                });
            }
        }
    }

    pub fn poll_next_message(&mut self) -> Option<RogTerminalMessage> {
        self.next_message.take()
    }
}

impl<'a, B: UsbBus> UsbClass<B> for AsusRogTerminalHidClass<'a, B> {
    #[inline]
    fn get_configuration_descriptors(
        &self,
        writer: &mut usb_device::descriptor::DescriptorWriter,
    ) -> usbd_hid::Result<()> {
        self.inner.get_configuration_descriptors(writer)
    }

    #[inline]
    fn get_bos_descriptors(
        &self,
        writer: &mut usb_device::descriptor::BosWriter,
    ) -> usbd_hid::Result<()> {
        self.inner.get_bos_descriptors(writer)
    }

    #[inline]
    fn get_string(
        &self,
        index: usb_device::bus::StringIndex,
        lang_id: usb_device::LangID,
    ) -> Option<&str> {
        self.inner.get_string(index, lang_id)
    }

    #[inline]
    fn reset(&mut self) {
        self.inner.reset()
    }

    #[inline]
    fn control_out(&mut self, xfer: usb_device::class::ControlOut<B>) {
        self.inner.control_out(xfer)
    }

    #[inline]
    fn control_in(&mut self, xfer: usb_device::class::ControlIn<B>) {
        self.inner.control_in(xfer)
    }

    #[inline]
    fn endpoint_setup(&mut self, addr: usb_device::endpoint::EndpointAddress) {
        self.inner.endpoint_setup(addr)
    }

    #[inline]
    fn endpoint_out(&mut self, addr: usb_device::endpoint::EndpointAddress) {
        self.inner.endpoint_out(addr)
    }

    #[inline]
    fn endpoint_in_complete(&mut self, addr: usb_device::endpoint::EndpointAddress) {
        self.inner.endpoint_in_complete(addr)
    }

    #[inline]
    fn get_alt_setting(&mut self, interface: usb_device::bus::InterfaceNumber) -> Option<u8> {
        self.inner.get_alt_setting(interface)
    }

    #[inline]
    fn set_alt_setting(
        &mut self,
        interface: usb_device::bus::InterfaceNumber,
        alternative: u8,
    ) -> bool {
        self.inner.set_alt_setting(interface, alternative)
    }

    fn poll(&mut self) {
        self.inner.poll();
        let mut reportbuf: AuraOutputReport = [0; AURA_OUTPUT_REPORT_SIZE];
        match self.inner.pull_raw_report(&mut reportbuf) {
            Ok(_) => {
                self.handle_report(&reportbuf);
            }
            Err(e) =>
            {
                #[cfg(feature = "log")]
                if !matches!(e, UsbError::WouldBlock) {
                    dev_error!("Fail to pull report: {:?}", e);
                }
            }
        }
        let _ = self.push_ready_data();
    }
}

pub fn rog_terminal_usb_device_builder<B: UsbBus>(
    alloc: &UsbBusAllocator<B>,
) -> UsbDeviceBuilder<B> {
    UsbDeviceBuilder::new(alloc, UsbVidPid(0x0b05, 0x1889))
}
