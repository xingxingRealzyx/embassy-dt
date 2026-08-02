//! 宿主端演示：DTS 设备树 → 依赖序异步上线引擎（`init_devices`）。
//!
//! 树里的每个节点被包成一个 mock 设备；引擎保证总线先于设备上线。
//! 真实固件里，节点会由驱动 crate 实现 [`embassy_dt::AsyncDevice`]。
//!
//! 运行：`cargo run --example async_init`

use std::cell::RefCell;
use std::rc::Rc;

use core::future::Future;
use core::pin::pin;
use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

use embassy_dt::device_tree;
use embassy_dt::{init_devices, AsyncDevice};

device_tree! {
    from "examples/boards/demo.dts";
}

fn block_on<F: Future>(fut: F) -> F::Output {
    fn noop_raw(_: *const ()) -> RawWaker {
        RawWaker::new(core::ptr::null(), &VTABLE)
    }
    static VTABLE: RawWakerVTable = RawWakerVTable::new(noop_raw, |_| {}, |_| {}, |_| {});
    let waker = unsafe { Waker::from_raw(noop_raw(core::ptr::null())) };
    let mut cx = Context::from_waker(&waker);
    let mut fut = pin!(fut);
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(value) => return value,
            Poll::Pending => core::hint::spin_loop(),
        }
    }
}

/// 测试用假设备：记录上线顺序。
struct Mock {
    id: &'static str,
    deps: &'static [&'static str],
    log: Rc<RefCell<Vec<&'static str>>>,
}

impl AsyncDevice for Mock {
    type Error = &'static str;

    fn id(&self) -> &'static str {
        self.id
    }

    fn deps(&self) -> &'static [&'static str] {
        self.deps
    }

    async fn init(&mut self) -> Result<(), Self::Error> {
        self.log.borrow_mut().push(self.id);
        Ok(())
    }
}

fn main() {
    // TREE 是 'static 数据：节点 id 与依赖直接来自 DTS 文件。
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut devices: Vec<Mock> = TREE
        .nodes
        .iter()
        .map(|node| Mock {
            id: node.id,
            deps: node.deps,
            log: log.clone(),
        })
        .collect();

    let n = block_on(init_devices(&mut devices)).unwrap();
    assert_eq!(n, TREE.len());

    println!("{} 的异步上线日志:", TREE.name);
    for id in log.borrow().iter() {
        println!("  ✓ {id}");
    }

    // 无论调度顺序如何，依赖一定先于依赖者上线。
    for dev in &devices[..n] {
        for dep in dev.deps {
            let dep_pos = devices[..n].iter().position(|d| d.id == *dep).unwrap();
            let dev_pos = devices[..n].iter().position(|d| d.id == dev.id).unwrap();
            assert!(dep_pos < dev_pos, "{} 必须先于 {} 上线", dep, dev.id);
        }
    }
    println!("依赖序校验通过 ✅");
}
