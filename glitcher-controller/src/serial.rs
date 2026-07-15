use defmt::{panic, unwrap};
use embassy_executor::Spawner;
use embassy_rp::Peri;
use embassy_rp::bind_interrupts;
use embassy_rp::peripherals::USB;
use embassy_rp::usb::{self, Driver};
use embassy_usb::UsbDevice;
use embassy_usb::class::cdc_acm;
use embassy_usb::driver::EndpointError;
use static_cell::StaticCell;

bind_interrupts!(struct USBIrqs {
    USBCTRL_IRQ => usb::InterruptHandler<USB>;
});

pub fn init(
    spawner: Spawner,
    usb: Peri<'static, USB>,
) -> cdc_acm::CdcAcmClass<'static, Driver<'static, USB>> {
    let driver = Driver::new(usb, USBIrqs);

    let config = {
        let mut config = embassy_usb::Config::new(0xc0de, 0xcafe);
        config.manufacturer = Some("Glitcher");
        config.product = Some("Controller");
        config.serial_number = Some("12345678");
        config.max_power = 100;
        config.max_packet_size_0 = 64;
        config
    };

    let mut builder = {
        static CONFIG_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
        static BOS_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
        static CONTROL_BUF: StaticCell<[u8; 64]> = StaticCell::new();

        embassy_usb::Builder::new(
            driver,
            config,
            CONFIG_DESCRIPTOR.init([0; 256]),
            BOS_DESCRIPTOR.init([0; 256]),
            &mut [], // no msos descriptors
            CONTROL_BUF.init([0; 64]),
        )
    };

    let class = {
        static STATE: StaticCell<cdc_acm::State> = StaticCell::new();
        cdc_acm::CdcAcmClass::new(&mut builder, STATE.init(cdc_acm::State::new()), 64)
    };

    spawner.spawn(unwrap!(usb_task(builder.build())));

    class
}

type MyUsbDriver = Driver<'static, USB>;
type MyUsbDevice = UsbDevice<'static, MyUsbDriver>;

#[embassy_executor::task]
async fn usb_task(mut usb: MyUsbDevice) -> ! {
    usb.run().await
}

pub struct Disconnected;

impl From<EndpointError> for Disconnected {
    fn from(value: EndpointError) -> Self {
        match value {
            EndpointError::BufferOverflow => panic!("Buffer overflow"),
            EndpointError::Disabled => Self,
        }
    }
}
