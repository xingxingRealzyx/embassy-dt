//! 宿主端演示：声明设备树 → 编译期校验 → 打印上线顺序。
//!
//! 运行：`cargo run --example demo`

use embassy_dt::device_tree;

device_tree! {
    name "demo-board";
    bus i2c0: I2c { periph: "I2C1", scl: "PB8", sda: "PB7", freq: 400_000 };
    bus uart0: Uart { periph: "USART1", tx: "PA9", rx: "PA10", baud: 115_200 };
    device bme280: Bme280Driver { bus: i2c0, addr: 0x76 };
    device gps: NmeaGps { bus: uart0 };
}

fn main() {
    assert_eq!(TREE.name, "demo-board");

    let mut order = [0usize; 8];
    let n = TREE.topo_order(&mut order).unwrap();
    println!("{} 的上线顺序:", TREE.name);
    for &i in &order[..n] {
        let node = &TREE.nodes[i];
        println!("  - {} ({:?})", node.id, node.kind);
    }
}
