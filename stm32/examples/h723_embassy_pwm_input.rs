#![no_std]
#![no_main]

//! 对照 embassy 官方 `stm32f4/pwm_input.rs` 示例：
//! 用 PB1 产生方波（`gpio ...: Pin` 保留引脚），TIM8_CH1 测量占空比。
//! 硬件上把 PB1 与 PC6 用电阻连起来即可。

use defmt::*;
use defmt_rtt as _;
use embassy_dt::device_tree;
use embassy_executor::Spawner;
use embassy_stm32::gpio::{Level, Output, Speed};
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

    // 信号源：设备树里的裸引脚。
    let mut src = Output::new(board.pwm_src, Level::High, Speed::Low);
    let mut pwm_input = board.pwm_in0;
    pwm_input.enable();

    loop {
        src.toggle();
        Timer::after_millis(300).await;
        let period = pwm_input.get_period_ticks();
        let width = pwm_input.get_width_ticks();
        let duty_cycle = pwm_input.get_duty_cycle();
        info!(
            "period ticks: {} width ticks: {} duty cycle: {}",
            period, width, duty_cycle
        );
    }
}
