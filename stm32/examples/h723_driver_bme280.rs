#![no_std]
#![no_main]

//! 设备树驱动闭环：BME280 通过设备树声明（`bus`/`addr`/`driver`），
//! 宏生成的 `Board::init_devices()` 按依赖序把 I2C 总线注入驱动。
//!
//! 硬件：把 BME280 接到 Nucleo-H723ZI 的 I2C1（PB8/PB7）。

use defmt::*;
use defmt_rtt as _;
use embassy_dt::device_tree;
use embassy_executor::Spawner;
use embassy_time::Timer;
use panic_probe as _;

device_tree! {
    name "nucleo-h723zi";
    backend stm32;
    chip "stm32h723zg";
    from "boards/nucleo-h723zi.dts";
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_stm32::init(clock_config());
    let board = Board::init(p);

    // 宏生成：按依赖序构造设备（I2C 总线所有权移入驱动）。
    let mut devices = board.init_devices().await.expect("bme280 init failed");

    loop {
        let t = devices.bme280.temperature().await.expect("temp read");
        let h = devices.bme280.humidity().await.expect("hum read");
        info!("temperature: {}.{:02} °C", t / 100, (t % 100).abs());
        info!("humidity: {}.{:03} %", h / 1000, h % 1000);
        Timer::after_millis(1000).await;
    }
}
