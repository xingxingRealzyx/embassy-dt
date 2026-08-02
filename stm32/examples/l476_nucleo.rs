#![no_std]
#![no_main]

//! Nucleo-L476RG：基础板（心跳 LED），设备树生成时钟与全部外设。

use embassy_dt::device_tree;

#[path = "common/app.rs"]
mod app;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

device_tree! {
    name "nucleo-l476rg";
    backend stm32;
    chip "stm32l476rg";
    from "boards/nucleo-l476rg.dts";
}

#[embassy_executor::main]
async fn main(_spawner: embassy_executor::Spawner) {
    let mut board = Board::init(embassy_stm32::init(clock_config()));
    app::heartbeat(&mut board.led0).await;
}
