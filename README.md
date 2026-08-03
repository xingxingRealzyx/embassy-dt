# embassy-dt

> An async device tree for [Embassy]: describe your hardware **once**, run it on **every** board.

`embassy-dt` separates *what the hardware looks like* from *what the application does*.
Hardware is described with a device tree — either a Rust DSL or real `.dts` / `.dtsi`
files. A procedural macro validates the tree at compile time and produces a static tree
description; the STM32 backend translates that tree into typed Embassy HAL instances
with interrupt bindings and DMA wiring. Switching boards means changing the
configuration only — application code does not change.

## Highlights

- **Two configuration inputs**: a Rust DSL (`device_tree!`) and real device tree source
  (`.dts` / `.dtsi`) or binary (`.dtb`) files.
- **Compile-time validation**: duplicate ids, dangling dependencies, dependency cycles,
  unknown node types, missing required properties and invalid clock mixes fail at build
  time, with `file:line:column` error locations for `.dts` input.
- **Type-safety as the boundary**: pins, DMA channels and interrupt vectors are checked
  by the Embassy type system. Using `"PA0"` as an `I2C1` SCL pin is a compile error, not
  a runtime surprise.
- **Zero allocation**: the core crate is `no_std`, `#![forbid(unsafe_code)]`, heap-free
  and chip-agnostic; the device tree is `'static` data.
- **Board overlays**: a board `.dts` can `#include` a chip `.dtsi` and override only the
  differences (pins, frequencies, removed nodes).
- **Shared buses**: `shared;` on an I2C/SPI bus generates a `Mutex`-based shared proxy;
  drivers keep implementing plain `embedded-hal-async` traits.
- **Intent-based clocks**: declare target frequencies (`system = <170000000>;`) and the
  macro computes PLL dividers/multipliers for the target chip.

## Repository layout

```text
embassy-dt/
├── src/             Core: TreeDesc / NodeDesc / properties / topo-sort / async init engine
├── macros/          device_tree! proc-macro: DSL + DTS/DTSI parser + validation + backend codegen
├── stm32/           STM32 backend (embassy-dt-stm32): H7 / F4 / L4 / G4 / F1 / F0
└── drivers/         Device drivers following the device-tree init convention (e.g. bme280)
```

The core `embassy-dt` crate has no dependency on any chip HAL.

## Quick start

### 1) `.dts` / `.dtsi` files (recommended)

```rust
use embassy_dt::device_tree;

device_tree! {
    name "nucleo-h723zi";       // optional: defaults to the `model` property of the DTS root
    backend stm32;              // optional: additionally generate the typed Board
    chip "stm32h723zg";         // chip-specific construction differences (ADC/CRC/CAN ...)
    from "boards/nucleo-h723zi.dts";
}
```

The DTS loader supports:

- `#include "xxx.dtsi"` and `/include/ "xxx.dtsi"` (recursive, duplicate-include safe)
- Comments, labels (`i2c0: i2c@40005400 { ... }`), `<...>` integers / `&label` references,
  `<(...)>` integer expressions, `&{/path}` path references, `[...]` byte arrays, string
  concatenation and boolean properties
- **Board overlay merging**: nodes with the same label are merged property-by-property,
  so `board.dts` can include `chip.dtsi` and only describe the differences
- `/delete-node/ &label;` and `/delete-property/ name;`
- The tree name is taken from `model`; all loaded files are tracked by rustc, so editing
  a `.dtsi` triggers a rebuild

`.dtb` binaries are also supported (`from = "board.dtb"`, auto-detected by extension):
an embedded FDT parser resolves `phandle` references into dependencies automatically.
DTB files carry no labels, so node ids are derived from paths (`/i2c@40005400` →
`i2c_40005400`).

```dts
// chip.dtsi — chip-level base configuration (shared by many boards)
/ {
    i2c0: i2c@40005400 {
        compatible = "embassy-dt,bus-i2c";
        periph = "I2C1";
        scl = "PB8"; sda = "PB7";
        dma-tx = "DMA1_CH0"; dma-rx = "DMA1_CH1";
        frequency = <400000>;
    };
};

// board.dts — board-level differences (this is all you change per board)
/dts-v1/;
#include "chip.dtsi"
/ {
    model = "Custom-H723-Board";
    i2c0: i2c@40005400 {
        scl = "PB6";          // override the pin
        frequency = <100000>;
    };
};
```

### 2) Rust DSL

```rust
device_tree! {
    name "demo-board";
    bus i2c0: I2c { periph: "I2C1", scl: "PB8", sda: "PB7", dma_tx: "DMA1_CH0", dma_rx: "DMA1_CH1", freq: 400_000 };
    bus uart0: Uart { periph: "USART1", rx: "PA10", tx: "PA9", dma_tx: "DMA1_CH4", dma_rx: "DMA1_CH5", baud: 115_200 };
    gpio led0: Out { pin: "PC13", level: "high" };
    device bme280: Bme280Driver { bus: i2c0, addr: 0x76 };
}
```

Duplicate ids, dangling dependencies and dependency cycles are compile-time errors.

## Async init engine

`init_devices` probes the nodes of the tree asynchronously in dependency order
(bus → device), with zero heap allocation. Host-side demo (DTS driven):

```sh
cargo run --offline --example async_init
# Host-Demo-Board async bring-up log:
#   ✓ i2c0
#   ✓ uart0
#   ✓ gps
#   ✓ bme280
```

## Device driver loop (v1)

A `device` node selects a driver with the `driver` property; the macro then generates
`Board::init_devices()` which injects buses into drivers in dependency order. Marking a
bus with `shared;` automatically generates an `embassy_sync::Mutex` + shared proxy
(e.g. `I2cDevice`, which is `Clone`), so a single I2C bus can host several devices and
drivers remain plain `embedded-hal` implementations:

```rust
let board = Board::init(p);
let mut devices = board.init_devices().await?;   // buses are injected into devices in dependency order
let t  = devices.bme280.temperature().await?;
let t2 = devices.bme280b.temperature().await?;   // same shared bus
```

Driver contract: the type provides
`async fn init(deps..., &NodeDesc) -> Result<Self, DeviceError>`, with deps passed by
value in device-tree dependency order. Reference implementation:
[`drivers/bme280`](drivers/bme280) (a real BME280 driver: reset / ID check / calibration /
integer-compensated temperature and humidity, with datasheet-example unit tests).
The BME280 driver is **not published** to crates.io, and the STM32 backend does not
depend on it, so the publish chain stays clean. The driver and its STM32 demo examples
live in this repository as reference code; a separate unpublished demo crate may wire
them back up later.

**Shared SPI** works the same way: `shared;` on a SPI bus, each device declares its own
`cs = <&cs-node>`; the macro generates an `SpiDevice` that drives CS low during a
transfer and releases it on completion/cancel. UART is point-to-point and therefore not
shareable.

## STM32 backend (`embassy-dt-stm32`)

### Supported nodes

| Node | Properties | Generated type |
| --- | --- | --- |
| `bus ...: I2c` | `periph` / `scl` / `sda` / `dma_tx` / `dma_rx` / `freq\|frequency` | `i2c::I2c<'static, Async, Master>` |
| `bus ...: Spi` | `periph` / `sck` / `mosi` / `miso` / `dma_tx` / `dma_rx` / `freq\|frequency` | `spi::Spi<'static, Async, Master>` |
| `bus ...: Uart` | `periph` / `rx` / `tx` / `dma_tx` / `dma_rx` / `baud\|baudrate` | `usart::Uart<'static, Async>` |
| `gpio ...: Out` | `pin` / `level` (`high`\|`low`) | `gpio::Output<'static>` |
| `gpio ...: In` | `pin` / `pull` (`up`\|`down`\|`none`) | `gpio::Input<'static>` |
| `gpio ...: Pin` | `pin` | `Peri<'static, PIN>` (raw pin ownership, e.g. for ADC) |
| `periph ...: Rng` | `periph` | `rng::Rng<'static, RNG>` |
| `periph ...: Adc` | `periph` | `adc::Adc<'static, ADC1>` |
| `periph ...: Crc` | `periph` | `crc::Crc<'static>` |
| `periph ...: Dac` | `periph` / `pin` (blocking) | `dac::DacChannel<'static, Blocking>` |
| `periph ...: Pwm` | `periph` / optional `ch1..ch4` / `freq` | `timer::simple_pwm::SimplePwm<'static, TIM2>` |
| `periph ...: Can` | `periph` / `rx` / `tx` | `can::Can<'static>` (H723/G4 FDCAN, F4/F1/F0 bxCAN) |
| `periph ...: Usb` | `periph` / `dp` / `dm` / `ep_out` | `usb::Driver<'static, USB_OTG_HS/FS>` (endpoint buffers auto-generated) |
| `periph ...: Qei` | `periph` / `ch1` / `ch2` | `timer::qei::Qei<'static, TIM3>` (quadrature decoder) |
| `periph ...: InputCapture` | `periph` / optional `ch1..ch4` / `freq` | `timer::input_capture::InputCapture<'static, TIM4>` |
| `periph ...: Sdmmc` | `periph` / `clk` / `cmd` / `d0` | `sdmmc::Sdmmc<'static>` (H723, 1-bit) |
| `periph ...: I2s` | `periph` / `sd` / `ws` / `ck` / `dma` / `buffer` | `i2s::I2S<'static, u16>` (TX-only, DMA buffer auto-generated) |
| `bus ...: Spi` + `mode = "slave"` | extra `cs` | `spi::Spi<'static, Async, Slave>` |

The macro generates `bind_interrupts!` (`I2C1_EV/ER`, `USART1`, `DMA1_STREAMn`, ...)
automatically. Pin alternate functions, DMA compatibility and interrupt bindings are all
guaranteed at compile time by the Embassy type system.

### Supported chips

- `stm32h723zg` (default, Nucleo-H723ZI)
- `stm32h723vg` (ST NUCLEO-H723ZG pin-compatible / DM-MC02, verified on real hardware)
- `stm32f411ce` (WeAct BlackPill; no DMAMUX — fixed DMA channel mapping; the QFP48
  metapac data lacks RNG/DAC/CAN, so its examples cover ADC/CRC/PWM)
- `stm32l476rg` (Nucleo-L476RG; L4 fixed DMA mapping, `DMA1_CHANNELn` interrupts,
  USB from MSI 48 MHz)
- `stm32g474re` / `stm32g474ve` (Nucleo-G474RE / ATK-DMG474, verified on real hardware;
  needs the `single-bank` feature; `DMA1_CHANNELn` interrupts; USB is usb_v1 (non-OTG);
  G4 ADC; FDCAN)
- `stm32f103c8` (BluePill; medium density: no CAN/RNG/DAC; fixed DMA mapping, AFIO pin
  remapping, USB shares an interrupt vector with CAN1, ADC needs the `ADC1_2` interrupt)
- `stm32f103re` (Nucleo-F103RB; high density: bxCAN (`CAN`) and DAC)
- `stm32f072rb` (Nucleo-F072RB; first Cortex-M0; USB has its own vector (`USB`), CAN is
  merged into `CEC_CAN`; DMA/EXTI/I2C use group vectors or single interrupts — the
  generator merges multiple handlers on the same vector into one `bind_interrupts` line)

### Intent-based clocks

The clock node supports an intent syntax: declare the target frequencies and the macro
plans PLL dividers/multipliers for the chip (v1 explicit syntax is also supported, but
the two styles are mutually exclusive):

```dts
clock {
    system = <400000000>;    // target system clock (PLL computed automatically)
    usb = <48000000>;        // USB 48 MHz (optional)
    i2s = <50000000>;        // H7: SPI1/2/3 kernel clock (PLL2_P)
    sdmmc = <25000000>;      // H7: SDMMC clock (PLL2_R)
    // hse = <8000000>;      // optional: external crystal; defaults to the internal HSI
};
```

Clock input selection is automatic: H7 uses HSE when `hse` is present (up to 550 MHz),
otherwise HSI 64 MHz; F4 uses HSE when present, otherwise HSI 16 MHz. G4 supports
170 MHz with automatic boost above 150 MHz, USB from HSI48 and ADC kernel clock from SYS
(the driver divides automatically; `adc12`/`fdcan`/`clk48` muxes can be overridden
explicitly).

### `rt` feature requirement

The STM32 backend enables Embassy's `rt` feature (interrupt vector generation) on
`embassy-stm32` for you. **Do not disable it**: without `rt`, interrupt vectors
(including the time driver's timer ISR) are not compiled and every `Timer::after_*`
await hangs. This was found and fixed during the first real-hardware bring-up.

## One application, six board families

`stm32/examples/common/app.rs` is a single application logic (LED heartbeat) shared by
all boards — each board only differs in its `.dts`:

```sh
cd stm32
cargo check --offline --target thumbv7em-none-eabihf --example h723_nucleo    # Nucleo-H723ZI
cargo check --offline --target thumbv7em-none-eabihf --example h723_custom   # custom H723 board (pin override + USART3)
cargo check --offline --target thumbv7em-none-eabihf \
    --no-default-features --features stm32f411ce --example f411_blackpill    # F411CE (different family)
cargo check --offline --target thumbv7em-none-eabihf \
    --no-default-features --features stm32l476rg --example l476_nucleo       # L476RG
cargo check --offline --target thumbv7em-none-eabihf \
    --no-default-features --features stm32g474re --example g474_nucleo       # G474RE
cargo check --offline --target thumbv7em-none-eabihf \
    --no-default-features --features stm32f103c8 --example f103_bluepill      # F103C8
cargo check --offline --target thumbv7em-none-eabihf \
    --no-default-features --features stm32f103re --example f103_embassy_can  # F103RE (bxCAN)
cargo check --offline --target thumbv7em-none-eabihf \
    --no-default-features --features stm32f072rb --example f072_nucleo       # F072RB (Cortex-M0)
```

Board-overlay demos:

- G474RE (no PD0/PD1 in this package): `nucleo-g474re-can.dts` moves I2C1 to I2C2
  (PA9/PA8) and gives PB8/PB9 to FDCAN1.
- F103 USB shares an interrupt vector with CAN1 (`USB_LP_CAN1_RX0`):
  `nucleo-f103rb.dts` uses `/delete-node/` to drop `usb0` and `spi0` (LD2 occupies PA5)
  and gives PB8/PB9 to CAN1.
- F072 USB (`USB`) and CAN (`CEC_CAN`) are independent vectors and can coexist:
  `nucleo-f072rb-can.dts` only moves I2C1 to I2C2 (PB10/PB11).
- F1 AFIO pins may implement several remap candidates (e.g. PA0 is both
  `AfioRemap<0>` and `<2>` for TIM2_CH1); the generated code explicitly selects the
  default remap 0.
- F0 DMA interrupts are group vectors (`DMA1_CHANNEL4_5_6_7`, ...); handlers on the same
  vector are merged into one `bind_interrupts` line.

## Embassy example rewrites (device-tree style)

`stm32/examples/h723_embassy_*.rs` re-implement the official
`embassy/examples/stm32h7` examples in the device-tree style: all hardware setup lives in
`device_tree!`, and the application logic mirrors the official examples line by line
(I2C, PWM, RNG, ADC, USART, SPI, CAN, DAC, button EXTI, input capture, QEI, USB
CDC-ACM, I2S, USB HID keyboard/mouse, PWM input, complementary PWM, plus G4/F1/F0
variants of CAN/USB/ADC and the BME280 driver loop).

## Real-hardware status

- **ATK-DMG474 (STM32G474VET6)**: verified — LED heartbeat on real hardware
  (`embassy-dt-dmg474` project, 170 MHz, HSI16-based PLL). This bring-up caught and
  fixed the missing `rt` feature described above.
- **DM-MC02 (STM32H723VGT6)**: project ready and compiling; hardware bring-up pending
  (the first probe/board combination failed at the SWD physical layer and was set aside).
- The remaining STM32 examples are cross-compiled only and have not been flashed to
  hardware yet.

## Publishing to crates.io

Status: independent git repository initialized (MIT / Apache-2.0), MSRV measured:
core + macros **1.80**, STM32 backend **1.88** (dependency chain requires edition2024,
heapless 0.9, embassy-stm32 build-script let-chains).

Publishing order (there are inter-crate dependencies):

```sh
cargo publish -p embassy-dt-macros   # macros first
cargo publish -p embassy-dt          # core (depends on the macros)
cargo publish -p embassy-dt-stm32    # STM32 backend (chip HAL cannot build on the host:
                                     # use cargo publish --no-verify)
```

`embassy-dt-bme280` (drivers/bme280) is an optional demo driver and is **not part of
the publish chain**.

Local package verification:

```sh
cargo package --offline -p embassy-dt-macros          # full verification
cargo package --offline --no-verify -p embassy-dt     # --no-verify until macros are published
cargo package --offline --no-verify -p embassy-dt-stm32
```

## Relationship to the ecosystem

| Crate | What it does | Relationship |
| --- | --- | --- |
| `fdt-rs` / `dtoolkit` | parse DTB/FDT binaries | future DTB import data sources; not reimplemented |
| `embedded-hal-async` | async bus/IO traits | Board handles implement these traits; drivers work unchanged |
| `embassy-supervisor` | task lifecycle graphs | different layer: tasks vs hardware; composable |
| `rdrive` | dynamic driver management (heap + dynamic dispatch) | contrast: this project uses compile-time static dispatch, zero overhead |

## Design decisions

1. **Configuration is the single source of truth**: `device_tree!` generates both the
   static `TREE` data (runtime introspection, future overlays) and the typed `Board`
   (backend codegen).
2. **Compile-time binding first**: pin types are the safety boundary; the tree compiles
   into typed handles, not `dyn` lookup tables. Runtime DTB parsing stays an optional
   dynamic path.
3. **Zero allocation**: the tree is `'static` data; the bring-up engine is heap-free.
4. **Handles implement `embedded-hal-async`**: no new driver traits are invented.
5. **Backends are separated from the core**: the core depends on no chip HAL; the
   backend owns the "pin name → HAL peripheral" mapping and interrupt bindings.
6. **Fail early**: cycles, duplicate ids and dangling dependencies are compile-time
   errors in the macro; pin/DMA mistakes are rejected by the type system.

## Known limitations / roadmap

- DTS preprocessor (`#define` / `#ifdef`) is not implemented (`#include` works)
- Device-node driver handle generation is ready for the driver ecosystem to plug in
- Multi-core / interrupt-level executor and power-management orchestration belong to
  `embassy-supervisor` and are composable
- Peripherals not yet covered: ETH, SAI/I2S, LTDC/LCD/DSI, QSPI/OSPI/FMC, CORDIC/FMAC,
  crypto (AES/CRYP/HASH/PKA), advanced-timer modes (complementary PWM / one-pulse),
  SDMMC 4/8-bit, ADC/DAC async DMA modes, CAN-FD advanced bit timing, I2C slave
  (not available in embassy-stm32 0.6). The construction is table-driven, so these can
  be added item by item.

## Verification

```sh
# Host side: core + macros (incl. DTS parse/overlay tests) + doc tests
cargo test --offline
cargo test --offline -p embassy-dt-macros

# Compile-fail UI tests: duplicate id / dangling dep / cycle / unknown type / missing chip / missing prop
cargo test --offline -p embassy-dt --test trybuild

# STM32 type-system rejection tests: wrong pins must fail at compile time
./scripts/compile-fail-stm32.sh

# Host-side DTS demos
cargo run --offline --example demo_dts
cargo run --offline --example async_init

# STM32 firmware cross-check (needs the thumbv7em-none-eabihf target)
cd stm32 && cargo check --offline --target thumbv7em-none-eabihf --example h723_nucleo
```

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

[Embassy]: https://github.com/embassy-rs/embassy
