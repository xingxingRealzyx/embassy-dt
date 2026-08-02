#![no_std]
#![no_main]

//! 与 `h723_embassy_i2s` 完全相同的应用逻辑，
//! 只换板级 DTS（F411 用 SPI2 的 I2S2）。

use defmt_rtt as _;
use embassy_dt::device_tree;
use embassy_executor::Spawner;
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

    let mut wavetable = [0u16; 1200];
    for (i, frame) in wavetable.chunks_mut(2).enumerate() {
        frame[0] = ((((i / 150) % 2) * 2048) as i16 - 1024) as u16;
        frame[1] = ((((i / 100) % 2) * 2048) as i16 - 1024) as u16;
    }

    let mut i2s = board.i2s0;
    i2s.start();

    for _ in 0..10 {
        i2s.write(&wavetable).await.ok();
    }

    i2s.stop().await;
}
