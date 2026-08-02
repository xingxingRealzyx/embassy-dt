#![no_std]
#![no_main]

//! 第五个芯片家族（F1）的驱动闭环：两个 BME280 共享 I2C1。
//! 应用代码与 H723/F411/L476/G474 版本完全相同。

use defmt::*;
use defmt_rtt as _;
use embassy_dt::device_tree;
use embassy_executor::Spawner;
use embassy_time::Timer;
use panic_probe as _;

device_tree! {
    name "bluepill-f103c8";
    backend stm32;
    chip "stm32f103c8";
    from "boards/bluepill-f103.dts";
}

type Bme280Shared = embassy_dt_bme280::Bme280<
    embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice<
        'static,
        embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
        embassy_stm32::i2c::I2c<
            'static,
            embassy_stm32::mode::Async,
            embassy_stm32::i2c::mode::Master,
        >,
    >,
>;

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_stm32::init(clock_config());
    let board = Board::init(p);

    let mut devices = board.init_devices().await.expect("bme280 init failed");

    loop {
        let (t0, h0) = read_both(&mut devices.bme280).await;
        let (t1, h1) = read_both(&mut devices.bme280b).await;
        info!(
            "sensor0: {}.{:02} °C, {}.{:03} %",
            t0 / 100,
            (t0 % 100).abs(),
            h0 / 1000,
            h0 % 1000
        );
        info!(
            "sensor1: {}.{:02} °C, {}.{:03} %",
            t1 / 100,
            (t1 % 100).abs(),
            h1 / 1000,
            h1 % 1000
        );
        Timer::after_millis(1000).await;
    }
}

async fn read_both(bme: &mut Bme280Shared) -> (i32, i32) {
    let t = bme.temperature().await.expect("temp read");
    let h = bme.humidity().await.expect("hum read");
    (t, h)
}
