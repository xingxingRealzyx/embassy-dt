//! stm32 后端 UART 缺少 rx 引脚必须在编译期报错。
use embassy_dt::device_tree;

device_tree! {
    name "nouart";
    backend stm32;
    chip "stm32h723zg";
    bus uart0: Uart { periph: "USART1", tx: "PA9", dma_tx: "DMA1_CH4", dma_rx: "DMA1_CH5", baud: 115_200 };
}

fn main() {}
