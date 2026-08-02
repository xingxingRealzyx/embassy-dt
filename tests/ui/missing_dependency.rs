//! 悬空依赖必须在编译期报错。
use embassy_dt::device_tree;

device_tree! {
    name "missing";
    device bme280: Bme280Driver { bus: nope, addr: 0x76 };
}

fn main() {}
