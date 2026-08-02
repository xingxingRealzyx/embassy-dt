#![no_std]
#![no_main]

//! 对照 embassy 官方 `stm32h7/rng.rs` 示例：硬件随机数。

use defmt::*;
use defmt_rtt as _;
use embassy_dt::device_tree;
use embassy_executor::Spawner;
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

    let mut rng = board.rng0;
    let mut buf = [0u8; 16];
    unwrap!(rng.async_fill_bytes(&mut buf).await);
    info!("random bytes: {:02x}", buf);
}
