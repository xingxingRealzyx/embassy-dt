#![no_std]
#![no_main]

//! BluePill-F103C8：基础板（心跳 LED），设备树生成时钟与全部外设。

use embassy_dt::device_tree;

#[path = "common/app.rs"]
mod app;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

device_tree! {
    name "bluepill-f103c8";
    backend stm32;
    chip "stm32f103c8";
    from "boards/bluepill-f103.dts";
}

#[embassy_executor::main]
async fn main(_spawner: embassy_executor::Spawner) {
    let mut board = Board::init(embassy_stm32::init(clock_config()));
    app::heartbeat(&mut board.led0).await;
}
