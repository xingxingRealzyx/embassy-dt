#![no_std]
#![no_main]

//! 对照 embassy 官方 `stm32h7/dac.rs` 示例：DAC 波形输出。
//! 官方用 micromath 生成正弦波，这里用三角波保持零依赖。

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

    let mut dac = board.dac0;

    loop {
        for v in 0..=255u8 {
            let x = if v < 128 {
                v.wrapping_mul(2)
            } else {
                255u8.wrapping_sub(v.wrapping_sub(128).wrapping_mul(2))
            };
            dac.set(embassy_stm32::dac::Value::Bit8(x));
        }
    }
}
