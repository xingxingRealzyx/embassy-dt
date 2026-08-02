#![no_std]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! `embassy-dt` 的 STM32 后端。
//!
//! 当前支持：
//!
//! - **STM32H723ZG**（`stm32h723zg` feature，默认开启，Nucleo-H723ZI）
//! - **STM32F411CE**（`stm32f411ce` feature，WeAct BlackPill）
//!
//! ## 使用
//!
//! 推荐用 `.dts` / `.dtsi` 文件描述硬件：芯片级 dtsi 复用，板级 dts 做
//! overlay 覆盖。在 `device_tree!` 中声明 `backend stm32;`，宏会额外生成
//! 类型化的 `Board` 结构：
//!
//! ```rust,ignore
//! use embassy_dt::device_tree;
//!
//! device_tree! {
//!     name "my-board";
//!     backend stm32;
//!     from "boards/my-board.dts";
//! }
//!
//! // 应用代码：
//! // #[embassy_executor::main]
//! // async fn main(_s: Spawner) {
//! //     let board = Board::init(embassy_stm32::init(Default::default()));
//! //     app::heartbeat(&mut board.led0).await;
//! // }
//! ```
//!
//! 引脚与外设的 AF 兼容性由 embassy-stm32 的类型系统在编译期保证：
//! 例如 `PB8` 若不是 `I2C1` 的合法 SCL 引脚，代码直接编译不过。
//!
//! 支持的节点：`I2c` / `Spi` / `Uart`（async + DMA）与 `gpio Out/In`。
//! 完整示例见 `examples/`（三块板子共享 `examples/common/app.rs`）。

pub mod f411;
pub mod h723;
