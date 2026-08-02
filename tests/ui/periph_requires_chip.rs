//! stm32 后端使用芯片相关外设但未声明 chip 必须在编译期报错。
use embassy_dt::device_tree;

device_tree! {
    name "nochip";
    backend stm32;
    periph can0: Can { periph: "FDCAN1", rx: "PA11", tx: "PA12" };
}

fn main() {}
