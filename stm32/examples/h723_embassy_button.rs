#![no_std]
#![no_main]

//! 对照 embassy 官方 `stm32h7/button_exti.rs` 示例：
//! 设备树里给 gpio-in 加 `exti;`，Board 自动生成 `ExtiInput` 与中断绑定。

use defmt::*;
use defmt_rtt as _;
use embassy_dt::device_tree;
use embassy_executor::Spawner;
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

    let mut button = board.btn0;
    info!("Press the USER button...");

    loop {
        button.wait_for_rising_edge().await;
        info!("Pressed!");
        button.wait_for_falling_edge().await;
        info!("Released!");
    }
}
