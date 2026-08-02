#![no_std]
#![no_main]

//! 对照 embassy 官方 `stm32h7/spi.rs` 示例：SPI 回环传输。

use defmt::*;
use defmt_rtt as _;
use embassy_dt::device_tree;
use embassy_executor::Spawner;
use embassy_time::Timer;
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

    let mut spi = board.spi0;
    let mut buf = [0u8; 32];
    let mut n = 0u8;

    loop {
        for (i, b) in buf.iter_mut().enumerate() {
            *b = n.wrapping_add(i as u8);
        }
        unwrap!(spi.transfer_in_place(&mut buf).await);
        info!("spi: {:02x}", &buf[..8]);
        n = n.wrapping_add(1);
        Timer::after_millis(1000).await;
    }
}
