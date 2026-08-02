# Changelog

本仓库采用 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/) 风格。
尚未发布到 crates.io；所有变更都在 `[Unreleased]` 中累积。

## [Unreleased]

### 核心（embassy-dt）

- 静态设备树模型：`TreeDesc` / `NodeDesc` / 属性（字符串、数值、数组、布尔）
- 拓扑排序与结构校验（重复 id / 悬空依赖 / 依赖环）
- 依赖序异步上线引擎 `init_devices`（零堆分配）
- `DeviceError`：设备 probe / 属性 / 节点错误（no_std 零分配）

### 宏（embassy-dt-macros）

- `device_tree!` DSL：`bus` / `device` / `gpio` / `periph` / `node` 节点，
  `name` / `backend` / `chip` / `from` 声明
- 编译期校验：重复 id、悬空依赖、依赖环、未知类型、缺必填属性、
  意图式与显式时钟混用
- **DTS/DTSI 文件**：`#include` / `/include/` 递归、标签、注释、表达式、
  `&label` 与 `&{/path}` 引用、`[...]` 字节、字符串拼接、布尔属性、
  `/delete-node/` / `/delete-property/`、同标签 overlay 合并
- **DTB 二进制**：内置 FDT 解析器 + phandle 解析
- 错误定位：DTS 错误带 文件:行:列
- 后端代码生成（`backend stm32;`）：
  - 总线：I2C（含共享）、SPI（主/从/共享）、UART
  - GPIO：输出 / 输入（含 EXTI 异步）/ 裸引脚所有权
  - 外设：RNG、ADC、CRC、DAC、PWM、CAN（FDCAN/bxCAN）、USB、
    QEI、输入捕获、SDMMC、I2S、PWM 输入、互补 PWM
  - 设备：`driver` 属性指定类型，`init_devices()` 按依赖序注入
    （共享 I2C 用 `I2cDevice`、共享 SPI 用 `SpiDevice` + CS 引脚）
  - 时钟：`clock` 节点——v1 显式枚举 + v2 意图式（只写目标频率，
    按芯片自动计算 PLL；H7 PLL2 外设时钟；输入源 HSI/HSE/MSI 自动选择）
  - G4 支持：意图式时钟（PLLR ≤ 170 MHz、>150 MHz 自动 boost、
    HSI48/CRS 供 USB、ADC 内核走 SYS）、DMA 中断 `DMA1_CHANNELn`、
    usb_v1（非 OTG）分支、adc_g4（带 `AdcConfig`）分支、
    `adc12`/`fdcan`/`clk48` mux 覆盖属性
  - F1 支持：意图式时钟（PLL 直接输出 ≤ 72 MHz，HSI 上限 64 MHz；
    USB 要求 PLL=72/48 MHz；ADC 走 PCLK2÷adc_pre）、
    AFIO 引脚默认 remap 0（消除 `PwmPin` 的 A 泛型歧义）、
    ADC1_2 中断绑定、bxCAN 合并中断名（`USB_HP_CAN1_TX` 等）、
    DMA 中断 `DMA1_CHANNELn`

### STM32 后端（embassy-dt-stm32）

- 芯片：STM32H723ZG（默认）、STM32F411CE、STM32L476RG、STM32G474RE、
  STM32F103C8、STM32F103RE
- 板级 DTS：Nucleo-H723ZI、自定义 H723、BlackPill-F411、Nucleo-L476RG、
  Nucleo-G474RE（含 FDCAN 专用 overlay：I2C1→I2C2，PB8/PB9 让给 FDCAN1）、
  BluePill-F103C8、Nucleo-F103RB（`/delete-node/` overlay：删 usb0/spi0
  后把 PB8/PB9 给 CAN1）
- 示例：39 个固件（对照 embassy 官方示例重写 + 驱动闭环演示）

### 驱动（embassy-dt-bme280）

- 真实 BME280 驱动：reset / ID 校验 / 校准参数 / 温湿度整数补偿
- 单元测试：datasheet 温度示例、自洽湿度向量（含饱和 clamp）

### 测试与工程

- trybuild 编译失败测试（8 个 UI 用例）
- STM32 类型拦截脚本 `scripts/compile-fail-stm32.sh`
- 时钟算法单元测试（H7/F4/L4 各频率组合）
- LICENSE（MIT / Apache-2.0）、docs.rs metadata、MSRV 实测
  （核心/宏 1.80，STM32 后端 1.88）
