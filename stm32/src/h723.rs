//! STM32H723 芯片后端。
//!
//! 该模块是芯片相关约定的文档入口；实际代码由 `device_tree!`
//! （`backend stm32;`）生成，引用 `::embassy_stm32`。
//!
//! 已验证的配置示例（Nucleo-H723ZI 常见引脚）：
//!
//! ```rust,ignore
//! device_tree! {
//!     name "nucleo-h723zi";
//!     backend stm32;
//!     bus i2c0: I2c {
//!         periph: "I2C1", scl: "PB8", sda: "PB7",
//!         dma_tx: "DMA1_CH0", dma_rx: "DMA1_CH1", freq: 400_000,
//!     };
//! }
//! ```
//!
//! 说明：
//!
//! - I2C 使用 async 模式，需要一对 DMA 通道（`dma_tx` / `dma_rx`）。
//! - 中断名遵循 H7 风格：`I2C1_EV` / `I2C1_ER`、`DMA1_STREAMn`。
//! - 支持的 bus 类型（当前版本）：`I2c`。`Spi` / `Uart` 在后续版本加入。
