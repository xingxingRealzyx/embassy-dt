//! 多板共用的应用逻辑：与具体芯片 / 板子无关。
//!
//! 换板子时这个文件一行都不用改 —— 差异全部收敛在 `.dts/.dtsi` 里。

use embassy_stm32::gpio::Output;
use embassy_time::Timer;

/// 心跳：LED 每 500ms 翻转一次。
pub async fn heartbeat(led: &mut Output<'_>) -> ! {
    loop {
        led.toggle();
        Timer::after_millis(500).await;
    }
}
