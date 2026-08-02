#![no_std]
#![no_main]

//! 对照 embassy 官方 `stm32h7/i2c.rs` 示例，使用设备树风格重写：
//! I2C 从设备树创建，应用只关心地址与数据。

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

const ADDRESS: u8 = 0x5F;
const WHOAMI: u8 = 0x0F;

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("Hello world!");
    let p = embassy_stm32::init(clock::clock_config());
    let board = Board::init(p);

    let mut i2c = board.i2c0;
    let mut data = [0u8; 1];

    match i2c.blocking_write_read(ADDRESS, &[WHOAMI], &mut data) {
        Ok(()) => info!("Whoami: {}", data[0]),
        Err(e) => error!("I2C Error: {:?}", e),
    }
}
