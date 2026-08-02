#![no_std]
#![no_main]

//! 共享 SPI 总线：两个设备（各自 CS 引脚）共用 SPI4。
//! 硬件：把 MOSI(PE6) 与 MISO(PE5) 短接做回环。

use defmt::*;
use defmt_rtt as _;
use embassy_dt::device_tree;
use embassy_dt::{DeviceError, NodeDesc};
use embassy_executor::Spawner;
use embassy_time::Timer;
use embedded_hal_async::spi::Operation;
use panic_probe as _;

device_tree! {
    name "nucleo-h723zi";
    backend stm32;
    chip "stm32h723zg";
    from "boards/nucleo-h723-spi-shared.dts";
}

/// 极简 SPI 设备驱动（设备树约定：`init(deps..., &NodeDesc)`）。
struct SpiEcho<S> {
    spi: S,
}

impl<S> SpiEcho<S> {
    async fn init(spi: S, _node: &NodeDesc) -> Result<Self, DeviceError> {
        Ok(Self { spi })
    }

    /// 回环读取两个字节（0xAA 命令 + 返回数据）。
    async fn read(&mut self) -> Result<u16, DeviceError>
    where
        S: embedded_hal_async::spi::SpiDevice<u8>,
    {
        let mut buf = [0u8; 2];
        self.spi
            .transaction(&mut [
                Operation::Write(&[0xAA]),
                Operation::Read(&mut buf[1..]),
            ])
            .await
            .map_err(|_| DeviceError::msg("spi echo failed"))?;
        Ok(u16::from_be_bytes(buf))
    }
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_stm32::init(clock_config());
    let board = Board::init(p);

    // 宏生成：两个设备各拿一个 SpiDevice（共享总线 + 各自 CS）。
    let mut devices = board.init_devices().await.expect("spi echo init");

    loop {
        let v0 = devices.echo0.read().await.expect("echo0");
        let v1 = devices.echo1.read().await.expect("echo1");
        info!("echo0: {:#x}  echo1: {:#x}", v0, v1);
        Timer::after_millis(1000).await;
    }
}
