use embassy_rp::Peri;
use embassy_rp::gpio::{Flex, Pull};
use embassy_rp::peripherals::PIN_5;
use embassy_time::{Duration, with_timeout};

pub async fn count_chip_selects(timeout_s: u32, slave_cs_pin: &mut Peri<'static, PIN_5>) -> u32 {
    let mut slave_cs_pin = Flex::new(slave_cs_pin.reborrow());
    slave_cs_pin.set_pull(Pull::None);
    slave_cs_pin.set_as_input();

    let mut count: u32 = 0;
    while with_timeout(
        Duration::from_secs(timeout_s as u64),
        slave_cs_pin.wait_for_falling_edge(),
    )
    .await
    .is_ok()
    {
        count = count.saturating_add(1);
    }

    count
}
