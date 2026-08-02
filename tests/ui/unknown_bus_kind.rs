//! 未知总线类型必须在编译期报错。
use embassy_dt::device_tree;

device_tree! {
    name "bad";
    bus x: Can { periph: "FDCAN1" };
}

fn main() {}
