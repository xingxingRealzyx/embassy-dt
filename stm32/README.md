# embassy-dt-stm32

`embassy-dt` 的 STM32 后端。

当前支持：STM32H723ZG（默认）、STM32F411CE。配置使用 `.dts` / `.dtsi`
设备树文件，宏自动生成类型化的 Embassy HAL 实例（含中断绑定与 DMA）。

```rust
device_tree! {
    name "my-board";
    backend stm32;
    chip "stm32h723zg";
    from "boards/my-board.dts";
}
```

完整示例见仓库 `stm32/examples/`（20+ 个对照 embassy 官方示例的固件）。
