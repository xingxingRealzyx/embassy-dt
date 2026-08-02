//! 未知外设类型必须在编译期报错。
use embassy_dt::device_tree;

device_tree! {
    name "bad";
    periph x: Foo { periph: "X" };
}

fn main() {}
