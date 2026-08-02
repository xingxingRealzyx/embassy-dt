#![no_std]
#![no_main]

//! 对照 embassy 官方 `stm32f4/input_capture.rs` 风格：
//! 输入捕获测脉冲边沿时间。

use defmt::*;
use defmt_rtt as _;
use embassy_dt::device_tree;
use embassy_executor::Spawner;
use embassy_stm32::timer::Channel;
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

    let mut ic = board.ic0;
    let ch = Channel::Ch4;

    loop {
        let value = ic.wait_for_rising_edge(ch).await;
        info!("captured: {}", value);
    }
}
