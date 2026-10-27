//! 属性 / 值支持表（两条 dict 规则共用）。
//!
//! 数据直接以 const 形式定义在本模块，来源与证据见各条目的行尾注释。
//!
//! 来源：/Volumes/2tb-ssd/nebula/Work/skyline-skills/skills/skyline-wxss/SKILL.md
//!       /Volumes/2tb-ssd/nebula/Work/skyline-skills/skills/skyline-wxss/references/*.md
//! 同步方式：
//!   [`UNSUPPORTED_PROPERTIES`] 按 SKILL.md「NEVER: 使用不支持的 CSS 属性」表逐条更新，
//!     一格多属性拆成多条；表外补充的属性以行尾注释标注 references 出处。
//!   [`UNSUPPORTED_VALUES`] 按 references 中各属性的「支持值」表与 ⚠️ 标注更新，
//!     只收录文档明确写「不支持 / 不可用 / 映射为其他值」的整值，不从语法推断。
//!   [`LENGTH_VALUED_PROPERTIES`] 收录「只接受关键字、写了长度值整条声明就会被丢弃」的属性。

/// Skyline 完全不支持的属性 → 建议。
///
/// 匹配：属性名小写比较。
const UNSUPPORTED_PROPERTIES: &[(&str, &str)] = &[
    ("float", "使用 flex 布局"),                         // SKILL.md NEVER 表
    ("contain", "无替代（`-wx-contain` 同样不被接受）"), // SKILL.md NEVER 表；css_parser_impl.h:172 WxContain 亦为 kInvalid
    ("resize", "无替代"),                                // SKILL.md NEVER 表
    ("writing-mode", "无替代"),                          // SKILL.md NEVER 表
    ("text-indent", "使用 padding-left 模拟"),           // SKILL.md NEVER 表
    ("overflow-wrap", "使用 word-break"),                // SKILL.md NEVER 表
    ("text-decoration-thickness", "无替代"),             // SKILL.md NEVER 表
    ("background-attachment", "无替代"),                 // SKILL.md NEVER 表
    ("background-clip", "无替代"),                       // SKILL.md NEVER 表
    ("background-origin", "无替代"),                     // SKILL.md NEVER 表
    ("mask-origin", "无替代"),                           // SKILL.md NEVER 表（一格多属性拆分）
    ("mask-clip", "无替代"),                             // SKILL.md NEVER 表（一格多属性拆分）
    ("mask-mode", "无替代"),                             // SKILL.md NEVER 表（一格多属性拆分）
    ("justify-items", "无替代"),                         // SKILL.md NEVER 表
    ("list-style-type", "无替代"),                       // SKILL.md NEVER 表（一格多属性拆分）
    ("list-style-image", "无替代"),                      // SKILL.md NEVER 表（一格多属性拆分）
    ("list-style-position", "无替代"),                   // SKILL.md NEVER 表（一格多属性拆分）
    ("text-transform", "改用 WXML 里预先转换好的文本"), // css_property_names.h 无该属性 id，声明整条丢弃
    (
        "user-select",
        "改用 `<text user-select=\"{{false}}\">` 属性",
    ), // css_property_names.h 无该属性 id
    (
        "-webkit-line-clamp",
        "多行省略改用 `<text max-lines=\"\">`（引擎把该属性映射成 kInvalid，css_parser_impl.h:47）",
    ),
    // 标准名同样不是属性名：fp 只有私有 `_wx_line_clamp`（property.rs:34），映射 kInvalid。
    (
        "line-clamp",
        "多行省略改用 `<text max-lines=\"\">`（引擎只认私有 `_wx_line_clamp`）",
    ),
    // 老式 `-webkit-box` 弹性盒（display: -webkit-box 与 -webkit-box-*）按规则块合并，
    // 归 `skyline/legacy-flexbox` 报，避免同一块布局被拆成三、四条 error。
    // 引擎里没有这两个属性（css/、style/ 零命中），图片裁剪用 <image mode>。
    (
        "object-fit",
        "图片裁剪改用 `<image mode=\"aspectFill\">` 等属性",
    ),
    ("object-position", "图片裁剪改用 `<image mode>`"),
];

// 已移除 direction / overflow-x / overflow-y：引擎实际接受（css_parser_impl.h:136 映射 kDirection，
//   direction.cc 实现 ltr/rtl 并由 style_builder.cc:1212 应用到 Text；css_parser_impl.h:40-41 映射
//   kOverflowX/kOverflowY，overflow_x.cc/overflow_y.cc 的 Parse 把值折叠进 kOverflow，
//   overflow.cc:20-37 应用 visible/hidden）。文档「不支持」的原文只对 auto/scroll 与「分轴独立设置」成立。

/// border-*-style 系列共用同一套支持值。
const BORDER_STYLE_PROPERTIES: &[&str] = &[
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
    (&["display"], "grid", "改用 flex 布局替代 grid"), // layout.md「display」、SKILL.md MUST 示例
    (&["display"], "flow-root", "改用 flex 或 block"), // layout.md「display」
    (&["display"], "inline-grid", "改用 inline-flex"), // display.cc 无 InlineGrid case（property.rs:213 可解析）
    (
        &["position"],
        "sticky",
        "改用 relative/absolute/fixed；吸顶使用 sticky-header/sticky-section 组件",
    ), // layout.md「position」、SKILL.md MUST 示例
    (
        &["overflow"],
        "auto",
        "改用 overflow: hidden；滚动使用 scroll-view 组件",
    ), // layout.md「overflow」、SKILL.md MUST 示例
    (
        &["overflow"],
        "scroll",
        "改用 overflow: hidden；滚动使用 scroll-view 组件",
    ), // layout.md「overflow」
    // overflow-x/overflow-y 的属性名本身是支持的（visible/hidden 会折叠成 overflow），只有这两个值被拒
    // （overflow_x.cc:33 / overflow_y.cc 同构）。属性条目因此不收录，值按分轴属性分别收录。
    (
        OVERFLOW_AXIS_PROPERTIES,
        "auto",
        "改用 overflow: hidden；滚动使用 scroll-view 组件",
    ), // overflow_x.cc:33 拒绝
    (
        OVERFLOW_AXIS_PROPERTIES,
        "scroll",
        "改用 overflow: hidden；滚动使用 scroll-view 组件",
    ), // overflow_x.cc:33 拒绝
    (&["visibility"], "collapse", "改用 visible 或 hidden"), // layout.md「visibility」
    (&["white-space"], "pre", "改用 normal 或 nowrap"), // text.md「white-space」、SKILL.md Quick Reference
    (&["white-space"], "pre-wrap", "改用 normal 或 nowrap"), // text.md「white-space」、SKILL.md Quick Reference
    (&["white-space"], "pre-line", "改用 normal 或 nowrap"), // text.md「white-space」、SKILL.md Quick Reference
    (
        &["font-weight"],
        "bolder",
        "改用 normal、bold 或 100-900 的数值",
    ), // text.md「font-weight」
    (
        &["font-weight"],
        "lighter",
        "改用 normal、bold 或 100-900 的数值",
    ), // text.md「font-weight」
    (&["font-style"], "oblique", "改用 normal 或 italic"),   // text.md「font-style」
    // word-break: break-word 不是无效值（被映射为 normal），由 skyline/value-degraded 按 warning 报。
    (
        &["vertical-align"],
        "text-top",
        "改用 baseline/top/middle/bottom",
    ), // text.md「vertical-align」
    (
        &["vertical-align"],
        "text-bottom",
        "改用 baseline/top/middle/bottom",
    ), // text.md「vertical-align」
    (
        &["text-align"],
        "justify-all",
        "改用 left/center/right/justify/start/end",
    ), // text.md「text-align」
    (
        &["text-align"],
        "match-parent",
        "改用 left/center/right/justify/start/end",
    ), // text.md「text-align」
    (
        &["text-decoration-line"],
        "blink",
        "仅支持 none/underline/overline/line-through，且只能单值",
    ), // SKILL.md Quick Reference
    (
        BORDER_STYLE_PROPERTIES,
        "double",
        "border-*-style 仅支持 none/solid/dashed/dotted",
    ), // SKILL.md Quick Reference、visual.md「border-style」
    (
        BORDER_STYLE_PROPERTIES,
        "groove",
        "border-*-style 仅支持 none/solid/dashed/dotted",
    ), // SKILL.md Quick Reference、visual.md「border-style」
    (
        BORDER_STYLE_PROPERTIES,
        "ridge",
        "border-*-style 仅支持 none/solid/dashed/dotted",
    ), // SKILL.md Quick Reference、visual.md「border-style」
    (
        BORDER_STYLE_PROPERTIES,
        "inset",
        "border-*-style 仅支持 none/solid/dashed/dotted",
    ), // SKILL.md Quick Reference、visual.md「border-style」
    (
        BORDER_STYLE_PROPERTIES,
        "outset",
        "border-*-style 仅支持 none/solid/dashed/dotted",
    ), // SKILL.md Quick Reference、visual.md「border-style」
];

// background-repeat/mask-repeat 的 space / round 被降级为 repeat；will-change 的
// scroll-position 与自定义 ident 不生效。三者都由 skyline/value-degraded 按 warning 报，不在本表。

/// 只接受关键字、写了长度值整条声明就被丢弃的属性 → 建议。
///
/// `vertical-align` 在 fp 的值语法里只有关键字分支
/// （float-pigment-css property.rs:426-433），长度值直接解析失败，整条声明被丢弃
/// （float-pigment-css parser/mod.rs:1768-1781 会回滚该属性）；即便能到引擎，
/// ParseValue 也只映射 baseline/top/middle/bottom，其余走 default → `CSSUnsetValue`
/// （skyline/css/css_properties/vertical_align.cc:12-43）。
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

/// 单个长度值：`<number><unit>` 或百分比。入参须已 [`normalize_value`]（trim + 小写）。
///
/// 单位取 fp `<length>` 接受的集合（float-pigment-css parser/property_value/mod.rs:150-163）。
fn is_length(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut i = usize::from(matches!(bytes.first(), Some(b'+') | Some(b'-')));
    let mut seen_digit = false;
    while i < bytes.len() {
        match bytes[i] {
            b'0'..=b'9' => {
                seen_digit = true;
                i += 1;
            }
            b'.' => i += 1,
            _ => break,
        }
    }
    seen_digit
        && matches!(
            &value[i..],
            "%" | "px" | "rpx" | "rem" | "em" | "vw" | "vh" | "vmin" | "vmax"
        )
}

#[cfg(test)]
mod tests {
    use super::is_length;

    #[test]
    fn is_length_matches_number_with_unit_or_percentage() {
        for value in ["4px", "-2px", "+1.5em", ".5rem", "50%", "4rpx", "2vmin"] {
            assert!(is_length(value), "{value}");
        }
        for value in ["0", "4", "4pxx", "px", "auto", "4 px", "4deg", "calc(4px)"] {
            assert!(!is_length(value), "{value}");
        }
    }
}
