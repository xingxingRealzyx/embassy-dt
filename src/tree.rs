//! 设备树的静态描述模型。

use core::fmt;

/// 树中节点的稳定标识。
///
/// 在 Phase 1 的 DSL 里由 `bus i2c0` / `device bme280` 自动生成；
/// 同一棵树内必须唯一。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub &'static str);

impl NodeId {
    /// 构造一个节点标识。
    pub const fn new(id: &'static str) -> Self {
        Self(id)
    }

    /// 取出字符串。
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Debug for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// 总线类型。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BusKind {
    /// I2C 总线。
    I2c,
    /// SPI 总线。
    Spi,
    /// UART 串口。
    Uart,
}

/// 节点种类。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NodeKind {
    /// 总线节点。
    Bus(BusKind),
    /// 挂接在总线上的设备节点。
    Device,
    /// GPIO 节点（输入/输出）。
    Gpio,
    /// 可实例化的独立外设（RNG / ADC / DAC / PWM / CRC / CAN 等）。
    Peripheral,
}

/// 节点属性的值。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Prop {
    /// 字符串属性，如 `compatible = "bosch,bme280"`。
    Str(&'static str),
    /// 数值属性，如 `addr = 0x76`、`freq = 400_000`。
    U32(u32),
    /// 数值数组属性，对应 DTS 的 `<...>` 多元素或 `[...]` 字节串。
    Array(&'static [u32]),
    /// 布尔属性，对应 DTS 的无值属性。
    Bool(bool),
}

impl Prop {
    /// 构造字符串属性。
    pub const fn str(value: &'static str) -> Self {
        Self::Str(value)
    }

    /// 构造数值属性。
    pub const fn u32(value: u32) -> Self {
        Self::U32(value)
    }

    /// 以字符串形式取值。
    pub fn as_str(self) -> Option<&'static str> {
        match self {
            Self::Str(v) => Some(v),
            Self::U32(_) => None,
            Self::Array(_) => None,
            Self::Bool(_) => None,
        }
    }

    /// 以数值形式取值。
    pub fn as_u32(self) -> Option<u32> {
        match self {
            Self::U32(v) => Some(v),
            Self::Str(_) => None,
            Self::Array(_) => None,
            Self::Bool(_) => None,
        }
    }

    /// 以数组形式取值（DTS `<a b c>` / `[aa bb]`）。
    pub fn as_array(self) -> Option<&'static [u32]> {
        match self {
            Self::Array(v) => Some(v),
            _ => None,
        }
    }

    /// 以布尔形式取值（DTS 无值属性）。
    pub fn as_bool(self) -> Option<bool> {
        match self {
            Self::Bool(v) => Some(v),
            _ => None,
        }
    }
}

/// 单个节点的静态描述。
#[derive(Clone, Copy, Debug)]
pub struct NodeDesc {
    /// 节点标识，树内唯一。
    pub id: &'static str,
    /// 节点种类（总线/设备）。
    pub kind: NodeKind,
    /// 依赖的父节点 id（如设备依赖它所在的总线）。
    pub deps: &'static [&'static str],
    /// 属性表，如引脚、地址、频率。
    pub props: &'static [(&'static str, Prop)],
}

impl NodeDesc {
    /// 构造节点描述。
    pub const fn new(
        id: &'static str,
        kind: NodeKind,
        deps: &'static [&'static str],
        props: &'static [(&'static str, Prop)],
    ) -> Self {
        Self {
            id,
            kind,
            deps,
            props,
        }
    }

    /// 按名字查属性。
    pub fn prop(&self, name: &str) -> Option<Prop> {
        self.props
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| *value)
    }

    /// 若节点是总线，返回总线类型。
    pub fn bus_kind(&self) -> Option<BusKind> {
        match self.kind {
            NodeKind::Bus(kind) => Some(kind),
            NodeKind::Device => None,
            NodeKind::Gpio => None,
            NodeKind::Peripheral => None,
        }
    }
}

/// 整棵树的静态描述。
#[derive(Clone, Copy, Debug)]
pub struct TreeDesc {
    /// 树名，如板名。
    pub name: &'static str,
    /// 所有节点。
    pub nodes: &'static [NodeDesc],
}

impl TreeDesc {
    /// 构造一棵树。
    pub const fn new(name: &'static str, nodes: &'static [NodeDesc]) -> Self {
        Self { name, nodes }
    }

    /// 节点数量。
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// 树是否为空。
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// 按 id 找节点。
    pub fn node(&self, id: &str) -> Option<&NodeDesc> {
        self.nodes.iter().find(|node| node.id == id)
    }

    /// 按 id 找节点下标。
    pub fn node_index(&self, id: &str) -> Option<usize> {
        self.nodes.iter().position(|node| node.id == id)
    }

    /// 某个节点的依赖（已解析为节点引用）。
    pub fn deps_of<'a>(&'a self, id: &'a str) -> impl Iterator<Item = &'a NodeDesc> + 'a {
        self.node(id)
            .into_iter()
            .flat_map(move |node| node.deps.iter().filter_map(move |dep| self.node(dep)))
    }

    /// 依赖指定父节点的子节点。
    pub fn children_of<'a>(&'a self, parent: &'a str) -> impl Iterator<Item = &'a NodeDesc> + 'a {
        self.nodes
            .iter()
            .filter(move |node| node.deps.contains(&parent))
    }

    /// 结构校验：id 重复、悬空依赖。
    ///
    /// 环检测不在本方法内（需要额外缓冲），由 [`TreeDesc::topo_order`] 或
    /// 初始化引擎负责。
    pub fn validate(&self) -> Result<(), ValidationError> {
        for (i, node) in self.nodes.iter().enumerate() {
            if self.nodes[..i].iter().any(|prev| prev.id == node.id) {
                return Err(ValidationError::DuplicateNode(node.id));
            }
            for dep in node.deps {
                if self.node(dep).is_none() {
                    return Err(ValidationError::MissingDependency {
                        node: node.id,
                        dep,
                    });
                }
            }
        }
        Ok(())
    }

    /// 计算一个拓扑序，把节点下标依次写入 `out`，返回写入的节点数。
    ///
    /// `out` 必须至少能容纳 [`TreeDesc::len`] 项。若存在环，返回
    /// [`ValidationError::Cycle`]。
    ///
    /// 复杂度 O(n³)，n 为节点数；设备树的节点通常是个位数到几十个，
    /// 且只在启动时执行一次，可以接受。
    pub fn topo_order(&self, out: &mut [usize]) -> Result<usize, ValidationError> {
        let n = self.nodes.len();
        if out.len() < n {
            return Err(ValidationError::BufferTooSmall {
                needed: n,
                given: out.len(),
            });
        }
        self.validate()?;

        let mut done = 0usize;
        while done < n {
            let mut candidate = None;
            'scan: for i in 0..n {
                if out[..done].contains(&i) {
                    continue;
                }
                let node = &self.nodes[i];
                for dep in node.deps {
                    // validate() 已保证依赖存在。
                    let dep_index = self.node_index(dep).unwrap();
                    if !out[..done].contains(&dep_index) {
                        continue 'scan;
                    }
                }
                candidate = Some(i);
                break;
            }
            match candidate {
                Some(i) => {
                    out[done] = i;
                    done += 1;
                }
                None => {
                    // validate() 已排除悬空依赖，剩下的节点必然构成环。
                    for i in 0..n {
                        if !out[..done].contains(&i) {
                            return Err(ValidationError::Cycle(self.nodes[i].id));
                        }
                    }
                    unreachable!("validate() 通过时不可能走到这里");
                }
            }
        }
        Ok(n)
    }
}

/// 树结构校验错误。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidationError {
    /// 节点 id 重复。
    DuplicateNode(&'static str),
    /// 依赖的节点不存在。
    MissingDependency {
        /// 缺少依赖的节点。
        node: &'static str,
        /// 悬空的依赖。
        dep: &'static str,
    },
    /// 存在依赖环；给出环中一个节点。
    Cycle(&'static str),
    /// 调用方提供的输出缓冲太小。
    BufferTooSmall {
        /// 需要的容量。
        needed: usize,
        /// 实际容量。
        given: usize,
    },
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateNode(id) => write!(f, "duplicate node id `{id}`"),
            Self::MissingDependency { node, dep } => {
                write!(f, "node `{node}` depends on missing node `{dep}`")
            }
            Self::Cycle(id) => write!(f, "dependency cycle detected at node `{id}`"),
            Self::BufferTooSmall { needed, given } => {
                write!(f, "buffer too small: need {needed} slots, got {given}")
            }
        }
    }
}
