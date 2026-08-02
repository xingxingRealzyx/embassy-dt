//! 依赖环必须在编译期报错。
use embassy_dt::device_tree;

device_tree! {
    name "cycle";
    device a: A { bus: b };
    device b: B { bus: a };
}

fn main() {}
