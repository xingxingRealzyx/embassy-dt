#![no_std]
#![no_main]

//! Nucleo-F072RB：基础板（心跳 LED），设备树生成时钟与全部外设。
//! 第一个 Cortex-M0 芯片家族（F0）。

use embassy_dt::device_tree;

#[path = "common/app.rs"]
mod app;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

device_tree! {
    name "nucleo-f072rb";
    backend stm32;
    chip "stm32f072rb";
    from "boards/nucleo-f072rb.dts";
}

#[embassy_executor::main]
async fn main(_spawner: embassy_executor::Spawner) {
    let mut board = Board::init(embassy_stm32::init(clock_config()));
    app::heartbeat(&mut board.led0).await;
}
