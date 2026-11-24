//! 属性 / 值支持表（两条 dict 规则共用）。
//!
//! 数据直接以 const 形式定义在本模块，来源与证据见各条目的行尾注释。
//!
//! 同步方式：
//!   [`UNSUPPORTED_PROPERTIES`] 按「不支持的 CSS 属性」清单逐条更新，
//!     一格多属性拆成多条；表外补充的属性以行尾注释标注 references 出处。
//!   [`UNSUPPORTED_VALUES`] 按各属性的「支持值」表与 ⚠️ 标注更新，
//!     只收录文档明确写「不支持 / 不可用 / 映射为其他值」的整值，不从语法推断。
//!   [`LENGTH_VALUED_PROPERTIES`] 收录「只接受关键字、写了长度值整条声明就会被丢弃」的属性。

/// Skyline 完全不支持的属性 → 建议。
///
/// 匹配：属性名小写比较。
const UNSUPPORTED_PROPERTIES: &[(&str, &str)] = &[
    ("float", "使用 flex 布局"),                         // 属性支持表
    ("contain", "无替代（`-wx-contain` 同样不被接受）"), // 属性支持表；属性表里 `contain` 亦为无效
    ("resize", "无替代"),                                // 属性支持表
    ("writing-mode", "无替代"),                          // 属性支持表
    ("text-indent", "使用 padding-left 模拟"),           // 属性支持表
    ("overflow-wrap", "使用 word-break"),                // 属性支持表
    ("word-wrap", "使用 word-break"), // `overflow-wrap` 的旧名，引擎属性表里同样没有该 id
    // 属性表（fp 的 `property_list!`）里没有任何 `border-image-*`，声明整条丢弃；
    // 官方天空属性支持表也未列该项。丢弃后 `border-color` 回落 `currentColor`，
    // 常见后果是彩色下划线变成当前文字色（往往接近黑色）。
    ("border-image", "改用叠加的背景色块或独立元素绘制"),
    ("border-image-source", "改用叠加的背景色块或独立元素绘制"),
    ("border-image-slice", "无替代"),
    ("border-image-width", "无替代"),
    ("border-image-outset", "无替代"),
    ("border-image-repeat", "无替代"),
    (
        "clip-path",
        "裁切动画改用外层 `overflow: hidden` + 内层 `transform` 位移",
    ), // 属性名表无该 id，声明整条丢弃
    ("text-decoration-thickness", "无替代"), // 属性支持表
    ("background-attachment", "无替代"),     // 属性支持表
    ("background-clip", "无替代"),           // 属性支持表
    ("background-origin", "无替代"),         // 属性支持表
    ("mask-origin", "无替代"),               // 属性支持表（一格多属性拆分）
    ("mask-clip", "无替代"),                 // 属性支持表（一格多属性拆分）
    ("mask-mode", "无替代"),                 // 属性支持表（一格多属性拆分）
    ("justify-items", "无替代"),             // 属性支持表
    ("list-style-type", "无替代"),           // 属性支持表（一格多属性拆分）
    ("list-style-image", "无替代"),          // 属性支持表（一格多属性拆分）
    ("list-style-position", "无替代"),       // 属性支持表（一格多属性拆分）
    ("text-transform", "改用 WXML 里预先转换好的文本"), // 属性名表无该属性 id，声明整条丢弃
    (
        "user-select",
        "改用 `<text user-select=\"{{false}}\">` 属性",
    ), // 属性名表里没有该属性
    (
        "-webkit-line-clamp",
        "多行省略改用 `<text max-lines=\"\">`（引擎把它映射成无效值）",
    ),
    // 标准名同样不是属性名：只有私有 `_wx_line_clamp`，标准名被映射成无效值。
    (
        "line-clamp",
        "多行省略改用 `<text max-lines=\"\">`（引擎只认私有 `_wx_line_clamp`）",
    ),
    // 老式 `-webkit-box` 弹性盒（display: -webkit-box 与 -webkit-box-*）按规则块合并，
    // 归 `skyline/legacy-flexbox` 报，避免同一块布局被拆成三、四条 error。
    // 引擎里没有这两个属性，图片裁剪用 <image mode>。
    (
        "object-fit",
        "图片裁剪改用 `<image mode=\"aspectFill\">` 等属性",
    ),
    ("object-position", "图片裁剪改用 `<image mode>`"),
];

/// 属性在该渲染器下的 CSS 初始值。
///
/// 不支持的属性被丢弃后走的就是初始值：声明恰好等于初始值（`object-fit: fill`、
/// `background-clip: border-box`）时，丢弃与不丢弃渲染结果相同，不算迁移差异。
const INITIAL_VALUES: &[(&str, &[&str])] = &[
    ("float", &["none"]),
    ("contain", &["none"]),
    ("resize", &["none"]),
    ("writing-mode", &["horizontal-tb"]),
    ("text-indent", &["0"]),
    ("overflow-wrap", &["normal"]),
    ("word-wrap", &["normal"]),
    ("clip-path", &["none"]),
    ("text-decoration-thickness", &["auto"]),
    ("background-attachment", &["scroll"]),
    ("background-clip", &["border-box"]),
    ("background-origin", &["padding-box"]),
    ("mask-origin", &["border-box"]),
    ("mask-clip", &["border-box"]),
    ("mask-mode", &["match-source"]),
    ("justify-items", &["legacy"]),
    ("list-style-type", &["disc"]),
    ("list-style-image", &["none"]),
    ("list-style-position", &["outside"]),
    ("text-transform", &["none"]),
    ("user-select", &["auto"]),
    ("object-fit", &["fill"]),
    ("object-position", &["50% 50%", "center", "center center"]),
];

/// 声明的取值是否就是该属性的初始值（即丢弃它不会改变渲染结果）。
pub(crate) fn is_initial_value(name: &str, value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    INITIAL_VALUES.iter().any(|(property, initial)| {
        *property == name && initial.iter().any(|candidate| *candidate == value)
    })
}

// 已移除 direction / overflow-x / overflow-y：引擎实际接受（direction 与两轴 overflow 都有属性 id，
//   两轴的值会折叠进 `overflow`，再按 visible/hidden 应用）。
//   文档「不支持」的原文只对 auto/scroll 与「分轴独立设置」成立。

/// border-*-style 系列共用同一套支持值。
pub(crate) const BORDER_STYLE_PROPERTIES: &[&str] = &[
    "border-style",
    "border-top-style",
    "border-right-style",
    "border-bottom-style",
    "border-left-style",
];

/// 分轴 overflow 属性共用同一套支持值。
const OVERFLOW_AXIS_PROPERTIES: &[&str] = &["overflow-x", "overflow-y"];

/// 属性可用、但整值不支持的条目：`(属性名列表, 整值, 建议)`。
///
/// 属性名列表对应原表一格多属性；匹配口径：属性名小写比较，值 trim + 小写后整值相等才命中；
/// 含 var( / env( / {{ 的值跳过。按声明顺序首次命中优先。
const UNSUPPORTED_VALUES: &[(&[&str], &str, &str)] = &[
    (&["display"], "grid", "改用 flex 布局替代 grid"), // 属性表「display」、属性规范示例
    (&["display"], "flow-root", "改用 flex 或 block"), // 属性表「display」
    (&["display"], "inline-grid", "改用 inline-flex"), // 引擎不认 InlineGrid 这个 case（属性语法可解析）
    (
        &["display"],
        "contents",
        "`contents` 在 Skyline 下整条声明被丢弃（fp 的 Display 枚举无此值，float-pigment-css/src/typing.rs:522），元素会保留默认 flex 盒子：改成显式 flex 容器并调整父子层级",
    ), // display 值表（none/block/inline/inline-block/flex/inline-flex）不含 contents
    (
        &["position"],
        "sticky",
        "吸顶改用 `sticky-section` + `sticky-header`，二者必须放在 `<scroll-view type=\"custom\">` 里，`sticky-header` 必须是 `sticky-section` 的第一个子元素，页面级滚动也要整体搬进这个 scroll-view；其余场景改用 relative/absolute/fixed",
    ), // position 值表；sticky 解析成未设置后回落到 relative
    (
        &["overflow"],
        "auto",
        "改用 overflow: hidden；滚动使用 scroll-view 组件",
    ), // overflow 值表、属性规范示例
    (
        &["overflow"],
        "scroll",
        "改用 overflow: hidden；滚动使用 scroll-view 组件",
    ), // overflow 值表
    // overflow-x/overflow-y 的属性名本身是支持的（visible/hidden 会折叠成 overflow），只有这两个值被拒
    // （两轴同构）。属性条目因此不收录，值按分轴属性分别收录。
    (
        OVERFLOW_AXIS_PROPERTIES,
        "auto",
        "改用 overflow: hidden；滚动使用 scroll-view 组件",
    ), // 分轴的两个值都被拒
    (
        OVERFLOW_AXIS_PROPERTIES,
        "scroll",
        "改用 overflow: hidden；滚动使用 scroll-view 组件",
    ), // 分轴的两个值都被拒
    (&["visibility"], "collapse", "改用 visible 或 hidden"), // visibility 值表
    (&["white-space"], "pre", "改用 normal 或 nowrap"),      // 属性表「white-space」、属性速查表
    (&["white-space"], "pre-wrap", "改用 normal 或 nowrap"), // 属性表「white-space」、属性速查表
    (&["white-space"], "pre-line", "改用 normal 或 nowrap"), // 属性表「white-space」、属性速查表
    (
        &["font-weight"],
        "bolder",
        "改用 normal、bold 或 100-900 的数值",
    ), // 属性表「font-weight」
    (
        &["font-weight"],
        "lighter",
        "改用 normal、bold 或 100-900 的数值",
    ), // 属性表「font-weight」
    (&["font-style"], "oblique", "改用 normal 或 italic"),   // 属性表「font-style」
    // word-break: break-word 不是无效值（被映射为 normal），由 skyline/value-degraded 按 warning 报。
    (
        &["vertical-align"],
        "text-top",
        "改用 baseline/top/middle/bottom",
    ), // 属性表「vertical-align」
    (
        &["vertical-align"],
        "text-bottom",
        "改用 baseline/top/middle/bottom",
    ), // 属性表「vertical-align」
    (
        &["text-align"],
        "justify-all",
        "改用 left/center/right/justify/start/end",
    ), // 属性表「text-align」
    (
        &["text-align"],
        "match-parent",
        "改用 left/center/right/justify/start/end",
    ), // 属性表「text-align」
    (
        &["text-decoration-line"],
        "blink",
        "仅支持 none/underline/overline/line-through，且只能单值",
    ), // 属性速查表
    (
        BORDER_STYLE_PROPERTIES,
        "double",
        "border-*-style 仅支持 none/solid/dashed/dotted",
    ), // 属性速查表、视觉表「border-style」
    (
        BORDER_STYLE_PROPERTIES,
        "groove",
        "border-*-style 仅支持 none/solid/dashed/dotted",
    ), // 属性速查表、视觉表「border-style」
    (
        BORDER_STYLE_PROPERTIES,
        "ridge",
        "border-*-style 仅支持 none/solid/dashed/dotted",
    ), // 属性速查表、视觉表「border-style」
    (
        BORDER_STYLE_PROPERTIES,
        "inset",
        "border-*-style 仅支持 none/solid/dashed/dotted",
    ), // 属性速查表、视觉表「border-style」
    (
        BORDER_STYLE_PROPERTIES,
        "outset",
        "border-*-style 仅支持 none/solid/dashed/dotted",
    ), // 属性速查表、视觉表「border-style」
];

// background-repeat/mask-repeat 的 space / round 被降级为 repeat；will-change 的
// scroll-position 与自定义 ident 不生效。三者都由 skyline/value-degraded 按 warning 报，不在本表。

/// 只接受关键字、写了长度值整条声明就被丢弃的属性 → 建议。
///
/// `vertical-align` 在 fp 的值语法里只有关键字分支
/// （float-pigment-css property.rs:426-433），长度值直接解析失败，整条声明被丢弃
/// （float-pigment-css parser/mod.rs:1768-1781 会回滚该属性）；即便能到引擎，
/// 也只映射 baseline/top/middle/bottom，其余回落到「未设置」。
const LENGTH_VALUED_PROPERTIES: &[(&str, &str)] = &[(
    "vertical-align",
    "Skyline 的 vertical-align 只认 baseline/top/middle/bottom，长度值会让整条声明被丢弃；需要偏移改用 margin/padding 或 line-height",
)];

pub(crate) struct Table {
    properties: &'static [(&'static str, &'static str)],
    values: &'static [(&'static [&'static str], &'static str, &'static str)],
    length_valued: &'static [(&'static str, &'static str)],
}

impl Table {
    pub(crate) fn property(&self, name: &str) -> Option<&'static str> {
        self.properties
            .iter()
            .find(|(property, _)| *property == name)
            .map(|(_, advice)| *advice)
    }

    pub(crate) fn value(&self, name: &str, value: &str) -> Option<&'static str> {
        self.values
            .iter()
            .find(|(names, candidate, _)| *candidate == value && names.contains(&name))
            .map(|(_, _, advice)| *advice)
    }

    /// 属性只接受关键字、而这里写了长度值时返回建议。
    pub(crate) fn length_valued(&self, name: &str, value: &str) -> Option<&'static str> {
        self.length_valued
            .iter()
            .find(|(property, _)| *property == name && is_length(value))
            .map(|(_, advice)| *advice)
    }

    /// 多值形态里逐项判：`overflow: hidden auto`、`border-style: solid double` 这类
    /// 声明整值匹配不到，但其中不支持的项会被引擎按项丢弃（分轴 / 分边展开），
    /// 其余项照常生效。返回「不支持的项 + 各自建议」。
    pub(crate) fn unsupported_items(
        &self,
        name: &str,
        value: &str,
    ) -> Vec<(&'static str, &'static str)> {
        value
            .split_ascii_whitespace()
            .filter_map(|item| {
                self.values
                    .iter()
                    .find(|(names, candidate, _)| *candidate == item && names.contains(&name))
                    .map(|(_, candidate, advice)| (*candidate, *advice))
            })
            .collect()
    }
}

pub(crate) fn table() -> &'static Table {
    static TABLE: Table = Table {
        properties: UNSUPPORTED_PROPERTIES,
        values: UNSUPPORTED_VALUES,
        length_valued: LENGTH_VALUED_PROPERTIES,
    };
    &TABLE
}

/// 值归一化：`trim` + 小写；含运行时变量/插值时返回 `None`（无法静态判定，口径同
/// [`super::value_util::text_is_dynamic`]）。
pub(crate) fn normalize_value(value: &str) -> Option<String> {
    let text = value.trim();
    if text.is_empty() || super::value_util::text_is_dynamic(text) {
        return None;
    }
    Some(text.to_ascii_lowercase())
}

/// 单个长度值：`<number><unit>`、百分比，或无单位零。入参须已 [`normalize_value`]
/// （trim + 小写）。
///
/// 单位取 fp `<length>` 接受的集合（float-pigment-css parser/property_value/mod.rs:150-163）；
/// 无单位零是合法 `<length>`（CSS 只允许零省略单位）。
fn is_length(value: &str) -> bool {
    let Some(end) = super::value_util::number_prefix_end(value) else {
        return false;
    };
    let unit = &value[end..];
    if unit.is_empty() {
        // `0` / `0.0` / `-0` 都是合法 `<length>`。
        return value.bytes().filter(u8::is_ascii_digit).all(|b| b == b'0');
    }
    matches!(
        unit,
        "%" | "px" | "rpx" | "rem" | "em" | "vw" | "vh" | "vmin" | "vmax"
    )
}

#[cfg(test)]
mod tests {
    use super::is_length;

    #[test]
    fn is_length_matches_number_with_unit_or_percentage() {
        for value in [
            "4px", "-2px", "+1.5em", ".5rem", "50%", "4rpx", "2vmin", "0", "0.0",
        ] {
            assert!(is_length(value), "{value}");
        }
        for value in ["4", "04", "4pxx", "px", "auto", "4 px", "4deg", "calc(4px)"] {
            assert!(!is_length(value), "{value}");
        }
    }
}
