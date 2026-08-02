#![no_std]
#![no_main]

//! 正交解码器（QEI）示例：读取编码器计数与方向。

use defmt::*;
use defmt_rtt as _;
use embassy_dt::device_tree;
use embassy_executor::Spawner;
use embassy_stm32::timer::qei::Direction;
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

    let qei = board.qei0;
    info!("QEI ready");

    loop {
        match qei.read_direction() {
            Direction::Upcounting => info!("count: {} dir: up", qei.count()),
            Direction::Downcounting => info!("count: {} dir: down", qei.count()),
        }
        Timer::after_millis(500).await;
    }
}
