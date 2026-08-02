//! 构建期 DTB（Flattened Device Tree 二进制）解析器。
//!
//! `device_tree! { from = "board.dtb" }` 自动按 `.dtb` 后缀走这里。
//! DTB 不保留 DTS 标签，因此节点 id 使用路径（`/i2c@40005400` →
//! `i2c_40005400`）；`phandle` 引用会被解析为对节点的依赖。

use syn::Result;

use crate::dts::{
    canonicalize, dts_err, finalize, join_path, kind_from_compatible, normalize_key,
    resolve_path, sanitize_ident, CellItem, DtsLoad, DtsNode, DtsPropVal,
};

const FDT_BEGIN_NODE: u32 = 1;
const FDT_END_NODE: u32 = 2;
const FDT_PROP: u32 = 3;
const FDT_NOP: u32 = 4;
const FDT_END: u32 = 9;

pub(crate) fn load_dtb(entry: &str) -> Result<DtsLoad> {
    let path = canonicalize(&resolve_path(entry)?);
    let bytes = std::fs::read(&path)
        .map_err(|e| dts_err(&format!("cannot read `{}`: {e}", path.display())))?;
    let (nodes, model) = parse_fdt(&bytes)?;
    finalize(nodes, model, vec![path.to_string_lossy().into_owned()])
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn u32(&mut self) -> Result<u32> {
        if self.pos + 4 > self.b.len() {
            return Err(dts_err("truncated FDT: unexpected end of data"));
        }
        let v = u32::from_be_bytes(self.b[self.pos..self.pos + 4].try_into().unwrap());
        self.pos += 4;
        Ok(v)
    }

    fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.pos + n > self.b.len() {
            return Err(dts_err("truncated FDT: unexpected end of data"));
        }
        let v = &self.b[self.pos..self.pos + n];
        self.pos += n;
        Ok(v)
    }
}

fn parse_fdt(bytes: &[u8]) -> Result<(Vec<DtsNode>, Option<String>)> {
    let mut r = Reader { b: bytes, pos: 0 };
    let magic = r.u32()?;
    if magic != 0xD00D_FEED {
        return Err(dts_err(&format!(
            "not a DTB file (bad magic {magic:#x})"
        )));
    }
    let _totalsize = r.u32()?;
    let off_struct = r.u32()? as usize;
    let off_strings = r.u32()? as usize;
    let _off_rsvmap = r.u32()?;
    let _version = r.u32()?;
    let _last_comp = r.u32()?;
    let _boot_cpuid = r.u32()?;
    let size_strings = r.u32()? as usize;
    let _size_struct = r.u32()?;

    let strings = bytes
        .get(off_strings..off_strings + size_strings)
        .ok_or_else(|| dts_err("invalid FDT strings block offset"))?;

    let mut sr = Reader {
        b: bytes,
        pos: off_struct,
    };
    let mut nodes: Vec<DtsNode> = Vec::new();
    let mut root_model: Option<String> = None;
    // 打开中的节点栈（栈顶是当前节点）。
    let mut stack: Vec<DtsNode> = Vec::new();

    loop {
        let token = sr.u32()?;
        match token {
            FDT_BEGIN_NODE => {
                let name = read_cstr(&mut sr)?;
                let path = if name == "/" {
                    "/".to_string()
                } else {
                    match stack.last() {
                        Some(parent) => join_path(&parent.path, &name),
                        None => join_path("/", &name),
                    }
                };
                stack.push(DtsNode {
                    label: None,
                    path,
                    kind: None,
                    has_compatible: false,
                    props: Vec::new(),
                    deleted_props: Vec::new(),
                });
            }
            FDT_END_NODE => {
                let node = stack
                    .pop()
                    .ok_or_else(|| dts_err("FDT_END_NODE without matching BEGIN_NODE"))?;
                if node.path == "/" {
                    // 根节点：只提取 model
                    for (key, value) in &node.props {
                        if key == "model" {
                            if let DtsPropVal::Str(s) = value {
                                root_model = Some(s.clone());
                            }
                        }
                    }
                } else {
                    nodes.push(node);
                }
            }
            FDT_PROP => {
                let len = sr.u32()? as usize;
                let name_off = sr.u32()? as usize;
                let value = sr.bytes(len)?.to_vec();
                // 对齐到 4 字节
                let pad = (4 - (len % 4)) % 4;
                sr.pos += pad;
                let name = cstr_at(strings, name_off)?;
                let key = normalize_key(&name);
                let value = decode_value(&value);
                let node = stack
                    .last_mut()
                    .ok_or_else(|| dts_err("FDT_PROP outside of a node"))?;
                if key == "compatible" {
                    node.has_compatible = true;
                    if let DtsPropVal::Str(s) = &value {
                        node.kind = Some(kind_from_compatible(s));
                    }
                }
                node.props.push((key, value));
            }
            FDT_NOP => {}
            FDT_END => break,
            other => {
                return Err(dts_err(&format!(
                    "unknown FDT token {other:#x} in structure block"
                )))
            }
        }
    }

    resolve_phandles(&mut nodes)?;
    Ok((nodes, root_model))
}

/// 把 `phandle` / `linux,phandle` 数值替换为 `&label` 风格的引用。
fn resolve_phandles(nodes: &mut [DtsNode]) -> Result<()> {
    let mut phandle_to_id: std::collections::HashMap<u32, String> = Default::default();
    for node in nodes.iter() {
        for (key, value) in &node.props {
            if key == "phandle" || key == "linux_phandle" {
                if let DtsPropVal::Cells(items) = value {
                    if let Some(CellItem::Num(n)) = items.first() {
                        phandle_to_id.insert(*n, sanitize_ident(node.path.trim_start_matches('/')));
                    }
                }
            }
        }
    }
    for node in nodes.iter_mut() {
        for (_key, value) in node.props.iter_mut() {
            if let DtsPropVal::Cells(items) = value {
                for item in items.iter_mut() {
                    if let CellItem::Num(n) = *item {
                        if let Some(id) = phandle_to_id.get(&n) {
                            *item = CellItem::Ref(id.clone());
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn read_cstr(r: &mut Reader<'_>) -> Result<String> {
    let start = r.pos;
    while r.pos < r.b.len() && r.b[r.pos] != 0 {
        r.pos += 1;
    }
    if r.pos >= r.b.len() {
        return Err(dts_err("truncated FDT string"));
    }
    let s = String::from_utf8_lossy(&r.b[start..r.pos]).into_owned();
    r.pos += 1;
    // 对齐到 4 字节
    let pad = (4 - ((r.pos - start) % 4)) % 4;
    r.pos += pad;
    Ok(s)
}

fn cstr_at(strings: &[u8], off: usize) -> Result<String> {
    let bytes = strings
        .get(off..)
        .ok_or_else(|| dts_err("invalid FDT string offset"))?;
    let end = bytes
        .iter()
        .position(|&b| b == 0)
        .ok_or_else(|| dts_err("unterminated FDT string"))?;
    Ok(String::from_utf8_lossy(&bytes[..end]).into_owned())
}

fn decode_value(v: &[u8]) -> DtsPropVal {
    if v.is_empty() {
        return DtsPropVal::Bool;
    }
    // 字符串属性以 \0 结尾且其余可打印（长度不必是 4 的倍数）。
    let is_string = v[v.len() - 1] == 0
        && v[..v.len() - 1]
            .iter()
            .all(|&b| b.is_ascii_graphic() || b == b' ');
    if is_string {
        return DtsPropVal::Str(String::from_utf8_lossy(&v[..v.len() - 1]).into_owned());
    }
    if v.len() % 4 == 0 {
        let words = v
            .chunks_exact(4)
            .map(|w| u32::from_be_bytes(w.try_into().unwrap()))
            .map(CellItem::Num)
            .collect();
        return DtsPropVal::Cells(words);
    }
    DtsPropVal::Bytes(v.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BusKindAst, NodeKindAst, PropValue};
    use std::fs;

    fn cstr(s: &str) -> Vec<u8> {
        let mut v = s.as_bytes().to_vec();
        v.push(0);
        while v.len() % 4 != 0 {
            v.push(0);
        }
        v
    }

    fn prop(name_off: u32, value: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend(FDT_PROP.to_be_bytes());
        v.extend((value.len() as u32).to_be_bytes());
        v.extend(name_off.to_be_bytes());
        v.extend_from_slice(value);
        while v.len() % 4 != 0 {
            v.push(0);
        }
        v
    }

    fn begin(name: &str) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend(FDT_BEGIN_NODE.to_be_bytes());
        v.extend(cstr(name));
        v
    }

    fn end_node() -> Vec<u8> {
        FDT_END_NODE.to_be_bytes().to_vec()
    }

    fn end() -> Vec<u8> {
        FDT_END.to_be_bytes().to_vec()
    }

    /// 用测试专用迷你编码器生成一棵 DTB。
    fn encode_test_dtb() -> Vec<u8> {
        let strings = [
            "model\0",
            "compatible\0",
            "periph\0",
            "scl\0",
            "sda\0",
            "frequency\0",
            "phandle\0",
            "bus\0",
            "addr\0",
            "embassy-dt,bus-i2c\0",
            "bosch,bme280\0",
        ];
        let mut strings_block = Vec::new();
        let mut offs = Vec::new();
        for s in strings {
            offs.push(strings_block.len() as u32);
            strings_block.extend_from_slice(s.as_bytes());
        }

        let mut st = Vec::new();
        // 根节点
        st.extend(begin("/"));
        st.extend(prop(
            offs[0],
            b"Fdt-Test-Board\0",
        ));
        st.extend(prop(offs[1], b"embassy-dt,demo\0"));
        // /i2c@40005400
        st.extend(begin("i2c@40005400"));
        st.extend(prop(offs[1], b"embassy-dt,bus-i2c\0"));
        st.extend(prop(offs[2], b"I2C1\0"));
        st.extend(prop(offs[3], b"PB8\0"));
        st.extend(prop(offs[4], b"PB7\0"));
        st.extend(prop(offs[5], &400_000u32.to_be_bytes()));
        st.extend(prop(offs[6], &1u32.to_be_bytes()));
        st.extend(end_node());
        // /bme@76
        st.extend(begin("bme@76"));
        st.extend(prop(offs[1], b"bosch,bme280\0"));
        st.extend(prop(offs[7], &1u32.to_be_bytes()));
        st.extend(prop(offs[8], &0x76u32.to_be_bytes()));
        st.extend(end_node());
        st.extend(end_node());
        st.extend(end());

        let off_struct = 40usize + 16; // header + 空 rsvmap
        let off_strings = off_struct + st.len();
        let totalsize = off_strings + strings_block.len();

        let mut out = Vec::new();
        out.extend(0xD00D_FEEDu32.to_be_bytes());
        out.extend((totalsize as u32).to_be_bytes());
        out.extend((off_struct as u32).to_be_bytes());
        out.extend((off_strings as u32).to_be_bytes());
        out.extend(40u32.to_be_bytes()); // off_mem_rsvmap
        out.extend(17u32.to_be_bytes()); // version
        out.extend(16u32.to_be_bytes()); // last_comp_version
        out.extend(0u32.to_be_bytes()); // boot_cpuid_phys
        out.extend((strings_block.len() as u32).to_be_bytes());
        out.extend((st.len() as u32).to_be_bytes());
        out.extend([0u8; 16]); // rsvmap：空
        out.extend(st);
        out.extend(strings_block);
        out
    }

    #[test]
    fn parses_dtb_and_resolves_phandles() {
        let dir = std::env::temp_dir().join(format!(
            "embassy-dt-fdt-{}-test",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("board.dtb");
        fs::write(&file, encode_test_dtb()).unwrap();

        let load = load_dtb(file.to_str().unwrap()).unwrap();

        assert_eq!(load.model.as_deref(), Some("Fdt-Test-Board"));

        let i2c = load
            .nodes
            .iter()
            .find(|n| n.id == "i2c_40005400")
            .unwrap();
        assert!(matches!(i2c.kind, NodeKindAst::Bus(BusKindAst::I2c)));
        assert_eq!(i2c.prop_str("periph").unwrap(), "I2C1");
        assert_eq!(i2c.prop_str("scl").unwrap(), "PB8");
        assert_eq!(i2c.prop_u32_any(&["freq", "frequency"]), Some(400_000));

        let bme = load.nodes.iter().find(|n| n.id == "bme_76").unwrap();
        assert!(matches!(bme.kind, NodeKindAst::Device));
        // phandle 1 → i2c_40005400
        assert!(bme.deps.iter().any(|d| d == "i2c_40005400"));
        assert_eq!(bme.prop_u32_any(&["addr"]), Some(0x76));
        assert!(matches!(
            bme.prop("bus").unwrap().value,
            PropValue::Str(_)
        ));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_non_dtb() {
        let dir = std::env::temp_dir().join(format!(
            "embassy-dt-fdt-{}-bad",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("bad.dtb");
        fs::write(&file, b"not a device tree").unwrap();
        let err = load_dtb(file.to_str().unwrap()).unwrap_err();
        assert!(err.to_string().contains("bad magic"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_cstr_alignment() {
        let data = b"abc\0xy\0\0zzzz\0";
        let mut r = Reader { b: data, pos: 0 };
        assert_eq!(read_cstr(&mut r).unwrap(), "abc");
        assert_eq!(read_cstr(&mut r).unwrap(), "xy");
        assert_eq!(read_cstr(&mut r).unwrap(), "zzzz");
    }
}
