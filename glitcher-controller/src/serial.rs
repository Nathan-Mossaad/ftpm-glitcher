use defmt::{panic, unwrap};
use embassy_executor::Spawner;
use embassy_rp::Peri;
use embassy_rp::bind_interrupts;
use embassy_rp::peripherals::USB;
use embassy_rp::usb::{self, Driver};
use embassy_usb::UsbDevice;
use embassy_usb::class::cdc_acm;
use embassy_usb::driver::EndpointError;
use glitcher_rpc::{CHUNK_BYTES, Chunk, ChunkStatus, Controller2HostMessage, RpcMessage, postcard};
use static_cell::StaticCell;

bind_interrupts!(struct USBIrqs {
    USBCTRL_IRQ => usb::InterruptHandler<USB>;
});

pub type UsbSerial = cdc_acm::CdcAcmClass<'static, Driver<'static, USB>>;

pub fn init(spawner: Spawner, usb: Peri<'static, USB>) -> UsbSerial {
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

pub async fn write_chunked(
    class: &mut UsbSerial,
    request_id: u32,
    data: &[u8],
    status: ChunkStatus,
    buffer: &mut [u8; 64],
) -> Result<(), ChunkWriteError> {
    let chunk_count = data.len().div_ceil(CHUNK_BYTES).max(1);
    for chunk_index in 0..chunk_count {
        let offset = chunk_index * CHUNK_BYTES;
        let end = (offset + CHUNK_BYTES).min(data.len());
        let mut chunk_data = [0; CHUNK_BYTES];
        chunk_data[..end.saturating_sub(offset)].copy_from_slice(&data[offset..end]);
        let is_last = chunk_index + 1 == chunk_count;
        let response = RpcMessage {
            id: request_id,
            message: Controller2HostMessage::Chunk(Chunk {
                offset: offset as u16,
                data: chunk_data,
                byte_count: end.saturating_sub(offset) as u8,
                is_last,
                status: if is_last {
                    status
                } else {
                    ChunkStatus::Complete
                },
            }),
        };
        let bytes =
            postcard::to_slice_cobs(&response, buffer).map_err(ChunkWriteError::Serialize)?;
        class
            .write_packet(bytes)
            .await
            .map_err(|_| ChunkWriteError::Disconnected)?;
    }
    Ok(())
}

type MyUsbDriver = Driver<'static, USB>;
type MyUsbDevice = UsbDevice<'static, MyUsbDriver>;

#[embassy_executor::task]
async fn usb_task(mut usb: MyUsbDevice) -> ! {
    usb.run().await
}

pub struct Disconnected;

pub enum ChunkWriteError {
    Serialize(postcard::Error),
    Disconnected,
}

impl From<EndpointError> for Disconnected {
    fn from(value: EndpointError) -> Self {
        match value {
            EndpointError::BufferOverflow => panic!("Buffer overflow"),
            EndpointError::Disabled => Self,
        }
    }
}
