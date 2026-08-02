//! 重复节点 id 必须在编译期报错。
use embassy_dt::device_tree;

device_tree! {
    name "dup";
    bus i2c0: I2c { periph: "I2C1", scl: "PB8", sda: "PB7", freq: 400_000 };
    bus i2c0: I2c { periph: "I2C1", scl: "PB8", sda: "PB7", freq: 400_000 };
}

fn main() {}
