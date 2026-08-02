#![no_std]
#![no_main]

//! 与 `h723_embassy_pwm_input` 相同的应用逻辑（F411：PB2 产生方波，
//! TIM3_CH1 测量）。硬件上把 PB2 与 PA6 用电阻连起来。

use defmt::*;
use defmt_rtt as _;
use embassy_dt::device_tree;
use embassy_executor::Spawner;
use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_time::Timer;
use panic_probe as _;

#[path = "common/clock.rs"]
mod clock;

device_tree! {
    name "blackpill-f411ce";
    backend stm32;
    chip "stm32f411ce";
    from "boards/blackpill-f411.dts";
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_stm32::init(clock::clock_config());
    let board = Board::init(p);

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
