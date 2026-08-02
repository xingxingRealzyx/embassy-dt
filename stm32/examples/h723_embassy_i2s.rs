#![no_std]
#![no_main]

//! 对照 embassy 官方 `stm32f4/i2s_dma.rs` 示例：
//! I2S 发送波形（H723 用 SPI3 的 I2S3，引脚来自设备树）。

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

    // 立体声方波表（左 160Hz / 右 240Hz）。
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
