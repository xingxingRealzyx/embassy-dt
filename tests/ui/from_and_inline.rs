//! `from` 与内联节点混用必须在编译期报错。
use embassy_dt::device_tree;

device_tree! {
    name "mixed";
    from "boards/x.dts";
    bus i2c0: I2c { periph: "I2C1" };
}

fn main() {}
