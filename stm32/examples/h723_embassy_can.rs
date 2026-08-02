#![no_std]
#![no_main]

//! 对照 embassy 官方 `stm32h7/can.rs` 示例：CAN 发送/接收。

use defmt::*;
use defmt_rtt as _;
use embassy_dt::device_tree;
use embassy_executor::Spawner;
use embassy_stm32::can::frame::Frame;
use embassy_time::Timer;
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

    let mut can = board.can0;
    info!("CAN Configured");

    let mut i = 0u8;
    loop {
        let frame = Frame::new_extended(0x123456F, &[i; 8]).unwrap();
        info!("Writing frame");
        _ = can.write(&frame).await;

        match can.read().await {
            Ok(envelope) => {
                let (rx_frame, _ts) = envelope.parts();
                info!("RX: {:x}", rx_frame.data());
            }
            Err(e) => error!("CAN read error: {:?}", e),
        }

        i = i.wrapping_add(1);
        Timer::after_millis(1000).await;
    }
}
