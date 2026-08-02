#![no_std]
#![no_main]

//! 与 `h723_driver_bme280` 相同的驱动闭环（F411 BlackPill 的 I2C1）。

use defmt::*;
use defmt_rtt as _;
use embassy_dt::device_tree;
use embassy_executor::Spawner;
use embassy_time::Timer;
use panic_probe as _;

device_tree! {
    name "blackpill-f411ce";
    backend stm32;
    chip "stm32f411ce";
    from "boards/blackpill-f411.dts";
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_stm32::init(clock_config());
    let board = Board::init(p);

    let mut devices = board.init_devices().await.expect("bme280 init failed");

    loop {
        let t = devices.bme280.temperature().await.expect("temp read");
        let h = devices.bme280.humidity().await.expect("hum read");
        info!("temperature: {}.{:02} °C", t / 100, (t % 100).abs());
        info!("humidity: {}.{:03} %", h / 1000, h % 1000);
        Timer::after_millis(1000).await;
    }
}
