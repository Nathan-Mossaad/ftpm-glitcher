use embassy_rp::peripherals::{DMA_CH0, DMA_CH1, DMA_CH2, DMA_CH3, PIN_2, PIN_4, PIN_5, SPI0};
use embassy_rp::spi::{Config, Spi};
use embassy_rp::{Peri, bind_interrupts, dma};
use embassy_time::{Duration, with_timeout};
use glitcher_rpc::{SPI_TAP_MAX_BYTES, SpiTapError};

bind_interrupts!(struct Irqs {
    DMA_IRQ_0 => dma::InterruptHandler<DMA_CH0>, dma::InterruptHandler<DMA_CH1>, dma::InterruptHandler<DMA_CH2>, dma::InterruptHandler<DMA_CH3>;
});

pub async fn tap_spi(
    spi0: &mut Peri<'static, SPI0>,
    slave_clk: &mut Peri<'static, PIN_2>,
    slave_miso: &mut Peri<'static, PIN_4>,
    slave_cs_pin: &mut Peri<'static, PIN_5>,
    spi_tx_dma: &mut Peri<'static, DMA_CH2>,
    spi_rx_dma: &mut Peri<'static, DMA_CH3>,
    capture: &mut [u8; SPI_TAP_MAX_BYTES],
    byte_count: u32,
    timeout_s: u32,
) -> Result<SpiTapResult, SpiTapError> {
    let byte_count = byte_count as usize;
    if !(1..=SPI_TAP_MAX_BYTES).contains(&byte_count) {
        return Err(SpiTapError::InvalidByteCount);
    }

    let mut config = Config::default();
    config.phase = embassy_rp::spi::Phase::CaptureOnSecondTransition;
    config.polarity = embassy_rp::spi::Polarity::IdleHigh;
    let mut spi = Spi::new_slave_rxonly(
        spi0.reborrow(),
        slave_clk.reborrow(),
        slave_miso.reborrow(),
        slave_cs_pin.reborrow(),
        spi_tx_dma.reborrow(),
        spi_rx_dma.reborrow(),
        Irqs,
        config,
    );
    let result = with_timeout(
        Duration::from_secs(timeout_s as u64),
        spi.read(&mut capture[..byte_count]),
    )
    .await;

    // On timeout, dropping the SPI read future aborts the RX DMA transfer.
    // The RP2040 DMA transfer count then reports how many bytes were still
    // outstanding, so the leading bytes in `capture` are valid received data.
    drop(spi);
    let remaining = embassy_rp::pac::DMA.ch(3).trans_count().read() as usize;

    match result {
        Ok(Ok(())) => Ok(SpiTapResult {
            byte_count,
            timed_out: false,
        }),
        Ok(Err(_)) => Err(SpiTapError::ReadFailed),
        Err(_) => Ok(SpiTapResult {
            byte_count: byte_count.saturating_sub(remaining.min(byte_count)),
            timed_out: true,
        }),
    }
}

pub struct SpiTapResult {
    pub byte_count: usize,
    pub timed_out: bool,
}
