#![no_std]

use ringbuffer::{ConstGenericRingBuffer, RingBuffer};
use tinyvec::ArrayVec;
use usb_device::{bus::{UsbBus, UsbBusAllocator}, class::UsbClass, device::{UsbDeviceBuilder, UsbVidPid}};
use usbd_hid::{descriptor::gen_hid_descriptor, hid_class::{HIDClass, ReportInfo}, UsbError};

const AURA_REPORT_ID: u8 = 0xec;
const AURA_REQUEST_FIRMWARE_VERSION: u8 = 0x82;
const AURA_REQUEST_CONFIG_TABLE: u8 = 0xB0;
const AURA_REQUEST_SET_LEDS: u8 = 0x40;

const AURA_MAX_DIRECT_LED_COUNT: u8 = 20;

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

pub const ROG_AURA_TERMINAL_HID_DESCRIPTOR: [u8; 36] = [
    0x06, 0x72, 0xff,              // Usage Page (Vendor Usage Page 0xff72)
    0x09, 0xa1,                    // Usage (Vendor Usage 0xa1)
    0xa1, 0x01,                    // Collection (Application)
    0x85, 0xec,                    //  Report ID (236)
    0x09, 0x10,                    //  Usage (Vendor Usage 0x10)
    0x15, 0x00,                    //  Logical Minimum (0)
    0x26, 0xff, 0x00,              //  Logical Maximum (255)
    0x75, 0x08,                    //  Report Size (8)
    0x95, 0x3F,                    //  Report Count (63) [In the original device, this is 64]
    0x81, 0x02,                    //  Input (Data,Var,Abs)
    0x09, 0x11,                    //  Usage (Vendor Usage 0x11)
    0x15, 0x00,                    //  Logical Minimum (0)
    0x26, 0xff, 0x00,              //  Logical Maximum (255)
    0x75, 0x08,                    //  Report Size (8)
    0x95, 0x40,                    //  Report Count (64)
    0x91, 0x02,                    //  Output (Data,Var,Abs)
    0xc0,                          // End Collection
];

#[cfg(feature = "rgb-crate")]
pub use rgb::RGB8;

#[cfg(not(feature = "rgb-crate"))]
#[repr(packed, C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct RGB8 {
    pub r: u8,
    pub g: u8,
    pub b: u8
}

const _: () = assert!(size_of::<RGB8>() == 3, "Current RGB8 implementation does not have the required size (3)");
pub fn rgb_from_raw_slice(slice: &[u8]) -> &[RGB8] {
    unsafe {
        // SAFETY: every triple of u8 is valid represented as a RGB
        // color. RGB8 size of 3 is ensured with a constant time
        // assertion.
        let ptr: *const RGB8 = core::mem::transmute(slice.as_ptr());
        core::slice::from_raw_parts(ptr, slice.len() / size_of::<RGB8>())
    }
}

// The original ASUS ROG Terminal firmware specifies IN transfer size
// of 65 bytes. However, usbd-hid and synopsys-usb-otg have limited
// the transfer size to wMaxPacketSize, which for full-speed devices
// is 64b. Therefore, reducing it to 64.
const FIRMWARE_VERSION_RESPONSE: [u8; 64] = const_data!(64, {
    [AURA_REPORT_ID, 0x02],
    b"AUTA0-S072-0101",
    ..
});

// From my own tests, Armoury Crate doesn't give a fluff about any of
// this data, except for the header (first 2 bytes) and, for whatever
// reason, the byte at index 8. For now just sending all the data
// possible to try to honor the original device behavior, just in case
const CONFIG_TABLE_RESPONSE: [u8; 64] = const_data!(64, {
    [
        AURA_REPORT_ID, 0x30,               // Report ID, response code
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
        apply: bool,
        offset: u8,
        led_data: ArrayVec<[RGB8; AURA_MAX_DIRECT_LED_COUNT as usize]>
    }
}

enum RogTerminalReadyData {
    FirmwareVersion,
    ConfigTable
}

pub struct AsusRogTerminalHidClass<'a, B: UsbBus> {
    inner: HIDClass<'a, B>,
    data_rdy: ConstGenericRingBuffer<RogTerminalReadyData, 4>,
    next_message: Option<RogTerminalMessage>
}

impl<'a, B: UsbBus> AsusRogTerminalHidClass<'a, B> {
    pub fn new(alloc: &'a UsbBusAllocator<B>) -> Self {
        Self {
            inner: HIDClass::new_ep_in(alloc, &ROG_AURA_TERMINAL_HID_DESCRIPTOR, 4),
            data_rdy: ConstGenericRingBuffer::new(),
            next_message: None
        }
    }

    pub fn new_with_hid(hid: HIDClass<'a, B>) -> Self {
        Self {
            inner: hid,
            data_rdy: ConstGenericRingBuffer::new(),
            next_message: None
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
            dev_info!("Peek!");
            self.inner.push_raw_input(match elem {
                RogTerminalReadyData::FirmwareVersion => &FIRMWARE_VERSION_RESPONSE,
                RogTerminalReadyData::ConfigTable => &CONFIG_TABLE_RESPONSE,
            })?;
            self.data_rdy.dequeue();
        }

        Ok(())
    }

    fn handle_report(&mut self, buf: &[u8]) {
        if buf[0] != AURA_REPORT_ID {
            dev_error!("Received SET_REPORT with invalid Report ID: {}", buf[0]);
            return;
        }

        match buf[1] {
            AURA_REQUEST_FIRMWARE_VERSION => {
                dev_info!("Host requested firmware version");
                self.data_rdy.push(RogTerminalReadyData::FirmwareVersion)
            }

            AURA_REQUEST_CONFIG_TABLE => {
                dev_info!("Host requested device configuration table");
                self.data_rdy.push(RogTerminalReadyData::ConfigTable)
            }

            AURA_REQUEST_SET_LEDS => {
                dev_info!("Received direct leds update");

                let apply = (buf[2] & 0x80) > 0;
                let offset = buf[3];
                let mut num_leds = buf[4];
                if num_leds > AURA_MAX_DIRECT_LED_COUNT {
                    dev_error!("Host sent a led count greater than maximum ({})", num_leds);
                    num_leds = AURA_MAX_DIRECT_LED_COUNT;
                }

                let mut led_data = [RGB8 { r: 0, g: 0, b: 0 }; AURA_MAX_DIRECT_LED_COUNT as usize];
                led_data[0..num_leds as usize].copy_from_slice(rgb_from_raw_slice(&buf[5..5 + num_leds as usize]));

                self.next_message = Some(RogTerminalMessage::UpdateLeds {
                    apply,
                    offset,
                    led_data: ArrayVec::from_array_len(led_data, num_leds as usize)
                });
            }

            invalid => {
                dev_error!("Received unrecognized request ID: {}. Ignored", invalid);
            }
        }
    }

    pub fn poll(&mut self) -> Option<RogTerminalMessage> {
        let next = self.next_message.take();
        let _ = self.push_ready_data();
        next
    }
}

impl<'a, B: UsbBus> UsbClass<B> for AsusRogTerminalHidClass<'a, B> {
    fn get_configuration_descriptors(&self, writer: &mut usb_device::descriptor::DescriptorWriter) -> usbd_hid::Result<()> {
        self.inner.get_configuration_descriptors(writer)
    }

    fn get_bos_descriptors(&self, writer: &mut usb_device::descriptor::BosWriter) -> usbd_hid::Result<()> {
        self.inner.get_bos_descriptors(writer)
    }

    fn get_string(&self, index: usb_device::bus::StringIndex, lang_id: usb_device::LangID) -> Option<&str> {
        self.inner.get_string(index, lang_id)
    }

    fn reset(&mut self) {
        self.inner.reset()
    }

    fn control_out(&mut self, xfer: usb_device::class::ControlOut<B>) {
        self.inner.control_out(xfer)
    }

    fn control_in(&mut self, xfer: usb_device::class::ControlIn<B>) {
        self.inner.control_in(xfer)
    }

    fn endpoint_setup(&mut self, addr: usb_device::endpoint::EndpointAddress) {
        self.inner.endpoint_setup(addr)
    }

    fn endpoint_out(&mut self, addr: usb_device::endpoint::EndpointAddress) {
        self.inner.endpoint_out(addr)
    }

    fn endpoint_in_complete(&mut self, addr: usb_device::endpoint::EndpointAddress) {
        self.inner.endpoint_in_complete(addr)
    }

    fn get_alt_setting(&mut self, interface: usb_device::bus::InterfaceNumber) -> Option<u8> {
        self.inner.get_alt_setting(interface)
    }

    fn set_alt_setting(&mut self, interface: usb_device::bus::InterfaceNumber, alternative: u8) -> bool {
        self.inner.set_alt_setting(interface, alternative)
    }

    fn poll(&mut self) {
        self.inner.poll();
        let mut reportbuf: [u8; 65] = [0; 65];
        match self.inner.pull_raw_report(&mut reportbuf) {
            Ok(_) => {
                self.handle_report(&reportbuf);
            },
            Err(e) => {
                #[cfg(feature = "log")]
                if !matches!(e, UsbError::WouldBlock) {
                    dev_error!("Fail to pull SET_REPORT: {:?}", e);
                }
            }
        }
        let _ = self.push_ready_data();
    }
}

pub fn rog_terminal_usb_device_builder<B: UsbBus>(alloc: &UsbBusAllocator<B>) -> UsbDeviceBuilder<B> {
    UsbDeviceBuilder::new(alloc, UsbVidPid(0x0b05, 0x1889))
}
