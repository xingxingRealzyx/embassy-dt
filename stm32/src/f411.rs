//! STM32F411CE 芯片后端（WeAct BlackPill）。
//!
//! 配置示例见 `boards/blackpill-f411.dts` 与 `examples/f411_blackpill.rs`。
//!
//! 注意：F4 没有 DMAMUX，DMA 通道与外设是固定映射（例如 I2C1 TX/RX 固定
//! 使用 `DMA1_CH1` / `DMA1_CH0`），写错通道会由类型系统直接报错。
