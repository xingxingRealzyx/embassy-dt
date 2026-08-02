#![no_std]
#![no_main]

//! 对照 embassy 官方 `stm32h7/adc.rs` 示例：ADC 采样。
//! ADC 引脚通过 `gpio ...: Pin` 保留在 Board 中，供 `blocking_read` 使用。

use defmt::*;
use defmt_rtt as _;
use embassy_dt::device_tree;
use embassy_executor::Spawner;
use embassy_stm32::adc::SampleTime;
use embassy_time::Timer;
use panic_probe as _;

#[path = "common/clock.rs"]
mod clock;

device_tree! {
    name "nucleo-h723zi";
    backend stm32;
    chip "stm32h723zg";
    from "boards/nucleo-h723zi.dts";
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_stm32::init(clock::clock_config());
    let board = Board::init(p);

    let mut adc = board.adc0;
    let mut pin = board.adc_in;

    loop {
        // H723 是 adc_v4：采样时间变体为 CYCLES32_5 等。
        let measured = adc.blocking_read(&mut pin, SampleTime::CYCLES32_5);
        info!("measured: {}", measured);
        Timer::after_millis(500).await;
    }
}
