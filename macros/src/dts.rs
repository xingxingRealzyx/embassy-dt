//! 构建期 DTS/DTSI 解析器（`device_tree! { from = "..." }` 的数据来源）。
//!
//! 支持：`/dts-v1/`、`#include` / `/include/`（相对路径递归）、注释、标签、
//! `<...>` 数值/`&label`/`&{/path}` 引用、`<(...)>` 整数表达式、`[...]`
//! 字节串、字符串拼接、布尔属性、`/delete-node/`、`/delete-property/`，
//! 以及「同标签节点合并」——即板级 overlay 语义（`board.dts` include
//! `chip.dtsi` 后覆盖属性）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use proc_macro2::Span;
use syn::{Ident, LitInt, LitStr, Result};

use crate::{BusKindAst, DslNode, DslProp, GpioKindAst, NodeKindAst, PropValue};

const MAX_INCLUDE_DEPTH: usize = 16;

// ---------------------------------------------------------------------------
// 共享模型：DTS 文本与 DTB 二进制两个来源都产出这些类型
// ---------------------------------------------------------------------------

/// 一次加载的结果（已转换为宏 IR 节点）。
#[derive(Debug)]
pub(crate) struct DtsLoad {
    pub nodes: Vec<DslNode>,
    pub model: Option<String>,
    /// 加载过的所有文件（含 include），用于 rustc 重建跟踪。
    pub files: Vec<String>,
}

/// 原始节点（未定 id、未解析引用）。
#[derive(Debug)]
pub(crate) struct DtsNode {
    pub label: Option<String>,
    /// 树中的绝对路径，如 `/soc@40000000/i2c@40005400`。
    pub path: String,
    pub kind: Option<NodeKindAst>,
    pub has_compatible: bool,
    pub props: Vec<(String, DtsPropVal)>,
    pub deleted_props: Vec<String>,
}

#[derive(Debug)]
pub(crate) enum DtsPropVal {
    Str(String),
    Cells(Vec<CellItem>),
    Bytes(Vec<u8>),
    Bool,
}

#[derive(Debug)]
pub(crate) enum CellItem {
    Num(u32),
    /// `&label`
    Ref(String),
    /// `&{/path}`
    PathRef(String),
}

pub(crate) fn dts_err(msg: &str) -> syn::Error {
    syn::Error::new(Span::call_site(), format!("dts: {msg}"))
}

pub(crate) fn normalize_key(key: &str) -> String {
    key.replace('-', "_")
}

pub(crate) fn kind_from_compatible(compatible: &str) -> NodeKindAst {
    match compatible {
        "embassy-dt,bus-i2c" => NodeKindAst::Bus(BusKindAst::I2c),
        "embassy-dt,bus-spi" => NodeKindAst::Bus(BusKindAst::Spi),
        "embassy-dt,bus-uart" => NodeKindAst::Bus(BusKindAst::Uart),
        "embassy-dt,gpio-out" => NodeKindAst::Gpio(GpioKindAst::Out),
        "embassy-dt,gpio-in" => NodeKindAst::Gpio(GpioKindAst::In),
        "embassy-dt,gpio-pin" => NodeKindAst::Gpio(GpioKindAst::Pin),
        "embassy-dt,periph-rng" => NodeKindAst::Peripheral(crate::PeriphKindAst::Rng),
        "embassy-dt,periph-adc" => NodeKindAst::Peripheral(crate::PeriphKindAst::Adc),
        "embassy-dt,periph-crc" => NodeKindAst::Peripheral(crate::PeriphKindAst::Crc),
        "embassy-dt,periph-dac" => NodeKindAst::Peripheral(crate::PeriphKindAst::Dac),
        "embassy-dt,periph-pwm" => NodeKindAst::Peripheral(crate::PeriphKindAst::Pwm),
        "embassy-dt,periph-can" => NodeKindAst::Peripheral(crate::PeriphKindAst::Can),
        "embassy-dt,periph-usb" => NodeKindAst::Peripheral(crate::PeriphKindAst::Usb),
        "embassy-dt,periph-qei" => NodeKindAst::Peripheral(crate::PeriphKindAst::Qei),
        "embassy-dt,periph-input-capture" => {
            NodeKindAst::Peripheral(crate::PeriphKindAst::InputCapture)
        }
        "embassy-dt,periph-sdmmc" => NodeKindAst::Peripheral(crate::PeriphKindAst::Sdmmc),
        "embassy-dt,periph-i2s" => NodeKindAst::Peripheral(crate::PeriphKindAst::I2s),
        "embassy-dt,periph-pwm-input" => {
            NodeKindAst::Peripheral(crate::PeriphKindAst::PwmInput)
        }
        "embassy-dt,periph-complementary-pwm" => {
            NodeKindAst::Peripheral(crate::PeriphKindAst::ComplementaryPwm)
        }
        _ => NodeKindAst::Device,
    }
}

pub(crate) fn sanitize_ident(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    out
}

/// 拼接节点路径：父路径为 `/` 时避免双斜杠。
pub(crate) fn join_path(parent: &str, child: &str) -> String {
    if parent == "/" {
        format!("/{child}")
    } else {
        format!("{parent}/{child}")
    }
}

pub(crate) fn path_exists(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|md| md.is_file())
        .unwrap_or(false)
}

pub(crate) fn canonicalize(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

pub(crate) fn resolve_path(entry: &str) -> Result<PathBuf> {
    let path = Path::new(entry);
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }

    // 依次尝试：CARGO_MANIFEST_DIR（编译目标 crate 的根）、当前目录、
    // 以及从当前目录向上最多 5 层（覆盖 workspace 根作为 cwd 的情况）。
    let mut bases: Vec<PathBuf> = Vec::new();
    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        bases.push(PathBuf::from(manifest));
    }
    if let Ok(cwd) = std::env::current_dir() {
        bases.push(cwd.clone());
        let mut dir = cwd.as_path();
        for _ in 0..5 {
            match dir.parent() {
                Some(parent) => {
                    bases.push(parent.to_path_buf());
                    dir = parent;
                }
                None => break,
            }
        }
    }

    for base in &bases {
        let candidate = base.join(path);
        if path_exists(&candidate) {
            return Ok(candidate);
        }
    }

    Err(dts_err(&format!(
        "cannot resolve `{entry}` (searched under: {})",
        bases
            .iter()
            .map(|b| b.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )))
}

/// 收尾：计算最终 id（label 优先，否则用路径）、解析引用、转成宏 IR。
pub(crate) fn finalize(
    nodes: Vec<DtsNode>,
    model: Option<String>,
    files: Vec<String>,
) -> Result<DtsLoad> {
    let mut ids: Vec<String> = Vec::with_capacity(nodes.len());
    let mut path_to_id: HashMap<String, String> = HashMap::new();

    for node in &nodes {
        let id = match &node.label {
            Some(label) => label.clone(),
            None => sanitize_ident(node.path.trim_start_matches('/')),
        };
        path_to_id.insert(node.path.clone(), id.clone());
        ids.push(id);
    }

    // 最终 id 必须唯一（label 冲突 / 无标签节点路径 sanitize 后冲突）。
    let mut seen: HashMap<&str, &str> = HashMap::new();
    for (node, id) in nodes.iter().zip(ids.iter()) {
        if let Some(prev_path) = seen.insert(id, &node.path) {
            return Err(dts_err(&format!(
                "duplicate node id `{id}` (paths `{prev_path}` and `{}`)",
                node.path
            )));
        }
    }

    let mut dsl_nodes = Vec::with_capacity(nodes.len());
    for (node, id) in nodes.into_iter().zip(ids) {
        dsl_nodes.push(to_dsl_node(node, id, &path_to_id)?);
    }
    Ok(DtsLoad {
        nodes: dsl_nodes,
        model,
        files,
    })
}

fn to_dsl_node(
    node: DtsNode,
    id: String,
    path_to_id: &HashMap<String, String>,
) -> Result<DslNode> {
    let DtsNode {
        label,
        path,
        kind,
        has_compatible: _,
        props,
        deleted_props: _,
    } = node;

    // DTS 设备节点可用 `driver = "crate::Bme280Driver"` 指定驱动类型；
    // 未指定时保持文档性节点（不进 BoardDevices）。
    let driver = props
        .iter()
        .find(|(k, _)| k == "driver")
        .and_then(|(_, v)| match v {
            DtsPropVal::Str(s) => syn::parse_str::<syn::Path>(s).ok(),
            _ => None,
        });

    let id_ident: Ident = syn::parse_str::<Ident>(&id).map_err(|_| {
        dts_err(&format!(
            "node `{path}` (label `{label:?}`) is not a valid Rust identifier"
        ))
    })?;

    let mut deps: Vec<String> = Vec::new();
    let props = props
        .into_iter()
        .map(|(key, value)| {
            let key = syn::parse_str::<Ident>(&sanitize_ident(&key))
                .map_err(|_| dts_err(&format!("property `{key}` is not a valid identifier")))?;
            let value = match value {
                DtsPropVal::Str(s) => PropValue::Str(LitStr::new(&s, Span::call_site())),
                DtsPropVal::Cells(items) => {
                    let mut nums: Vec<u32> = Vec::new();
                    let mut first_ref: Option<String> = None;
                    for item in items {
                        match item {
                            CellItem::Num(n) => nums.push(n),
                            CellItem::Ref(label) => {
                                if !path_to_id.contains_key(&label) && !is_label_known(&label) {
                                    // 标签本身即 id；未知标签留到 validate 报错。
                                }
                                deps.push(label.clone());
                                if first_ref.is_none() {
                                    first_ref = Some(label);
                                }
                            }
                            CellItem::PathRef(p) => {
                                let resolved = path_to_id.get(&p).cloned().ok_or_else(|| {
                                    dts_err(&format!(
                                        "path reference `&{{{p}}}` does not match any node"
                                    ))
                                })?;
                                deps.push(resolved.clone());
                                if first_ref.is_none() {
                                    first_ref = Some(resolved);
                                }
                            }
                        }
                    }
                    match (nums.len(), first_ref) {
                        (1, None) => PropValue::U32(LitInt::new(
                            &nums[0].to_string(),
                            Span::call_site(),
                        )),
                        (0, Some(r)) | (_, Some(r)) => {
                            PropValue::Str(LitStr::new(&r, Span::call_site()))
                        }
                        (n, None) => PropValue::Array(nums[..n].to_vec()),
                    }
                }
                DtsPropVal::Bytes(bytes) => {
                    PropValue::Array(bytes.into_iter().map(u32::from).collect())
                }
                DtsPropVal::Bool => PropValue::Bool(true),
            };
            Ok(DslProp { key, value })
        })
        .collect::<Result<Vec<_>>>()?;

    let deps = deps
        .into_iter()
        .map(|dep| {
            syn::parse_str::<Ident>(&dep)
                .map_err(|_| dts_err(&format!("reference `{dep}` is not a valid identifier")))
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(DslNode {
        id: id_ident,
        kind: kind.unwrap_or(NodeKindAst::Device),
        driver,
        props,
        deps,
    })
}

fn is_label_known(label: &str) -> bool {
    // 标签即最终 id；悬空引用由宏的 validate() 统一报错。
    let _ = label;
    true
}

// ---------------------------------------------------------------------------
// 词法与语法分析
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Str(String),
    Num(u32),
    Punct(char),
    Eof,
}

/// 词法输出：token 与每个 token 的 (行, 列)。
type TokensWithSpans = (Vec<Tok>, Vec<(usize, usize)>);

/// 词法分析：返回 token 与每个 token 的 (行, 列)（从 1 开始）。
fn lex(text: &str, src: &str) -> Result<TokensWithSpans> {
    let mut toks = Vec::new();
    let mut spans = Vec::new();
    let b = text.as_bytes();
    let mut i = 0usize;
    let mut line = 1usize;
    let mut col = 1usize;

    while i < b.len() {
        let c = b[i] as char;
        match c {
            ' ' | '\t' | '\r' => {
                i += 1;
                col += 1;
            }
            '\n' => {
                i += 1;
                line += 1;
                col = 1;
            }
            '/' if i + 1 < b.len() && b[i + 1] as char == '/' => {
                while i < b.len() && b[i] as char != '\n' {
                    i += 1;
                    col += 1;
                }
            }
            '/' if i + 1 < b.len() && b[i + 1] as char == '*' => {
                let mut end = i + 2;
                while end + 1 < b.len() && !(b[end] as char == '*' && b[end + 1] as char == '/') {
                    if b[end] as char == '\n' {
                        line += 1;
                        col = 1;
                    } else {
                        col += 1;
                    }
                    end += 1;
                }
                if end + 1 >= b.len() {
                    return Err(dts_err(&format!("unterminated block comment in `{src}`")));
                }
                i = end + 2;
                col += 2;
            }
            '"' => {
                let mut s = String::new();
                i += 1;
                col += 1;
                while i < b.len() && b[i] as char != '"' {
                    let ch = b[i] as char;
                    if ch == '\\' && i + 1 < b.len() {
                        i += 1;
                        col += 1;
                        match b[i] as char {
                            'n' => s.push('\n'),
                            't' => s.push('\t'),
                            'r' => s.push('\r'),
                            '"' => s.push('"'),
                            '\\' => s.push('\\'),
                            other => {
                                s.push('\\');
                                s.push(other);
                            }
                        }
                    } else {
                        s.push(ch);
                    }
                    i += 1;
                    col += 1;
                }
                if i >= b.len() {
                    return Err(dts_err(&format!("unterminated string in `{src}`")));
                }
                i += 1;
                col += 1;
                push_tok(&mut toks, &mut spans, line, col, Tok::Str(s));
            }
            '0'..='9' => {
                let start = i;
                if b[i] as char == '0' && i + 1 < b.len() && matches!(b[i + 1] as char, 'x' | 'X')
                {
                    i += 2;
                    while i < b.len() && (b[i] as char).is_ascii_hexdigit() {
                        i += 1;
                    }
                    let text = &text[start..i];
                    let n = u32::from_str_radix(&text[2..], 16)
                        .map_err(|_| dts_err(&format!("invalid hex number `{text}` in `{src}`")))?;
                    push_tok(&mut toks, &mut spans, line, col, Tok::Num(n));
                } else {
                    while i < b.len() && (b[i] as char).is_ascii_digit() {
                        i += 1;
                    }
                    let n = text[start..i].parse::<u32>().map_err(|_| {
                        dts_err(&format!("invalid number `{}` in `{src}`", &text[start..i]))
                    })?;
                    push_tok(&mut toks, &mut spans, line, col, Tok::Num(n));
                }
                col += i - start;
            }
            'a'..='z' | 'A'..='Z' | '_' => {
                let start = i;
                while i < b.len() {
                    let c = b[i] as char;
                    if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' || c == ',' {
                        i += 1;
                    } else {
                        break;
                    }
                }
                push_tok(
                    &mut toks,
                    &mut spans,
                    line,
                    col,
                    Tok::Ident(text[start..i].to_string()),
                );
                col += i - start;
            }
            '#' | '{' | '}' | ';' | '=' | '<' | '>' | '[' | ']' | '(' | ')' | '/' | '&'
            | '@' | ':' | '+' | '-' | '*' | '%' => {
                push_tok(&mut toks, &mut spans, line, col, Tok::Punct(c));
                i += 1;
                col += 1;
            }
            other => {
                return Err(dts_err(&format!(
                    "unsupported character `{other}` in `{src}` (at byte {i})"
                )))
            }
        }
    }
    push_tok(&mut toks, &mut spans, line, col, Tok::Eof);
    Ok((toks, spans))
}

fn push_tok(
    toks: &mut Vec<Tok>,
    spans: &mut Vec<(usize, usize)>,
    line: usize,
    col: usize,
    tok: Tok,
) {
    spans.push((line, col));
    toks.push(tok);
}

#[derive(Default)]
struct Collector {
    nodes: Vec<DtsNode>,
    by_key: HashMap<String, usize>,
    model: Option<String>,
    files: Vec<String>,
}

impl Collector {
    fn parse_file(&mut self, path: &Path, depth: usize) -> Result<()> {
        if depth > MAX_INCLUDE_DEPTH {
            return Err(dts_err(&format!(
                "include depth limit ({MAX_INCLUDE_DEPTH}) exceeded at `{}`",
                path.display()
            )));
        }
        let text = std::fs::read_to_string(path)
            .map_err(|e| dts_err(&format!("cannot read `{}`: {e}", path.display())))?;
        self.files.push(path.to_string_lossy().into_owned());
        let (toks, spans) = lex(&text, &path.display().to_string())?;
        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let mut parser = Parser {
            toks,
            spans,
            pos: 0,
            dir,
            file: path.to_string_lossy().into_owned(),
            depth,
            collector: self,
        };
        parser.parse_top()
    }

    /// 节点合并（overlay 语义）：同 key（标签或路径）的节点按属性覆盖；
    /// 后定义的 compatible 优先；`/delete-property/` 优先于覆盖。
    fn add_node(&mut self, node: DtsNode) {
        let key = node
            .label
            .clone()
            .unwrap_or_else(|| node.path.clone());
        if let Some(&i) = self.by_key.get(&key) {
            let existing = &mut self.nodes[i];
            if node.has_compatible {
                existing.kind = node.kind;
            }
            for key in &node.deleted_props {
                existing.props.retain(|(k, _)| k != key);
            }
            for (prop_key, value) in node.props {
                if node.deleted_props.contains(&prop_key) {
                    continue;
                }
                if let Some(slot) = existing
                    .props
                    .iter_mut()
                    .find(|(k, _)| *k == prop_key)
                {
                    slot.1 = value;
                } else {
                    existing.props.push((prop_key, value));
                }
            }
        } else {
            self.by_key.insert(key, self.nodes.len());
            self.nodes.push(node);
        }
    }

    fn remove_node(&mut self, key: &str) {
        if let Some(&i) = self.by_key.get(key) {
            self.nodes.remove(i);
            self.by_key.clear();
            for (j, node) in self.nodes.iter().enumerate() {
                let key = node.label.clone().unwrap_or_else(|| node.path.clone());
                self.by_key.insert(key, j);
            }
        }
    }
}

struct Parser<'a> {
    toks: Vec<Tok>,
    spans: Vec<(usize, usize)>,
    pos: usize,
    dir: PathBuf,
    file: String,
    depth: usize,
    collector: &'a mut Collector,
}

impl<'a> Parser<'a> {
    /// 带 文件:行:列 的错误。
    fn err(&self, msg: &str) -> syn::Error {
        let (line, col) = self
            .spans
            .get(self.pos)
            .copied()
            .unwrap_or((0, 0));
        dts_err(&format!("{}:{line}:{col}: {msg}", self.file))
    }

    fn peek(&self) -> &Tok {
        &self.toks[self.pos]
    }

    fn peek2(&self) -> &Tok {
        &self.toks[(self.pos + 1).min(self.toks.len() - 1)]
    }

    fn expect_punct(&mut self, p: char) -> Result<()> {
        if self.peek() == &Tok::Punct(p) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.err(&format!("expected `{p}`, got `{:?}`", self.peek())))
        }
    }

    fn expect_num(&mut self) -> Result<u32> {
        match self.peek().clone() {
            Tok::Num(n) => {
                self.pos += 1;
                Ok(n)
            }
            other => Err(self.err(&format!("expected number, got `{other:?}`"))),
        }
    }

    fn parse_top(&mut self) -> Result<()> {
        loop {
            match self.peek().clone() {
                Tok::Eof => return Ok(()),
                Tok::Punct('/') => {
                    self.pos += 1;
                    match self.peek().clone() {
                        Tok::Ident(s) if s == "dts-v1" => {
                            self.pos += 1;
                            self.expect_punct('/')?;
                            self.expect_punct(';')?;
                        }
                        Tok::Ident(s) if s == "include" => {
                            self.pos += 1;
                            self.expect_punct('/')?;
                            self.include()?;
                        }
                        Tok::Ident(s) if s == "delete-node" => {
                            self.pos += 1;
                            self.expect_punct('/')?;
                            let target = self.delete_target()?;
                            self.expect_punct(';')?;
                            self.collector.remove_node(&target);
                        }
                        Tok::Punct('{') => {
                            self.pos += 1;
                            self.node_body(None, "/".to_string(), "/".to_string())?;
                        }
                        other => {
                            return Err(self.err(&format!(
                                "unexpected `/` at top level: `{other:?}`"
                            )))
                        }
                    }
                }
                Tok::Punct('#') => {
                    self.pos += 1;
                    match self.peek().clone() {
                        Tok::Ident(s) if s == "include" => {
                            self.pos += 1;
                        }
                        other => {
                            return Err(self.err(&format!(
                                "expected `include` after `#`, got `{other:?}`"
                            )))
                        }
                    }
                    self.include()?;
                }
                Tok::Ident(_) => {
                    let (label, name) = self.node_start()?;
                    let path = join_path("/", &name);
                    self.parse_node(label, name, path)?;
                }
                Tok::Punct(';') => {
                    self.pos += 1;
                }
                other => {
                    return Err(self.err(&format!(
                        "unexpected token `{other:?}` at top level"
                    )))
                }
            }
        }
    }

    /// 解析节点起始：`[label:] name[@addr]`，消费到 `{` 之前。
    fn node_start(&mut self) -> Result<(Option<String>, String)> {
        if let Tok::Ident(label) = self.peek().clone() {
            if self.peek2() == &Tok::Punct(':') {
                self.pos += 2;
                return Ok((Some(label), self.node_name()?));
            }
        }
        Ok((None, self.node_name()?))
    }

    fn node_name(&mut self) -> Result<String> {
        let name = match self.peek().clone() {
            Tok::Ident(s) => {
                self.pos += 1;
                s
            }
            other => {
                return Err(self.err(&format!("expected node name, got `{other:?}`")))
            }
        };
        if self.peek() == &Tok::Punct('@') {
            self.pos += 1;
            // 单元地址可能是无 0x 前缀的十六进制（如 `i2s@40003c00`），
            // 词法上会拆成 Num("40003") + Ident("c00")，这里拼接回去。
            let mut addr = String::new();
            loop {
                match self.peek().clone() {
                    Tok::Num(n) => {
                        self.pos += 1;
                        addr.push_str(&n.to_string());
                    }
                    Tok::Ident(s)
                        if !s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit()) =>
                    {
                        self.pos += 1;
                        addr.push_str(&s);
                    }
                    _ => break,
                }
            }
            if addr.is_empty() {
                return Err(self.err("expected unit address after `@`"));
            }
            Ok(format!("{name}@{addr}"))
        } else {
            Ok(name)
        }
    }

    fn parse_node(&mut self, label: Option<String>, name: String, path: String) -> Result<()> {
        self.expect_punct('{')?;
        self.node_body(label, name, path)
    }

    fn node_body(&mut self, label: Option<String>, name: String, path: String) -> Result<()> {
        let is_root = name == "/";
        let mut node = DtsNode {
            label,
            path: path.clone(),
            kind: None,
            has_compatible: false,
            props: Vec::new(),
            deleted_props: Vec::new(),
        };

        loop {
            match self.peek().clone() {
                Tok::Punct('}') => {
                    self.pos += 1;
                    self.expect_punct(';')?;
                    break;
                }
                Tok::Eof => {
                    return Err(self.err(&format!(
                        "unexpected end of file inside node `{name}`"
                    )))
                }
                Tok::Punct(';') => {
                    self.pos += 1;
                }
                Tok::Punct('#') => {
                    // `#address-cells` 之类的属性名
                    self.pos += 1;
                    match self.peek().clone() {
                        Tok::Ident(key) => {
                            self.pos += 1;
                            self.property(&mut node, is_root, format!("#{key}"))?;
                        }
                        other => {
                            return Err(self.err(&format!(
                                "unexpected `#` followed by `{other:?}` in node `{name}`"
                            )))
                        }
                    }
                }
                Tok::Punct('/') => {
                    self.pos += 1;
                    match self.peek().clone() {
                        Tok::Ident(s) if s == "delete-property" => {
                            self.pos += 1;
                            self.expect_punct('/')?;
                            let key = match self.peek().clone() {
                                Tok::Ident(k) => {
                                    self.pos += 1;
                                    normalize_key(&k)
                                }
                                Tok::Punct('#') => {
                                    self.pos += 1;
                                    match self.peek().clone() {
                                        Tok::Ident(k) => {
                                            self.pos += 1;
                                            format!("#{k}")
                                        }
                                        other => {
                                            return Err(self.err(&format!(
                                                "expected property name after `#`, got `{other:?}`"
                                            )))
                                        }
                                    }
                                }
                                other => {
                                    return Err(self.err(&format!(
                                        "expected property name after `/delete-property/`, got `{other:?}`"
                                    )))
                                }
                            };
                            self.expect_punct(';')?;
                            node.deleted_props.push(key);
                        }
                        Tok::Ident(s) if s == "delete-node" => {
                            self.pos += 1;
                            self.expect_punct('/')?;
                            let target = self.delete_target()?;
                            self.expect_punct(';')?;
                            self.collector.remove_node(&target);
                        }
                        other => {
                            return Err(self.err(&format!(
                                "unexpected `/` in node `{name}`: `{other:?}`"
                            )))
                        }
                    }
                }
                Tok::Ident(first) => {
                    if self.peek2() == &Tok::Punct(':') {
                        // 子节点带标签：label: name { ... }
                        self.pos += 2;
                        let child_name = self.node_name()?;
                        let child_path = join_path(&path, &child_name);
                        self.parse_node(Some(first), child_name, child_path)?;
                    } else if self.peek2() == &Tok::Punct('=')
                        || self.peek2() == &Tok::Punct(';')
                    {
                        self.pos += 1;
                        self.property(&mut node, is_root, normalize_key(&first))?;
                    } else if self.peek() == &Tok::Punct('@') || self.peek() == &Tok::Punct('{') {
                        // 无标签子节点
                        let child_name = if self.peek() == &Tok::Punct('@') {
                            self.pos += 1;
                            let addr = self.expect_num()?;
                            format!("{first}@{addr}")
                        } else {
                            first
                        };
                        let child_path = join_path(&path, &child_name);
                        self.parse_node(None, child_name, child_path)?;
                    } else {
                        return Err(self.err(&format!(
                            "unexpected token after `{first}` in node `{name}`"
                        )));
                    }
                }
                other => {
                    return Err(self.err(&format!(
                        "unexpected token `{other:?}` in node `{name}`"
                    )))
                }
            }
        }

        if !is_root {
            self.collector.add_node(node);
        }
        Ok(())
    }

    /// `/delete-node/ &label` 或 `/delete-node/ name` 的目标。
    fn delete_target(&mut self) -> Result<String> {
        match self.peek().clone() {
            Tok::Punct('&') => {
                self.pos += 1;
                match self.peek().clone() {
                    Tok::Ident(label) => {
                        self.pos += 1;
                        Ok(label)
                    }
                    other => Err(self.err(&format!(
                        "expected label after `&` in delete-node, got `{other:?}`"
                    ))),
                }
            }
            Tok::Ident(name) => {
                self.pos += 1;
                Ok(name)
            }
            other => Err(self.err(&format!(
                "expected label or node name in delete-node, got `{other:?}`"
            ))),
        }
    }

    /// 属性：`key = value;` 或 `key;`（布尔）。
    fn property(&mut self, node: &mut DtsNode, is_root: bool, key: String) -> Result<()> {
        if self.peek() == &Tok::Punct('=') {
            self.pos += 1;
            let value = self.parse_value()?;
            self.expect_punct(';')?;
            self.apply_prop(node, is_root, key, value);
        } else {
            self.expect_punct(';')?;
            node.props.push((key, DtsPropVal::Bool));
        }
        Ok(())
    }

    fn apply_prop(&mut self, node: &mut DtsNode, is_root: bool, key: String, value: DtsPropVal) {
        if is_root && key == "model" {
            if let DtsPropVal::Str(s) = &value {
                self.collector.model = Some(s.clone());
            }
        }
        if key == "compatible" {
            node.has_compatible = true;
            if let DtsPropVal::Str(s) = &value {
                node.kind = Some(kind_from_compatible(s));
            }
        }
        node.props.push((key, value));
    }

    fn parse_value(&mut self) -> Result<DtsPropVal> {
        match self.peek().clone() {
            Tok::Str(first) => {
                self.pos += 1;
                let mut out = first;
                while let Tok::Str(next) = self.peek().clone() {
                    self.pos += 1;
                    out.push_str(&next);
                }
                Ok(DtsPropVal::Str(out))
            }
            Tok::Punct('<') => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    match self.peek().clone() {
                        Tok::Punct('>') => {
                            self.pos += 1;
                            break;
                        }
                        Tok::Eof => return Err(self.err("unterminated `<...>` value")),
                        Tok::Punct('&') => {
                            self.pos += 1;
                            match self.peek().clone() {
                                Tok::Ident(label) => {
                                    self.pos += 1;
                                    items.push(CellItem::Ref(label));
                                }
                                Tok::Punct('{') => {
                                    self.pos += 1;
                                    let p = match self.peek().clone() {
                                        Tok::Punct('/') => {
                                            self.pos += 1;
                                            let mut p = String::from("/");
                                            loop {
                                                match self.peek().clone() {
                                                    Tok::Ident(s) => {
                                                        self.pos += 1;
                                                        p.push_str(&s);
                                                    }
                                                    Tok::Punct('/') => {
                                                        self.pos += 1;
                                                        p.push('/');
                                                    }
                                                    Tok::Punct('@') => {
                                                        self.pos += 1;
                                                        p.push('@');
                                                    }
                                                    Tok::Num(n) => {
                                                        self.pos += 1;
                                                        p.push_str(&n.to_string());
                                                    }
                                                    Tok::Punct('}') => {
                                                        self.pos += 1;
                                                        break;
                                                    }
                                                    Tok::Eof => {
                                                        return Err(self.err(
                                                            "unterminated `&{/path}` reference",
                                                        ))
                                                    }
                                                    other => {
                                                        return Err(self.err(&format!(
                                                            "unexpected token `{other:?}` in `&{{/path}}`"
                                                        )))
                                                    }
                                                }
                                            }
                                            p
                                        }
                                        other => {
                                            return Err(self.err(&format!(
                                                "expected `/` after `&{{`, got `{other:?}`"
                                            )))
                                        }
                                    };
                                    items.push(CellItem::PathRef(p));
                                }
                                other => {
                                    return Err(self.err(&format!(
                                        "expected label after `&`, got `{other:?}`"
                                    )))
                                }
                            }
                        }
                        Tok::Punct('(') => {
                            // `<(...)>` 整数表达式
                            items.push(CellItem::Num(self.parse_expr()?));
                        }
                        Tok::Punct('-') => {
                            self.pos += 1;
                            let n = self.expect_num()?;
                            items.push(CellItem::Num(n.wrapping_neg()));
                        }
                        Tok::Num(n) => {
                            self.pos += 1;
                            items.push(CellItem::Num(n));
                        }
                        other => {
                            return Err(self.err(&format!(
                                "unexpected token `{other:?}` inside `<...>`"
                            )))
                        }
                    }
                }
                Ok(DtsPropVal::Cells(items))
            }
            Tok::Punct('[') => {
                self.pos += 1;
                let mut bytes = Vec::new();
                loop {
                    match self.peek().clone() {
                        Tok::Punct(']') => {
                            self.pos += 1;
                            break;
                        }
                        Tok::Eof => return Err(self.err("unterminated `[...]` value")),
                        Tok::Num(n) => {
                            self.pos += 1;
                            if n > 0xFF {
                                return Err(self.err(&format!(
                                    "byte `{n:#x}` out of range in `[...]`"
                                )));
                            }
                            bytes.push(n as u8);
                        }
                        Tok::Ident(s) => {
                            self.pos += 1;
                            let n = u32::from_str_radix(&s, 16).map_err(|_| {
                                dts_err(&format!("invalid byte `{s}` in `[...]`"))
                            })?;
                            if n > 0xFF {
                                return Err(self.err(&format!(
                                    "byte `{n:#x}` out of range in `[...]`"
                                )));
                            }
                            bytes.push(n as u8);
                        }
                        other => {
                            return Err(self.err(&format!(
                                "unexpected token `{other:?}` inside `[...]`"
                            )))
                        }
                    }
                }
                Ok(DtsPropVal::Bytes(bytes))
            }
            other => Err(self.err(&format!(
                "expected property value, got `{other:?}`"
            ))),
        }
    }

    /// 整数表达式：`+ - * / %` 与括号，32 位回绕。
    fn parse_expr(&mut self) -> Result<u32> {
        let mut lhs = self.parse_mul()?;
        loop {
            match self.peek().clone() {
                Tok::Punct('+') => {
                    self.pos += 1;
                    lhs = lhs.wrapping_add(self.parse_mul()?);
                }
                Tok::Punct('-') => {
                    self.pos += 1;
                    lhs = lhs.wrapping_sub(self.parse_mul()?);
                }
                _ => break,
            }
        }
        Ok(lhs)
    }

    fn parse_mul(&mut self) -> Result<u32> {
        let mut lhs = self.parse_atom()?;
        loop {
            match self.peek().clone() {
                Tok::Punct('*') => {
                    self.pos += 1;
                    lhs = lhs.wrapping_mul(self.parse_atom()?);
                }
                Tok::Punct('/') => {
                    self.pos += 1;
                    let rhs = self.parse_atom()?;
                    if rhs == 0 {
                        return Err(self.err("division by zero in expression"));
                    }
                    lhs /= rhs;
                }
                Tok::Punct('%') => {
                    self.pos += 1;
                    let rhs = self.parse_atom()?;
                    if rhs == 0 {
                        return Err(self.err("modulo by zero in expression"));
                    }
                    lhs %= rhs;
                }
                _ => break,
            }
        }
        Ok(lhs)
    }

    fn parse_atom(&mut self) -> Result<u32> {
        match self.peek().clone() {
            Tok::Num(n) => {
                self.pos += 1;
                Ok(n)
            }
            Tok::Punct('-') => {
                self.pos += 1;
                Ok(self.parse_atom()?.wrapping_neg())
            }
            Tok::Punct('(') => {
                self.pos += 1;
                let v = self.parse_expr()?;
                self.expect_punct(')')?;
                Ok(v)
            }
            other => Err(self.err(&format!(
                "expected number or `(` in expression, got `{other:?}`"
            ))),
        }
    }

    fn include(&mut self) -> Result<()> {
        let name = match self.peek().clone() {
            Tok::Str(s) => {
                self.pos += 1;
                s
            }
            other => {
                return Err(self.err(&format!(
                    "expected include path string, got `{other:?}`"
                )))
            }
        };
        if self.peek() == &Tok::Punct(';') {
            self.pos += 1;
        }

        let inc = if Path::new(&name).is_absolute() {
            PathBuf::from(&name)
        } else {
            self.dir.join(&name)
        };
        let inc = canonicalize(&inc);
        if self
            .collector
            .files
            .iter()
            .any(|f| f.as_str() == inc.to_string_lossy().as_ref())
        {
            // 已加载过的文件跳过（防重复 include 重复合并）。
            return Ok(());
        }
        let text = std::fs::read_to_string(&inc)
            .map_err(|e| dts_err(&format!("cannot read include `{}`: {e}", inc.display())))?;
        self.collector.files.push(inc.to_string_lossy().into_owned());
        let (toks, spans) = lex(&text, &inc.display().to_string())?;
        let dir = inc.parent().unwrap_or(Path::new(".")).to_path_buf();
        let mut sub = Parser {
            toks,
            spans,
            pos: 0,
            dir,
            file: inc.to_string_lossy().into_owned(),
            depth: self.depth + 1,
            collector: &mut *self.collector,
        };
        sub.parse_top()
    }
}

pub(crate) fn load_dts(entry: &str) -> Result<DtsLoad> {
    let path = canonicalize(&resolve_path(entry)?);
    let mut collector = Collector::default();
    collector.parse_file(&path, 0)?;
    finalize(collector.nodes, collector.model, collector.files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("embassy-dt-dts-{}-{}", std::process::id(), name))
    }

    #[test]
    fn lexes_basic_dts() {
        let toks = lex(
            r#"
            /dts-v1/;
            /* comment */
            a = "x" "y";          // concat
            b = <0x10 20 &label>;
            c = [aa bb];
            d;
            "#,
            "test",
        )
        .unwrap();
        let s = format!("{toks:?}");
        assert!(s.contains("Str(\"x\")"));
        assert!(s.contains("Str(\"y\")"));
        assert!(s.contains("Num(16)"));
        assert!(s.contains("Num(20)"));
        assert!(s.contains("\"label\""));
        assert!(s.contains("Ident(\"aa\")"));
        assert!(s.contains("Ident(\"bb\")"));
    }

    #[test]
    fn parses_dts_with_include_and_overlay() {
        let dir = temp_dir("include");
        fs::create_dir_all(dir.join("boards")).unwrap();
        fs::write(
            dir.join("boards/chip.dtsi"),
            r#"
            / {
                i2c0: i2c@40005400 {
                    compatible = "embassy-dt,bus-i2c";
                    periph = "I2C1";
                };
                led0: led@0 {
                    compatible = "embassy-dt,gpio-out";
                    pin = "PC13";
                    level = "low";
                };
            };
            "#,
        )
        .unwrap();
        fs::write(
            dir.join("boards/board.dts"),
            r#"
            /dts-v1/;
            #include "chip.dtsi"
            / {
                model = "Test-Board";
                i2c0: i2c@40005400 {
                    scl = "PB8";
                    sda = "PB7";
                    frequency = <400000>;
                };
                bme280: bme@76 {
                    compatible = "bosch,bme280";
                    bus = <&i2c0>;
                    addr = <0x76>;
                };
            };
            "#,
        )
        .unwrap();

        let entry = dir.join("boards/board.dts");
        let load = load_dts(entry.to_str().unwrap()).unwrap();
        assert_eq!(load.model.as_deref(), Some("Test-Board"));
        assert_eq!(load.files.len(), 2);

        let i2c = load.nodes.iter().find(|n| n.id == "i2c0").unwrap();
        assert!(matches!(i2c.kind, NodeKindAst::Bus(BusKindAst::I2c)));
        assert_eq!(i2c.prop_str("periph").unwrap(), "I2C1");
        assert_eq!(i2c.prop_str("scl").unwrap(), "PB8");
        assert_eq!(i2c.prop_u32_any(&["freq", "frequency"]), Some(400_000));

        let led = load.nodes.iter().find(|n| n.id == "led0").unwrap();
        assert!(matches!(led.kind, NodeKindAst::Gpio(GpioKindAst::Out)));
        assert_eq!(led.prop_str("level").unwrap(), "low");

        let bme = load.nodes.iter().find(|n| n.id == "bme280").unwrap();
        assert!(bme.deps.iter().any(|d| d == "i2c0"));
        assert_eq!(bme.prop_u32_any(&["addr"]), Some(0x76));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn overlay_overrides_and_deletes() {
        let dir = temp_dir("overlay");
        fs::create_dir_all(dir.join("boards")).unwrap();
        fs::write(
            dir.join("boards/base.dtsi"),
            r#"
            / {
                i2c0: i2c@40005400 {
                    compatible = "embassy-dt,bus-i2c";
                    periph = "I2C1";
                    scl = "PB8";
                    sda = "PB7";
                    frequency = <400000>;
                };
                spi0: spi@40013000 {
                    compatible = "embassy-dt,bus-spi";
                    periph = "SPI1";
                };
                unneeded: dev@99 {
                    compatible = "vendor,thing";
                };
            };
            "#,
        )
        .unwrap();
        fs::write(
            dir.join("boards/board.dts"),
            r#"
            /dts-v1/;
            /include/ "base.dtsi"
            / {
                i2c0: i2c@40005400 {
                    scl = "PB6";
                    frequency = <100000>;
                    /delete-property/ sda;
                };
                /delete-node/ &unneeded;
            };
            "#,
        )
        .unwrap();

        let entry = dir.join("boards/board.dts");
        let load = load_dts(entry.to_str().unwrap()).unwrap();
        let i2c = load.nodes.iter().find(|n| n.id == "i2c0").unwrap();
        assert_eq!(i2c.prop_str("scl").unwrap(), "PB6"); // 覆盖
        assert_eq!(i2c.prop_u32_any(&["freq", "frequency"]), Some(100_000));
        assert!(i2c.prop("sda").is_none()); // 已删除
        assert_eq!(i2c.prop_str("periph").unwrap(), "I2C1"); // 保留
        assert!(!load.nodes.iter().any(|n| n.id == "unneeded"));
        assert!(load.nodes.iter().any(|n| n.id == "spi0"));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parses_expressions() {
        let dir = temp_dir("expr");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("expr.dts");
        fs::write(
            &file,
            r#"
            /dts-v1/;
            / {
                node0: dev@0 {
                    a = <(1 + 2 * 3)>;
                    b = <(0x1000 + 0x54) 7>;
                    c = <(-(1 + 1))>;
                };
            };
            "#,
        )
        .unwrap();
        let load = load_dts(file.to_str().unwrap()).unwrap();
        let node = load.nodes.iter().find(|n| n.id == "node0").unwrap();
        assert_eq!(node.prop_u32_any(&["a"]), Some(7));
        // 多元素 `<...>` 映射为数组属性
        match &node.prop("b").unwrap().value {
            PropValue::Array(v) => assert_eq!(v, &[0x1054, 7]),
            other => panic!("expected array prop, got {other:?}"),
        }
        assert_eq!(node.prop_u32_any(&["c"]), Some((-2i32) as u32));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolves_path_references() {
        let dir = temp_dir("pathref");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("path.dts");
        fs::write(
            &file,
            r#"
            /dts-v1/;
            / {
                soc: soc@40000000 {
                    i2c0: i2c@40005400 {
                        compatible = "embassy-dt,bus-i2c";
                        periph = "I2C1";
                    };
                };
                dev0: dev@0 {
                    compatible = "vendor,thing";
                    bus = <&{/soc@40000000/i2c@40005400}>;
                };
            };
            "#,
        )
        .unwrap();
        let load = load_dts(file.to_str().unwrap()).unwrap();
        let dev = load.nodes.iter().find(|n| n.id == "dev0").unwrap();
        assert!(dev.deps.iter().any(|d| d == "i2c0"));
        assert_eq!(
            dev.prop("bus").and_then(|p| match &p.value {
                PropValue::Str(s) => Some(s.value()),
                _ => None,
            }),
            Some("i2c0".to_string())
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn errors_carry_file_line_column() {
        let dir = temp_dir("errloc");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("bad.dts");
        fs::write(
            &file,
            "// 注释\n/dts-v1/;\n/ {\n    foo\n};\n",
        )
        .unwrap();
        let err = load_dts(file.to_str().unwrap()).unwrap_err();
        let msg = err.to_string();
        // 错误应包含 文件:行:列（foo 在第 4 行）。
        assert!(msg.contains("bad.dts:4:5"), "message was: {msg}");
        fs::remove_dir_all(&dir).ok();
    }
}
