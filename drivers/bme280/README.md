# embassy-dt-bme280

BME280 温湿度传感器驱动，遵循 `embassy-dt` 的设备树初始化约定：
`async fn init(deps..., &NodeDesc) -> Result<Self, DeviceError>`。

在设备树中声明后，由宏生成的 `init_devices()` 按依赖顺序自动注入 I2C
总线并完成 reset / ID 校验 / 校准参数读取 / 温湿度整数补偿：

```dts
bme280: bme@76 {
    compatible = "bosch,bme280";
    bus = <&i2c0>;
    addr = <0x76>;
    driver = "embassy_dt_bme280::Bme280<...>";
};
```

完整文档见 [embassy-dt](https://docs.rs/embassy-dt)。
