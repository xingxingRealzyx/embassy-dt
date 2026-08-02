#![no_std]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! 给 Embassy 的异步设备树（Async Device Tree）。
//!
//! 目标：把「硬件长什么样」与「应用逻辑」分离——硬件配置（总线、引脚、
//! 设备、依赖关系）写成一份静态描述，由 chip 后端翻译成具体的 Embassy HAL
//! 调用；应用代码只面向树编程，换板子/换芯片时不用改逻辑。
//!
//! 三个组成部分：
//!
//! - [`device_tree!`]（来自 `embassy-dt-macros`）：DSL 宏，解析树配置并做
//!   **编译期校验**（重复 id、悬空依赖、依赖环）；声明 `backend stm32;`
//!   时还会生成类型化的 `Board` 结构。
//! - [`TreeDesc`]：纯静态、`no_std` 的设备树描述模型（节点、总线、设备、
//!   属性、依赖），以及校验和拓扑排序。
//! - [`init_devices`]：按依赖序异步上线设备的引擎，零堆分配。
//!
//! 示例（宿主端）：
//!
//! ```rust
//! use embassy_dt::device_tree;
//!
//! device_tree! {
//!     name "demo-board";
//!     bus i2c0: I2c { periph: "I2C1", scl: "PB8", sda: "PB7", freq: 400_000 };
//!     device bme280: Bme280Driver { bus: i2c0, addr: 0x76 };
//! }
//!
//! # fn main() {
//! assert_eq!(TREE.name, "demo-board");
//! assert_eq!(TREE.node("bme280").unwrap().prop("addr"), Some(embassy_dt::Prop::U32(0x76)));
//! # }
//! ```

pub mod init;
pub mod tree;

pub use embassy_dt_macros::device_tree;
pub use init::{init_devices, AsyncDevice, DeviceError, InitError};
pub use tree::{BusKind, NodeDesc, NodeId, NodeKind, Prop, TreeDesc, ValidationError};
