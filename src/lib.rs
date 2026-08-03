#![no_std]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! An async device tree for [Embassy](https://github.com/embassy-rs/embassy).
//!
//! The goal: separate *what the hardware looks like* from *what the application
//! does*. Hardware configuration (buses, pins, devices, dependencies) is written once
//! as a static description; a chip backend translates it into concrete Embassy HAL
//! calls. Application code only programs against the tree, so changing boards or chips
//! does not require changing logic.
//!
//! Three parts:
//!
//! - [`device_tree!`] (from `embassy-dt-macros`): the DSL macro that parses tree
//!   configuration and performs **compile-time validation** (duplicate ids, dangling
//!   dependencies, dependency cycles); declaring `backend stm32;` additionally
//!   generates a typed `Board` structure.
//! - [`TreeDesc`]: the purely static, `no_std` device-tree description model (nodes,
//!   buses, devices, properties, dependencies) with validation and topological sort.
//! - [`init_devices`]: the engine that asynchronously brings devices up in dependency
//!   order, with zero heap allocation.
//!
//! Example (host side):
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
