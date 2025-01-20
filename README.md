# ASUS ROG Aura Terminal USB Device Library

This library is built on top of the
[usb-device](https://docs.rs/usb-device/latest/usb_device/) and
[usbd\_hid](https://docs.rs/usbd-hid/latest/usbd_hid/) crates, and
implements the USB HID protocol of the device side used by the [ASUS
ROG Aura
Terminal](https://rog.asus.com/apparel-bags-gear/gear/rog-aura-terminal-model/),
which is a RGB controller hub manufactured by ASUS. This library
allows you to build custom USB devices that can impersonate this
device, allowing to build cheap RGB controllers that can work out of
the box with software like Armoury crate, Signal RGB and Open RGB.

The ROG Aura Terminal is a device that seems to be already
discontinued, but it used to be part of te Aura ecosystem of ASUS, and
it seems that is still supported by the majority of RGB controller
software. The device is limited to 4 channels (5 if you count with the
logo LED, that is just a single LED), with up to 90 LEDs on each. The
protocol implemented by this library inherits those limitations, as it
seems they are hardcoded on software like Armoury Crate.
