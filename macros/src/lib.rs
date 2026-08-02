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
        let (is_bus, is_device, is_gpio, is_periph) = (
            input.peek(kw::bus),
            input.peek(kw::device),
            input.peek(kw::gpio),
            input.peek(kw::periph),
        );
        if !is_bus && !is_device && !is_gpio && !is_periph {
            return Err(input.error("expected `bus`, `device`, `gpio` or `periph`"));
        }
        if is_bus {
            input.parse::<kw::bus>()?;
        } else if is_device {
            input.parse::<kw::device>()?;
        } else if is_gpio {
            input.parse::<kw::gpio>()?;
        } else {
            input.parse::<kw::periph>()?;
        }

        let id: Ident = input.parse()?;
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
    let mut fields = Vec::new();
    let mut inits = Vec::new();
    // 模块级 static（如 USB 端点缓冲）。
    let mut statics = Vec::new();
    // 中断绑定条目，按中断名去重。
    let mut bindings: Vec<(String, TokenStream2)> = Vec::new();

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

                push_dma_bindings(&mut bindings, &dma_tx_str, &dma_tx, node)?;
                push_dma_bindings(&mut bindings, &dma_rx_str, &dma_rx, node)?;
                let ev_irq = format_ident!("{}_EV", periph.to_string());
                let er_irq = format_ident!("{}_ER", periph.to_string());
                push_binding(&mut bindings, ev_irq, quote! {
                    ::embassy_stm32::i2c::EventInterruptHandler<::embassy_stm32::peripherals::#periph>
                });
                push_binding(&mut bindings, er_irq, quote! {
                    ::embassy_stm32::i2c::ErrorInterruptHandler<::embassy_stm32::peripherals::#periph>
                });

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

                push_dma_bindings(&mut bindings, &dma_tx_str, &dma_tx, node)?;
                push_dma_bindings(&mut bindings, &dma_rx_str, &dma_rx, node)?;

                if is_slave {
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
                push_dma_bindings(&mut bindings, &dma_tx_str, &dma_tx, node)?;
                push_dma_bindings(&mut bindings, &dma_rx_str, &dma_rx, node)?;

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
                            // H723 是 adc_v3、F411 是 adc_v2：构造都是 `Adc::new(adc)`，
                            // 无中断绑定。v1（F1/F3/L4 等）需要 IRQ，暂不支持。
                            Some(chip) if chip.contains("h723") || chip.contains("f411") => {
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
                                    "stm32 backend: `adc` is not supported for this chip (supported: stm32h723zg / stm32f411ce)",
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
                            Some(chip) if chip.contains("f411") => {
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
                        let ch1 = pwm_pin(ch1);
                        let ch2 = pwm_pin(ch2);
                        let ch3 = pwm_pin(ch3);
                        let ch4 = pwm_pin(ch4);
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
                            Some(chip) if chip.contains("h723") => {
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
                                let tx_irq = format_ident!("{}_TX", periph.to_string());
                                let rx0 = format_ident!("{}_RX0", periph.to_string());
                                let rx1 = format_ident!("{}_RX1", periph.to_string());
                                let sce = format_ident!("{}_SCE", periph.to_string());
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
                                    "stm32 backend: `can` is not supported for this chip (declare `chip \"stm32h723zg\"` or a F4/F1 chip with bxCAN)",
                                ))
                            }
                        }
                    }
                    PeriphKindAst::Usb => {
                        let dp = node.pin_ident("dp")?;
                        let dm = node.pin_ident("dm")?;
                        let ep_out =
                            node.prop_u32_any(&["ep_out", "ep_out_buf"]).unwrap_or(256) as usize;
                        let ep_buf = format_ident!("{}_EP_BUF", field.to_string());
                        let periph_str = periph.to_string();
                        let irq_name = periph_str
                            .strip_prefix("USB_")
                            .unwrap_or(&periph_str)
                            .to_string();
                        let irq = format_ident!("{}", irq_name);
                        push_binding(&mut bindings, irq, quote! {
                            ::embassy_stm32::usb::InterruptHandler<::embassy_stm32::peripherals::#periph>
                        });
                        fields.push(quote! {
                            pub #field: ::embassy_stm32::usb::Driver<'static, ::embassy_stm32::peripherals::#periph>
                        });
                        statics.push(quote! {
                            #[allow(non_upper_case_globals)]
                            static mut #ep_buf: [u8; #ep_out] = [0; #ep_out];
                        });
                        match chip_name(tree)? {
                            Some(chip) if chip.contains("h723") => {
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
                            _ => {
                                return Err(syn::Error::new(
                                    node.id.span(),
                                    "stm32 backend: `usb` is not supported for this chip (declare `chip \"stm32h723zg\"` or `chip \"stm32f411ce\"`)",
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
                        push_dma_bindings(&mut bindings, &dma_str, &dma, node)?;
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
                        let ch1 = pwm_pin(node.pin_ident_opt("ch1")?);
                        let ch1n = comp_pin(node.pin_ident_opt("ch1n")?);
                        let ch2 = pwm_pin(node.pin_ident_opt("ch2")?);
                        let ch2n = comp_pin(node.pin_ident_opt("ch2n")?);
                        let ch3 = pwm_pin(node.pin_ident_opt("ch3")?);
                        let ch3n = comp_pin(node.pin_ident_opt("ch3n")?);
                        let ch4 = pwm_pin(node.pin_ident_opt("ch4")?);
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
            NodeKindAst::Device => continue,
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

/// 把 `"DMA1_CH0"` 这类通道名转成对应的中断名（H7/F4 风格 `DMA1_STREAM0`）；
/// 若通道名已经是 `DMA1_STREAMn` 则原样使用。
fn stream_irq(channel: &str, node: &DslNode) -> Result<Ident> {
    let irq = if channel.contains("_STREAM") {
        channel.to_string()
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
) -> Result<()> {
    push_binding(bindings, stream_irq(channel, node)?, quote! {
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

fn pwm_pin(pin: Option<Ident>) -> TokenStream2 {
    match pin {
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

    #[test]
    fn stm32_backend_rejects_bad_gpio_level() {
        let tree = parse(
            r#"
            backend stm32;
            gpio led0: Out { pin: "PC13", level: "medium" };
        "#,
        )
        .unwrap();
        let err = expand(tree).unwrap_err();
        assert!(err.to_string().contains("`level` must be `high` or `low`"));
    }
}
