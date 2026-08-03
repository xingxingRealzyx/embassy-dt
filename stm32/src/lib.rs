#![no_std]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! The STM32 backend for [`embassy-dt`](https://docs.rs/embassy-dt).
//!
//! Supported chips (one feature per build):
//!
//! - `stm32h723zg` (default) / `stm32h723vg` (H7)
//! - `stm32f411ce` (F4)
//! - `stm32l476rg` (L4)
//! - `stm32g474re` / `stm32g474ve` (G4)
//! - `stm32f103c8` / `stm32f103re` (F1)
//! - `stm32f072rb` (F0)
//!
//! ## Usage
//!
//! Describe the hardware with `.dts` / `.dtsi` files: chip-level `.dtsi` is shared, and
//! board-level `.dts` overlays the differences. Declaring `backend stm32;` in
//! `device_tree!` generates a typed `Board` structure:
//!
//! ```rust,ignore
//! use embassy_dt::device_tree;
//!
//! device_tree! {
//!     name "my-board";
//!     backend stm32;
//!     chip "stm32h723zg";
//!     from "boards/my-board.dts";
//! }
//!
//! // Application code:
//! // #[embassy_executor::main]
//! // async fn main(_s: Spawner) {
//! //     let board = Board::init(embassy_stm32::init(clock_config()));
//! //     app::heartbeat(&mut board.led0).await;
//! // }
//! ```
//!
//! Pin/peripheral AF compatibility is guaranteed at compile time by the
//! `embassy-stm32` type system: for example, if `PB8` is not a legal I2C1 SCL pin, the
//! code simply does not compile.
//!
//! Supported nodes: `I2c` / `Spi` / `Uart` (async + DMA, including shared buses),
//! `gpio Out/In/Pin` (EXTI async inputs), and peripherals `Rng` / `Adc` / `Crc` /
//! `Dac` / `Pwm` / `Can` / `Usb` / `Qei` / `InputCapture` / `Sdmmc` / `I2s` /
//! `PwmInput` / `ComplementaryPwm`, plus intent-based clock configuration. Complete
//! examples live in the repository `examples/` directory.

pub mod f411;
pub mod h723;
