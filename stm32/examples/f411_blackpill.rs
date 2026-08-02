#![no_std]
#![no_main]

//! WeAct BlackPill（STM32F411CE）：同一份应用代码 + 独立板级 `.dts`。
//! 证明「一次配置（同一套 API 与 DSL），不同芯片家族也能跑」。
//!
//! 交叉检查（注意与 H723 互斥的 chip feature）：
//!
//! ```sh
//! cargo check --offline --target thumbv7em-none-eabihf \
//!     --no-default-features --features stm32f411ce --example f411_blackpill
//! ```

use embassy_dt::device_tree;

#[path = "common/app.rs"]
mod app;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

device_tree! {
    name "blackpill-f411ce";
    backend stm32;
    chip "stm32f411ce";
    from "boards/blackpill-f411.dts";
}

#[embassy_executor::main]
async fn main(_spawner: embassy_executor::Spawner) {
    let mut board = Board::init(embassy_stm32::init(clock_config()));
    app::heartbeat(&mut board.led0).await;
}
