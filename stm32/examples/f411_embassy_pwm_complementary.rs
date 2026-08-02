#![no_std]
#![no_main]

//! 对照 embassy 官方 `stm32f4/pwm_complementary.rs` 示例：
//! TIM1 互补 PWM（CH1=PA8 / CH1N=PB13），含死区时间配置。

use defmt::*;
use defmt_rtt as _;
use embassy_dt::device_tree;
use embassy_executor::Spawner;
use embassy_stm32::timer::Channel;
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

    let mut pwm = board.pwm_adv0;
    let max = pwm.get_max_duty();
    pwm.set_dead_time((max / 1024) as u16);
    pwm.enable(Channel::Ch1);

    info!("PWM initialized");
    info!("PWM max duty {}", max);

    loop {
        pwm.set_duty(Channel::Ch1, 0);
        Timer::after_millis(300).await;
        pwm.set_duty(Channel::Ch1, max / 4);
        Timer::after_millis(300).await;
        pwm.set_duty(Channel::Ch1, max / 2);
        Timer::after_millis(300).await;
        pwm.set_duty(Channel::Ch1, max - 1);
        Timer::after_millis(300).await;
    }
}
