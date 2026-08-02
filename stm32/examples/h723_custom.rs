#![no_std]
#![no_main]

//! 自定义 H723 板：与 `h723_nucleo` 共享同一个应用模块（`common/app.rs`）
//! 和同一份芯片级 `stm32h723.dtsi`，只换了板级 `.dts` —— 应用代码零改动。
//!
//! 交叉检查：
//!
//! ```sh
//! cargo check --offline --target thumbv7em-none-eabihf --example h723_custom
//! ```

use embassy_dt::device_tree;

#[path = "common/app.rs"]
mod app;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

device_tree! {
    name "custom-h723";
    backend stm32;
    chip "stm32h723zg";
    from "boards/custom-h723.dts";
}

#[embassy_executor::main]
async fn main(_spawner: embassy_executor::Spawner) {
    let mut board = Board::init(embassy_stm32::init(clock_config()));
    app::heartbeat(&mut board.led0).await;
}
