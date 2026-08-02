#![no_std]
#![no_main]

//! 对照 embassy 官方 `stm32h7/usart.rs` 示例：UART 回显。

use defmt::*;
use defmt_rtt as _;
use embassy_dt::device_tree;
use embassy_executor::Spawner;
use panic_probe as _;

device_tree! {
    name "nucleo-h723zi";
    backend stm32;
    chip "stm32h723zg";
    from "boards/nucleo-h723zi.dts";
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_stm32::init(clock_config());
    let board = Board::init(p);

    let mut usart = board.uart0;
    unwrap!(usart.write(b"Hello Embassy World!\r\n").await);
    info!("wrote Hello, starting echo");

    let mut buf = [0u8; 1];
    loop {
        unwrap!(usart.read(&mut buf).await);
        unwrap!(usart.write(&buf).await);
    }
}
