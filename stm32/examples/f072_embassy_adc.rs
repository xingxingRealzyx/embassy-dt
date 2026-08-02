#![no_std]
#![no_main]

//! 对照 embassy 官方 `stm32f0/adc.rs` 示例：ADC 采样。
//! F0 是 adc_v1：`Adc::new(adc, irq)`，时钟走内部 HSI14（无需配置）。

use defmt::*;
use defmt_rtt as _;
use embassy_dt::device_tree;
use embassy_executor::Spawner;
use embassy_stm32::adc::SampleTime;
use embassy_time::Timer;
use panic_probe as _;

device_tree! {
    name "nucleo-f072rb";
    backend stm32;
    chip "stm32f072rb";
    from "boards/nucleo-f072rb.dts";
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_stm32::init(clock_config());
    let board = Board::init(p);

    let mut adc = board.adc0;
    let mut pin = board.adc_in;

    loop {
        let measured = adc.read(&mut pin, SampleTime::CYCLES7_5).await;
        info!("measured: {}", measured);
        Timer::after_millis(500).await;
    }
}
