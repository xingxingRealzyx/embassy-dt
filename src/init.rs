//! 依赖序异步上线引擎。

use core::fmt;
use core::future::Future;

/// 需要由总线/设备驱动实现的异步上线接口。
///
/// 实现方给出自己的 id、依赖（父总线/设备），并提供 `async fn init()`。
/// 引擎保证：init 按依赖序调用、每个节点只调用一次；某节点失败则立即
/// 返回错误，已上线的节点都交换到切片前部。
pub trait AsyncDevice {
    /// 初始化失败的错误类型。
    type Error: fmt::Debug;

    /// 节点 id，切片内必须唯一。
    fn id(&self) -> &'static str;

    /// 依赖的节点 id 列表。
    fn deps(&self) -> &'static [&'static str];

    /// 异步初始化（probe）。依赖保证先于本节点完成。
    ///
    /// 实现方可以直接写成 `async fn init(&mut self) -> Result<(), Self::Error>`；
    /// 这里用 `impl Future` 声明，便于在需要时给返回的 future 增加
    /// `Send` 等 auto-trait 约束（配合 Embassy 多执行器/多核场景）。
    fn init(&mut self) -> impl Future<Output = Result<(), Self::Error>>;
}

/// 初始化失败信息。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitError<E> {
    /// 切片里出现重复的节点 id。
    DuplicateId(&'static str),
    /// 依赖的节点根本不在切片里。
    MissingDependency {
        /// 缺少依赖的节点。
        node: &'static str,
        /// 悬空的依赖。
        dep: &'static str,
    },
    /// 依赖成环，无法上线；给出环中一个节点。
    CyclicDependency(&'static str),
    /// 某个节点初始化失败。
    Failed {
        /// 失败的节点。
        node: &'static str,
        /// 底层错误。
        error: E,
    },
}

/// 按依赖序异步初始化一组设备。
///
/// 成功上线的节点会被交换到 `devices[..n]`，且其中任意节点的依赖都排在
/// 它之前；返回 `n`。失败时返回 [`InitError`]，已上线节点仍在切片前部。
///
/// 零堆分配：使用「已上线节点交换到前部」的技巧跟踪状态，复杂度
/// O(n³)，适合个位数到几十个节点的启动路径。
pub async fn init_devices<D: AsyncDevice>(
    devices: &mut [D],
) -> Result<usize, InitError<D::Error>> {
    let n = devices.len();
    let mut done = 0usize;

    while done < n {
        let mut candidate = None;
        'scan: for i in done..n {
            let id = devices[i].id();
            if is_present(&devices[..done], id) {
                return Err(InitError::DuplicateId(id));
            }
            let deps = devices[i].deps();
            if deps.iter().all(|dep| is_present(&devices[..done], dep)) {
                candidate = Some(i);
                break 'scan;
            }
        }

        match candidate {
            Some(i) => {
                let id = devices[i].id();
                devices[i]
                    .init()
                    .await
                    .map_err(|error| InitError::Failed { node: id, error })?;
                devices.swap(done, i);
                done += 1;
            }
            None => {
                // 没人可上线：要么有悬空依赖，要么成环。
                for i in done..n {
                    let node = devices[i].id();
                    for dep in devices[i].deps() {
                        if !devices.iter().any(|d| d.id() == *dep) {
                            return Err(InitError::MissingDependency { node, dep });
                        }
                    }
                }
                return Err(InitError::CyclicDependency(devices[done].id()));
            }
        }
    }

    Ok(n)
}

fn is_present<D: AsyncDevice>(devices: &[D], id: &str) -> bool {
    devices.iter().any(|device| device.id() == id)
}
