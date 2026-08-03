# Changelog

All notable changes to this project are documented in this file. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.2.0] - 2026-08-03

### Added

- **G4 clock: PLL P/Q dividers** (`pllp` / `pllq` properties on the `clock`
  node). The official STM32 Motor Control SDK FOC clock (HSE 8 MHz, PLLM=2,
  PLLN=85, PLLP=DIV8 → 42.5 MHz ADC, PLLQ=DIV2, PLLR=DIV2 → 170 MHz SYSCLK)
  can now be expressed exactly, including `adc12 = "pll1_p"`.
- **G4 intent-clock passthrough**: explicit `fdcan` / `adc12` / `adc345` /
  `pllp` / `pllq` properties can be mixed with the intent syntax
  (`system = <170000000>; ...`) and are forwarded to the synthesized clock node.
- **G4 FDCAN kernel clock auto-selection**: when the tree declares a CAN node
  but HSE is disabled and no explicit `fdcan` clock is given, the intent clock
  now selects `FDCANSEL = PCLK1` automatically. Fixes a real-hardware hang
  where FDCAN init spun forever on the default (HSE) kernel clock.
- **Complementary PWM counting modes**: `counting` property on
  `embassy-dt,periph-complementary-pwm` nodes (`edge-aligned-up` (default) /
  `edge-aligned-down` / `center-aligned-1` / `center-aligned-2` /
  `center-aligned-3`), required for FOC motor control.
- **CORDIC peripheral node**: `embassy-dt,periph-cordic` (G4) generates a
  `cordic::Cordic` handle configured as Cos function, 24 iterations, two
  results (q1.31) — the hardware-accelerated sin/cos setup used by FOC loops.

### Fixed

- G4 intent clocks with a CAN node could leave `FDCANSEL` at its reset value
  (HSE) while HSE was disabled, hanging `Board::init` in the FDCAN wait loop
  (found and fixed on real hardware, ATK-DMG474).

### Documentation

- README: document complementary PWM `counting` modes, the CORDIC node, the
  G4 `pllp`/`pllq` dividers and FDCAN auto clock selection; update the
  real-hardware status and known-limitations list.

## [0.1.0] - 2026-08-03

### Core (`embassy-dt`)

- Static device-tree model: `TreeDesc` / `NodeDesc` / properties (string, integer,
  array, boolean)
- Topological sort and structural validation (duplicate ids / dangling dependencies /
  dependency cycles)
- Dependency-ordered asynchronous bring-up engine `init_devices` (zero heap allocation)
- `DeviceError`: device probe / property / node errors (`no_std`, zero allocation)

### Macros (`embassy-dt-macros`)

- `device_tree!` DSL: `bus` / `device` / `gpio` / `periph` / `node` nodes and
  `name` / `backend` / `chip` / `from` declarations
- Compile-time validation: duplicate ids, dangling dependencies, dependency cycles,
  unknown node types, missing required properties, mixing intent and explicit clocks
- **DTS/DTSI files**: recursive `#include` / `/include/`, labels, comments, integer
  expressions, `&label` and `&{/path}` references, `[...]` byte arrays, string
  concatenation, boolean properties, `/delete-node/` / `/delete-property/`, and
  same-label overlay merging
- **DTB binaries**: built-in FDT parser with `phandle` resolution
- Error reporting with `file:line:column` locations for `.dts` input
- Backend code generation (`backend stm32;`):
  - Buses: I2C (including shared), SPI (master / slave / shared), UART
  - GPIO: output / input (including async EXTI) / raw pin ownership
  - Peripherals: RNG, ADC, CRC, DAC, PWM, CAN (FDCAN/bxCAN), USB, QEI, input capture,
    SDMMC, I2S, PWM input, complementary PWM
  - Devices: `driver` property selects the driver type; `init_devices()` injects
    dependencies in order (shared I2C via `I2cDevice`, shared SPI via `SpiDevice` + CS pin)
  - Clocks: `clock` node — v1 explicit enumeration + v2 intent syntax (target
    frequencies only; PLL computed per chip; H7 PLL2 peripheral clocks; automatic
    HSI/HSE/MSI input selection)
  - G4 support: intent clocks (PLLR ≤ 170 MHz, automatic boost above 150 MHz,
    HSI48/CRS for USB, ADC kernel from SYS), `DMA1_CHANNELn` DMA interrupts, usb_v1
    (non-OTG) branch, adc_g4 branch (with `AdcConfig`), and `adc12` / `fdcan` / `clk48`
    mux override properties
  - F1 support: intent clocks (PLL output ≤ 72 MHz, HSI ceiling 64 MHz; USB requires
    PLL = 72/48 MHz; ADC from PCLK2÷adc_pre), AFIO pins default to remap 0 (removes the
    `PwmPin` A-generic ambiguity), `ADC1_2` interrupt binding, bxCAN merged interrupt
    names (`USB_HP_CAN1_TX` etc.), `DMA1_CHANNELn` DMA interrupts
  - F0 support: intent clocks (HSI 8 MHz without ÷2, PLL ≤ 48 MHz; USB from
    HSI48/CRS, independent of PLL), DMA group-vector mapping
    (`DMA1_CHANNEL2_3` / `DMA1_CHANNEL4_5_6_7`), **identical-interrupt handler
    deduplication** (multiple handlers on one `bind_interrupts` line: CEC_CAN
    quad-vector, DMA group channels, I2C1 single EV/ER vector), EXTI grouping
    (EXTI0_1/EXTI2_3/EXTI4_15), `ADC1_COMP`, CRC v3 (with Config), independent USB
    `USB` vector (coexists with CAN)

### STM32 backend (`embassy-dt-stm32`)

- Chips: STM32H723ZG (default), STM32H723VG, STM32F411CE, STM32L476RG,
  STM32G474RE, STM32G474VE, STM32F103C8, STM32F103RE, STM32F072RB
- Board DTS files: Nucleo-H723ZI, custom H723, BlackPill-F411, Nucleo-L476RG,
  Nucleo-G474RE (including the FDCAN overlay: I2C1→I2C2, PB8/PB9 to FDCAN1),
  BluePill-F103C8, Nucleo-F103RB (`/delete-node/` overlay: drops `usb0`/`spi0`, then
  gives PB8/PB9 to CAN1), Nucleo-F072RB (CAN overlay: I2C1→I2C2; USB and CAN
  interrupts are independent and coexist)
- Examples: 44 firmware targets (rewrites of official Embassy examples + driver-loop
  demos)
- **Fixed**: BDMA interrupt name mapping for H7 (`BDMA_CH0` → `BDMA_CHANNEL0`, used by
  D3-domain peripherals such as SPI6)
- **Fixed**: duplicate interrupt bindings for pins in the same EXTI group (e.g. three
  buttons on PE12/PE13/PE14)
- **Fixed**: enable `embassy-stm32/rt` in the backend dependency so interrupt vectors
  (including the time-driver timer ISR) are actually compiled; without it, every
  `Timer::after_*` await hangs. Found during real-hardware bring-up.
- The BME280 driver and its STM32 demo examples are kept in the repository as reference
  code; they are not dependencies of the published backend and not part of the
  crates.io publish chain.

### Drivers (`embassy-dt-bme280`)

- Real BME280 driver: reset / ID check / calibration parameters / integer-compensated
  temperature and humidity
- Unit tests: datasheet temperature vector, self-consistent humidity vectors
  (including saturation clamping)

### Tests & tooling

- trybuild compile-fail tests (8 UI cases)
- STM32 type-rejection script `scripts/compile-fail-stm32.sh`
- Clock-planning unit tests (H7/F4/L4/G4 frequency combinations)
- LICENSE (MIT / Apache-2.0), docs.rs metadata, MSRV measured (core/macros 1.80,
  STM32 backend 1.88)
- English documentation for publishing (root README, backend/macros/driver READMEs)
- Real-hardware verification: ATK-DMG474 (STM32G474VET6) LED heartbeat
