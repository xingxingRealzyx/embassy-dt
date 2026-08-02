#![no_std]
#![no_main]

//! Nucleo-H723ZI：配置来自 `boards/nucleo-h723zi.dts`（include 芯片级
//! `stm32h723.dtsi` 后做板级覆盖）。
//!
//! 交叉检查：
//!
//! ```sh
//! cargo check --offline --target thumbv7em-none-eabihf --example h723_nucleo
//! ```

use embassy_dt::device_tree;

#[path = "common/app.rs"]
mod app;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

device_tree! {
    name "nucleo-h723zi";
    backend stm32;
    chip "stm32h723zg";
    from "boards/nucleo-h723zi.dts";
}

#[embassy_executor::main]
async fn main(_spawner: embassy_executor::Spawner) {
    let mut board = Board::init(embassy_stm32::init(clock_config()));
    app::heartbeat(&mut board.led0).await;
}
