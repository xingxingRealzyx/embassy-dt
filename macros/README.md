# embassy-dt-macros

Procedural macros for [`embassy-dt`](https://docs.rs/embassy-dt): the `device_tree!` DSL.

What it provides:

- Parsing of the Rust DSL **and** real `.dts` / `.dtsi` files (recursive `#include`,
  overlay merging, `/delete-node/`, `/delete-property/`) and `.dtb` binaries
- Compile-time validation: duplicate ids, dangling dependencies, dependency cycles,
  unknown node types, missing required properties, conflicting clock styles
- With `backend stm32;`: typed `Board` code generation for I2C/SPI/UART/GPIO/
  RNG/ADC/CRC/DAC/PWM/CAN/USB/I2S/QEI/input capture/SDMMC, plus `clock_config()`
  intent-based clock planning and automatic `bind_interrupts!`
- Error reporting with `file:line:column` locations for `.dts` input

Full documentation: [`embassy-dt`](https://docs.rs/embassy-dt).

