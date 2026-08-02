#![no_std]
#![no_main]

//! 对照 embassy 官方 `stm32g4/adc.rs` 示例：ADC 采样。
//! G4 的 ADC 是专用 adc_g4：`Adc::new(adc, AdcConfig)`，内核时钟走 SYS。

use defmt::*;
use defmt_rtt as _;
use embassy_dt::device_tree;
use embassy_executor::Spawner;
use embassy_stm32::adc::SampleTime;
use embassy_time::Timer;
use panic_probe as _;

device_tree! {
    name "nucleo-g474re";
    backend stm32;
    chip "stm32g474re";
    from "boards/nucleo-g474re.dts";
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_stm32::init(clock_config());
    let board = Board::init(p);

    let mut adc = board.adc0;
    let mut pin = board.adc_in;

    loop {
        // G4 是 adc_g4：采样时间变体为 CYCLES2_5 等（v3 风格）。
        let measured = adc.blocking_read(&mut pin, SampleTime::CYCLES12_5);
        info!("measured: {}", measured);
        Timer::after_millis(500).await;
    }
}
