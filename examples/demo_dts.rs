//! 宿主端演示：从 `.dts` / `.dtsi` 文件加载设备树。
//!
//! 运行：`cargo run --example demo_dts`

use embassy_dt::device_tree;

device_tree! {
    from "examples/boards/demo.dts";
}

fn main() {
    // 树名自动取自 DTS 根节点的 model。
    assert_eq!(TREE.name, "Host-Demo-Board");

    let mut order = [0usize; 8];
    let n = TREE.topo_order(&mut order).unwrap();
    println!("{} 的上线顺序:", TREE.name);
    for &i in &order[..n] {
        let node = &TREE.nodes[i];
        println!("  - {} ({:?})", node.id, node.kind);
    }

    // dtsi 继承 + 板级覆盖/追加都生效了。
    assert_eq!(
        TREE.node("i2c0").unwrap().prop("periph"),
        Some(embassy_dt::Prop::Str("I2C1"))
    );
    assert!(TREE.node("bme280").is_some());
    assert!(TREE.node("gps").is_some());
}
