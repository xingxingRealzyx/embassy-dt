# embassy-dt-macros

`embassy-dt` 的过程宏：`device_tree!`。

- 解析 Rust DSL 或 `.dts` / `.dtsi` 文件（含 `#include`、overlay 合并）
- 编译期校验：重复 id、悬空依赖、依赖环
- `backend stm32;` 时生成类型化 `Board`（支持 I2C/SPI/UART/GPIO/
  RNG/ADC/CRC/DAC/PWM/CAN/USB/I2S/QEI/输入捕获/SDMMC 等）

完整文档见 [embassy-dt](https://docs.rs/embassy-dt)。
