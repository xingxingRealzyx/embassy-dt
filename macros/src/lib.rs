//! `embassy-dt` 的过程宏：`device_tree!`。
//!
//! 两种配置来源：
//!
//! ```text
//! // 1) Rust DSL
//! device_tree! {
//!     name "board-name";                // 可选，默认取 DTS model / "board"
//!     backend stm32;                    // 可选；额外生成 STM32 类型化 Board
//!     bus i2c0: I2c { periph: "I2C1", scl: "PB8", sda: "PB7", freq: 400_000 };
//!     bus uart0: Uart { periph: "USART1", rx: "PA10", tx: "PA9", baud: 115_200 };
//!     gpio led0: Out { pin: "PC13", level: "high" };
//!     periph rng0: Rng { periph: "RNG" };
//!     device bme280: Bme280Driver { bus: i2c0, addr: 0x76 };
//! }
//!
//! // 2) DTS/DTSI 文件（推荐，支持 #include 与板级 overlay 合并）
//! device_tree! {
//!     name "nucleo-h723zi";
//!     backend stm32;
//!     chip "stm32h723zg";
//!     from "boards/nucleo-h723zi.dts";
//! }
//! ```
//!
//! `chip` 用于芯片相关的代码生成规则（如 ADC/CRC/CAN 的构造差异）。
//!
//! 宏在编译期校验：重复 id、悬空依赖、依赖环，全部直接 `compile_error!`。

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{braced, Ident, LitInt, LitStr, Path, Result, Token};

mod dts;
mod fdt;

mod kw {
    syn::custom_keyword!(name);
    syn::custom_keyword!(backend);
    syn::custom_keyword!(from);
    syn::custom_keyword!(bus);
    syn::custom_keyword!(device);
    syn::custom_keyword!(gpio);
    syn::custom_keyword!(periph);
    syn::custom_keyword!(chip);
    syn::custom_keyword!(node);
}

/// `device_tree!` 宏：解析设备树（Rust DSL 或 DTS/DTSI 文件），编译期校验，
/// 生成 `TREE` 静态描述；声明 `backend stm32;` 时额外生成类型化 `Board`。
#[proc_macro]
pub fn device_tree(input: TokenStream) -> TokenStream {
    match expand(syn::parse_macro_input!(input as DslTree)) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BusKindAst {
    I2c,
    Spi,
    Uart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GpioKindAst {
    Out,
    In,
    /// 仅保留引脚所有权（Board 字段就是 `peripherals::PB0` 本身），
    /// 供 ADC 读取等需要原始引脚的场景使用。
    Pin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PeriphKindAst {
    Rng,
    Adc,
    Crc,
    Dac,
    Pwm,
    Can,
    Usb,
    Qei,
    InputCapture,
    Sdmmc,
    I2s,
    PwmInput,
    ComplementaryPwm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backend {
    Stm32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeKindAst {
    Bus(BusKindAst),
    Gpio(GpioKindAst),
    Peripheral(PeriphKindAst),
    Device,
}

#[derive(Debug)]
enum PropValue {
    Str(LitStr),
    U32(LitInt),
    Ref(Ident),
    Array(Vec<u32>),
    Bool(bool),
}

#[derive(Debug)]
struct DslProp {
    key: Ident,
    value: PropValue,
}

#[derive(Debug)]
struct DslNode {
    id: Ident,
    kind: NodeKindAst,
    #[allow(dead_code)] // 设备驱动类型，后续阶段用于生成设备句柄
    driver: Option<Path>,
    props: Vec<DslProp>,
    deps: Vec<Ident>,
}

#[derive(Debug)]
struct DslTree {
    name: Option<LitStr>,
    backend: Option<Backend>,
    chip: Option<LitStr>,
    from: Option<LitStr>,
    nodes: Vec<DslNode>,
}

impl Parse for DslTree {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut tree_name = None;
        let mut backend_kind = None;
        let mut chip = None;
        let mut from = None;
        let mut nodes = Vec::new();

        while !input.is_empty() {
            if input.peek(kw::name) {
                input.parse::<kw::name>()?;
                tree_name = Some(input.parse()?);
                input.parse::<Token![;]>()?;
            } else if input.peek(kw::backend) {
                input.parse::<kw::backend>()?;
                let kw_ident: Ident = input.parse()?;
                input.parse::<Token![;]>()?;
                backend_kind = Some(match kw_ident.to_string().as_str() {
                    "stm32" => Backend::Stm32,
                    other => {
                        return Err(syn::Error::new(
                            kw_ident.span(),
                            format!("unknown backend `{other}`; supported: `stm32`"),
                        ))
                    }
                });
            } else if input.peek(kw::chip) {
                input.parse::<kw::chip>()?;
                chip = Some(input.parse()?);
                input.parse::<Token![;]>()?;
            } else if input.peek(kw::from) {
                input.parse::<kw::from>()?;
                from = Some(input.parse()?);
                input.parse::<Token![;]>()?;
            } else {
                nodes.push(input.parse()?);
            }
        }

        Ok(Self {
            name: tree_name,
            backend: backend_kind,
            chip,
            from,
            nodes,
        })
    }
}

impl Parse for DslNode {
    fn parse(input: ParseStream) -> Result<Self> {
        let (is_bus, is_device, is_gpio, is_periph, is_node) = (
            input.peek(kw::bus),
            input.peek(kw::device),
            input.peek(kw::gpio),
            input.peek(kw::periph),
            input.peek(kw::node),
        );
        if !is_bus && !is_device && !is_gpio && !is_periph && !is_node {
            return Err(input.error("expected `bus`, `device`, `gpio`, `periph` or `node`"));
        }
        if is_bus {
            input.parse::<kw::bus>()?;
        } else if is_device {
            input.parse::<kw::device>()?;
        } else if is_gpio {
            input.parse::<kw::gpio>()?;
        } else if is_periph {
            input.parse::<kw::periph>()?;
        } else {
            input.parse::<kw::node>()?;
        }

        let id: Ident = input.parse()?;

        if is_node {
            // `node clock { ... };` —— 文档性节点（无类型、无 driver）。
            let content;
            braced!(content in input);
            let props: Punctuated<DslProp, Token![,]> =
                content.parse_terminated(DslProp::parse, Token![,])?;
            let props: Vec<DslProp> = props.into_iter().collect();
            let deps: Vec<Ident> = props
                .iter()
                .filter_map(|p| match &p.value {
                    PropValue::Ref(ident) => Some(ident.clone()),
                    _ => None,
                })
                .collect();
            input.parse::<Option<Token![;]>>()?;
            return Ok(Self {
                id,
                kind: NodeKindAst::Device,
                driver: None,
                props,
                deps,
            });
        }

        input.parse::<Token![:]>()?;

        let (kind, driver) = if is_bus {
            let kw_ident: Ident = input.parse()?;
            let kind = match kw_ident.to_string().as_str() {
                "I2c" => BusKindAst::I2c,
                "Spi" => BusKindAst::Spi,
                "Uart" => BusKindAst::Uart,
                other => {
                    return Err(syn::Error::new(
                        kw_ident.span(),
                        format!("unknown bus kind `{other}`; expected `I2c`, `Spi` or `Uart`"),
                    ))
                }
            };
            (NodeKindAst::Bus(kind), None)
        } else if is_gpio {
            let kw_ident: Ident = input.parse()?;
            let kind = match kw_ident.to_string().as_str() {
                "Out" => GpioKindAst::Out,
                "In" => GpioKindAst::In,
                "Pin" => GpioKindAst::Pin,
                other => {
                    return Err(syn::Error::new(
                        kw_ident.span(),
                        format!("unknown gpio kind `{other}`; expected `Out`, `In` or `Pin`"),
                    ))
                }
            };
            (NodeKindAst::Gpio(kind), None)
        } else if is_periph {
            let kw_ident: Ident = input.parse()?;
            let kind = match kw_ident.to_string().as_str() {
                "Rng" => PeriphKindAst::Rng,
                "Adc" => PeriphKindAst::Adc,
                "Crc" => PeriphKindAst::Crc,
                "Dac" => PeriphKindAst::Dac,
                "Pwm" => PeriphKindAst::Pwm,
                "Can" => PeriphKindAst::Can,
                "Usb" => PeriphKindAst::Usb,
                "Qei" => PeriphKindAst::Qei,
                "InputCapture" => PeriphKindAst::InputCapture,
                "Sdmmc" => PeriphKindAst::Sdmmc,
                "I2s" => PeriphKindAst::I2s,
                "PwmInput" => PeriphKindAst::PwmInput,
                "ComplementaryPwm" => PeriphKindAst::ComplementaryPwm,
                other => {
                    return Err(syn::Error::new(
                        kw_ident.span(),
                        format!(
                            "unknown peripheral kind `{other}`; expected `Rng`/`Adc`/`Crc`/`Dac`/`Pwm`/`Can`/`Usb`/`Qei`/`InputCapture`/`Sdmmc`/`I2s`/`PwmInput`/`ComplementaryPwm`"
                        ),
                    ))
                }
            };
            (NodeKindAst::Peripheral(kind), None)
        } else {
            (NodeKindAst::Device, Some(input.parse()?))
        };

        let content;
        braced!(content in input);
        let props: Punctuated<DslProp, Token![,]> =
            content.parse_terminated(DslProp::parse, Token![,])?;
        let props: Vec<DslProp> = props.into_iter().collect();
        let deps: Vec<Ident> = props
            .iter()
            .filter_map(|p| match &p.value {
                PropValue::Ref(ident) => Some(ident.clone()),
                _ => None,
            })
            .collect();

        // 节点后的分号可选（`}` 后）。
        input.parse::<Option<Token![;]>>()?;

        Ok(Self {
            id,
            kind,
            driver,
            props,
            deps,
        })
    }
}

impl Parse for DslProp {
    fn parse(input: ParseStream) -> Result<Self> {
        let key: Ident = input.parse()?;
        input.parse::<Token![:]>()?;
        let value = if input.peek(LitStr) {
            PropValue::Str(input.parse()?)
        } else if input.peek(LitInt) {
            PropValue::U32(input.parse()?)
        } else if input.peek(Ident) {
            PropValue::Ref(input.parse()?)
        } else {
            return Err(input.error("expected string, integer or node reference"));
        };
        Ok(Self { key, value })
    }
}

impl DslNode {
    fn prop(&self, key: &str) -> Option<&DslProp> {
        self.props.iter().find(|p| p.key == key)
    }

    fn prop_str(&self, key: &str) -> Result<String> {
        let prop = self.prop(key).ok_or_else(|| {
            syn::Error::new(
                self.id.span(),
                format!("`{}` is missing required prop `{key}`", self.id),
            )
        })?;
        match &prop.value {
            PropValue::Str(lit) => Ok(lit.value()),
            _ => Err(syn::Error::new(
                prop.key.span(),
                format!("prop `{key}` must be a string"),
            )),
        }
    }

    fn prop_u32_any(&self, keys: &[&str]) -> Option<u32> {
        keys.iter()
            .find_map(|key| self.prop(key))
            .and_then(|prop| match &prop.value {
                PropValue::U32(lit) => lit.base10_parse().ok(),
                _ => None,
            })
    }

    /// 数组属性（DTS 的 `<a b c>` 多元素）。
    fn prop_array(&self, key: &str) -> Option<&[u32]> {
        match self.prop(key).map(|p| &p.value) {
            Some(PropValue::Array(v)) => Some(v),
            _ => None,
        }
    }

    fn prop_str_opt(&self, key: &str) -> Result<Option<String>> {
        match self.prop(key) {
            None => Ok(None),
            Some(prop) => match &prop.value {
                PropValue::Str(s) => Ok(Some(s.value())),
                _ => Err(syn::Error::new(
                    prop.key.span(),
                    format!("prop `{key}` must be a string"),
                )),
            },
        }
    }

    /// 布尔属性：DTS 的 `exti;`（Bool）或 DSL 的 `exti: 1` / `exti: "true"`。
    fn prop_bool(&self, key: &str) -> bool {
        match self.prop(key).map(|p| &p.value) {
            Some(PropValue::Bool(b)) => *b,
            Some(PropValue::U32(lit)) => lit
                .base10_parse::<u32>()
                .map(|v| v != 0)
                .unwrap_or(false),
            Some(PropValue::Str(s)) => s.value() == "true" || s.value() == "1",
            _ => false,
        }
    }

    fn pin_ident(&self, key: &str) -> Result<Ident> {
        let pin = self.prop_str(key)?;
        syn::parse_str::<Ident>(&pin).map_err(|_| {
            syn::Error::new(
                self.id.span(),
                format!("prop `{key}` value `{pin}` is not a valid pin name"),
            )
        })
    }

    fn pin_ident_opt(&self, key: &str) -> Result<Option<Ident>> {
        if self.prop(key).is_none() {
            return Ok(None);
        }
        self.pin_ident(key).map(Some)
    }

    fn dma_channel(&self, key: &str) -> Result<(String, Ident)> {
        let name = self.prop_str(key)?;
        let ident = syn::parse_str::<Ident>(&name).map_err(|_| {
            syn::Error::new(
                self.id.span(),
                format!("prop `{key}` value `{name}` is not a valid DMA channel name"),
            )
        })?;
        Ok((name, ident))
    }
}

fn expand(mut tree: DslTree) -> Result<TokenStream2> {
    let mut track = TokenStream2::new();
    if let Some(from) = tree.from.take() {
        if !tree.nodes.is_empty() {
            return Err(syn::Error::new(
                from.span(),
                "cannot combine inline nodes with `from` (use either Rust DSL nodes or a DTS file)",
            ));
        }
        let path = from.value();
        let loaded = if path.ends_with(".dtb") {
            fdt::load_dtb(&path)
        } else {
            dts::load_dts(&path)
        }
        .map_err(|e| {
            syn::Error::new(from.span(), format!("failed to load `{path}`: {e}"))
        })?;
        tree.nodes = loaded.nodes;
        if tree.name.is_none() {
            if let Some(model) = loaded.model {
                tree.name = Some(LitStr::new(&model, Span::call_site()));
            }
        }
        // 让 rustc 跟踪所有加载的文件：DTS 改动会触发重新编译。
        for file in loaded.files {
            let lit = LitStr::new(&file, Span::call_site());
            track.extend(quote! {
                const _: &[u8] = include_bytes!(#lit);
            });
        }
    }

    validate(&tree)?;

    let name = tree
        .name
        .as_ref()
        .map(|lit| lit.value())
        .unwrap_or_else(|| "board".to_string());
    let name_lit = LitStr::new(&name, Span::call_site());

    let nodes = tree.nodes.iter().map(node_expr);
    let mut out = quote! {
        #[allow(non_upper_case_globals)]
        pub static TREE: ::embassy_dt::TreeDesc = ::embassy_dt::TreeDesc::new(
            #name_lit,
            &[ #(#nodes),* ],
        );
        #track
    };

    if tree.backend.is_some() {
        out.extend(stm32_board(&tree)?);
    }

    Ok(out)
}

fn node_expr(node: &DslNode) -> TokenStream2 {
    let id = LitStr::new(&node.id.to_string(), node.id.span());
    let kind = match &node.kind {
        NodeKindAst::Bus(kind) => {
            let kind = match kind {
                BusKindAst::I2c => quote!(I2c),
                BusKindAst::Spi => quote!(Spi),
                BusKindAst::Uart => quote!(Uart),
            };
            quote!(::embassy_dt::NodeKind::Bus(::embassy_dt::BusKind::#kind))
        }
        NodeKindAst::Gpio(_) => quote!(::embassy_dt::NodeKind::Gpio),
        NodeKindAst::Peripheral(_) => quote!(::embassy_dt::NodeKind::Peripheral),
        NodeKindAst::Device => quote!(::embassy_dt::NodeKind::Device),
    };
    let deps = node.deps.iter().map(|d| LitStr::new(&d.to_string(), d.span()));
    let props = node.props.iter().map(|p| {
        let key = LitStr::new(&p.key.to_string(), p.key.span());
        let value = match &p.value {
            PropValue::Str(lit) => quote!(::embassy_dt::Prop::Str(#lit)),
            PropValue::U32(lit) => quote!(::embassy_dt::Prop::U32(#lit)),
            PropValue::Ref(ident) => {
                let s = LitStr::new(&ident.to_string(), ident.span());
                quote!(::embassy_dt::Prop::Str(#s))
            }
            PropValue::Array(vals) => {
                let vals = vals.iter();
                quote!(::embassy_dt::Prop::Array(&[#(#vals),*]))
            }
            PropValue::Bool(b) => quote!(::embassy_dt::Prop::Bool(#b)),
        };
        quote!((#key, #value))
    });
    quote! {
        ::embassy_dt::NodeDesc::new(
            #id,
            #kind,
            &[#(#deps),*],
            &[#(#props),*],
        )
    }
}

/// 编译期校验：重复 id、悬空依赖、依赖环。
fn validate(tree: &DslTree) -> Result<()> {
    for (i, node) in tree.nodes.iter().enumerate() {
        if tree.nodes[..i].iter().any(|prev| prev.id == node.id) {
            return Err(syn::Error::new(
                node.id.span(),
                format!("duplicate node id `{}`", node.id),
            ));
        }
    }

    for node in &tree.nodes {
        for dep in &node.deps {
            if !tree.nodes.iter().any(|n| &n.id == dep) {
                return Err(syn::Error::new(
                    dep.span(),
                    format!("node `{}` depends on unknown node `{}`", node.id, dep),
                ));
            }
        }
    }

    // Kahn 拓扑排序检测环：入度 = 节点自身的依赖数；无依赖的节点先出队，
    // 依赖它的节点入度减一，减到零即可上线。
    let n = tree.nodes.len();
    let index_of = |id: &Ident| tree.nodes.iter().position(|n| &n.id == id).unwrap();
    let mut indeg = vec![0usize; n];
    for node in &tree.nodes {
        indeg[index_of(&node.id)] = node.deps.len();
    }
    let mut queue: Vec<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
    let mut removed = 0usize;
    while let Some(i) = queue.pop() {
        removed += 1;
        for (j, node) in tree.nodes.iter().enumerate() {
            if node.deps.iter().any(|d| index_of(d) == i) {
                indeg[j] -= 1;
                if indeg[j] == 0 {
                    queue.push(j);
                }
            }
        }
    }
    if removed < n {
        for (i, node) in tree.nodes.iter().enumerate() {
            if indeg[i] > 0 {
                return Err(syn::Error::new(
                    node.id.span(),
                    format!("dependency cycle involving node `{}`", node.id),
                ));
            }
        }
    }

    Ok(())
}

/// 为 `backend stm32;` 生成类型化 `Board`。
///
/// 支持的节点：
///
/// - `bus ...: I2c`：async I2C + DMA（`periph`/`scl`/`sda`/`dma_tx`/`dma_rx`/`freq|frequency`）
/// - `bus ...: Spi`：async SPI + DMA（`periph`/`sck`/`mosi`/`miso`/`dma_tx`/`dma_rx`/`freq|frequency`）
/// - `bus ...: Uart`：async UART + DMA（`periph`/`rx`/`tx`/`dma_tx`/`dma_rx`/`baud|baudrate`）
/// - `gpio ...: Out`：输出（`pin`/`level` = high|low）
/// - `gpio ...: In`：输入（`pin`/`pull` = up|down|none）
/// - `periph ...: Rng`：硬件随机数（`periph`）
/// - `periph ...: Adc`：ADC（`periph`；H723 无中断，F411 需 `ADC` 中断）
/// - `periph ...: Crc`：CRC（`periph`；H723 带 Config，F411 无）
/// - `periph ...: Dac`：DAC 通道（`periph`/`pin`，阻塞模式）
/// - `periph ...: Pwm`：定时器简单 PWM（`periph`/`ch1..ch4` 可选/`freq`）
/// - `periph ...: Can`：CAN（`periph`/`rx`/`tx`；H723 FDCAN / F4 bxCAN）
///
/// 部分外设的构造方式因芯片而异，需要声明 `chip "stm32h723zg"` 等。
///
/// 引脚 AF、DMA 兼容性、中断绑定全部由 embassy-stm32 的类型系统在编译期
/// 保证（例如 `PB8` 若不是 `I2C1` 的合法 SCL 引脚，代码直接编译不过）。
fn stm32_board(tree: &DslTree) -> Result<TokenStream2> {
    let chip = chip_name(tree)?;
    let mut fields = Vec::new();
    let mut inits = Vec::new();
    // 模块级 static（如 USB 端点缓冲）。
    let mut statics = Vec::new();
    // 中断绑定条目，按中断名去重。
    let mut bindings: Vec<(String, TokenStream2)> = Vec::new();
    // 需要实例化的设备（driver 类型来自 DSL）。
    let mut devices: Vec<(&DslNode, &Path)> = Vec::new();

    for node in &tree.nodes {
        match &node.kind {
            NodeKindAst::Bus(BusKindAst::I2c) => {
                let periph = periph_ident(node)?;
                let scl = node.pin_ident("scl")?;
                let sda = node.pin_ident("sda")?;
                let (dma_tx_str, dma_tx) = node.dma_channel("dma_tx")?;
                let (dma_rx_str, dma_rx) = node.dma_channel("dma_rx")?;
                let freq = node.prop_u32_any(&["freq", "frequency"]).unwrap_or(100_000);
                let field = &node.id;
                let shared = node.prop_bool("shared");

                push_dma_bindings(&mut bindings, &dma_tx_str, &dma_tx, node, chip.as_deref())?;
                push_dma_bindings(&mut bindings, &dma_rx_str, &dma_rx, node, chip.as_deref())?;
                let ev_irq = format_ident!("{}_EV", periph.to_string());
                let er_irq = format_ident!("{}_ER", periph.to_string());
                push_binding(&mut bindings, ev_irq, quote! {
                    ::embassy_stm32::i2c::EventInterruptHandler<::embassy_stm32::peripherals::#periph>
                });
                push_binding(&mut bindings, er_irq, quote! {
                    ::embassy_stm32::i2c::ErrorInterruptHandler<::embassy_stm32::peripherals::#periph>
                });

                if shared {
                    let mux_name = format_ident!("{}_MUTEX", field.to_string());
                    statics.push(quote! {
                        #[allow(non_upper_case_globals)]
                        static mut #mux_name: Option<
                            ::embassy_sync::mutex::Mutex<
                                ::embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
                                ::embassy_stm32::i2c::I2c<
                                    'static,
                                    ::embassy_stm32::mode::Async,
                                    ::embassy_stm32::i2c::mode::Master,
                                >,
                            >,
                        > = None;
                    });
                    fields.push(quote! {
                        pub #field: ::embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice<
                            'static,
                            ::embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
                            ::embassy_stm32::i2c::I2c<
                                'static,
                                ::embassy_stm32::mode::Async,
                                ::embassy_stm32::i2c::mode::Master,
                            >,
                        >
                    });
                    inits.push(quote! {
                        #field: {
                            let raw = {
                                let mut config = ::embassy_stm32::i2c::Config::default();
                                config.frequency = ::embassy_stm32::time::Hertz(#freq);
                                ::embassy_stm32::i2c::I2c::new(
                                    p.#periph, p.#scl, p.#sda,
                                    p.#dma_tx, p.#dma_rx,
                                    DtIrqs,
                                    config,
                                )
                            };
                            unsafe {
                                #mux_name = Some(::embassy_sync::mutex::Mutex::new(raw));
                            }
                            ::embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice::new(
                                unsafe { #mux_name.as_ref() }.expect("i2c mutex"),
                            )
                        },
                    });
                } else {
                    fields.push(quote! {
                        pub #field: ::embassy_stm32::i2c::I2c<
                            'static,
                            ::embassy_stm32::mode::Async,
                            ::embassy_stm32::i2c::mode::Master,
                        >
                    });
                    inits.push(quote! {
                        #field: {
                            let mut config = ::embassy_stm32::i2c::Config::default();
                            config.frequency = ::embassy_stm32::time::Hertz(#freq);
                            ::embassy_stm32::i2c::I2c::new(
                                p.#periph, p.#scl, p.#sda,
                                p.#dma_tx, p.#dma_rx,
                                DtIrqs,
                                config,
                            )
                        },
                    });
                }
            }
            NodeKindAst::Bus(BusKindAst::Spi) => {
                let periph = periph_ident(node)?;
                let sck = node.pin_ident("sck")?;
                let mosi = node.pin_ident("mosi")?;
                let miso = node.pin_ident("miso")?;
                let (dma_tx_str, dma_tx) = node.dma_channel("dma_tx")?;
                let (dma_rx_str, dma_rx) = node.dma_channel("dma_rx")?;
                let freq = node.prop_u32_any(&["freq", "frequency"]).unwrap_or(1_000_000);
                let field = &node.id;
                let is_slave = node
                    .prop_str("mode")
                    .map(|m| m == "slave")
                    .unwrap_or(false);
                let shared = node.prop_bool("shared");

                push_dma_bindings(&mut bindings, &dma_tx_str, &dma_tx, node, chip.as_deref())?;
                push_dma_bindings(&mut bindings, &dma_rx_str, &dma_rx, node, chip.as_deref())?;

                if shared {
                    let mux_name = format_ident!("{}_MUTEX", field.to_string());
                    statics.push(quote! {
                        #[allow(non_upper_case_globals)]
                        static mut #mux_name: Option<
                            ::embassy_sync::mutex::Mutex<
                                ::embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
                                ::embassy_stm32::spi::Spi<
                                    'static,
                                    ::embassy_stm32::mode::Async,
                                    ::embassy_stm32::spi::mode::Master,
                                >,
                            >,
                        > = None;
                    });
                    fields.push(quote! {
                        /// 共享 SPI 总线互斥体（设备用 `SpiDevice` 包 CS 访问）。
                        pub #field: &'static ::embassy_sync::mutex::Mutex<
                            ::embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
                            ::embassy_stm32::spi::Spi<
                                'static,
                                ::embassy_stm32::mode::Async,
                                ::embassy_stm32::spi::mode::Master,
                            >,
                        >
                    });
                    inits.push(quote! {
                        #field: {
                            let raw = {
                                let mut config = ::embassy_stm32::spi::Config::default();
                                config.frequency = ::embassy_stm32::time::Hertz(#freq);
                                ::embassy_stm32::spi::Spi::new(
                                    p.#periph, p.#sck, p.#mosi, p.#miso,
                                    p.#dma_tx, p.#dma_rx,
                                    DtIrqs,
                                    config,
                                )
                            };
                            unsafe {
                                #mux_name = Some(::embassy_sync::mutex::Mutex::new(raw));
                            }
                            unsafe { #mux_name.as_ref() }.expect("spi mutex")
                        },
                    });
                } else if is_slave {
                    let cs = node.pin_ident("cs")?;
                    fields.push(quote! {
                        pub #field: ::embassy_stm32::spi::Spi<
                            'static,
                            ::embassy_stm32::mode::Async,
                            ::embassy_stm32::spi::mode::Slave,
                        >
                    });
                    inits.push(quote! {
                        #field: {
                            let mut config = ::embassy_stm32::spi::Config::default();
                            config.frequency = ::embassy_stm32::time::Hertz(#freq);
                            ::embassy_stm32::spi::Spi::new_slave(
                                p.#periph, p.#sck, p.#mosi, p.#miso, p.#cs,
                                p.#dma_tx, p.#dma_rx,
                                DtIrqs,
                                config,
                            )
                        },
                    });
                } else {
                    fields.push(quote! {
                        pub #field: ::embassy_stm32::spi::Spi<
                            'static,
                            ::embassy_stm32::mode::Async,
                            ::embassy_stm32::spi::mode::Master,
                        >
                    });
                    inits.push(quote! {
                        #field: {
                            let mut config = ::embassy_stm32::spi::Config::default();
                            config.frequency = ::embassy_stm32::time::Hertz(#freq);
                            ::embassy_stm32::spi::Spi::new(
                                p.#periph, p.#sck, p.#mosi, p.#miso,
                                p.#dma_tx, p.#dma_rx,
                                DtIrqs,
                                config,
                            )
                        },
                    });
                }
            }
            NodeKindAst::Bus(BusKindAst::Uart) => {
                let periph = periph_ident(node)?;
                let rx = node.pin_ident("rx")?;
                let tx = node.pin_ident("tx")?;
                let (dma_tx_str, dma_tx) = node.dma_channel("dma_tx")?;
                let (dma_rx_str, dma_rx) = node.dma_channel("dma_rx")?;
                let baud = node.prop_u32_any(&["baud", "baudrate"]).unwrap_or(115_200);
                let field = &node.id;

                push_binding(&mut bindings, periph.clone(), quote! {
                    ::embassy_stm32::usart::InterruptHandler<::embassy_stm32::peripherals::#periph>
                });
                push_dma_bindings(&mut bindings, &dma_tx_str, &dma_tx, node, chip.as_deref())?;
                push_dma_bindings(&mut bindings, &dma_rx_str, &dma_rx, node, chip.as_deref())?;

                fields.push(quote! {
                    pub #field: ::embassy_stm32::usart::Uart<'static, ::embassy_stm32::mode::Async>
                });
                inits.push(quote! {
                    #field: {
                        let mut config = ::embassy_stm32::usart::Config::default();
                        config.baudrate = #baud;
                        ::embassy_stm32::usart::Uart::new(
                            p.#periph, p.#rx, p.#tx,
                            p.#dma_tx, p.#dma_rx,
                            DtIrqs,
                            config,
                        )
                        .expect("embassy-dt: invalid UART configuration")
                    },
                });
            }
            NodeKindAst::Gpio(gpio_kind) => {
                let pin = node.pin_ident("pin")?;
                let field = &node.id;
                match gpio_kind {
                    GpioKindAst::Out => {
                        let level = match node.prop_str("level")?.as_str() {
                            "high" => quote!(High),
                            "low" => quote!(Low),
                            other => {
                                return Err(syn::Error::new(
                                    node.id.span(),
                                    format!("gpio `{}`: `level` must be `high` or `low`, got `{other}`", node.id),
                                ))
                            }
                        };
                        fields.push(quote! {
                            pub #field: ::embassy_stm32::gpio::Output<'static>
                        });
                        inits.push(quote! {
                            #field: ::embassy_stm32::gpio::Output::new(
                                p.#pin,
                                ::embassy_stm32::gpio::Level::#level,
                                ::embassy_stm32::gpio::Speed::VeryHigh,
                            ),
                        });
                    }
                    GpioKindAst::In => {
                        let pull = match node.prop_str("pull")?.as_str() {
                            "up" => quote!(Up),
                            "down" => quote!(Down),
                            "none" => quote!(None),
                            other => {
                                return Err(syn::Error::new(
                                    node.id.span(),
                                    format!("gpio `{}`: `pull` must be `up`/`down`/`none`, got `{other}`", node.id),
                                ))
                            }
                        };
                        if node.prop_bool("exti") {
                            // 异步输入：EXTI 中断驱动（Wait trait）。
                            let (ch, irq, tl) = exti_info(&pin.to_string())?;
                            push_binding(&mut bindings, irq, quote! {
                                ::embassy_stm32::exti::InterruptHandler<::embassy_stm32::interrupt::typelevel::#tl>
                            });
                            fields.push(quote! {
                                pub #field: ::embassy_stm32::exti::ExtiInput<'static, ::embassy_stm32::mode::Async>
                            });
                            inits.push(quote! {
                                #field: ::embassy_stm32::exti::ExtiInput::new(
                                    p.#pin,
                                    p.#ch,
                                    ::embassy_stm32::gpio::Pull::#pull,
                                    DtIrqs,
                                ),
                            });
                        } else {
                            fields.push(quote! {
                                pub #field: ::embassy_stm32::gpio::Input<'static>
                            });
                            inits.push(quote! {
                                #field: ::embassy_stm32::gpio::Input::new(
                                    p.#pin,
                                    ::embassy_stm32::gpio::Pull::#pull,
                                ),
                            });
                        }
                    }
                    GpioKindAst::Pin => {
                        // 原始引脚所有权：Peripherals 字段是 `Peri<'static, PB0>`，
                        // 直接保留给 ADC 等 API 使用。
                        fields.push(quote! {
                            pub #field: ::embassy_stm32::Peri<'static, ::embassy_stm32::peripherals::#pin>
                        });
                        inits.push(quote! {
                            #field: p.#pin,
                        });
                    }
                }
            }
            NodeKindAst::Peripheral(kind) => {
                let periph = periph_ident(node)?;
                let field = &node.id;
                match kind {
                    PeriphKindAst::Rng => {
                        push_binding(&mut bindings, periph.clone(), quote! {
                            ::embassy_stm32::rng::InterruptHandler<::embassy_stm32::peripherals::#periph>
                        });
                        fields.push(quote! {
                            pub #field: ::embassy_stm32::rng::Rng<'static, ::embassy_stm32::peripherals::#periph>
                        });
                        inits.push(quote! {
                            #field: ::embassy_stm32::rng::Rng::new(p.#periph, DtIrqs),
                        });
                    }
                    PeriphKindAst::Adc => {
                        match chip_name(tree)? {
                            // H723 是 adc_v3、F411/L4 是 adc_v2：构造都是 `Adc::new(adc)`；
                            // G4 是专用 adc_g4，多一个 config 参数；
                            // F1 是 adc_f1：`Adc::new(adc)` + ADC1_2 中断绑定。
                            Some(chip)
                                if chip.contains("h723")
                                    || chip.contains("f411")
                                    || chip.contains("l476")
                                    || chip.contains("g4") =>
                            {
                                fields.push(quote! {
                                    pub #field: ::embassy_stm32::adc::Adc<'static, ::embassy_stm32::peripherals::#periph>
                                });
                                if chip.contains("g4") {
                                    inits.push(quote! {
                                        #field: ::embassy_stm32::adc::Adc::new(
                                            p.#periph,
                                            ::embassy_stm32::adc::AdcConfig::default(),
                                        ),
                                    });
                                } else {
                                    inits.push(quote! {
                                        #field: ::embassy_stm32::adc::Adc::new(p.#periph),
                                    });
                                }
                            }
                            Some(chip) if chip.contains("f103") || chip.contains("f1") => {
                                // F103 的 ADC1/ADC2 共用 ADC1_2 中断。
                                push_binding(&mut bindings, Ident::new("ADC1_2", Span::call_site()), quote! {
                                    ::embassy_stm32::adc::InterruptHandler<::embassy_stm32::peripherals::#periph>
                                });
                                fields.push(quote! {
                                    pub #field: ::embassy_stm32::adc::Adc<'static, ::embassy_stm32::peripherals::#periph>
                                });
                                inits.push(quote! {
                                    #field: ::embassy_stm32::adc::Adc::new(p.#periph),
                                });
                            }
                            _ => {
                                return Err(syn::Error::new(
                                    node.id.span(),
                                    "stm32 backend: `adc` is not supported for this chip (supported: stm32h723zg / stm32f411ce / stm32l476rg / stm32g474re / stm32f103c8)",
                                ))
                            }
                        }
                    }
                    PeriphKindAst::Crc => {
                        match chip_name(tree)? {
                            Some(chip) if chip.contains("h723") => {
                                fields.push(quote! {
                                    pub #field: ::embassy_stm32::crc::Crc<'static>
                                });
                                inits.push(quote! {
                                    #field: ::embassy_stm32::crc::Crc::new(
                                        p.#periph,
                                        ::embassy_stm32::crc::Config::new(
                                            ::embassy_stm32::crc::InputReverseConfig::None,
                                            false,
                                            ::embassy_stm32::crc::PolySize::Width32,
                                            0xFFFF_FFFF,
                                            0x04C1_1DB7,
                                        )
                                        .expect("embassy-dt: invalid CRC config"),
                                    ),
                                });
                            }
                            Some(chip)
                                if chip.contains("f411")
                                    || chip.contains("l476")
                                    || chip.contains("f103")
                                    || chip.contains("f1") =>
                            {
                                fields.push(quote! {
                                    pub #field: ::embassy_stm32::crc::Crc<'static>
                                });
                                inits.push(quote! {
                                    #field: ::embassy_stm32::crc::Crc::new(p.#periph),
                                });
                            }
                            _ => {
                                return Err(syn::Error::new(
                                    node.id.span(),
                                    "stm32 backend: `crc` is not supported for this chip (declare `chip \"stm32h723zg\"` or `chip \"stm32f411ce\"`)",
                                ))
                            }
                        }
                    }
                    PeriphKindAst::Dac => {
                        let pin = node.pin_ident("pin")?;
                        fields.push(quote! {
                            pub #field: ::embassy_stm32::dac::DacChannel<'static, ::embassy_stm32::mode::Blocking>
                        });
                        inits.push(quote! {
                            #field: ::embassy_stm32::dac::DacChannel::new_blocking(p.#periph, p.#pin),
                        });
                    }
                    PeriphKindAst::Pwm => {
                        let ch1 = node.pin_ident_opt("ch1")?;
                        let ch2 = node.pin_ident_opt("ch2")?;
                        let ch3 = node.pin_ident_opt("ch3")?;
                        let ch4 = node.pin_ident_opt("ch4")?;
                        let freq = node.prop_u32_any(&["freq", "frequency"]).unwrap_or(1_000);
                        // F1（AFIO）的引脚可能同时实现多个 remap 候选
                        // （如 PA0 同时是 TIM2_CH1 的 AfioRemap<0> 和 <2>），
                        // 生成代码显式选择默认 remap 0。
                        let afio = chip_name(tree)?
                            .as_deref()
                            .map(|c| c.contains("f1"))
                            .unwrap_or(false);
                        let ch1 = pwm_pin(ch1, afio);
                        let ch2 = pwm_pin(ch2, afio);
                        let ch3 = pwm_pin(ch3, afio);
                        let ch4 = pwm_pin(ch4, afio);
                        fields.push(quote! {
                            pub #field: ::embassy_stm32::timer::simple_pwm::SimplePwm<'static, ::embassy_stm32::peripherals::#periph>
                        });
                        inits.push(quote! {
                            #field: ::embassy_stm32::timer::simple_pwm::SimplePwm::new(
                                p.#periph,
                                #ch1, #ch2, #ch3, #ch4,
                                ::embassy_stm32::time::Hertz(#freq),
                                ::embassy_stm32::timer::low_level::CountingMode::EdgeAlignedUp,
                            ),
                        });
                    }
                    PeriphKindAst::Can => {
                        let rx = node.pin_ident("rx")?;
                        let tx = node.pin_ident("tx")?;
                        match chip_name(tree)? {
                            Some(chip) if chip.contains("h723") || chip.contains("g4") => {
                                let it0 = format_ident!("{}_IT0", periph.to_string());
                                let it1 = format_ident!("{}_IT1", periph.to_string());
                                push_binding(&mut bindings, it0, quote! {
                                    ::embassy_stm32::can::IT0InterruptHandler<::embassy_stm32::peripherals::#periph>
                                });
                                push_binding(&mut bindings, it1, quote! {
                                    ::embassy_stm32::can::IT1InterruptHandler<::embassy_stm32::peripherals::#periph>
                                });
                                fields.push(quote! {
                                    pub #field: ::embassy_stm32::can::Can<'static>
                                });
                                inits.push(quote! {
                                    #field: ::embassy_stm32::can::CanConfigurator::new(
                                        p.#periph, p.#rx, p.#tx, DtIrqs,
                                    )
                                    .into_normal_mode(),
                                });
                            }
                            Some(chip) if chip.contains("f4") || chip.contains("f1") => {
                                // F1 的 CAN 中断与 USB 共享向量：TX=USB_HP_CAN1_TX、
                                // RX0=USB_LP_CAN1_RX0（因此 USB 与 CAN 不能同树）。
                                let f1 = chip.contains("f1");
                                let tx_irq = if f1 {
                                    Ident::new("USB_HP_CAN1_TX", Span::call_site())
                                } else {
                                    format_ident!("{}_TX", periph.to_string())
                                };
                                let rx0 = if f1 {
                                    Ident::new("USB_LP_CAN1_RX0", Span::call_site())
                                } else {
                                    format_ident!("{}_RX0", periph.to_string())
                                };
                                let rx1 = if f1 {
                                    Ident::new("CAN1_RX1", Span::call_site())
                                } else {
                                    format_ident!("{}_RX1", periph.to_string())
                                };
                                let sce = if f1 {
                                    Ident::new("CAN1_SCE", Span::call_site())
                                } else {
                                    format_ident!("{}_SCE", periph.to_string())
                                };
                                push_binding(&mut bindings, tx_irq, quote! {
                                    ::embassy_stm32::can::TxInterruptHandler<::embassy_stm32::peripherals::#periph>
                                });
                                push_binding(&mut bindings, rx0, quote! {
                                    ::embassy_stm32::can::Rx0InterruptHandler<::embassy_stm32::peripherals::#periph>
                                });
                                push_binding(&mut bindings, rx1, quote! {
                                    ::embassy_stm32::can::Rx1InterruptHandler<::embassy_stm32::peripherals::#periph>
                                });
                                push_binding(&mut bindings, sce, quote! {
                                    ::embassy_stm32::can::SceInterruptHandler<::embassy_stm32::peripherals::#periph>
                                });
                                fields.push(quote! {
                                    pub #field: ::embassy_stm32::can::Can<'static>
                                });
                                inits.push(quote! {
                                    #field: ::embassy_stm32::can::Can::new(p.#periph, p.#rx, p.#tx, DtIrqs),
                                });
                            }
                            _ => {
                                return Err(syn::Error::new(
                                    node.id.span(),
                                    "stm32 backend: `can` is not supported for this chip (declare `chip \"stm32h723zg\"` / `chip \"stm32g474re\"` / `chip \"stm32f103c8\"` or a F4 chip with bxCAN)",
                                ))
                            }
                        }
                    }
                    PeriphKindAst::Usb => {
                        let dp = node.pin_ident("dp")?;
                        let dm = node.pin_ident("dm")?;
                        let chip = chip_name(tree)?;
                        let ep_out =
                            node.prop_u32_any(&["ep_out", "ep_out_buf"]).unwrap_or(256) as usize;
                        let ep_buf = format_ident!("{}_EP_BUF", field.to_string());
                        let periph_str = periph.to_string();
                        // G4/F1 的 USB 是 usb_v1（非 OTG）：外设 `USB`。
                        // G4 中断 `USB_LP`；F103 是合并向量 `USB_LP_CAN1_RX0`。
                        // OTG 芯片的外设名是 `USB_OTG_FS/HS`，中断同名。
                        let irq_name = if chip.as_deref().map(|c| c.contains("f1")).unwrap_or(false) {
                            "USB_LP_CAN1_RX0".to_string()
                        } else if chip.as_deref().map(|c| c.contains("g4")).unwrap_or(false) {
                            format!("{}_LP", periph_str)
                        } else {
                            periph_str
                                .strip_prefix("USB_")
                                .unwrap_or(&periph_str)
                                .to_string()
                        };
                        let irq = format_ident!("{}", irq_name);
                        push_binding(&mut bindings, irq, quote! {
                            ::embassy_stm32::usb::InterruptHandler<::embassy_stm32::peripherals::#periph>
                        });
                        fields.push(quote! {
                            pub #field: ::embassy_stm32::usb::Driver<'static, ::embassy_stm32::peripherals::#periph>
                        });
                        match chip {
                            Some(chip) if chip.contains("h723") => {
                                statics.push(quote! {
                                    #[allow(non_upper_case_globals)]
                                    static mut #ep_buf: [u8; #ep_out] = [0; #ep_out];
                                });
                                inits.push(quote! {
                                    #field: unsafe {
                                        ::embassy_stm32::usb::Driver::new_hs(
                                            p.#periph, DtIrqs, p.#dp, p.#dm,
                                            &mut #ep_buf,
                                            ::embassy_stm32::usb::Config::default(),
                                        )
                                    },
                                });
                            }
                            Some(chip) if chip.contains("f411") => {
                                statics.push(quote! {
                                    #[allow(non_upper_case_globals)]
                                    static mut #ep_buf: [u8; #ep_out] = [0; #ep_out];
                                });
                                inits.push(quote! {
                                    #field: unsafe {
                                        ::embassy_stm32::usb::Driver::new_fs(
                                            p.#periph, DtIrqs, p.#dp, p.#dm,
                                            &mut #ep_buf,
                                            ::embassy_stm32::usb::Config::default(),
                                        )
                                    },
                                });
                            }
                            Some(chip) if chip.contains("g4") || chip.contains("f1") => {
                                // usb_v1：无需端点缓冲，非 unsafe，无 SOF 引脚。
                                inits.push(quote! {
                                    #field: ::embassy_stm32::usb::Driver::new(
                                        p.#periph, DtIrqs, p.#dp, p.#dm,
                                    ),
                                });
                            }
                            _ => {
                                return Err(syn::Error::new(
                                    node.id.span(),
                                    "stm32 backend: `usb` is not supported for this chip (declare `chip \"stm32h723zg\"` / `chip \"stm32f411ce\"` / `chip \"stm32g474re\"` / `chip \"stm32f103c8\"`)",
                                ))
                            }
                        }
                    }
                    PeriphKindAst::Qei => {
                        let ch1 = node.pin_ident("ch1")?;
                        let ch2 = node.pin_ident("ch2")?;
                        fields.push(quote! {
                            pub #field: ::embassy_stm32::timer::qei::Qei<'static, ::embassy_stm32::peripherals::#periph>
                        });
                        inits.push(quote! {
                            #field: ::embassy_stm32::timer::qei::Qei::new(
                                p.#periph, p.#ch1, p.#ch2,
                                ::embassy_stm32::timer::qei::Config::default(),
                            ),
                        });
                    }
                    PeriphKindAst::InputCapture => {
                        let ch1 = capture_pin(node.pin_ident_opt("ch1")?);
                        let ch2 = capture_pin(node.pin_ident_opt("ch2")?);
                        let ch3 = capture_pin(node.pin_ident_opt("ch3")?);
                        let ch4 = capture_pin(node.pin_ident_opt("ch4")?);
                        let freq = node
                            .prop_u32_any(&["freq", "frequency"])
                            .unwrap_or(1_000_000);
                        push_binding(&mut bindings, cc_irq(&periph), quote! {
                            ::embassy_stm32::timer::CaptureCompareInterruptHandler<::embassy_stm32::peripherals::#periph>
                        });
                        fields.push(quote! {
                            pub #field: ::embassy_stm32::timer::input_capture::InputCapture<'static, ::embassy_stm32::peripherals::#periph>
                        });
                        inits.push(quote! {
                            #field: ::embassy_stm32::timer::input_capture::InputCapture::new(
                                p.#periph,
                                #ch1, #ch2, #ch3, #ch4,
                                DtIrqs,
                                ::embassy_stm32::time::Hertz(#freq),
                                ::embassy_stm32::timer::low_level::CountingMode::EdgeAlignedUp,
                            ),
                        });
                    }
                    PeriphKindAst::Sdmmc => {
                        let clk = node.pin_ident("clk")?;
                        let cmd = node.pin_ident("cmd")?;
                        let d0 = node.pin_ident("d0")?;
                        match chip_name(tree)? {
                            Some(chip) if chip.contains("h723") => {
                                push_binding(&mut bindings, periph.clone(), quote! {
                                    ::embassy_stm32::sdmmc::InterruptHandler<::embassy_stm32::peripherals::#periph>
                                });
                                fields.push(quote! {
                                    pub #field: ::embassy_stm32::sdmmc::Sdmmc<'static>
                                });
                                inits.push(quote! {
                                    #field: ::embassy_stm32::sdmmc::Sdmmc::new_1bit(
                                        p.#periph, DtIrqs,
                                        p.#clk, p.#cmd, p.#d0,
                                        ::embassy_stm32::sdmmc::Config::default(),
                                    ),
                                });
                            }
                            _ => {
                                return Err(syn::Error::new(
                                    node.id.span(),
                                    "stm32 backend: `sdmmc` is not supported for this chip (supported: stm32h723zg)",
                                ))
                            }
                        }
                    }
                    PeriphKindAst::I2s => {
                        let sd = node.pin_ident("sd")?;
                        let ws = node.pin_ident("ws")?;
                        let ck = node.pin_ident("ck")?;
                        let (dma_str, dma) = node.dma_channel("dma")?;
                        let buf_words =
                            node.prop_u32_any(&["buffer", "buf"]).unwrap_or(256) as usize;
                        let buf = format_ident!("{}_DMA_BUF", field.to_string());
                        push_dma_bindings(&mut bindings, &dma_str, &dma, node, chip.as_deref())?;
                        fields.push(quote! {
                            pub #field: ::embassy_stm32::i2s::I2S<'static, u16>
                        });
                        statics.push(quote! {
                            #[allow(non_upper_case_globals)]
                            static mut #buf: [u16; #buf_words] = [0; #buf_words];
                        });
                        inits.push(quote! {
                            #field: unsafe {
                                ::embassy_stm32::i2s::I2S::new_txonly_nomck(
                                    p.#periph, p.#sd, p.#ws, p.#ck,
                                    p.#dma,
                                    &mut #buf,
                                    DtIrqs,
                                    ::embassy_stm32::i2s::Config::default(),
                                )
                            },
                        });
                    }
                    PeriphKindAst::PwmInput => {
                        let pin = node.pin_ident("pin")?;
                        let freq = node
                            .prop_u32_any(&["freq", "frequency"])
                            .unwrap_or(10_000);
                        push_binding(&mut bindings, cc_irq(&periph), quote! {
                            ::embassy_stm32::timer::CaptureCompareInterruptHandler<::embassy_stm32::peripherals::#periph>
                        });
                        fields.push(quote! {
                            pub #field: ::embassy_stm32::timer::pwm_input::PwmInput<'static, ::embassy_stm32::peripherals::#periph>
                        });
                        inits.push(quote! {
                            #field: ::embassy_stm32::timer::pwm_input::PwmInput::new_ch1(
                                p.#periph, p.#pin, DtIrqs,
                                ::embassy_stm32::gpio::Pull::None,
                                ::embassy_stm32::time::Hertz(#freq),
                            ),
                        });
                    }
                    PeriphKindAst::ComplementaryPwm => {
                        let afio = chip_name(tree)?
                            .as_deref()
                            .map(|c| c.contains("f1"))
                            .unwrap_or(false);
                        let ch1 = pwm_pin(node.pin_ident_opt("ch1")?, afio);
                        let ch1n = comp_pin(node.pin_ident_opt("ch1n")?);
                        let ch2 = pwm_pin(node.pin_ident_opt("ch2")?, afio);
                        let ch2n = comp_pin(node.pin_ident_opt("ch2n")?);
                        let ch3 = pwm_pin(node.pin_ident_opt("ch3")?, afio);
                        let ch3n = comp_pin(node.pin_ident_opt("ch3n")?);
                        let ch4 = pwm_pin(node.pin_ident_opt("ch4")?, afio);
                        let ch4n = comp_pin(node.pin_ident_opt("ch4n")?);
                        let freq = node
                            .prop_u32_any(&["freq", "frequency"])
                            .unwrap_or(10_000);
                        fields.push(quote! {
                            pub #field: ::embassy_stm32::timer::complementary_pwm::ComplementaryPwm<'static, ::embassy_stm32::peripherals::#periph>
                        });
                        inits.push(quote! {
                            #field: ::embassy_stm32::timer::complementary_pwm::ComplementaryPwm::new(
                                p.#periph,
                                #ch1, #ch1n, #ch2, #ch2n,
                                #ch3, #ch3n, #ch4, #ch4n,
                                ::embassy_stm32::time::Hertz(#freq),
                                ::embassy_stm32::timer::low_level::CountingMode::EdgeAlignedUp,
                            ),
                        });
                    }
                }
            }
            NodeKindAst::Device => {
                let Some(driver) = &node.driver else {
                    continue; // 无 driver 的 device 节点保持文档性
                };
                for dep in &node.deps {
                    let dep_node = tree.nodes.iter().find(|n| &n.id == dep).ok_or_else(|| {
                        syn::Error::new(
                            dep.span(),
                            format!("device `{}` depends on unknown node `{}`", node.id, dep),
                        )
                    })?;
                    match dep_node.kind {
                        NodeKindAst::Device => {
                            return Err(syn::Error::new(
                                node.id.span(),
                                "device-to-device dependencies are not supported yet \
                                 (v1 设备独占总线；设备依赖设备留待共享总线)",
                            ))
                        }
                        NodeKindAst::Bus(_) | NodeKindAst::Gpio(_) | NodeKindAst::Peripheral(_) => {}
                    }
                }
                devices.push((node, driver));
            }
        }
    }

    // 共享总线检查：非共享总线只能被一个设备依赖；`shared` 总线允许多设备。
    for (i, (node, _)) in devices.iter().enumerate() {
        for dep in &node.deps {
            let dep_node = tree.nodes.iter().find(|n| &n.id == dep).unwrap();
            if dep_node.prop_bool("shared") {
                continue;
            }
            if let Some((prev, _)) = devices[..i]
                .iter()
                .find(|(prev, _)| prev.deps.iter().any(|d| d == dep))
            {
                return Err(syn::Error::new(
                    node.id.span(),
                    format!(
                        "bus `{dep}` is shared by devices `{}` and `{}`; \
                         shared buses are not supported yet",
                        prev.id, node.id
                    ),
                ));
            }
        }
    }

    if fields.is_empty() {
        return Err(syn::Error::new(
            Span::call_site(),
            "stm32 backend: the tree must declare at least one bus, gpio or periph node",
        ));
    }

    let irqs = bindings.iter().map(|(irq, handler)| {
        let irq = format_ident!("{}", irq);
        quote! {
            #irq => #handler;
        }
    });

    let device_tokens = if devices.is_empty() {
        TokenStream2::new()
    } else {
        let fields = devices.iter().map(|(node, driver)| {
            let id = &node.id;
            quote! { pub #id: #driver }
        });
        let inits = devices
            .iter()
            .map(|(node, driver)| -> Result<TokenStream2> {
            let id = &node.id;
            let dev_ty = format_ident!("{}_Ty", id.to_string());
            let id_str = LitStr::new(&id.to_string(), id.span());
            // 共享 SPI 设备：Pin 类型的依赖是 CS（已被 SpiDevice 包装消耗），
            // 不重复传给驱动。
            let has_shared_spi = node.deps.iter().any(|d| {
                tree.nodes.iter().any(|n| {
                    &n.id == d
                        && matches!(n.kind, NodeKindAst::Bus(BusKindAst::Spi))
                        && n.prop_bool("shared")
                })
            });
            let args = node
                .deps
                .iter()
                .filter(|dep| {
                    if !has_shared_spi {
                        return true;
                    }
                    !matches!(
                        tree.nodes
                            .iter()
                            .find(|n| &n.id == *dep)
                            .map(|n| n.kind),
                        Some(NodeKindAst::Gpio(GpioKindAst::Pin))
                    )
                })
                .map(|dep| {
                let field = format_ident!("{}", dep.to_string());
                let dep_node = tree.nodes.iter().find(|n| &n.id == dep).unwrap();
                match dep_node.kind {
                    NodeKindAst::Bus(BusKindAst::I2c) if dep_node.prop_bool("shared") => {
                        // 共享 I2C：驱动拿到 Clone 的共享代理。
                        Ok(quote! { self.#field.clone() })
                    }
                    NodeKindAst::Bus(BusKindAst::Spi) if dep_node.prop_bool("shared") => {
                        // 共享 SPI：用设备自己的 CS 引脚包一层 SpiDevice。
                        let cs = node
                            .deps
                            .iter()
                            .find(|d| {
                                tree.nodes.iter().any(|n| {
                                    &n.id == *d
                                        && matches!(
                                            n.kind,
                                            NodeKindAst::Gpio(GpioKindAst::Pin)
                                        )
                                })
                            })
                            .ok_or_else(|| {
                                syn::Error::new(
                                    node.id.span(),
                                    format!(
                                        "device `{}` on shared SPI `{dep}` needs a `gpio ...: Pin` CS dependency",
                                        node.id
                                    ),
                                )
                            })?;
                        let cs_field = format_ident!("{}", cs.to_string());
                        Ok(quote! {
                            ::embassy_embedded_hal::shared_bus::asynch::spi::SpiDevice::new(
                                self.#field,
                                ::embassy_stm32::gpio::Output::new(
                                    self.#cs_field,
                                    ::embassy_stm32::gpio::Level::High,
                                    ::embassy_stm32::gpio::Speed::VeryHigh,
                                ),
                            )
                        })
                    }
                    _ => Ok(quote! { self.#field }),
                }
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(quote! {
                // 类型别名规避宏输出中 `Type<...>>::method` 的解析限制。
                type #dev_ty = #driver;
                let #id = #dev_ty::init(#(#args),*, &TREE.node(#id_str).unwrap()).await?;
            })
            })
            .collect::<Result<Vec<_>>>()?;
        let members = devices.iter().map(|(node, _)| {
            let id = &node.id;
            quote! { #id }
        });
        quote! {
            /// 由 `device_tree!` 生成的设备集合（依赖的总线所有权已移入驱动）。
            #[allow(non_upper_case_globals)]
            pub struct BoardDevices {
                #(#fields),*
            }

            impl Board {
                /// 按依赖序构造设备。
                ///
                /// 驱动约定：`DriverType::init(deps..., &NodeDesc) -> Result<Self, DeviceError>`
                /// （deps 按设备树中依赖的顺序按值传入）。
                pub async fn init_devices(
                    self,
                ) -> Result<BoardDevices, ::embassy_dt::DeviceError> {
                    #(#inits)*
                    Ok(BoardDevices { #(#members),* })
                }
            }
        }
    };

    let clock_tokens = clock_config_tokens(tree)?;

    Ok(quote! {
        /// 由 `device_tree!` 为 STM32 后端生成的类型化板级结构。
        #[allow(non_upper_case_globals)]
        pub struct Board {
            #(#fields),*
        }

        impl Board {
            /// 从 Embassy `Peripherals` 构造板级实例（消耗外设所有权）。
            pub fn init(p: ::embassy_stm32::Peripherals) -> Self {
                Self { #(#inits)* }
            }
        }

        /// 由 `device_tree!` 自动生成的中断绑定。
        ::embassy_stm32::bind_interrupts!(struct DtIrqs {
            #(#irqs)*
        });

        #(#statics)*

        #device_tokens

        #clock_tokens
    })
}


fn periph_ident(node: &DslNode) -> Result<Ident> {
    let name = node.prop_str("periph")?;
    syn::parse_str::<Ident>(&name).map_err(|_| {
        syn::Error::new(
            node.id.span(),
            "stm32 backend: prop `periph` must be a valid peripheral name like `I2C1`",
        )
    })
}

/// `chip "stm32h723zg"` 声明；芯片相关外设（ADC/CRC/CAN）需要它。
fn chip_name(tree: &DslTree) -> Result<Option<String>> {
    Ok(tree
        .chip
        .as_ref()
        .map(|lit| lit.value()))
}

/// 设备树 `clock` 节点 → `clock_config()` 函数。
///
/// v1 语法（DTS 根节点下的 `clock {}` 子节点）：
///
/// ```dts
/// clock {
///     source = "hsi";              // PLL 源：hsi / csi / hse
///     pll1 = <4 50 2>;             // H7：<prediv mul divp [divq [divr]]>
///     sys = "pll1_p";
///     ahb = <2>; apb1 = <2>;       // 总线分频
///     usb = "hsi48";               // H7 USB 时钟源
///     hsi48;                       // 启用 HSI48（sync from USB）
///     voltage = "scale1";
/// };
/// ```
fn clock_config_tokens(tree: &DslTree) -> Result<TokenStream2> {
    let Some(clock) = tree.nodes.iter().find(|n| n.id == "clock") else {
        return Ok(TokenStream2::new());
    };
    // v2 意图式：只写目标频率，宏自动计算 PLL/分频。
    if let Some(sysclk) = clock.prop_u32_any(&["system"]) {
        return intent_clock(tree, clock, sysclk);
    }
    let stmts = match chip_name(tree)? {
        Some(chip) if chip.contains("h723") => h7_clock_config(clock)?,
        Some(chip) if chip.contains("f411") => f4_clock_config(clock)?,
        Some(chip) if chip.contains("l476") => l4_clock_config(clock)?,
        Some(chip) if chip.contains("g4") => g4_clock_config(clock)?,
        Some(chip) if chip.contains("f103") || chip.contains("f1") => {
            f1_clock_config(clock)?
        }
        _ => {
            return Err(syn::Error::new(
                clock.id.span(),
                "stm32 backend: `clock` node is not supported for this chip",
            ))
        }
    };
    Ok(quote! {
        /// 由 `device_tree!` 生成的时钟配置（传给 `embassy_stm32::init`）。
        pub fn clock_config() -> ::embassy_stm32::Config {
            let mut config = ::embassy_stm32::Config::default();
            #(#stmts)*
            config
        }
    })
}

/// 意图式时钟：`system = <400000000>; usb = <48000000>;`
/// 宏根据芯片的 PLL VCO 范围自动计算分频/倍频，再复用 v1 的代码生成。
fn intent_clock(tree: &DslTree, clock: &DslNode, sysclk: u32) -> Result<TokenStream2> {
    // 与 v1 显式属性互斥。
    for key in ["pll1", "pll", "sys", "ahb", "apb1", "clk48"] {
        if clock.prop(key).is_some() {
            return Err(syn::Error::new(
                clock.id.span(),
                format!(
                    "clock: cannot mix intent (`system`) with explicit property `{key}`"
                ),
            ));
        }
    }
    let usb = clock.prop_u32_any(&["usb"]);
    let hse = clock.prop_u32_any(&["hse"]);
    let i2s = clock.prop_u32_any(&["i2s"]);
    let adc = clock.prop_u32_any(&["adc"]);
    let sdmmc = clock.prop_u32_any(&["sdmmc"]);
    let node = match chip_name(tree)? {
        Some(chip) if chip.contains("h723") => {
            let plan = plan_h7(sysclk, usb, hse, i2s, adc, sdmmc).map_err(|msg| {
                syn::Error::new(clock.id.span(), format!("clock: {msg}"))
            })?;
            synth_h7_node(&plan)
        }
        Some(chip) if chip.contains("f411") => {
            let plan = plan_f4(sysclk, usb, hse).map_err(|msg| {
                syn::Error::new(clock.id.span(), format!("clock: {msg}"))
            })?;
            synth_f4_node(&plan)
        }
        Some(chip) if chip.contains("l476") => {
            let plan = plan_l4(sysclk, usb, hse).map_err(|msg| {
                syn::Error::new(clock.id.span(), format!("clock: {msg}"))
            })?;
            synth_l4_node(&plan)
        }
        Some(chip) if chip.contains("g4") => {
            let plan = plan_g4(sysclk, usb, hse, adc).map_err(|msg| {
                syn::Error::new(clock.id.span(), format!("clock: {msg}"))
            })?;
            synth_g4_node(&plan)
        }
        Some(chip) if chip.contains("f103") || chip.contains("f1") => {
            let plan = plan_f1(sysclk, usb, hse, adc).map_err(|msg| {
                syn::Error::new(clock.id.span(), format!("clock: {msg}"))
            })?;
            synth_f1_node(&plan)
        }
        _ => {
            return Err(syn::Error::new(
                clock.id.span(),
                "stm32 backend: `clock` node is not supported for this chip",
            ))
        }
    };
    let stmts = match chip_name(tree)? {
        Some(chip) if chip.contains("h723") => h7_clock_config(&node)?,
        Some(chip) if chip.contains("f411") => f4_clock_config(&node)?,
        Some(chip) if chip.contains("l476") => l4_clock_config(&node)?,
        Some(chip) if chip.contains("g4") => g4_clock_config(&node)?,
        Some(chip) if chip.contains("f103") || chip.contains("f1") => {
            f1_clock_config(&node)?
        }
        _ => unreachable!(),
    };
    Ok(quote! {
        /// 由 `device_tree!` 生成的时钟配置（意图式：按目标频率自动计算）。
        pub fn clock_config() -> ::embassy_stm32::Config {
            let mut config = ::embassy_stm32::Config::default();
            #(#stmts)*
            config
        }
    })
}

// ---------------------------------------------------------------------------
// 意图式时钟算法（v2）
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct H7Plan {
    prediv: u32,
    mul: u32,
    divp: u32,
    ahb: u32,
    apb: u32,
    scale1: bool,
    usb48: bool,
    source_hse: bool,
    pll2: Option<(u32, u32, u32, u32)>,
    spi123: Option<&'static str>,
    adc_sel: Option<&'static str>,
    sdmmc_sel: Option<&'static str>,
}

/// STM32H7：HSI 64 MHz 输入，PLL1 VCO 192–836 MHz，PLLN 4–512，PLLM 1–64，
/// PLLP 1–64；SYSCLK ≤ 550 MHz，AHB ≤ 275 MHz，APB ≤ 137.5 MHz。
fn plan_h7(
    sysclk: u32,
    usb: Option<u32>,
    hse: Option<u32>,
    i2s: Option<u32>,
    adc: Option<u32>,
    sdmmc: Option<u32>,
) -> std::result::Result<H7Plan, String> {
    // HSE 存在时用外部晶振作为 PLL 源，否则用内部 HSI 64 MHz。
    let input = hse.unwrap_or(64_000_000) as u64;
    let source_hse = hse.is_some();
    if let Some(h) = hse {
        if !(1_000_000..=50_000_000).contains(&h) {
            return Err(format!("HSE frequency {h} Hz out of range (1–50 MHz)"));
        }
    }
    const VCO_MIN: u64 = 192_000_000;
    const VCO_MAX: u64 = 836_000_000;

    if sysclk > 550_000_000 {
        return Err(format!("system clock {sysclk} Hz exceeds H723 maximum (550 MHz)"));
    }
    if let Some(u) = usb {
        if u != 48_000_000 {
            return Err("only 48 MHz USB clock is supported".into());
        }
    }

    // 外设时钟：SPI1/2/3 与 I2S 共用 PLL2_P，ADC 也用 PLL2_P，
    // SDMMC 用 PLL2_R。P 输出只有一个频率。
    if let (Some(i), Some(a)) = (i2s, adc) {
        if i != a {
            return Err(
                "`i2s` and `adc` share PLL2_P output; use the same frequency".into(),
            );
        }
    }
    let freq_p = i2s.or(adc);
    let pll2 = plan_h7_pll2(freq_p, sdmmc, input)?;
    let spi123 = i2s.map(|_| "pll2_p");
    let adc_sel = adc.map(|_| "pll2_p");
    let sdmmc_sel = sdmmc.map(|_| "pll2_r");

    // 找 VCO 最小的合法 (prediv, mul, divp)。
    let mut best: Option<(u64, u32, u32, u32)> = None;
    for divp in 1u32..=64 {
        let vco = sysclk as u64 * divp as u64;
        if !(VCO_MIN..=VCO_MAX).contains(&vco) {
            continue;
        }
        for prediv in 1u32..=64 {
            let num = vco * prediv as u64;
            if num % input != 0 {
                continue;
            }
            let mul = num / input;
            if (4..=512).contains(&mul) {
                let cand = (vco, prediv, mul as u32, divp);
                if best.as_ref().map_or(true, |b| cand.0 < b.0) {
                    best = Some(cand);
                }
            }
        }
    }
    let (_, prediv, mul, divp) = best
        .ok_or_else(|| format!("cannot find a valid PLL1 config for {sysclk} Hz"))?;
    let ahb = pick_div(sysclk, 275_000_000, &[1, 2, 4, 8, 16, 64, 128, 256, 512])
        .ok_or("cannot satisfy AHB frequency limit")?;
    let apb = pick_div(sysclk / ahb, 137_500_000, &[1, 2, 4, 8, 16])
        .ok_or("cannot satisfy APB frequency limit")?;
    Ok(H7Plan {
        prediv,
        mul,
        divp,
        ahb,
        apb,
        scale1: sysclk <= 400_000_000,
        usb48: usb.is_some(),
        source_hse,
        pll2,
        spi123,
        adc_sel,
        sdmmc_sel,
    })
}

/// PLL2 计算：P 输出（I2S/ADC）与 R 输出（SDMMC）共用一个 VCO。
fn plan_h7_pll2(
    freq_p: Option<u32>,
    freq_r: Option<u32>,
    input: u64,
) -> std::result::Result<Option<(u32, u32, u32, u32)>, String> {
    if freq_p.is_none() && freq_r.is_none() {
        return Ok(None);
    }
    let mut best: Option<(u64, u32, u32, u32, u32)> = None;
    for divp in 1u32..=128 {
        for divr in 1u32..=128 {
            let vco_p = freq_p.map(|f| f as u64 * divp as u64);
            let vco_r = freq_r.map(|f| f as u64 * divr as u64);
            let vco = match (vco_p, vco_r) {
                (Some(a), Some(b)) if a == b => a,
                (Some(a), None) => a,
                (None, Some(b)) => b,
                _ => continue,
            };
            if !(192_000_000..=836_000_000).contains(&vco) {
                continue;
            }
            for prediv in 1u32..=64 {
                let num = vco * prediv as u64;
                if num % input != 0 {
                    continue;
                }
                let mul = num / input;
                if (4..=512).contains(&mul) {
                    let cand = (vco, prediv, mul as u32, divp, divr);
                    if best.as_ref().map_or(true, |b| cand.0 < b.0) {
                        best = Some(cand);
                    }
                }
            }
        }
    }
    let (_, prediv, mul, divp, divr) = best.ok_or_else(|| {
        format!(
            "cannot find a valid PLL2 config for peripheral clocks (P={:?}, R={:?})",
            freq_p, freq_r
        )
    })?;
    Ok(Some((prediv, mul, divp, divr)))
}

#[derive(Debug)]
struct F4Plan {
    hse: u32,
    prediv: u32,
    mul: u32,
    divp: u32,
    divq: u32,
    usb48: bool,
    source_hse: bool,
    ahb: u32,
    apb1: u32,
    apb2: u32,
}

/// STM32F4（F411）：HSE 输入，PLL VCO 100–432 MHz，PLLM 2–63，PLLN 4–432，
/// PLLP ∈ {2,4,6,8}，PLLQ 2–15；SYSCLK ≤ 100 MHz，AHB ≤ 100 MHz，
/// APB1 ≤ 50 MHz，APB2 ≤ 100 MHz。
///
/// 注意：F4 没有 HSI48（embassy 未暴露），USB 48 MHz 只能来自 PLLQ——
/// 因此 100 MHz 系统时钟无法同时提供 48 MHz USB（可改用 96 MHz）。
fn plan_f4(
    sysclk: u32,
    usb: Option<u32>,
    hse: Option<u32>,
) -> std::result::Result<F4Plan, String> {
    // HSE 存在时用外部晶振，否则用内部 HSI 16 MHz（无晶振板子可用）。
    let input = hse.unwrap_or(16_000_000) as u64;
    let source_hse = hse.is_some();
    if sysclk > 100_000_000 {
        return Err(format!(
            "system clock {sysclk} Hz exceeds F411 maximum (100 MHz)"
        ));
    }
    if let Some(u) = usb {
        if u != 48_000_000 {
            return Err("only 48 MHz USB clock is supported".into());
        }
    }

    let mut best: Option<(u64, u32, u32, u32, u32)> = None;
    for divp in [2u32, 4, 6, 8] {
        let vco = sysclk as u64 * divp as u64;
        if !(100_000_000..=432_000_000).contains(&vco) {
            continue;
        }
        // PLLQ 若能提供 48 MHz（当 usb 需要时）。
        let divq = match usb {
            Some(u) => {
                if vco % u as u64 == 0 {
                    let q = vco / u as u64;
                    if (2..=15).contains(&q) {
                        Some(q as u32)
                    } else {
                        continue;
                    }
                } else {
                    continue;
                }
            }
            None => Some(4),
        };
        for prediv in 2u32..=63 {
            let num = vco * prediv as u64;
            if num % input != 0 {
                continue;
            }
            let mul = num / input;
            if (4..=432).contains(&mul) {
                let cand = (vco, prediv, mul as u32, divp, divq.unwrap_or(4));
                if best.as_ref().map_or(true, |b| cand.0 < b.0) {
                    best = Some(cand);
                }
            }
        }
    }
    let (_, prediv, mul, divp, divq) = best.ok_or_else(|| {
        format!(
            "cannot find a valid PLL config for {sysclk} Hz{} on F4 \
             (no HSI48; 100 MHz system cannot provide 48 MHz USB, try 96 MHz)",
            usb.map(|_| " + 48 MHz USB").unwrap_or("")
        )
    })?;
    let ahb = pick_div(sysclk, 100_000_000, &[1, 2, 4, 8, 16, 64, 128, 256, 512])
        .ok_or("cannot satisfy AHB frequency limit")?;
    let apb1 = pick_div(sysclk / ahb, 50_000_000, &[1, 2, 4, 8, 16])
        .ok_or("cannot satisfy APB1 frequency limit")?;
    let apb2 = pick_div(sysclk / ahb, 100_000_000, &[1, 2, 4, 8, 16])
        .ok_or("cannot satisfy APB2 frequency limit")?;
    Ok(F4Plan {
        hse: input as u32,
        prediv,
        mul,
        divp,
        divq,
        usb48: usb.is_some(),
        source_hse,
        ahb,
        apb1,
        apb2,
    })
}

/// 最小分频使 freq/div ≤ max。
fn pick_div(freq: u32, max: u32, divs: &[u32]) -> Option<u32> {
    divs.iter()
        .copied()
        .find(|&d| freq / d <= max)
}

fn synth_h7_node(plan: &H7Plan) -> DslNode {
    let mut props = vec![
        prop(
            "source",
            PropValue::Str(LitStr::new(
                if plan.source_hse { "hse" } else { "hsi" },
                Span::call_site(),
            )),
        ),
        prop(
            "pll1",
            PropValue::Array(vec![plan.prediv, plan.mul, plan.divp]),
        ),
        prop(
            "sys",
            PropValue::Str(LitStr::new("pll1_p", Span::call_site())),
        ),
        prop("ahb", PropValue::U32(LitInt::new(&plan.ahb.to_string(), Span::call_site()))),
        prop("apb1", PropValue::U32(LitInt::new(&plan.apb.to_string(), Span::call_site()))),
        prop("apb2", PropValue::U32(LitInt::new(&plan.apb.to_string(), Span::call_site()))),
        prop("apb3", PropValue::U32(LitInt::new(&plan.apb.to_string(), Span::call_site()))),
        prop("apb4", PropValue::U32(LitInt::new(&plan.apb.to_string(), Span::call_site()))),
        prop(
            "voltage",
            PropValue::Str(LitStr::new(
                if plan.scale1 { "scale1" } else { "scale0" },
                Span::call_site(),
            )),
        ),
    ];
    if let Some((prediv, mul, divp, divr)) = plan.pll2 {
        props.push(prop(
            "pll2",
            PropValue::Array(vec![prediv, mul, divp, divr]),
        ));
    }
    if let Some(sel) = plan.spi123 {
        props.push(prop(
            "spi123sel",
            PropValue::Str(LitStr::new(sel, Span::call_site())),
        ));
    }
    if let Some(sel) = plan.adc_sel {
        props.push(prop(
            "adcsel",
            PropValue::Str(LitStr::new(sel, Span::call_site())),
        ));
    }
    if let Some(sel) = plan.sdmmc_sel {
        props.push(prop(
            "sdmmcsel",
            PropValue::Str(LitStr::new(sel, Span::call_site())),
        ));
    }
    if plan.usb48 {
        props.push(prop("hsi48", PropValue::Bool(true)));
        props.push(prop(
            "usb",
            PropValue::Str(LitStr::new("hsi48", Span::call_site())),
        ));
    }
    DslNode {
        id: Ident::new("clock", Span::call_site()),
        kind: NodeKindAst::Device,
        driver: None,
        props,
        deps: Vec::new(),
    }
}

fn synth_f4_node(plan: &F4Plan) -> DslNode {
    let mut props = vec![
        prop(
            "source",
            PropValue::Str(LitStr::new(
                if plan.source_hse { "hse" } else { "hsi" },
                Span::call_site(),
            )),
        ),
        prop(
            "pll",
            PropValue::Array(vec![plan.prediv, plan.mul, plan.divp, plan.divq]),
        ),
        prop(
            "sys",
            PropValue::Str(LitStr::new("pll1_p", Span::call_site())),
        ),
        prop("ahb", PropValue::U32(LitInt::new(&plan.ahb.to_string(), Span::call_site()))),
        prop("apb1", PropValue::U32(LitInt::new(&plan.apb1.to_string(), Span::call_site()))),
        prop("apb2", PropValue::U32(LitInt::new(&plan.apb2.to_string(), Span::call_site()))),
    ];
    if plan.source_hse {
        props.push(prop("hse", PropValue::U32(LitInt::new(&plan.hse.to_string(), Span::call_site()))));
    }
    if plan.usb48 {
        props.push(prop(
            "clk48",
            PropValue::Str(LitStr::new("pll1_q", Span::call_site())),
        ));
    }
    DslNode {
        id: Ident::new("clock", Span::call_site()),
        kind: NodeKindAst::Device,
        driver: None,
        props,
        deps: Vec::new(),
    }
}

#[derive(Debug)]
struct L4Plan {
    hse: Option<u32>,
    prediv: u32,
    mul: u32,
    divr: u32,
    usb48: bool,
}

/// STM32L4（L476）：HSI16 或 HSE 输入，PLL VCO 96–344 MHz，
/// PLLM 1–8，PLLN 8–86，系统时钟走 PLLR ∈ {2,4,6,8}；
/// SYSCLK ≤ 80 MHz；USB 48 MHz 用专属 HSI48（无需 PLLQ）。
fn plan_l4(
    sysclk: u32,
    usb: Option<u32>,
    hse: Option<u32>,
) -> std::result::Result<L4Plan, String> {
    if sysclk > 80_000_000 {
        return Err(format!(
            "system clock {sysclk} Hz exceeds L476 maximum (80 MHz)"
        ));
    }
    if let Some(u) = usb {
        if u != 48_000_000 {
            return Err("only 48 MHz USB clock is supported".into());
        }
    }
    let input = hse.unwrap_or(16_000_000) as u64;

    let mut best: Option<(u64, u32, u32, u32)> = None;
    for divr in [2u32, 4, 6, 8] {
        let vco = sysclk as u64 * divr as u64;
        if !(96_000_000..=344_000_000).contains(&vco) {
            continue;
        }
        for prediv in 1u32..=8 {
            let num = vco * prediv as u64;
            if num % input != 0 {
                continue;
            }
            let mul = num / input;
            if (8..=86).contains(&mul) {
                let cand = (vco, prediv, mul as u32, divr);
                if best.as_ref().map_or(true, |b| cand.0 < b.0) {
                    best = Some(cand);
                }
            }
        }
    }
    let (_, prediv, mul, divr) = best
        .ok_or_else(|| format!("cannot find a valid PLL config for {sysclk} Hz"))?;
    Ok(L4Plan {
        hse,
        prediv,
        mul,
        divr,
        usb48: usb.is_some(),
    })
}

fn synth_l4_node(plan: &L4Plan) -> DslNode {
    let mut props = vec![
        prop(
            "source",
            PropValue::Str(LitStr::new(
                if plan.hse.is_some() { "hse" } else { "hsi" },
                Span::call_site(),
            )),
        ),
        prop(
            "pll",
            PropValue::Array(vec![plan.prediv, plan.mul, plan.divr]),
        ),
        prop(
            "sys",
            PropValue::Str(LitStr::new("pll1_r", Span::call_site())),
        ),
        prop("ahb", PropValue::U32(LitInt::new("1", Span::call_site()))),
        prop("apb1", PropValue::U32(LitInt::new("1", Span::call_site()))),
        prop("apb2", PropValue::U32(LitInt::new("1", Span::call_site()))),
    ];
    if let Some(h) = plan.hse {
        props.push(prop("hse", PropValue::U32(LitInt::new(&h.to_string(), Span::call_site()))));
    }
    if plan.usb48 {
        // L4 的 HSI48 支持由 embassy 的 `crs` cfg 控制（L476 不可用），
        // 用 MSI 48 MHz 作为 USB 时钟源。
        props.push(prop(
            "msi",
            PropValue::Str(LitStr::new("48m", Span::call_site())),
        ));
    }
    DslNode {
        id: Ident::new("clock", Span::call_site()),
        kind: NodeKindAst::Device,
        driver: None,
        props,
        deps: Vec::new(),
    }
}

#[derive(Debug)]
struct G4Plan {
    hse: Option<u32>,
    prediv: u32,
    mul: u32,
    divr: u32,
    usb48: bool,
    adc: bool,
    boost: bool,
}

/// STM32G4（G474）：HSI16 或 HSE 输入，PLL VCO 96–344 MHz，
/// PLLM 1–8，PLLN 8–127，系统时钟走 PLLR ∈ {2,4,6,8}；
/// SYSCLK ≤ 170 MHz（>150 MHz 需要 boost 模式）；
/// USB 48 MHz 用专属 HSI48（默认 CLK48SEL=HSI48，无需 PLLQ）；
/// ADC 内核时钟走 SYS（G4 的 ADC12SEL 只有 DISABLE/PLL1_P/SYS，
/// 驱动会自动把 SYS 分频到 ≤ 60 MHz）。
fn plan_g4(
    sysclk: u32,
    usb: Option<u32>,
    hse: Option<u32>,
    adc: Option<u32>,
) -> std::result::Result<G4Plan, String> {
    if sysclk > 170_000_000 {
        return Err(format!(
            "system clock {sysclk} Hz exceeds G474 maximum (170 MHz)"
        ));
    }
    if let Some(u) = usb {
        if u != 48_000_000 {
            return Err("only 48 MHz USB clock is supported".into());
        }
    }
    if let Some(h) = hse {
        if !(4_000_000..=48_000_000).contains(&h) {
            return Err(format!("HSE frequency {h} Hz out of range (4–48 MHz)"));
        }
    }
    let input = hse.unwrap_or(16_000_000) as u64;

    let mut best: Option<(u64, u32, u32, u32)> = None;
    for divr in [2u32, 4, 6, 8] {
        let vco = sysclk as u64 * divr as u64;
        if !(96_000_000..=344_000_000).contains(&vco) {
            continue;
        }
        for prediv in 1u32..=8 {
            let in_freq = input / prediv as u64;
            if !(2_660_000..=16_000_000).contains(&in_freq) {
                continue;
            }
            let num = vco * prediv as u64;
            if num % input != 0 {
                continue;
            }
            let mul = num / input;
            if (8..=127).contains(&mul) {
                let cand = (vco, prediv, mul as u32, divr);
                if best.as_ref().map_or(true, |b| cand.0 < b.0) {
                    best = Some(cand);
                }
            }
        }
    }
    let (_, prediv, mul, divr) = best
        .ok_or_else(|| format!("cannot find a valid PLL config for {sysclk} Hz"))?;
    Ok(G4Plan {
        hse,
        prediv,
        mul,
        divr,
        usb48: usb.is_some(),
        adc: adc.is_some(),
        boost: sysclk > 150_000_000,
    })
}

fn synth_g4_node(plan: &G4Plan) -> DslNode {
    let mut props = vec![
        prop(
            "source",
            PropValue::Str(LitStr::new(
                if plan.hse.is_some() { "hse" } else { "hsi" },
                Span::call_site(),
            )),
        ),
        prop(
            "pll",
            PropValue::Array(vec![plan.prediv, plan.mul, plan.divr]),
        ),
        prop(
            "sys",
            PropValue::Str(LitStr::new("pll1_r", Span::call_site())),
        ),
        prop("ahb", PropValue::U32(LitInt::new("1", Span::call_site()))),
        prop("apb1", PropValue::U32(LitInt::new("1", Span::call_site()))),
        prop("apb2", PropValue::U32(LitInt::new("1", Span::call_site()))),
    ];
    if let Some(h) = plan.hse {
        props.push(prop("hse", PropValue::U32(LitInt::new(&h.to_string(), Span::call_site()))));
    }
    if plan.usb48 {
        // G4 的 HSI48 由 embassy 原生支持（CRS 同步 USB SOF），
        // CLK48SEL 默认就是 HSI48，无需改 mux。
        props.push(prop("hsi48", PropValue::Bool(true)));
        props.push(prop("hsi48_sync", PropValue::Bool(true)));
    }
    if plan.adc {
        // G4 ADC12SEL：DISABLE / PLL1_P / SYS。选 SYS，驱动自动分频。
        props.push(prop(
            "adc12",
            PropValue::Str(LitStr::new("sys", Span::call_site())),
        ));
    }
    if plan.boost {
        props.push(prop("boost", PropValue::Bool(true)));
    }
    DslNode {
        id: Ident::new("clock", Span::call_site()),
        kind: NodeKindAst::Device,
        driver: None,
        props,
        deps: Vec::new(),
    }
}

#[derive(Debug)]
struct F1Plan {
    hse: Option<u32>,
    prediv: u32,
    mul: u32,
    apb1: u32,
    adc_pre: Option<u32>,
    usb48: bool,
}

/// STM32F1（F103C8）：HSI 8 MHz 或 HSE（4–16 MHz）输入，
/// PLL 结构最简单：`PLL = 源 / prediv × mul`（PLLMUL 2–16），
/// SYSCLK ≤ 72 MHz（HSE）或 ≤ 64 MHz（HSI ÷2 ×16）；
/// PCLK1 ≤ 36 MHz、PCLK2 ≤ 72 MHz；ADC 时钟 = PCLK2 / adc_pre ≤ 14 MHz。
///
/// USB 特殊性：F1 没有 HSI48，USB 48 MHz 由 PLL 直接派生——
/// PLL = 72 MHz 时内部 ÷1.5，PLL = 48 MHz 时 ÷1。
/// 因此请求 USB 时 SYSCLK 必须是 72 MHz（需要 HSE）或 48 MHz。
fn plan_f1(
    sysclk: u32,
    usb: Option<u32>,
    hse: Option<u32>,
    adc: Option<u32>,
) -> std::result::Result<F1Plan, String> {
    if let Some(h) = hse {
        if !(4_000_000..=16_000_000).contains(&h) {
            return Err(format!("HSE frequency {h} Hz out of range (4–16 MHz)"));
        }
    }
    let max_sys = if hse.is_some() { 72_000_000 } else { 64_000_000 };
    if sysclk > max_sys {
        return Err(format!(
            "system clock {sysclk} Hz exceeds F1 maximum ({max_sys} Hz, \
             HSI 上限 64 MHz，72 MHz 需要 HSE)"
        ));
    }
    if let Some(u) = usb {
        if u != 48_000_000 {
            return Err("only 48 MHz USB clock is supported".into());
        }
        // F1 的 USB 时钟只能来自 PLL（72M÷1.5 或 48M÷1）。
        if sysclk != 72_000_000 && sysclk != 48_000_000 {
            return Err(
                "F1 USB requires SYSCLK = 72 MHz (with HSE) or 48 MHz (HSI)".into(),
            );
        }
    }
    let input = hse.unwrap_or(8_000_000) as u64;

    let mut best: Option<(u32, u32)> = None;
    for prediv in 1u32..=2 {
        if hse.is_none() && prediv != 2 {
            // F1 的 HSI 作 PLL 源时硬件强制 ÷2（embassy init 会 panic）。
            continue;
        }
        let in_freq = input / prediv as u64;
        if !(1_000_000..=25_000_000).contains(&in_freq) {
            continue;
        }
        if sysclk as u64 % in_freq != 0 {
            continue;
        }
        let mul = sysclk as u64 / in_freq;
        if (2..=16).contains(&mul) {
            best = Some((prediv, mul as u32));
        }
    }
    let (prediv, mul) = best
        .ok_or_else(|| format!("cannot find a valid PLL config for {sysclk} Hz"))?;

    // PCLK1 ≤ 36 MHz：72/64 → ÷2，48/36 → ÷2/÷1。
    let apb1 = if sysclk > 36_000_000 { 2 } else { 1 };

    // ADC 时钟 = PCLK2（=SYSCLK）÷ adc_pre ≤ 14 MHz，取最小分频。
    let adc_pre = adc
        .map(|_| {
            [2u32, 4, 6, 8]
                .into_iter()
                .find(|pre| sysclk / pre <= 14_000_000)
                .ok_or_else(|| format!("cannot find ADC prescaler for {sysclk} Hz"))
        })
        .transpose()?;

    Ok(F1Plan {
        hse,
        prediv,
        mul,
        apb1,
        adc_pre,
        usb48: usb.is_some(),
    })
}

fn synth_f1_node(plan: &F1Plan) -> DslNode {
    let mut props = vec![
        prop(
            "source",
            PropValue::Str(LitStr::new(
                if plan.hse.is_some() { "hse" } else { "hsi" },
                Span::call_site(),
            )),
        ),
        prop(
            "pll",
            PropValue::Array(vec![plan.prediv, plan.mul]),
        ),
        prop(
            "sys",
            PropValue::Str(LitStr::new("pll1_p", Span::call_site())),
        ),
        prop("ahb", PropValue::U32(LitInt::new("1", Span::call_site()))),
        prop(
            "apb1",
            PropValue::U32(LitInt::new(&plan.apb1.to_string(), Span::call_site())),
        ),
        prop("apb2", PropValue::U32(LitInt::new("1", Span::call_site()))),
    ];
    if let Some(h) = plan.hse {
        props.push(prop("hse", PropValue::U32(LitInt::new(&h.to_string(), Span::call_site()))));
    }
    if let Some(pre) = plan.adc_pre {
        props.push(prop(
            "adc_pre",
            PropValue::U32(LitInt::new(&pre.to_string(), Span::call_site())),
        ));
    }
    let _ = plan.usb48; // F1 的 USB 时钟由 PLL 自动派生，无需额外属性。
    DslNode {
        id: Ident::new("clock", Span::call_site()),
        kind: NodeKindAst::Device,
        driver: None,
        props,
        deps: Vec::new(),
    }
}

fn prop(key: &str, value: PropValue) -> DslProp {
    DslProp {
        key: Ident::new(key, Span::call_site()),
        value,
    }
}

fn h7_clock_config(node: &DslNode) -> Result<Vec<TokenStream2>> {
    let mut stmts = Vec::new();

    if let Some(hsi) = node.prop_str_opt("hsi")? {
        let v = match hsi.as_str() {
            "div1" => quote!(DIV1),
            "div2" => quote!(DIV2),
            "div4" => quote!(DIV4),
            "div8" => quote!(DIV8),
            other => {
                return Err(syn::Error::new(
                    node.id.span(),
                    format!("clock: unsupported `hsi` value `{other}` (div1/div2/div4/div8)"),
                ))
            }
        };
        stmts.push(quote! {
            config.rcc.hsi = Some(::embassy_stm32::rcc::HSIPrescaler::#v);
        });
    }
    if node.prop_bool("csi") {
        stmts.push(quote! { config.rcc.csi = true; });
    }
    if node.prop_bool("hsi48") {
        stmts.push(quote! {
            config.rcc.hsi48 = Some(::embassy_stm32::rcc::Hsi48Config { sync_from_usb: true });
        });
    }
    if let Some(pll) = node.prop_array("pll1") {
        if pll.len() < 3 {
            return Err(syn::Error::new(
                node.id.span(),
                "clock: `pll1` needs at least <prediv mul divp>",
            ));
        }
        let source = node.prop_str_opt("source")?.ok_or_else(|| {
            syn::Error::new(node.id.span(), "clock: `source` is required with `pll1`")
        })?;
        let src = match source.as_str() {
            "hsi" => quote!(HSI),
            "csi" => quote!(CSI),
            "hse" => quote!(HSE),
            other => {
                return Err(syn::Error::new(
                    node.id.span(),
                    format!("clock: unsupported `source` `{other}` (hsi/csi/hse)"),
                ))
            }
        };
        let pre = format_ident!("DIV{}", pll[0]);
        let mul = format_ident!("MUL{}", pll[1]);
        let divp = format_ident!("DIV{}", pll[2]);
        let divq = match pll.get(3) {
            Some(n) => {
                let v = format_ident!("DIV{}", n);
                quote!(Some(::embassy_stm32::rcc::PllDiv::#v))
            }
            None => quote!(None),
        };
        let divr = match pll.get(4) {
            Some(n) => {
                let v = format_ident!("DIV{}", n);
                quote!(Some(::embassy_stm32::rcc::PllDiv::#v))
            }
            None => quote!(None),
        };
        stmts.push(quote! {
            config.rcc.pll1 = Some(::embassy_stm32::rcc::Pll {
                source: ::embassy_stm32::rcc::PllSource::#src,
                prediv: ::embassy_stm32::rcc::PllPreDiv::#pre,
                mul: ::embassy_stm32::rcc::PllMul::#mul,
                divp: Some(::embassy_stm32::rcc::PllDiv::#divp),
                divq: #divq,
                divr: #divr,
            });
        });
    }
    if let Some(sys) = node.prop_str_opt("sys")? {
        let v = match sys.as_str() {
            "hsi" => quote!(HSI),
            "csi" => quote!(CSI),
            "hse" => quote!(HSE),
            "pll1_p" => quote!(PLL1_P),
            "pll2_p" => quote!(PLL2_P),
            "pll3_p" => quote!(PLL3_P),
            other => {
                return Err(syn::Error::new(
                    node.id.span(),
                    format!("clock: unsupported `sys` `{other}`"),
                ))
            }
        };
        stmts.push(quote! { config.rcc.sys = ::embassy_stm32::rcc::Sysclk::#v; });
    }
    if let Some(n) = node.prop_u32_any(&["ahb"]) {
        let v = format_ident!("DIV{}", n);
        stmts.push(quote! { config.rcc.ahb_pre = ::embassy_stm32::rcc::AHBPrescaler::#v; });
    }
    for key in ["apb1", "apb2", "apb3", "apb4"] {
        if let Some(n) = node.prop_u32_any(&[key]) {
            let field = format_ident!("{}_pre", key);
            let v = format_ident!("DIV{}", n);
            stmts.push(quote! { config.rcc.#field = ::embassy_stm32::rcc::APBPrescaler::#v; });
        }
    }
    if let Some(usb) = node.prop_str_opt("usb")? {
        let v = match usb.as_str() {
            "disable" => quote!(DISABLE),
            "pll1_q" => quote!(PLL1_Q),
            "pll3_q" => quote!(PLL3_Q),
            "hsi48" => quote!(HSI48),
            other => {
                return Err(syn::Error::new(
                    node.id.span(),
                    format!("clock: unsupported `usb` `{other}`"),
                ))
            }
        };
        stmts.push(quote! { config.rcc.mux.usbsel = ::embassy_stm32::rcc::mux::Usbsel::#v; });
    }
    if let Some(pll2) = node.prop_array("pll2") {
        if pll2.len() < 4 {
            return Err(syn::Error::new(
                node.id.span(),
                "clock: `pll2` needs <prediv mul divp divr>",
            ));
        }
        let source = node.prop_str_opt("source")?.ok_or_else(|| {
            syn::Error::new(node.id.span(), "clock: `source` is required with `pll2`")
        })?;
        let src = match source.as_str() {
            "hsi" => quote!(HSI),
            "csi" => quote!(CSI),
            "hse" => quote!(HSE),
            other => {
                return Err(syn::Error::new(
                    node.id.span(),
                    format!("clock: unsupported `source` `{other}` (hsi/csi/hse)"),
                ))
            }
        };
        let pre = format_ident!("DIV{}", pll2[0]);
        let mul = format_ident!("MUL{}", pll2[1]);
        let divp = format_ident!("DIV{}", pll2[2]);
        let divr = format_ident!("DIV{}", pll2[3]);
        stmts.push(quote! {
            config.rcc.pll2 = Some(::embassy_stm32::rcc::Pll {
                source: ::embassy_stm32::rcc::PllSource::#src,
                prediv: ::embassy_stm32::rcc::PllPreDiv::#pre,
                mul: ::embassy_stm32::rcc::PllMul::#mul,
                divp: Some(::embassy_stm32::rcc::PllDiv::#divp),
                divq: None,
                divr: Some(::embassy_stm32::rcc::PllDiv::#divr),
            });
        });
    }
    if let Some(sel) = node.prop_str_opt("spi123sel")? {
        let v = match sel.as_str() {
            "pll1_q" => quote!(PLL1_Q),
            "pll2_p" => quote!(PLL2_P),
            "pll3_p" => quote!(PLL3_P),
            "per" => quote!(PER),
            other => {
                return Err(syn::Error::new(
                    node.id.span(),
                    format!("clock: unsupported `spi123sel` `{other}`"),
                ))
            }
        };
        stmts.push(quote! { config.rcc.mux.spi123sel = ::embassy_stm32::rcc::mux::Saisel::#v; });
    }
    if let Some(sel) = node.prop_str_opt("adcsel")? {
        let v = match sel.as_str() {
            "pll2_p" => quote!(PLL2_P),
            "pll3_r" => quote!(PLL3_R),
            "per" => quote!(PER),
            other => {
                return Err(syn::Error::new(
                    node.id.span(),
                    format!("clock: unsupported `adcsel` `{other}`"),
                ))
            }
        };
        stmts.push(quote! { config.rcc.mux.adcsel = ::embassy_stm32::rcc::mux::Adcsel::#v; });
    }
    if let Some(sel) = node.prop_str_opt("sdmmcsel")? {
        let v = match sel.as_str() {
            "pll1_q" => quote!(PLL1_Q),
            "pll2_r" => quote!(PLL2_R),
            other => {
                return Err(syn::Error::new(
                    node.id.span(),
                    format!("clock: unsupported `sdmmcsel` `{other}`"),
                ))
            }
        };
        stmts.push(quote! { config.rcc.mux.sdmmcsel = ::embassy_stm32::rcc::mux::Sdmmcsel::#v; });
    }
    if let Some(voltage) = node.prop_str_opt("voltage")? {
        let v = match voltage.as_str() {
            "scale0" => quote!(Scale0),
            "scale1" => quote!(Scale1),
            "scale2" => quote!(Scale2),
            "scale3" => quote!(Scale3),
            other => {
                return Err(syn::Error::new(
                    node.id.span(),
                    format!("clock: unsupported `voltage` `{other}`"),
                ))
            }
        };
        stmts.push(quote! { config.rcc.voltage_scale = ::embassy_stm32::rcc::VoltageScale::#v; });
    }
    Ok(stmts)
}

fn f4_clock_config(node: &DslNode) -> Result<Vec<TokenStream2>> {
    let mut stmts = Vec::new();

    if let Some(hse) = node.prop_u32_any(&["hse"]) {
        let mode = match node.prop_str_opt("hse-mode")?.as_deref() {
            None | Some("oscillator") => quote!(Oscillator),
            Some("bypass") => quote!(Bypass),
            Some(other) => {
                return Err(syn::Error::new(
                    node.id.span(),
                    format!("clock: unsupported `hse-mode` `{other}` (oscillator/bypass)"),
                ))
            }
        };
        stmts.push(quote! {
            config.rcc.hse = Some(::embassy_stm32::rcc::Hse {
                freq: ::embassy_stm32::time::Hertz(#hse),
                mode: ::embassy_stm32::rcc::HseMode::#mode,
            });
        });
    }
    if let Some(pll) = node.prop_array("pll") {
        if pll.len() < 4 {
            return Err(syn::Error::new(
                node.id.span(),
                "clock: `pll` needs <prediv mul divp divq>",
            ));
        }
        let source = node.prop_str_opt("source")?.ok_or_else(|| {
            syn::Error::new(node.id.span(), "clock: `source` is required with `pll`")
        })?;
        let src = match source.as_str() {
            "hse" => quote!(HSE),
            "hsi" => quote!(HSI),
            other => {
                return Err(syn::Error::new(
                    node.id.span(),
                    format!("clock: unsupported `source` `{other}` (hse/hsi)"),
                ))
            }
        };
        let pre = format_ident!("DIV{}", pll[0]);
        let mul = format_ident!("MUL{}", pll[1]);
        let divp = format_ident!("DIV{}", pll[2]);
        let divq = format_ident!("DIV{}", pll[3]);
        stmts.push(quote! {
            config.rcc.pll_src = ::embassy_stm32::rcc::PllSource::#src;
            config.rcc.pll = Some(::embassy_stm32::rcc::Pll {
                prediv: ::embassy_stm32::rcc::PllPreDiv::#pre,
                mul: ::embassy_stm32::rcc::PllMul::#mul,
                divp: Some(::embassy_stm32::rcc::PllPDiv::#divp),
                divq: Some(::embassy_stm32::rcc::PllQDiv::#divq),
                divr: None,
            });
        });
    }
    if let Some(sys) = node.prop_str_opt("sys")? {
        let v = match sys.as_str() {
            "hsi" => quote!(HSI),
            "hse" => quote!(HSE),
            "pll1_p" => quote!(PLL1_P),
            other => {
                return Err(syn::Error::new(
                    node.id.span(),
                    format!("clock: unsupported `sys` `{other}`"),
                ))
            }
        };
        stmts.push(quote! { config.rcc.sys = ::embassy_stm32::rcc::Sysclk::#v; });
    }
    if let Some(n) = node.prop_u32_any(&["ahb"]) {
        let v = format_ident!("DIV{}", n);
        stmts.push(quote! { config.rcc.ahb_pre = ::embassy_stm32::rcc::AHBPrescaler::#v; });
    }
    for key in ["apb1", "apb2"] {
        if let Some(n) = node.prop_u32_any(&[key]) {
            let field = format_ident!("{}_pre", key);
            let v = format_ident!("DIV{}", n);
            stmts.push(quote! { config.rcc.#field = ::embassy_stm32::rcc::APBPrescaler::#v; });
        }
    }
    if let Some(clk48) = node.prop_str_opt("clk48")? {
        let v = match clk48.as_str() {
            "pll1_q" => quote!(PLL1_Q),
            "pll_sai_q" => quote!(PLLSAI1_Q),
            other => {
                return Err(syn::Error::new(
                    node.id.span(),
                    format!("clock: unsupported `clk48` `{other}` (pll1_q/pll_sai_q)"),
                ))
            }
        };
        stmts.push(quote! { config.rcc.mux.clk48sel = ::embassy_stm32::rcc::mux::Clk48sel::#v; });
    }
    Ok(stmts)
}

fn l4_clock_config(node: &DslNode) -> Result<Vec<TokenStream2>> {
    let mut stmts = Vec::new();

    if let Some(source) = node.prop_str_opt("source")? {
        if source == "hsi" {
            // L4 的 HSI 是布尔开关（PLL 源在 `pll.source` 里指定）。
            stmts.push(quote! { config.rcc.hsi = true; });
        }
    }
    if let Some(hse) = node.prop_u32_any(&["hse"]) {
        let mode = match node.prop_str_opt("hse-mode")?.as_deref() {
            None | Some("oscillator") => quote!(Oscillator),
            Some("bypass") => quote!(Bypass),
            Some(other) => {
                return Err(syn::Error::new(
                    node.id.span(),
                    format!("clock: unsupported `hse-mode` `{other}`"),
                ))
            }
        };
        stmts.push(quote! {
            config.rcc.hse = Some(::embassy_stm32::rcc::Hse {
                freq: ::embassy_stm32::time::Hertz(#hse),
                mode: ::embassy_stm32::rcc::HseMode::#mode,
            });
        });
    }
    if let Some(pll) = node.prop_array("pll") {
        // L4：<prediv mul divr>（系统时钟走 PLLR）。
        if pll.len() < 3 {
            return Err(syn::Error::new(
                node.id.span(),
                "clock: `pll` needs <prediv mul divr> on L4",
            ));
        }
        let pre = format_ident!("DIV{}", pll[0]);
        let mul = format_ident!("MUL{}", pll[1]);
        let divr = format_ident!("DIV{}", pll[2]);
        let src = match node.prop_str_opt("source")?.as_deref() {
            None | Some("hsi") => quote!(HSI),
            Some("hse") => quote!(HSE),
            Some("msi") => quote!(MSI),
            Some(other) => {
                return Err(syn::Error::new(
                    node.id.span(),
                    format!("clock: unsupported `source` `{other}`"),
                ))
            }
        };
        stmts.push(quote! {
            config.rcc.pll = Some(::embassy_stm32::rcc::Pll {
                source: ::embassy_stm32::rcc::PllSource::#src,
                prediv: ::embassy_stm32::rcc::PllPreDiv::#pre,
                mul: ::embassy_stm32::rcc::PllMul::#mul,
                divp: None,
                divq: None,
                divr: Some(::embassy_stm32::rcc::PllRDiv::#divr),
            });
        });
    }
    if let Some(sys) = node.prop_str_opt("sys")? {
        let v = match sys.as_str() {
            "msi" => quote!(MSI),
            "hsi" => quote!(HSI),
            "hse" => quote!(HSE),
            "pll1_r" => quote!(PLL1_R),
            other => {
                return Err(syn::Error::new(
                    node.id.span(),
                    format!("clock: unsupported `sys` `{other}`"),
                ))
            }
        };
        stmts.push(quote! { config.rcc.sys = ::embassy_stm32::rcc::Sysclk::#v; });
    }
    if let Some(n) = node.prop_u32_any(&["ahb"]) {
        let v = format_ident!("DIV{}", n);
        stmts.push(quote! { config.rcc.ahb_pre = ::embassy_stm32::rcc::AHBPrescaler::#v; });
    }
    for key in ["apb1", "apb2"] {
        if let Some(n) = node.prop_u32_any(&[key]) {
            let field = format_ident!("{}_pre", key);
            let v = format_ident!("DIV{}", n);
            stmts.push(quote! { config.rcc.#field = ::embassy_stm32::rcc::APBPrescaler::#v; });
        }
    }
    if node.prop_bool("hsi48") {
        stmts.push(quote! {
            config.rcc.hsi48 = Some(::embassy_stm32::rcc::Hsi48Config::default());
        });
    }
    if let Some(msi) = node.prop_str_opt("msi")? {
        let v = match msi.as_str() {
            "48m" => quote!(RANGE48M),
            "24m" => quote!(RANGE24M),
            "16m" => quote!(RANGE16M),
            "4m" => quote!(RANGE4M),
            "2m" => quote!(RANGE2M),
            "1m" => quote!(RANGE1M),
            other => {
                return Err(syn::Error::new(
                    node.id.span(),
                    format!("clock: unsupported `msi` range `{other}`"),
                ))
            }
        };
        stmts.push(quote! { config.rcc.msi = Some(::embassy_stm32::rcc::MSIRange::#v); });
    }
    if let Some(voltage) = node.prop_str_opt("voltage")? {
        let v = match voltage.as_str() {
            "range1" => quote!(RANGE1),
            "range2" => quote!(RANGE2),
            other => {
                return Err(syn::Error::new(
                    node.id.span(),
                    format!("clock: unsupported `voltage` `{other}`"),
                ))
            }
        };
        stmts.push(quote! { config.rcc.voltage_scale = ::embassy_stm32::rcc::VoltageScale::#v; });
    }
    Ok(stmts)
}

fn g4_clock_config(node: &DslNode) -> Result<Vec<TokenStream2>> {
    let mut stmts = Vec::new();

    if let Some(source) = node.prop_str_opt("source")? {
        if source == "hsi" {
            // G4 的 HSI 是布尔开关（PLL 源在 `pll.source` 里指定）。
            stmts.push(quote! { config.rcc.hsi = true; });
        }
    }
    if let Some(hse) = node.prop_u32_any(&["hse"]) {
        let mode = match node.prop_str_opt("hse-mode")?.as_deref() {
            None | Some("oscillator") => quote!(Oscillator),
            Some("bypass") => quote!(Bypass),
            Some(other) => {
                return Err(syn::Error::new(
                    node.id.span(),
                    format!("clock: unsupported `hse-mode` `{other}`"),
                ))
            }
        };
        stmts.push(quote! {
            config.rcc.hse = Some(::embassy_stm32::rcc::Hse {
                freq: ::embassy_stm32::time::Hertz(#hse),
                mode: ::embassy_stm32::rcc::HseMode::#mode,
            });
        });
    }
    if let Some(pll) = node.prop_array("pll") {
        // G4：<prediv mul divr>（系统时钟走 PLLR）。
        if pll.len() < 3 {
            return Err(syn::Error::new(
                node.id.span(),
                "clock: `pll` needs <prediv mul divr> on G4",
            ));
        }
        let pre = format_ident!("DIV{}", pll[0]);
        let mul = format_ident!("MUL{}", pll[1]);
        let divr = format_ident!("DIV{}", pll[2]);
        let src = match node.prop_str_opt("source")?.as_deref() {
            None | Some("hsi") => quote!(HSI),
            Some("hse") => quote!(HSE),
            Some("msi") => quote!(MSI),
            Some(other) => {
                return Err(syn::Error::new(
                    node.id.span(),
                    format!("clock: unsupported `source` `{other}`"),
                ))
            }
        };
        stmts.push(quote! {
            config.rcc.pll = Some(::embassy_stm32::rcc::Pll {
                source: ::embassy_stm32::rcc::PllSource::#src,
                prediv: ::embassy_stm32::rcc::PllPreDiv::#pre,
                mul: ::embassy_stm32::rcc::PllMul::#mul,
                divp: None,
                divq: None,
                divr: Some(::embassy_stm32::rcc::PllRDiv::#divr),
            });
        });
    }
    if let Some(sys) = node.prop_str_opt("sys")? {
        let v = match sys.as_str() {
            "hsi" => quote!(HSI),
            "hse" => quote!(HSE),
            "pll1_r" => quote!(PLL1_R),
            other => {
                return Err(syn::Error::new(
                    node.id.span(),
                    format!("clock: unsupported `sys` `{other}`"),
                ))
            }
        };
        stmts.push(quote! { config.rcc.sys = ::embassy_stm32::rcc::Sysclk::#v; });
    }
    if let Some(n) = node.prop_u32_any(&["ahb"]) {
        let v = format_ident!("DIV{}", n);
        stmts.push(quote! { config.rcc.ahb_pre = ::embassy_stm32::rcc::AHBPrescaler::#v; });
    }
    for key in ["apb1", "apb2"] {
        if let Some(n) = node.prop_u32_any(&[key]) {
            let field = format_ident!("{}_pre", key);
            let v = format_ident!("DIV{}", n);
            stmts.push(quote! { config.rcc.#field = ::embassy_stm32::rcc::APBPrescaler::#v; });
        }
    }
    if node.prop_bool("hsi48") {
        let sync = node.prop_bool("hsi48_sync");
        stmts.push(quote! {
            config.rcc.hsi48 = Some(::embassy_stm32::rcc::Hsi48Config {
                sync_from_usb: #sync,
            });
        });
    }
    if node.prop_bool("boost") {
        stmts.push(quote! { config.rcc.boost = true; });
    }
    // G4 内核时钟 mux（默认零值：CLK48SEL=HSI48、ADC12SEL=DISABLE、
    // FDCANSEL=HSE，后两个在用到对应外设时必须显式设置）。
    if let Some(sel) = node.prop_str_opt("clk48")? {
        let v = match sel.as_str() {
            "hsi48" => quote!(HSI48),
            "pll1_q" => quote!(PLL1_Q),
            other => {
                return Err(syn::Error::new(
                    node.id.span(),
                    format!("clock: unsupported `clk48` `{other}` (hsi48/pll1_q)"),
                ))
            }
        };
        stmts.push(quote! { config.rcc.mux.clk48sel = ::embassy_stm32::rcc::mux::Clk48sel::#v; });
    }
    if let Some(sel) = node.prop_str_opt("adc12")? {
        let v = match sel.as_str() {
            "disable" => quote!(DISABLE),
            "pll1_p" => quote!(PLL1_P),
            "sys" => quote!(SYS),
            other => {
                return Err(syn::Error::new(
                    node.id.span(),
                    format!("clock: unsupported `adc12` `{other}` (disable/pll1_p/sys)"),
                ))
            }
        };
        stmts.push(quote! { config.rcc.mux.adc12sel = ::embassy_stm32::rcc::mux::Adcsel::#v; });
    }
    if let Some(sel) = node.prop_str_opt("fdcan")? {
        let v = match sel.as_str() {
            "hse" => quote!(HSE),
            "pll1_q" => quote!(PLL1_Q),
            "pclk1" => quote!(PCLK1),
            other => {
                return Err(syn::Error::new(
                    node.id.span(),
                    format!("clock: unsupported `fdcan` `{other}` (hse/pll1_q/pclk1)"),
                ))
            }
        };
        stmts.push(quote! { config.rcc.mux.fdcansel = ::embassy_stm32::rcc::mux::Fdcansel::#v; });
    }
    Ok(stmts)
}

fn f1_clock_config(node: &DslNode) -> Result<Vec<TokenStream2>> {
    let mut stmts = Vec::new();

    if let Some(source) = node.prop_str_opt("source")? {
        if source == "hsi" {
            // F1 的 HSI 是布尔开关（PLL 源在 `pll.src` 里指定）。
            stmts.push(quote! { config.rcc.hsi = true; });
        }
    }
    if let Some(hse) = node.prop_u32_any(&["hse"]) {
        let mode = match node.prop_str_opt("hse_mode")?.as_deref() {
            None | Some("oscillator") => quote!(Oscillator),
            Some("bypass") => quote!(Bypass),
            Some(other) => {
                return Err(syn::Error::new(
                    node.id.span(),
                    format!("clock: unsupported `hse-mode` `{other}`"),
                ))
            }
        };
        stmts.push(quote! {
            config.rcc.hse = Some(::embassy_stm32::rcc::Hse {
                freq: ::embassy_stm32::time::Hertz(#hse),
                mode: ::embassy_stm32::rcc::HseMode::#mode,
            });
        });
    }
    if let Some(pll) = node.prop_array("pll") {
        // F1：<prediv mul>（PLL 输出直接就是 SYSCLK 源）。
        if pll.len() < 2 {
            return Err(syn::Error::new(
                node.id.span(),
                "clock: `pll` needs <prediv mul> on F1",
            ));
        }
        let pre = format_ident!("DIV{}", pll[0]);
        let mul = format_ident!("MUL{}", pll[1]);
        let src = match node.prop_str_opt("source")?.as_deref() {
            None | Some("hsi") => quote!(HSI),
            Some("hse") => quote!(HSE),
            Some(other) => {
                return Err(syn::Error::new(
                    node.id.span(),
                    format!("clock: unsupported `source` `{other}`"),
                ))
            }
        };
        stmts.push(quote! {
            config.rcc.pll = Some(::embassy_stm32::rcc::Pll {
                src: ::embassy_stm32::rcc::PllSource::#src,
                prediv: ::embassy_stm32::rcc::PllPreDiv::#pre,
                mul: ::embassy_stm32::rcc::PllMul::#mul,
            });
        });
    }
    if let Some(sys) = node.prop_str_opt("sys")? {
        let v = match sys.as_str() {
            "hsi" => quote!(HSI),
            "hse" => quote!(HSE),
            "pll1_p" => quote!(PLL1_P),
            other => {
                return Err(syn::Error::new(
                    node.id.span(),
                    format!("clock: unsupported `sys` `{other}`"),
                ))
            }
        };
        stmts.push(quote! { config.rcc.sys = ::embassy_stm32::rcc::Sysclk::#v; });
    }
    if let Some(n) = node.prop_u32_any(&["ahb"]) {
        let v = format_ident!("DIV{}", n);
        stmts.push(quote! { config.rcc.ahb_pre = ::embassy_stm32::rcc::AHBPrescaler::#v; });
    }
    for key in ["apb1", "apb2"] {
        if let Some(n) = node.prop_u32_any(&[key]) {
            let field = format_ident!("{}_pre", key);
            let v = format_ident!("DIV{}", n);
            stmts.push(quote! { config.rcc.#field = ::embassy_stm32::rcc::APBPrescaler::#v; });
        }
    }
    if let Some(n) = node.prop_u32_any(&["adc_pre"]) {
        let v = format_ident!("DIV{}", n);
        stmts.push(quote! { config.rcc.adc_pre = ::embassy_stm32::rcc::ADCPrescaler::#v; });
    }
    Ok(stmts)
}

/// 把 `"DMA1_CH0"` 这类通道名转成对应的中断名。
/// - H7/F4：`DMA1_CH0` → `DMA1_STREAM0`
/// - L4：中断就叫 `DMA1_CH0`（embassy 统一命名）
fn stream_irq(channel: &str, node: &DslNode, chip: Option<&str>) -> Result<Ident> {
    let irq = if channel.contains("_STREAM") || channel.contains("_CHANNEL") {
        channel.to_string()
    } else if chip
        .map(|c| c.contains("l4") || c.contains("g4") || c.contains("f1"))
        .unwrap_or(false)
    {
        // L4/G4/F1：通道类型 `DMA1_CH1`，中断名 `DMA1_CHANNEL1`。
        channel.replace("_CH", "_CHANNEL")
    } else if channel.contains("_CH") {
        channel.replace("_CH", "_STREAM")
    } else {
        return Err(syn::Error::new(
            node.id.span(),
            format!(
                "stm32 backend: cannot derive interrupt name from DMA channel `{channel}` (expected `DMA1_CH0` / `DMA1_STREAM0` style)"
            ),
        ));
    };
    syn::parse_str::<Ident>(&irq).map_err(|_| {
        syn::Error::new(
            node.id.span(),
            format!("stm32 backend: cannot derive interrupt name from DMA channel `{channel}`"),
        )
    })
}

fn push_dma_bindings(
    bindings: &mut Vec<(String, TokenStream2)>,
    channel: &str,
    channel_ident: &Ident,
    node: &DslNode,
    chip: Option<&str>,
) -> Result<()> {
    push_binding(bindings, stream_irq(channel, node, chip)?, quote! {
        ::embassy_stm32::dma::InterruptHandler<::embassy_stm32::peripherals::#channel_ident>
    });
    Ok(())
}

fn push_binding(bindings: &mut Vec<(String, TokenStream2)>, irq: Ident, handler: TokenStream2) {
    let name = irq.to_string();
    if !bindings.iter().any(|(n, _)| *n == name) {
        bindings.push((name, handler));
    }
}

fn pwm_pin(pin: Option<Ident>, afio: bool) -> TokenStream2 {
    match pin {
        Some(pin) if afio => quote! {
            Some(::embassy_stm32::timer::simple_pwm::PwmPin::<_, _, ::embassy_stm32::gpio::AfioRemap<0>>::new(
                p.#pin,
                ::embassy_stm32::gpio::OutputType::PushPull,
            ))
        },
        Some(pin) => quote! {
            Some(::embassy_stm32::timer::simple_pwm::PwmPin::new(
                p.#pin,
                ::embassy_stm32::gpio::OutputType::PushPull,
            ))
        },
        None => quote!(None),
    }
}

fn capture_pin(pin: Option<Ident>) -> TokenStream2 {
    match pin {
        Some(pin) => quote! {
            Some(::embassy_stm32::timer::input_capture::CapturePin::new(
                p.#pin,
                ::embassy_stm32::gpio::Pull::None,
            ))
        },
        None => quote!(None),
    }
}

fn comp_pin(pin: Option<Ident>) -> TokenStream2 {
    match pin {
        Some(pin) => quote! {
            Some(::embassy_stm32::timer::complementary_pwm::ComplementaryPwmPin::new(
                p.#pin,
                ::embassy_stm32::gpio::OutputType::PushPull,
            ))
        },
        None => quote!(None),
    }
}

/// 由引脚名推导 EXTI 通道与中断（`PC0` → `EXTI0`，`PB10` → `EXTI15_10`）。
fn exti_info(pin: &str) -> Result<(Ident, Ident, Ident)> {
    let digits: String = pin.chars().skip_while(|c| !c.is_ascii_digit()).collect();
    let num: u32 = digits.parse().map_err(|_| {
        syn::Error::new(
            Span::call_site(),
            format!("cannot derive EXTI channel from pin `{pin}`"),
        )
    })?;
    if num > 15 {
        return Err(syn::Error::new(
            Span::call_site(),
            format!("pin `{pin}` has no EXTI channel"),
        ));
    }
    let irq_name = match num {
        0..=4 => format!("EXTI{num}"),
        5..=9 => "EXTI9_5".to_string(),
        _ => "EXTI15_10".to_string(),
    };
    Ok((
        format_ident!("EXTI{num}"),
        format_ident!("{}", irq_name),
        format_ident!("{}", irq_name),
    ))
}

/// 定时器捕获/比较中断名：TIM1/TIM8 在多数芯片上叫 `TIM1_CC`/`TIM8_CC`，
/// 其余定时器直接用外设名。
fn cc_irq(periph: &Ident) -> Ident {
    let name = periph.to_string();
    if name.starts_with("TIM1") || name.starts_with("TIM8") {
        format_ident!("{}_CC", name)
    } else {
        periph.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &str) -> Result<DslTree> {
        syn::parse_str(input)
    }

    #[test]
    fn stm32_backend_generates_h7_clock_config() {
        let clock = clock_node(&[
            ("source", PropValue::Str(LitStr::new("hsi", Span::call_site()))),
            ("hsi", PropValue::Str(LitStr::new("div1", Span::call_site()))),
            ("csi", PropValue::Bool(true)),
            ("hsi48", PropValue::Bool(true)),
            ("pll1", PropValue::Array(vec![4, 50, 2])),
            ("sys", PropValue::Str(LitStr::new("pll1_p", Span::call_site()))),
            ("ahb", PropValue::U32(LitInt::new("2", Span::call_site()))),
            ("apb1", PropValue::U32(LitInt::new("2", Span::call_site()))),
            ("usb", PropValue::Str(LitStr::new("hsi48", Span::call_site()))),
            (
                "voltage",
                PropValue::Str(LitStr::new("scale1", Span::call_site())),
            ),
        ]);
        let stmts = h7_clock_config(&clock).unwrap();
        let tokens = quote!(#(#stmts)*).to_string().replace(' ', "");
        assert!(tokens.contains("HSIPrescaler::DIV1"));
        assert!(tokens.contains("PllSource::HSI"));
        assert!(tokens.contains("PllMul::MUL50"));
        assert!(tokens.contains("Sysclk::PLL1_P"));
        assert!(tokens.contains("Usbsel::HSI48"));
        assert!(tokens.contains("VoltageScale::Scale1"));
    }

    #[test]
    fn stm32_backend_generates_f4_clock_config() {
        let clock = clock_node(&[
            ("source", PropValue::Str(LitStr::new("hse", Span::call_site()))),
            ("hse", PropValue::U32(LitInt::new("25000000", Span::call_site()))),
            ("pll", PropValue::Array(vec![4, 168, 2, 7])),
            ("sys", PropValue::Str(LitStr::new("pll1_p", Span::call_site()))),
            ("ahb", PropValue::U32(LitInt::new("1", Span::call_site()))),
            (
                "clk48",
                PropValue::Str(LitStr::new("pll1_q", Span::call_site())),
            ),
        ]);
        let stmts = f4_clock_config(&clock).unwrap();
        let tokens = quote!(#(#stmts)*).to_string().replace(' ', "");
        assert!(tokens.contains("HseMode::Oscillator"));
        assert!(tokens.contains("PllSource::HSE"));
        assert!(tokens.contains("PllMul::MUL168"));
        assert!(tokens.contains("PllQDiv::DIV7"));
        assert!(tokens.contains("Clk48sel::PLL1_Q"));
    }

    #[test]
    fn clock_requires_source_with_pll() {
        let clock = clock_node(&[("pll1", PropValue::Array(vec![4, 50, 2]))]);
        let err = h7_clock_config(&clock).unwrap_err();
        assert!(err.to_string().contains("`source` is required"));
    }

    #[test]
    fn parses_node_keyword_in_dsl() {
        let tree = parse(
            r#"
            node note { source: "hsi" };
        "#,
        )
        .unwrap();
        assert_eq!(tree.nodes.len(), 1);
        assert!(tree.nodes[0].driver.is_none());
    }

    #[test]
    fn intent_h7_400mhz() {
        let p = plan_h7(400_000_000, Some(48_000_000), None, None, None, None).unwrap();
        assert_eq!((p.prediv, p.mul, p.divp), (4, 25, 1));
        assert_eq!((p.ahb, p.apb), (2, 2));
        assert!(p.scale1);
        assert!(p.usb48);
        assert!(!p.source_hse);
        assert!(p.pll2.is_none());
    }

    #[test]
    fn intent_h7_550mhz_uses_scale0() {
        let p = plan_h7(550_000_000, None, None, None, None, None).unwrap();
        assert_eq!((p.prediv, p.mul, p.divp), (32, 275, 1));
        assert!(!p.scale1);
        assert!(!p.usb48);
    }

    #[test]
    fn intent_h7_550mhz_with_hse() {
        // 8 MHz 外部晶振：VCO=550M, prediv=4, mul=275, divp=1。
        let p = plan_h7(550_000_000, None, Some(8_000_000), None, None, None).unwrap();
        assert_eq!((p.prediv, p.mul, p.divp), (4, 275, 1));
        assert!(p.source_hse);
        assert!(!p.scale1);
    }

    #[test]
    fn intent_h7_i2s_50mhz() {
        let p = plan_h7(400_000_000, None, None, Some(50_000_000), None, None).unwrap();
        assert_eq!(p.pll2, Some((8, 25, 4, 1)));
        assert_eq!(p.spi123, Some("pll2_p"));
        assert_eq!(p.sdmmc_sel, None);
    }

    #[test]
    fn intent_h7_i2s_and_sdmmc_share_vco() {
        let p = plan_h7(
            400_000_000,
            None,
            None,
            Some(50_000_000),
            None,
            Some(25_000_000),
        )
        .unwrap();
        // P=50M×4 与 R=25M×8 共用 VCO=200M。
        assert_eq!(p.pll2, Some((8, 25, 4, 8)));
        assert_eq!(p.spi123, Some("pll2_p"));
        assert_eq!(p.sdmmc_sel, Some("pll2_r"));
    }

    #[test]
    fn intent_h7_adc_i2s_conflict() {
        let err = plan_h7(
            400_000_000,
            None,
            None,
            Some(50_000_000),
            Some(100_000_000),
            None,
        )
        .unwrap_err();
        assert!(err.contains("share PLL2_P"));
    }

    #[test]
    fn intent_f4_96mhz_with_usb() {
        let p = plan_f4(96_000_000, Some(48_000_000), Some(25_000_000)).unwrap();
        assert_eq!((p.prediv, p.mul, p.divp, p.divq), (25, 192, 2, 4));
        assert_eq!((p.ahb, p.apb1, p.apb2), (1, 2, 1));
        assert!(p.usb48);
        assert!(p.source_hse);
    }

    #[test]
    fn intent_f4_96mhz_hsi_no_crystal() {
        // 无晶振：内部 HSI 16 MHz，VCO=192M（F4 的 PLLM 最小为 2）。
        let p = plan_f4(96_000_000, Some(48_000_000), None).unwrap();
        assert_eq!((p.prediv, p.mul, p.divp, p.divq), (2, 24, 2, 4));
        assert!(!p.source_hse);
        assert!(p.usb48);
    }

    #[test]
    fn intent_f4_100mhz_usb_impossible() {
        let err = plan_f4(100_000_000, Some(48_000_000), Some(25_000_000)).unwrap_err();
        assert!(err.contains("96 MHz"));
    }

    #[test]
    fn intent_f4_100mhz_without_usb() {
        let p = plan_f4(100_000_000, None, Some(25_000_000)).unwrap();
        assert_eq!((p.prediv, p.mul, p.divp), (2, 16, 2));
        assert!(!p.usb48);
        assert!(p.source_hse);
    }

    #[test]
    fn intent_l4_80mhz_hsi() {
        // HSI16：VCO=160M（divr=2），prediv=1, mul=10。
        let p = plan_l4(80_000_000, Some(48_000_000), None).unwrap();
        assert_eq!((p.prediv, p.mul, p.divr), (1, 10, 2));
        assert!(p.usb48);
        assert!(p.hse.is_none());
    }

    #[test]
    fn intent_l4_80mhz_hse() {
        // 8 MHz 晶振：VCO=160M，prediv=1, mul=20。
        let p = plan_l4(80_000_000, None, Some(8_000_000)).unwrap();
        assert_eq!((p.prediv, p.mul, p.divr), (1, 20, 2));
        assert_eq!(p.hse, Some(8_000_000));
    }

    #[test]
    fn intent_l4_rejects_overclock() {
        let err = plan_l4(100_000_000, None, None).unwrap_err();
        assert!(err.contains("80 MHz"));
    }

    #[test]
    fn intent_g4_170mhz_hsi_with_usb() {
        // HSI16：VCO=340M（divr=2），prediv=4, mul=85；>150M 需要 boost。
        let p = plan_g4(170_000_000, Some(48_000_000), None, None).unwrap();
        assert_eq!((p.prediv, p.mul, p.divr), (4, 85, 2));
        assert!(p.usb48);
        assert!(p.boost);
        assert!(p.hse.is_none());
    }

    #[test]
    fn intent_g4_80mhz_hsi_with_adc() {
        // HSI16：VCO=160M（divr=2），prediv=1, mul=10；ADC 只作为开关。
        let p = plan_g4(80_000_000, None, None, Some(60_000_000)).unwrap();
        assert_eq!((p.prediv, p.mul, p.divr), (1, 10, 2));
        assert!(p.adc);
        assert!(!p.boost);
    }

    #[test]
    fn intent_g4_170mhz_hse() {
        // 8 MHz 晶振：VCO=340M，prediv=2, mul=85。
        let p = plan_g4(170_000_000, None, Some(8_000_000), None).unwrap();
        assert_eq!((p.prediv, p.mul, p.divr), (2, 85, 2));
        assert_eq!(p.hse, Some(8_000_000));
        assert!(p.boost);
    }

    #[test]
    fn intent_g4_rejects_overclock_and_bad_usb() {
        let err = plan_g4(200_000_000, None, None, None).unwrap_err();
        assert!(err.contains("170 MHz"));
        let err = plan_g4(80_000_000, Some(60_000_000), None, None).unwrap_err();
        assert!(err.contains("48 MHz"));
    }

    #[test]
    fn stm32_backend_generates_g4_clock_config() {
        let clock = clock_node(&[
            ("source", PropValue::Str(LitStr::new("hsi", Span::call_site()))),
            ("pll", PropValue::Array(vec![4, 85, 2])),
            ("sys", PropValue::Str(LitStr::new("pll1_r", Span::call_site()))),
            ("hsi48", PropValue::Bool(true)),
            ("hsi48_sync", PropValue::Bool(true)),
            ("boost", PropValue::Bool(true)),
            (
                "adc12",
                PropValue::Str(LitStr::new("sys", Span::call_site())),
            ),
            (
                "fdcan",
                PropValue::Str(LitStr::new("pclk1", Span::call_site())),
            ),
        ]);
        let stmts = g4_clock_config(&clock).unwrap();
        let tokens = quote!(#(#stmts)*).to_string().replace(' ', "");
        assert!(tokens.contains("PllPreDiv::DIV4"));
        assert!(tokens.contains("PllMul::MUL85"));
        assert!(tokens.contains("PllRDiv::DIV2"));
        assert!(tokens.contains("Sysclk::PLL1_R"));
        assert!(tokens.contains("sync_from_usb:true"));
        assert!(tokens.contains("config.rcc.boost=true"));
        assert!(tokens.contains("mux.adc12sel"));
        assert!(tokens.contains("Adcsel::SYS"));
        assert!(tokens.contains("Fdcansel::PCLK1"));
    }

    #[test]
    fn intent_f1_72mhz_hse_with_usb() {
        // 8 MHz 晶振：PLL = 8M ÷1 ×9 = 72 MHz（经典 BluePill 配置），
        // USB 由 PLL÷1.5 派生 48 MHz；APB1 ÷2；ADC ÷6 = 12 MHz。
        let p = plan_f1(72_000_000, Some(48_000_000), Some(8_000_000), Some(60_000_000)).unwrap();
        assert_eq!((p.prediv, p.mul), (1, 9));
        assert_eq!(p.apb1, 2);
        assert_eq!(p.adc_pre, Some(6));
        assert!(p.usb48);
        assert_eq!(p.hse, Some(8_000_000));
    }

    #[test]
    fn intent_f1_64mhz_hsi() {
        // HSI 8 MHz：必须 ÷2，×16 = 64 MHz（HSI 上限）。
        let p = plan_f1(64_000_000, None, None, None).unwrap();
        assert_eq!((p.prediv, p.mul), (2, 16));
        assert_eq!(p.apb1, 2);
        assert!(p.adc_pre.is_none());
    }

    #[test]
    fn intent_f1_48mhz_hsi_with_usb() {
        // HSI ÷2 ×12 = 48 MHz：USB 由 PLL÷1 派生 48 MHz。
        let p = plan_f1(48_000_000, Some(48_000_000), None, None).unwrap();
        assert_eq!((p.prediv, p.mul), (2, 12));
        assert_eq!(p.apb1, 2);
        assert!(p.usb48);
    }

    #[test]
    fn intent_f1_rejects_usb_without_pll_48_72() {
        let err = plan_f1(64_000_000, Some(48_000_000), None, None).unwrap_err();
        assert!(err.contains("72 MHz"));
        let err = plan_f1(80_000_000, None, Some(8_000_000), None).unwrap_err();
        assert!(err.contains("72 MHz"));
    }

    #[test]
    fn stm32_backend_generates_f1_clock_config() {
        let clock = clock_node(&[
            ("source", PropValue::Str(LitStr::new("hse", Span::call_site()))),
            ("hse", PropValue::U32(LitInt::new("8000000", Span::call_site()))),
            ("pll", PropValue::Array(vec![1, 9])),
            ("sys", PropValue::Str(LitStr::new("pll1_p", Span::call_site()))),
            ("ahb", PropValue::U32(LitInt::new("1", Span::call_site()))),
            ("apb1", PropValue::U32(LitInt::new("2", Span::call_site()))),
            ("apb2", PropValue::U32(LitInt::new("1", Span::call_site()))),
            ("adc_pre", PropValue::U32(LitInt::new("6", Span::call_site()))),
        ]);
        let stmts = f1_clock_config(&clock).unwrap();
        let tokens = quote!(#(#stmts)*).to_string().replace(' ', "");
        assert!(tokens.contains("PllSource::HSE"));
        assert!(tokens.contains("PllPreDiv::DIV1"));
        assert!(tokens.contains("PllMul::MUL9"));
        assert!(tokens.contains("Sysclk::PLL1_P"));
        assert!(tokens.contains("APBPrescaler::DIV2"));
        assert!(tokens.contains("ADCPrescaler::DIV6"));
    }
#[cfg(test)]
fn clock_node(props: &[(&str, PropValue)]) -> DslNode {
    DslNode {
        id: Ident::new("clock", Span::call_site()),
        kind: NodeKindAst::Device,
        driver: None,
        props: props
            .iter()
            .map(|(k, v)| DslProp {
                key: Ident::new(k, Span::call_site()),
                value: match v {
                    PropValue::Str(s) => {
                        PropValue::Str(LitStr::new(&s.value(), s.span()))
                    }
                    PropValue::U32(l) => {
                    PropValue::U32(LitInt::new(l.base10_digits(), l.span()))
                    }
                    PropValue::Ref(i) => PropValue::Ref(Ident::new(&i.to_string(), i.span())),
                    PropValue::Array(v) => PropValue::Array(v.clone()),
                    PropValue::Bool(b) => PropValue::Bool(*b),
                },
            })
            .collect(),
        deps: Vec::new(),
    }
}

    #[test]
    fn parses_tree_with_all_node_kinds() {
        let tree = parse(
            r#"
            name "demo";
            bus i2c0: I2c { periph: "I2C1", scl: "PB8", sda: "PB7", freq: 400_000 };
            bus uart0: Uart { periph: "USART1", rx: "PA10", tx: "PA9", baud: 115_200 };
            gpio led0: Out { pin: "PC13", level: "high" };
            periph rng0: Rng { periph: "RNG" };
            device bme280: Bme280Driver { bus: i2c0, addr: 0x76 };
        "#,
        )
        .unwrap();
        assert_eq!(tree.name.as_ref().unwrap().value(), "demo");
        assert_eq!(tree.nodes.len(), 5);
        assert!(matches!(
            tree.nodes[2].kind,
            NodeKindAst::Gpio(GpioKindAst::Out)
        ));
        assert!(matches!(
            tree.nodes[3].kind,
            NodeKindAst::Peripheral(PeriphKindAst::Rng)
        ));
        assert_eq!(tree.nodes[4].deps.len(), 1);
        assert_eq!(tree.nodes[4].deps[0].to_string(), "i2c0");
    }

    #[test]
    fn detects_duplicate_ids() {
        let tree = parse(
            r#"
            bus i2c0: I2c {};
            bus i2c0: I2c {};
        "#,
        )
        .unwrap();
        let err = validate(&tree).unwrap_err();
        assert!(err.to_string().contains("duplicate node id `i2c0`"));
    }

    #[test]
    fn detects_missing_dependency() {
        let tree = parse(r#"device a: A { bus: nope };"#).unwrap();
        let err = validate(&tree).unwrap_err();
        assert!(err.to_string().contains("unknown node `nope`"));
    }

    #[test]
    fn detects_cycle() {
        let tree = parse(
            r#"
            device a: A { bus: b };
            device b: B { bus: a };
        "#,
        )
        .unwrap();
        let err = validate(&tree).unwrap_err();
        assert!(err.to_string().contains("cycle"));
    }

    #[test]
    fn expand_generates_tree_static() {
        let tree = parse(
            r#"
            name "demo";
            bus i2c0: I2c { periph: "I2C1", scl: "PB8", sda: "PB7", freq: 400_000 };
            device bme280: Bme280Driver { bus: i2c0, addr: 0x76 };
        "#,
        )
        .unwrap();
        let tokens = expand(tree).unwrap().to_string();
        assert!(tokens.contains("pub static TREE"));
        assert!(tokens.contains("I2c"));
        assert!(tokens.contains("0x76"));
    }

    #[test]
    fn stm32_backend_generates_board_for_all_bus_kinds() {
        let tree = parse(
            r#"
            name "demo";
            backend stm32;
            bus i2c0: I2c { periph: "I2C1", scl: "PB8", sda: "PB7", dma_tx: "DMA1_CH0", dma_rx: "DMA1_CH1", freq: 400_000 };
            bus spi0: Spi { periph: "SPI1", sck: "PA5", mosi: "PA7", miso: "PA6", dma_tx: "DMA1_CH2", dma_rx: "DMA1_CH3" };
            bus uart0: Uart { periph: "USART1", rx: "PA10", tx: "PA9", dma_tx: "DMA1_CH4", dma_rx: "DMA1_CH5", baud: 115_200 };
            gpio led0: Out { pin: "PC13", level: "high" };
            gpio btn0: In { pin: "PA0", pull: "up" };
        "#,
        )
        .unwrap();
        // TokenStream 序列化时 `::` / `.` 周围可能带空格，先去掉再断言。
        let tokens = expand(tree).unwrap().to_string().replace(' ', "");
        assert!(tokens.contains("pubstructBoard"));
        assert!(tokens.contains("I2c::new"));
        assert!(tokens.contains("Spi::new"));
        assert!(tokens.contains("Uart::new"));
        assert!(tokens.contains("Output::new"));
        assert!(tokens.contains("Input::new"));
        assert!(tokens.contains("p.I2C1"));
        assert!(tokens.contains("p.DMA1_CH0"));
        assert!(tokens.contains("DtIrqs"));
        assert!(tokens.contains("I2C1_EV"));
        assert!(tokens.contains("I2C1_ER"));
        assert!(tokens.contains("DMA1_STREAM0"));
        assert!(tokens.contains("USART1=>"));
        assert!(tokens.contains("DMA1_STREAM4"));
    }

    #[test]
    fn stm32_backend_requires_uart_pins() {
        let tree = parse(
            r#"
            backend stm32;
            bus uart0: Uart { periph: "USART1" };
        "#,
        )
        .unwrap();
        let err = expand(tree).unwrap_err();
        assert!(err.to_string().contains("missing required prop `rx`"));
    }

    #[test]
    fn stm32_backend_generates_peripherals() {
        let tree = parse(
            r#"
            name "demo";
            backend stm32;
            chip "stm32h723zg";
            periph rng0: Rng { periph: "RNG" };
            periph adc0: Adc { periph: "ADC1" };
            periph crc0: Crc { periph: "CRC" };
            periph dac0: Dac { periph: "DAC1", pin: "PA4" };
            periph pwm0: Pwm { periph: "TIM2", ch1: "PA0", ch2: "PA1", ch3: "PA2", ch4: "PA3", freq: 1000 };
            periph can0: Can { periph: "FDCAN1", rx: "PB8", tx: "PB9" };
            periph usb0: Usb { periph: "USB_OTG_HS", dp: "PA12", dm: "PA11", ep_out: 256 };
            periph qei0: Qei { periph: "TIM3", ch1: "PB4", ch2: "PB5" };
            periph ic0: InputCapture { periph: "TIM4", ch1: "PB6", freq: 1000000 };
            periph sdmmc0: Sdmmc { periph: "SDMMC1", clk: "PC12", cmd: "PD2", d0: "PC8" };
            periph i2s0: I2s { periph: "SPI3", sd: "PB2", ws: "PA15", ck: "PB3", dma: "DMA2_CH3", buffer: 512 };
            periph pwm_in0: PwmInput { periph: "TIM8", pin: "PC6", freq: 10000 };
            periph pwm_adv0: ComplementaryPwm { periph: "TIM1", ch1: "PA8", ch1n: "PB13", freq: 10000 };
            bus spi1: Spi { periph: "SPI2", sck: "PB13", mosi: "PB15", miso: "PB14", cs: "PB12", mode: "slave", dma_tx: "DMA1_CH6", dma_rx: "DMA1_CH7" };
        "#,
        )
        .unwrap();
        let tokens = expand(tree).unwrap().to_string().replace(' ', "");
        assert!(tokens.contains("Rng::new"));
        assert!(tokens.contains("Adc::new"));
        assert!(tokens.contains("Crc::new"));
        assert!(tokens.contains("DacChannel::new_blocking"));
        assert!(tokens.contains("SimplePwm::new"));
        assert!(tokens.contains("CanConfigurator::new"));
        assert!(tokens.contains("into_normal_mode"));
        assert!(tokens.contains("FDCAN1_IT0"));
        assert!(tokens.contains("FDCAN1_IT1"));
        assert!(tokens.contains("RNG=>"));
        assert!(tokens.contains("PolySize::Width32"));
        assert!(tokens.contains("Driver::new_hs"));
        assert!(tokens.contains("OTG_HS=>"));
        assert!(tokens.contains("Qei::new"));
        assert!(tokens.contains("InputCapture::new"));
        assert!(tokens.contains("Sdmmc::new_1bit"));
        assert!(tokens.contains("new_slave"));
        assert!(tokens.contains("EP_BUF"));
        assert!(tokens.contains("new_txonly_nomck"));
        assert!(tokens.contains("DMA_BUF"));
        assert!(tokens.contains("PwmInput::new_ch1"));
        assert!(tokens.contains("ComplementaryPwm::new"));
        assert!(tokens.contains("ComplementaryPwmPin::new"));
    }

    #[test]
    fn stm32_backend_peripheral_requires_chip() {
        let tree = parse(
            r#"
            backend stm32;
            periph can0: Can { periph: "FDCAN1", rx: "PA11", tx: "PA12" };
        "#,
        )
        .unwrap();
        let err = expand(tree).unwrap_err();
        assert!(err.to_string().contains("declare `chip"));
    }

    #[test]
    fn stm32_backend_generates_f411_variant_peripherals() {
        let tree = parse(
            r#"
            backend stm32;
            chip "stm32f411ce";
            periph rng0: Rng { periph: "RNG" };
            periph adc0: Adc { periph: "ADC1" };
            periph crc0: Crc { periph: "CRC" };
            periph dac0: Dac { periph: "DAC1", pin: "PA4" };
            periph pwm0: Pwm { periph: "TIM2", ch1: "PA0", ch2: "PA1" };
        "#,
        )
        .unwrap();
        let tokens = expand(tree).unwrap().to_string().replace(' ', "");
        assert!(tokens.contains("Adc::new"));
        assert!(!tokens.contains("ADC=>"));
        assert!(tokens.contains("Crc::new"));
        assert!(!tokens.contains("PolySize"));
    }

}
