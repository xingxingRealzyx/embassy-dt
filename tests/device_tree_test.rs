use std::cell::RefCell;
use std::rc::Rc;

use core::future::Future;
use core::pin::pin;
use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

use embassy_dt::{
    init_devices, AsyncDevice, BusKind, InitError, NodeDesc, NodeKind, Prop, TreeDesc,
    ValidationError,
};

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

/// 一棵示例树：i2c0 → bme280，uart0 → gps。
static TREE: TreeDesc = TreeDesc::new(
    "demo-board",
    &[
        NodeDesc::new(
            "i2c0",
            NodeKind::Bus(BusKind::I2c),
            &[],
            &[("freq", Prop::U32(400_000))],
        ),
        NodeDesc::new(
            "bme280",
            NodeKind::Device,
            &["i2c0"],
            &[("addr", Prop::U32(0x76))],
        ),
        NodeDesc::new(
            "uart0",
            NodeKind::Bus(BusKind::Uart),
            &[],
            &[("baud", Prop::U32(115_200))],
        ),
        NodeDesc::new("gps", NodeKind::Device, &["uart0"], &[]),
    ],
);

#[test]
fn tree_lookup_and_children() {
    assert_eq!(TREE.node("i2c0").unwrap().bus_kind(), Some(BusKind::I2c));
    assert_eq!(
        TREE.node("bme280").unwrap().prop("addr"),
        Some(Prop::U32(0x76))
    );
    assert_eq!(
        TREE.deps_of("bme280").map(|n| n.id).collect::<Vec<_>>(),
        vec!["i2c0"]
    );
    assert_eq!(
        TREE.children_of("uart0").map(|n| n.id).collect::<Vec<_>>(),
        vec!["gps"]
    );
}

#[test]
fn tree_validation_and_topo_order() {
    TREE.validate().unwrap();

    let mut order = [0usize; 8];
    let n = TREE.topo_order(&mut order).unwrap();
    assert_eq!(n, TREE.len());
    for (pos, &index) in order[..n].iter().enumerate() {
        let node = &TREE.nodes[index];
        for dep in node.deps {
            let dep_pos = order[..n]
                .iter()
                .position(|&i| TREE.nodes[i].id == *dep)
                .unwrap();
            assert!(dep_pos < pos, "{} 必须在 {} 之前", dep, node.id);
        }
    }
}

#[test]
fn tree_validation_errors() {
    static DUP: TreeDesc = TreeDesc::new(
        "dup",
        &[
            NodeDesc::new("a", NodeKind::Device, &[], &[]),
            NodeDesc::new("a", NodeKind::Device, &[], &[]),
        ],
    );
    assert_eq!(DUP.validate(), Err(ValidationError::DuplicateNode("a")));

    static MISSING: TreeDesc = TreeDesc::new(
        "missing",
        &[NodeDesc::new("a", NodeKind::Device, &["nope"], &[])],
    );
    assert_eq!(
        MISSING.validate(),
        Err(ValidationError::MissingDependency {
            node: "a",
            dep: "nope"
        })
    );

    static CYCLE: TreeDesc = TreeDesc::new(
        "cycle",
        &[
            NodeDesc::new("a", NodeKind::Device, &["b"], &[]),
            NodeDesc::new("b", NodeKind::Device, &["a"], &[]),
        ],
    );
    assert!(matches!(
        CYCLE.topo_order(&mut [0usize; 8]),
        Err(ValidationError::Cycle(_))
    ));

    let mut tiny = [0usize; 2];
    assert_eq!(
        TREE.topo_order(&mut tiny),
        Err(ValidationError::BufferTooSmall {
            needed: 4,
            given: 2
        })
    );
}

/// 测试用假设备：记录上线顺序，可配置失败。
struct Mock {
    id: &'static str,
    deps: &'static [&'static str],
    log: Rc<RefCell<Vec<&'static str>>>,
    fail: bool,
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
        if self.fail {
            return Err("boom");
        }
        Ok(())
    }
}

#[test]
fn init_devices_follows_dependency_order() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut devices = [
        Mock {
            id: "gps",
            deps: &["uart0"],
            log: log.clone(),
            fail: false,
        },
        Mock {
            id: "uart0",
            deps: &[],
            log: log.clone(),
            fail: false,
        },
        Mock {
            id: "bme280",
            deps: &["i2c0"],
            log: log.clone(),
            fail: false,
        },
        Mock {
            id: "i2c0",
            deps: &[],
            log: log.clone(),
            fail: false,
        },
    ];

    let n = block_on(init_devices(&mut devices)).unwrap();
    assert_eq!(n, 4);

    // 引擎按依赖序调度：uart0 → gps，i2c0 → bme280（组间顺序不定）。
    let log = log.borrow();
    assert_eq!(log[0], "uart0");
    assert_eq!(log[1], "gps");
    assert_eq!(log[2], "i2c0");
    assert_eq!(log[3], "bme280");

    // 无论顺序如何，交换后切片前部必须满足“依赖在前”。
    for dev in &devices[..n] {
        for dep in dev.deps {
            let dep_pos = devices[..n]
                .iter()
                .position(|d| d.id == *dep)
                .unwrap();
            let dev_pos = devices[..n]
                .iter()
                .position(|d| d.id == dev.id)
                .unwrap();
            assert!(dep_pos < dev_pos, "{} 必须在 {} 之前", dep, dev.id);
        }
    }
}

#[test]
fn init_devices_reports_cycle() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut devices = [
        Mock {
            id: "a",
            deps: &["b"],
            log: log.clone(),
            fail: false,
        },
        Mock {
            id: "b",
            deps: &["a"],
            log: log.clone(),
            fail: false,
        },
    ];
    assert!(matches!(
        block_on(init_devices(&mut devices)),
        Err(InitError::CyclicDependency(_))
    ));
}

#[test]
fn init_devices_reports_missing_dependency() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut devices = [Mock {
        id: "a",
        deps: &["nope"],
        log,
        fail: false,
    }];
    assert_eq!(
        block_on(init_devices(&mut devices)),
        Err(InitError::MissingDependency {
            node: "a",
            dep: "nope"
        })
    );
}

#[test]
fn init_devices_stops_on_failure_and_keeps_prefix() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut devices = [
        Mock {
            id: "a",
            deps: &[],
            log: log.clone(),
            fail: false,
        },
        Mock {
            id: "b",
            deps: &[],
            log: log.clone(),
            fail: true,
        },
        Mock {
            id: "c",
            deps: &["a"],
            log: log.clone(),
            fail: false,
        },
    ];

    let err = block_on(init_devices(&mut devices)).unwrap_err();
    assert_eq!(
        err,
        InitError::Failed {
            node: "b",
            error: "boom"
        }
    );
    // a、b 已尝试上线（记录在日志里），c 因 b 失败未上线。
    assert_eq!(*log.borrow(), vec!["a", "b"]);
}
