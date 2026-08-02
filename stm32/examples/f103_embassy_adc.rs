#![no_std]
#![no_main]

//! 对照 embassy 官方 `stm32f1/adc.rs` 示例：ADC 采样。
//! F1 的 ADC 是 adc_f1：`Adc::new(adc)` + ADC1_2 中断，时钟走 PCLK2/adc_pre；
//! 只有异步 `read`（v2+ 才有 `blocking_read`）。

use defmt::*;
use defmt_rtt as _;
use embassy_dt::device_tree;
use embassy_executor::Spawner;
use embassy_stm32::adc::SampleTime;
use embassy_time::Timer;
use panic_probe as _;

device_tree! {
    name "bluepill-f103c8";
    backend stm32;
    chip "stm32f103c8";
    from "boards/bluepill-f103.dts";
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_stm32::init(clock_config());
    let board = Board::init(p);

    let mut adc = board.adc0;
    let mut pin = board.adc_in;

    loop {
        // F1 是 adc_f1：采样时间变体为 CYCLES1_5 / CYCLES7_5 / ... / CYCLES239_5。
        let measured = adc.read(&mut pin, SampleTime::CYCLES7_5).await;
        info!("measured: {}", measured);
        Timer::after_millis(500).await;
    }
}
