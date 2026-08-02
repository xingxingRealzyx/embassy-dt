# embassy-dt

> 给 Embassy 的异步设备树：**硬件配置写一次，固件到处跑。**

`embassy-dt` 把「硬件长什么样」与「应用逻辑」分离：硬件用设备树描述
（Rust DSL 或 **`.dts` / `.dtsi` 文件**），宏在编译期校验并生成静态树
描述；STM32 后端把树翻译成类型化的 Embassy HAL 实例。换板子只改配置，
应用代码不动。

## 仓库结构

```text
embassy-dt/
├── src/            核心：TreeDesc / 节点 / 属性 / 拓扑排序 / 异步上线引擎
├── macros/         device_tree! 过程宏：DSL + DTS/DTSI 解析 + 编译期校验 + 后端代码生成
├── stm32/          STM32 后端（embassy-dt-stm32）：H7/F4/L4/G4/F1 五大系列
├── examples/       宿主端 demo（DSL 与 DTS 两种入口）
```

核心 `embassy-dt` 是纯 `no_std`、零堆分配、`#![forbid(unsafe_code)]`，
不依赖任何芯片 HAL。

## 两种配置入口

### 1) `.dts` / `.dtsi` 文件（推荐）

```rust
use embassy_dt::device_tree;

device_tree! {
    name "nucleo-h723zi";       // 省略时自动取 DTS 根节点的 model
    backend stm32;              // 额外生成类型化 Board
    chip "stm32h723zg";         // 芯片相关外设（ADC/CRC/CAN）的构造差异
    from "boards/nucleo-h723zi.dts";
}
```

`from` 支持：

- `#include "xxx.dtsi"` 与 `/include/ "xxx.dtsi"`（相对路径递归，防重复 include）
- 注释、标签（`i2c0: i2c@40005400 { ... }`）、`<...>` 数值/`&label` 引用、
  `<(...)>` 整数表达式、`&{/path}` 路径引用、`[...]` 字节串、字符串拼接、布尔属性
- **板级 overlay 合并**：同标签节点按属性覆盖——`board.dts` include
  `chip.dtsi` 后只需写差异（换引脚、换频率）
- `/delete-node/ &label;` 与 `/delete-property/ name;`
- 树名自动取自 `model`；所有加载的文件都被 rustc 跟踪（改 `.dtsi` 会触发
  重新编译）

也支持 **`.dtb` 二进制**（`from = "board.dtb"`，按后缀自动识别）：内置
FDT 解析器，`phandle` 引用自动解析为依赖；DTB 不保留标签，节点 id 取路径
（`/i2c@40005400` → `i2c_40005400`）。

```dts
// chip.dtsi —— 芯片级基础配置（多板复用）
/ {
    i2c0: i2c@40005400 {
        compatible = "embassy-dt,bus-i2c";
        periph = "I2C1";
        scl = "PB8"; sda = "PB7";
        dma-tx = "DMA1_CH0"; dma-rx = "DMA1_CH1";
        frequency = <400000>;
    };
};

// board.dts —— 板级差异（换板只改这里）
/dts-v1/;
#include "chip.dtsi"
/ {
    model = "Custom-H723-Board";
    i2c0: i2c@40005400 {
        scl = "PB6";          // 覆盖引脚
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

重复 id、悬空依赖、依赖环都会在**编译期**报错。

## 异步上线引擎

`init_devices` 按依赖序异步 probe 树中的节点（总线 → 设备），零堆分配；
宿主端演示（DTS 驱动）：

```sh
cargo run --offline --example async_init
# Host-Demo-Board 的异步上线日志:
#   ✓ i2c0
#   ✓ uart0
#   ✓ gps
#   ✓ bme280
```

## 设备驱动闭环（v1）

`device` 节点通过 `driver` 属性指定驱动类型后，宏生成 `Board::init_devices()`：
按依赖序把总线注入驱动。总线加 `shared;` 属性后自动生成
`embassy_sync::Mutex` + `I2cDevice` 共享代理（Clone），一条 I2C 可以挂
多个设备，驱动无需改动（仍然只是 `impl I2c`）：

```rust
// 板级 DTS：
// i2c0: i2c@... { ...; shared; };
// bme280: bme@76 {
//     compatible = "bosch,bme280";
//     bus = <&i2c0>;
//     addr = <0x76>;
//     driver = "embassy_dt_bme280::Bme280<...I2cDevice<...>>";
// };
// bme280b: bme@77 { ...; addr = <0x77>; driver = "同类型"; };
// };

let board = Board::init(p);
let mut devices = board.init_devices().await?;   // 总线按依赖序注入设备
let t = devices.bme280.temperature().await?;
let t2 = devices.bme280b.temperature().await?;   // 共享同一总线
```

驱动约定：类型提供
`async fn init(deps..., &NodeDesc) -> Result<Self, DeviceError>`，deps 按
设备树依赖顺序按值传入。参考实现：[`drivers/bme280`](drivers/bme280)
（真实 BME280 驱动：reset/ID 校验/校准参数/温湿度补偿，带 datasheet
示例值的单元测试）。

**SPI 共享**：`bus ...: Spi` 加 `shared;` 后，设备通过 `cs = <&引脚节点>`
声明自己的片选，宏自动生成 `SpiDevice`（传输期间自动拉低 CS，取消/完成
自动拉高）：

```dts
spi4: spi@... { ...; shared; };
cs0: cs@0 { compatible = "embassy-dt,gpio-pin"; pin = "PE4"; };
echo0: echo@0 {
    bus = <&spi4>;
    cs = <&cs0>;
    driver = "SpiEcho<SpiDevice<...>>";
};
```

参考实现：`h723_embassy_spi_shared`（SPI4 + 两个 CS 设备的回环驱动）。
UART 是点对点总线，共享场景无意义，不支持。

## STM32 后端（embassy-dt-stm32）

支持节点：

| 节点 | 属性 | 生成的类型 |
| --- | --- | --- |
| `bus ...: I2c` | `periph` / `scl` / `sda` / `dma_tx` / `dma_rx` / `freq\|frequency` | `i2c::I2c<'static, Async, Master>` |
| `bus ...: Spi` | `periph` / `sck` / `mosi` / `miso` / `dma_tx` / `dma_rx` / `freq\|frequency` | `spi::Spi<'static, Async, Master>` |
| `bus ...: Uart` | `periph` / `rx` / `tx` / `dma_tx` / `dma_rx` / `baud\|baudrate` | `usart::Uart<'static, Async>` |
| `gpio ...: Out` | `pin` / `level`(high\|low) | `gpio::Output<'static>` |
| `gpio ...: In` | `pin` / `pull`(up\|down\|none) | `gpio::Input<'static>` |
| `periph ...: Rng` | `periph` | `rng::Rng<'static, RNG>` |
| `periph ...: Adc` | `periph` | `adc::Adc<'static, ADC1>` |
| `periph ...: Crc` | `periph` | `crc::Crc<'static>` |
| `periph ...: Dac` | `periph` / `pin`（阻塞模式） | `dac::DacChannel<'static, Blocking>` |
| `periph ...: Pwm` | `periph` / `ch1..ch4` 可选 / `freq` | `timer::simple_pwm::SimplePwm<'static, TIM2>` |
| `periph ...: Can` | `periph` / `rx` / `tx` | `can::Can<'static>`（H723 FDCAN / F4 bxCAN） |
| `periph ...: Usb` | `periph` / `dp` / `dm` / `ep_out` | `usb::Driver<'static, USB_OTG_HS/FS>`（自动生成端点缓冲） |
| `periph ...: Qei` | `periph` / `ch1` / `ch2` | `timer::qei::Qei<'static, TIM3>`（正交解码） |
| `periph ...: InputCapture` | `periph` / `ch1..ch4` 可选 / `freq` | `timer::input_capture::InputCapture<'static, TIM4>` |
| `periph ...: Sdmmc` | `periph` / `clk` / `cmd` / `d0` | `sdmmc::Sdmmc<'static>`（H723，1-bit） |
| `periph ...: I2s` | `periph` / `sd` / `ws` / `ck` / `dma` / `buffer` | `i2s::I2S<'static, u16>`（TX-only，自动生成 DMA 缓冲） |
| `bus ...: Spi` + `mode = "slave"` | 额外 `cs` | `spi::Spi<'static, Async, Slave>` |

宏自动生成 `bind_interrupts!`（`I2C1_EV/ER`、`USART1`、`DMA1_STREAMn`）。
引脚 AF、DMA 兼容性、中断绑定全部由 embassy-stm32 类型系统编译期保证：
把 `scl` 改成 `"PA0"` 会直接报 `PA0: SclPin<I2C1>` 不满足。

`chip "..."` 声明用于芯片相关的构造差异：H723（`adc_v3`/CRC 带 Config/
FDCAN）与 F411（`adc_v2`/CRC 无 Config/bxCAN）会自动生成不同的代码。
外设节点之间如果发生引脚冲突（比如两个节点都用 PA0），会被类型系统在
编译期直接拦住。

已支持的芯片：

- `stm32h723zg`（默认，Nucleo-H723ZI）
- `stm32f411ce`（WeAct BlackPill；无 DMAMUX，DMA 通道固定映射；
  该封装的 metapac 数据不含 RNG/DAC/CAN，因此示例只用到 ADC/CRC/PWM）
- `stm32l476rg`（Nucleo-L476RG；低功耗系列，L4 的 DMA 通道固定映射、
  中断名为 `DMA1_CHANNELn`、USB 时钟用 MSI 48 MHz）
- `stm32g474re`（Nucleo-G474RE；需要 `single-bank` 特性；DMA 中断同样叫
  `DMA1_CHANNELn`；USB 是 usb_v1 非 OTG：`Driver::new` + HSI48/CRS；
  ADC 是专用 adc_g4（`Adc::new(adc, AdcConfig)`），FDCAN 与 H723 同路径）
- `stm32f103c8`（BluePill；中容量：无 CAN/RNG/DAC，DMA 通道固定映射、
  AFIO 引脚重映射、USB 与 CAN1 共享中断向量、ADC 需要 ADC1_2 中断）
- `stm32f103re`（Nucleo-F103RB；高容量：有 bxCAN（外设名 `CAN`）与 DAC）

意图式时钟按芯片自动规划：H7（HSI/HSE + PLL1/PLL2）、F4（HSE/HSI +
PLL）、L4（HSI/HSE + PLL，系统走 PLLR ≤ 80 MHz）、G4（HSI/HSE + PLL，
系统走 PLLR ≤ 170 MHz，>150 MHz 自动开 boost；USB 用 HSI48，ADC 内核
时钟走 SYS 由驱动自动分频；`adc12`/`fdcan`/`clk48` 可显式覆盖 mux）、
F1（HSI 8M÷2 或 HSE，PLLMUL 2–16 直接输出 SYSCLK ≤ 72 MHz；USB 只能
在 PLL=72/48 MHz 时才有 48 MHz，因此请求 USB 时 SYSCLK 必须是 72 MHz
（需 HSE）或 48 MHz；ADC 时钟 = PCLK2÷adc_pre ≤ 14 MHz）。

## 五块板子，一份应用代码

`stm32/examples/common/app.rs` 是唯一一份应用逻辑（心跳 LED），五块板子
各自只有 `.dts` 差异：

```sh
cd stm32
cargo check --offline --target thumbv7em-none-eabihf --example h723_nucleo    # Nucleo-H723ZI
cargo check --offline --target thumbv7em-none-eabihf --example h723_custom   # 自定义 H723 板（覆盖引脚 + USART3）
cargo check --offline --target thumbv7em-none-eabihf \
    --no-default-features --features stm32f411ce --example f411_blackpill    # F411CE（换芯片家族）
cargo check --offline --target thumbv7em-none-eabihf \
    --no-default-features --features stm32l476rg --example l476_nucleo       # L476RG（换芯片家族）
cargo check --offline --target thumbv7em-none-eabihf \
    --no-default-features --features stm32g474re --example g474_nucleo       # G474RE（换芯片家族）
cargo check --offline --target thumbv7em-none-eabihf \
    --no-default-features --features stm32f103c8 --example f103_bluepill      # F103C8（换芯片家族）
cargo check --offline --target thumbv7em-none-eabihf \
    --no-default-features --features stm32f103re --example f103_embassy_can  # F103RE（bxCAN 示例）
```

F1 的两个「板级 overlay」演示：

- G474RE 封装没有 PD0/PD1，`boards/nucleo-g474re-can.dts` 把 I2C1 移到
  I2C2（PA9/PA8），把 PB8/PB9 让给 FDCAN1；
- F103 的 USB 与 CAN1 共享中断向量（`USB_LP_CAN1_RX0`），
  `boards/nucleo-f103rb.dts` 用 `/delete-node/` 删掉 usb0 和 spi0
  （LD2 占用 PA5），把 PB8/PB9 给 CAN1；
- F1 的 AFIO 引脚可能同时实现多个 remap 候选（如 PA0 同时是 TIM2_CH1 的
  `AfioRemap<0>` 和 `<2>`），生成代码显式选择默认 remap 0。

## 对照 embassy 官方示例（设备树风格重写）

`stm32/examples/h723_embassy_*.rs` 是官方 `embassy/examples/stm32h7` 的
设备树风格重写：硬件初始化全部收敛在 `device_tree!` 里，应用逻辑与官方
示例逐行对应：

| 示例 | 对照官方 | 使用的 Board 字段 |
| --- | --- | --- |
| `h723_embassy_i2c` | `i2c.rs` | `board.i2c0` |
| `h723_embassy_pwm` | `pwm.rs` | `board.pwm0` |
| `h723_embassy_rng` | `rng.rs` | `board.rng0` |
| `h723_embassy_adc` | `adc.rs` | `board.adc0` + `board.adc_in`（`gpio ...: Pin` 保留原始引脚） |
| `h723_embassy_usart` | `usart.rs` | `board.uart0` |
| `h723_embassy_spi` | `spi.rs` | `board.spi0` |
| `h723_embassy_can` | `can.rs` | `board.can0` |
| `h723_embassy_dac` | `dac.rs` | `board.dac0` |
| `h723_embassy_button` | `button_exti.rs` | `board.btn0`（dts 里 `exti;` 自动生成 `ExtiInput`） |
| `h723_embassy_input_capture` | `stm32f4/input_capture.rs` | `board.ic0` |
| `h723_embassy_qei` | 正交解码 | `board.qei0` |
| `h723_embassy_usb_serial` | `usb_serial.rs` | `board.usb0`（USB CDC-ACM 串口） |
| `f411_embassy_usb_serial` | 同上（换芯片） | `board.usb0` |
| `h723_embassy_i2s` / `f411_embassy_i2s` | `stm32f4/i2s_dma.rs` | `board.i2s0`（H723 I2S3 / F411 I2S2） |
| `h723_embassy_usb_hid_keyboard` / `f411_embassy_usb_hid_keyboard` | `stm32f4/usb_hid_keyboard.rs` | `board.usb0` + `board.btn0` |
| `h723_embassy_pwm_input` / `f411_embassy_pwm_input` | `stm32f4/pwm_input.rs` | `board.pwm_in0` + `board.pwm_src`（`gpio ...: Pin` 做信号源） |
| `f411_embassy_pwm_complementary` | `stm32f4/pwm_complementary.rs` | `board.pwm_adv0`（TIM1 互补 PWM） |
| `h723_embassy_usb_hid_mouse` / `f411_embassy_usb_hid_mouse` | `stm32f4/usb_hid_mouse.rs` | `board.usb0` |
| `g474_embassy_can` | `stm32g4/can.rs` | `board.can0`（FDCAN + I2C overlay） |
| `g474_embassy_usb_serial` | `stm32g4/usb_serial.rs` | `board.usb0`（usb_v1 + HSI48/CRS） |
| `g474_embassy_adc` | `stm32g4/adc.rs` | `board.adc0` + `board.adc_in` |
| `g474_driver_bme280` | 驱动闭环 | `board.init_devices()`（与 H723/F411/L476 同一份应用） |
| `f103_embassy_can` | `stm32f1/can.rs` | `board.can0`（bxCAN，F103RE + `/delete-node/` overlay） |
| `f103_embassy_usb_serial` | `stm32f1/usb_serial.rs` | `board.usb0`（usb_v1，PLL 72M÷1.5 派生 48M） |
| `f103_embassy_adc` | `stm32f1/adc.rs` | `board.adc0` + `board.adc_in`（async `read`，无 blocking） |
| `f103_driver_bme280` | 驱动闭环 | `board.init_devices()`（F1 固定 DMA 映射 I2C1=CH6/7） |

```sh
cd stm32
# 编译全部 H723 示例
cargo check --offline --target thumbv7em-none-eabihf --examples --features stm32h723zg
# F411 USB 串口
cargo check --offline --target thumbv7em-none-eabihf \
    --no-default-features --features stm32f411ce --example f411_embassy_usb_serial
```

其中 `gpio ...: Pin` 是「裸引脚所有权」节点：Board 字段类型就是
`Peri<'static, PB0>`，用于 ADC 采样这类需要把引脚交给驱动 API 的场景；
`gpio ...: In` 加 `exti;` 属性后自动变成异步 `ExtiInput`（含中断绑定）。

### 真机运行注意（时钟配置）

时钟配置已经设备树化，且支持**意图式**写法：只声明目标频率，宏根据
芯片的 PLL VCO 范围自动计算分频/倍频，生成 `clock_config()` 函数
（应用直接 `init(clock_config())`）：

```dts
clock {
    system = <400000000>;    // 目标系统时钟（宏自动算 PLL/分频）
    usb = <48000000>;        // USB 48 MHz（可选）
    i2s = <50000000>;        // H7：SPI1/2/3 内核时钟（PLL2_P）
    sdmmc = <25000000>;      // H7：SDMMC 时钟（PLL2_R）
    // hse = <8000000>;      // 可选：外部晶振；缺省用内部 HSI
};
```

输入源自动选择：H7 有 `hse` 属性时用外部晶振（可跑到 550 MHz），否则用
HSI 64 MHz；F4 有 `hse` 时用晶振，否则用 HSI 16 MHz（无晶振板子可用）。
H7 外设时钟自动规划 PLL2（P 输出给 I2S/SPI/ADC，R 输出给 SDMMC，
共用一个 VCO；`i2s` 与 `adc` 频率不同会编译报错）。

也保留 v1 显式写法（`source`/`pll1`/`pll`/`sys`/`ahb`/`apb*`/`usb`/
`clk48`/`voltage`），意图式与显式互斥（混用会编译报错）。

当前示例：H723（HSI + PLL1 → 400 MHz，HSI48 供 USB）、F411
（25 MHz HSE → 96 MHz + PLLQ 48 MHz——F4 无 HSI48，100 MHz 系统无法
同时满足 48 MHz USB，宏会提示改用 96 MHz；顺带修复了之前 F411 示例
168 MHz 超频的问题）。不同板子改 DTS 即可，应用代码里没有时钟代码。

目前全部示例只做过交叉编译验证，**尚未在真实硬件上烧录测试**。

### 错误定位

DTS 解析错误会带 文件:行:列：

```text
error: failed to load `boards/nucleo-h723zi.dts`:
  dts: .../nucleo-h723zi.dts:9:19: unexpected token after `BROKEN` in node `led@0`
```

## 发布到 crates.io

工程状态：已初始化独立 git 仓库（含 LICENSE-MIT / LICENSE-APACHE），
MSRV 已实测：核心与宏 **1.80**，STM32 后端 **1.88**（依赖链要求：
edition2024、heapless 0.9、embassy-stm32 build.rs let-chains）。

发布顺序（有依赖关系）：

```sh
cargo publish -p embassy-dt-macros   # 先发宏
cargo publish -p embassy-dt          # 核心（依赖宏）
cargo publish -p embassy-dt-stm32    # STM32 后端（芯片 HAL 无法在宿主机构建，
                                     # 需要 cargo publish --no-verify）
```

本地验证打包：

```sh
cargo package --offline -p embassy-dt-macros   # 可完整验证
cargo package --offline --no-verify -p embassy-dt      # 宏未发布前需 --no-verify
cargo package --offline --no-verify -p embassy-dt-stm32
```

## 与现有生态的关系

| crate | 做什么 | 与本项目的关系 |
| --- | --- | --- |
| `fdt-rs` / `dtoolkit` | 解析 DTB/FDT 二进制 | 未来 DTB 导入层的数据来源，不重复实现 |
| `embedded-hal-async` | 总线/IO 的异步 trait | Board 句柄实现这些 trait，驱动零改动接入 |
| `embassy-supervisor` | 任务生命周期图 | 不同层：它管任务，我们管硬件；可组合 |
| `rdrive` | 动态驱动管理（堆 + 动态分发） | 对照物：我们走编译期静态分发，零开销 |

## 设计决策

1. **配置是唯一事实来源**：`device_tree!` 同时生成静态 `TREE` 数据
   （运行时自省、未来 overlay 用）和类型化 `Board`（后端代码生成）。
2. **编译期绑定优先**：引脚类型即安全边界；树编译成类型化句柄，而不是
   `dyn` 查表。运行时 DTB 解析只作为可选的动态路径。
3. **零分配**：树是 `'static` 数据；上线引擎零堆。
4. **句柄实现 `embedded-hal-async`**：不发明新驱动 trait。
5. **后端与核心分离**：核心不依赖任何芯片 HAL；后端负责
   「引脚名 → HAL 外设」映射与中断绑定。
6. **错误尽早暴露**：环、重复 id、悬空依赖在宏里编译期报错；引脚/DMA
   错误由类型系统拦截。

## 已知限制 / 后续

- DTS 宏预处理（`#define` / `#ifdef`）未实现（`#include` 可用）
- 设备节点的驱动句柄生成（driver 类型化）留待驱动生态接入
- 多核 / 中断级执行器、电源管理编排属于 `embassy-supervisor` 的领地，可组合
- 未覆盖的外设：ETH、SAI/I2S、LTDC/LCD/DSI、QSPI/OSPI/FMC（存储控制器）、
  CORDIC/FMAC、加密类（AES/CRYP/HASH/PKA）、互补 PWM/单脉冲等高级定时器
  模式、SDMMC 4/8-bit、ADC/DAC 的 DMA 异步模式、CAN-FD 高级位时序、
  I2C 从机（embassy-stm32 0.6 暂无 API）。构造方式已表驱动，
  后续按条目逐个补齐即可

## 验证

```sh
# 宿主端：核心 + 宏（含 DTS 解析/overlay 测试）+ doc 测试
cargo test --offline
cargo test --offline -p embassy-dt-macros

# 编译失败 UI 测试：重复 id / 悬空依赖 / 依赖环 / 未知类型 / 缺 chip / 缺属性
cargo test --offline -p embassy-dt --test trybuild

# STM32 类型系统拦截测试：错误引脚必须在编译期报 SclPin 错误
./scripts/compile-fail-stm32.sh

# 宿主端 DTS demo
cargo run --offline --example demo_dts
cargo run --offline --example async_init   # DTS → 异步上线引擎

# STM32 固件交叉检查（需 thumbv7em-none-eabihf target）
cd stm32 && cargo check --offline --target thumbv7em-none-eabihf --example h723_nucleo
```
