//! 参考时钟配置。
//!
//! 之前所有示例都用 `init(Default::default())`——编译没问题，但真机上
//! USB / RNG / SDMMC / I2S 需要正确的时钟源。这里提供与官方示例对齐的
//! 参考配置；不同板子请按实际晶振与需求调整。

#[cfg(feature = "stm32h723zg")]
pub fn clock_config() -> embassy_stm32::Config {
    use embassy_stm32::rcc::*;

    let mut config = embassy_stm32::Config::default();
    // 纯内部时钟：HSI + PLL1 → 400 MHz，HSI48 供 USB / RNG。
    config.rcc.hsi = Some(HSIPrescaler::DIV1);
    config.rcc.csi = true;
    config.rcc.hsi48 = Some(Hsi48Config { sync_from_usb: true });
    config.rcc.pll1 = Some(Pll {
        source: PllSource::HSI,
        prediv: PllPreDiv::DIV4,
        mul: PllMul::MUL50,
        divp: Some(PllDiv::DIV2),
        divq: None,
        divr: None,
    });
    config.rcc.sys = Sysclk::PLL1_P; // 400 MHz
    config.rcc.ahb_pre = AHBPrescaler::DIV2; // 200 MHz
    config.rcc.apb1_pre = APBPrescaler::DIV2; // 100 MHz
    config.rcc.apb2_pre = APBPrescaler::DIV2;
    config.rcc.apb3_pre = APBPrescaler::DIV2;
    config.rcc.apb4_pre = APBPrescaler::DIV2;
    config.rcc.voltage_scale = VoltageScale::Scale1;
    config.rcc.mux.usbsel = mux::Usbsel::HSI48;
    config
}

#[cfg(feature = "stm32f411ce")]
pub fn clock_config() -> embassy_stm32::Config {
    use embassy_stm32::rcc::*;
    use embassy_stm32::time::Hertz;

    let mut config = embassy_stm32::Config::default();
    // BlackPill 常见 25 MHz HSE；按你的板子调整频率/模式。
    config.rcc.hse = Some(Hse {
        freq: Hertz(25_000_000),
        mode: HseMode::Oscillator,
    });
    config.rcc.pll_src = PllSource::HSE;
    config.rcc.pll = Some(Pll {
        prediv: PllPreDiv::DIV4,
        mul: PllMul::MUL168,
        divp: Some(PllPDiv::DIV2), // 168 MHz
        divq: Some(PllQDiv::DIV7), // 48 MHz，供 USB
        divr: None,
    });
    config.rcc.ahb_pre = AHBPrescaler::DIV1;
    config.rcc.apb1_pre = APBPrescaler::DIV4;
    config.rcc.apb2_pre = APBPrescaler::DIV2;
    config.rcc.sys = Sysclk::PLL1_P;
    config.rcc.mux.clk48sel = mux::Clk48sel::PLL1_Q;
    config
}
