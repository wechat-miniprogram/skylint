//! `skyline/text-box-property`：顶层 `<text>` 上的盒属性两端语义不同。
//!
//! 实测（同一探针页两端各跑一次），`<text>` 与 `<span>` 行为一致：
//!
//! - Skyline：`<text>` 在普通流里是**块级盒**——独占一行，`width` / `height` /
//!   `margin-top` 都生效；它内部的文字与嵌套 `<text>` 才是 inline。
//! - WebView：`<text>` 是 **inline**——宽度由内容决定，`width` / `height` /
//!   垂直 `margin` / 垂直 `padding` 都不参与布局。
//!
//! 所以顶层 `<text>` / `<span>` 写了这些属性时两端必然不同：Skyline 按声明值折行、
//! 截断、撑高，WebView 完全忽略。典型例子是给说明文字写 `width: 492rpx`——Skyline 的
//! 可用宽度比 WebView 窄，于是多折一行。
//!
//! 不报的情况（实测两端一致，报出来是噪声）：
//!
//! - 嵌套在 `text` / `span` 里的 `<text>`：两端都是 inline；//! - 作为 flex 子项：两端都按 flex item 处理，宽度声明两边都生效；
//! - 自身显式 `display: block` / `inline-block` / `flex` / `grid`：两端都按块级盒排；
//! - 取值等于初始值（`width: auto`、`max-width: none`、间距为 0）；
//! - 模板定义体与 `<include>` 片段：祖先链不全，判不出是否嵌套，宁可漏报。

use crate::diag::{Diagnostic, Severity};
use crate::input::wxml::WxmlElement;
use crate::matcher::Tree;
use crate::project::Project;
use crate::rules::wxml::{
    ElementSite, element_diagnostic, flex_container, for_each_element, resolved_property_with,
};
use crate::rules::{Rule, RuleMeta};

pub const ID: &str = "skyline/text-box-property";

const HELP: &str = "把这些属性挪到外层 `<view>`；或给文本元素显式 `display: block`（两端一致）；只是想让文字与相邻元素同排时，改用 flex 布局";

/// 文本节点容器：两端都把元素当成 inline 盒子，顶层的盒属性在两端语义不同。
const TEXT_CONTAINERS: [&str; 2] = ["text", "span"];

/// 只在 Skyline 里生效、WebView 里被忽略的盒属性。
const BOX_PROPERTIES: [&str; 10] = [
    "width",
    "min-width",
    "max-width",
    "height",
    "min-height",
    "max-height",
    "margin-top",
    "margin-bottom",
    "padding-top",
    "padding-bottom",
];

/// 显式成块（或非行内）的 `display` 取值：两端都按块级盒排，不报。
const NON_INLINE_DISPLAY: [&str; 8] = [
    "block",
    "inline-block",
    "flex",
    "inline-flex",
    "grid",
    "inline-grid",
    "flow-root",
    "none",
];

pub struct TextBoxProperty;

impl Rule for TextBoxProperty {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Warning,
            description: "顶层 `<text>` / `<span>` 上写了宽高或垂直间距：Skyline 会按块级盒应用，WebView 把这些声明当 inline 忽略",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        for_each_element(project, self.target(), |site| {
            check_element(site, &mut out);
        });
        out
    }
}

fn check_element(site: &ElementSite<'_, '_>, out: &mut Vec<Diagnostic>) {
    let (project, doc, tree, element) = (site.project, site.doc, site.tree, site.element);
    if !TEXT_CONTAINERS.contains(&element.tag.as_str()) {
        return;
    }
    // 片段与模板定义体的祖先链不全：判不出是否嵌套、父级是不是 flex。
    if doc.fragment {
        return;
    }
    if site
        .ancestors
        .iter()
        .any(|ancestor| TEXT_CONTAINERS.contains(&ancestor.tag.as_str()))
    {
        return;
    }
    if let Some(parent) = tree.parent_of(element)
        && flex_container(project, tree, parent)
    {
        return;
    }
    if !inline_in_skyline(project, tree, element) {
        return;
    }

    let mut hits: Vec<String> = Vec::new();
    for property in BOX_PROPERTIES {
        let (resolved, undecidable) = resolved_property_with(project, tree, element, property);
        if undecidable {
            return;
        }
        if let Some(resolved) = resolved
            && is_meaningful(property, &resolved.value)
        {
            hits.push(format!("`{property}: {}`", resolved.value.trim()));
        }
    }
    if let Some(value) = misaligned_text_align(project, tree, element) {
        hits.push(format!("`text-align: {value}`"));
    }
    if hits.is_empty() {
        return;
    }

    out.push(
        element_diagnostic(ID, doc, element, doc.path(), Severity::Warning, format!("`<{}>` 上的这些盒属性只在 Skyline 生效、WebView 会忽略：Skyline 把它当块级盒，WebView 当 inline", element.tag))
            .help(format!("这里见到的是 {}。{HELP}", hits.join("、"))),
    );
}

/// 元素在 Skyline 下是否仍是行内模式的 `<text>`（没有显式改成块级）。
///
/// `display: inline` 不算例外：Skyline 会忽略它，`<text>` 依旧是块级盒。
fn inline_in_skyline(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> bool {
    match resolved_property_with(project, tree, element, "display") {
        (_, true) => false,
        (Some(resolved), false) => {
            !NON_INLINE_DISPLAY.contains(&resolved.value.trim().to_ascii_lowercase().as_str())
        }
        (None, false) => true,
    }
}

/// 取值是不是「写了就会改变渲染」。
fn is_meaningful(property: &str, value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    let initial = matches!(
        value.as_str(),
        "" | "auto" | "none" | "initial" | "unset" | "inherit" | "revert"
    );
    if initial {
        return false;
    }
    match property {
        // 垂直间距为 0 时两端都看不出变化。
        "margin-top" | "margin-bottom" | "padding-top" | "padding-bottom" => !is_zero(&value),
        _ => true,
    }
}

fn is_zero(value: &str) -> bool {
    let number: String = value
        .chars()
        .take_while(|c| c.is_ascii_digit() || matches!(c, '.' | '-'))
        .collect();
    let unit = value[number.len()..].trim();
    let known_unit = matches!(unit, "" | "px" | "rpx" | "%" | "em" | "rem" | "vw" | "vh");
    known_unit && !number.is_empty() && number.chars().all(|c| matches!(c, '0' | '.' | '-'))
}

/// `text-align` 只在 Skyline 的块级 `<text>` 里生效；WebView 用最近块级祖先的对齐。
/// 只有与该祖先的有效对齐不一致时才真的看得出差别。
fn misaligned_text_align(
    project: &Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
) -> Option<String> {
    // RTL 与竖排下 `text-align` 的值到物理方向的映射两端不一定一致，没实测过就不判。
    if has_exotic_flow(project, tree, element) {
        return None;
    }
    let (resolved, undecidable) = resolved_property_with(project, tree, element, "text-align");
    if undecidable {
        return None;
    }
    let declared = resolved?.value.trim().to_ascii_lowercase();
    if !matches!(declared.as_str(), "center" | "right" | "end" | "justify") {
        return None;
    }
    (declared != inherited_text_align(project, tree, element)).then_some(declared)
}

/// 元素或祖先声明了 RTL / 竖排：`text-align` 的 `left` / `right` / `start` 在两端到物理方向
/// 的映射未验证，不判。
fn has_exotic_flow(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> bool {
    let mut current = Some(element);
    while let Some(node) = current {
        let exotic = |property: &str| match resolved_property_with(project, tree, node, property) {
            (Some(resolved), false) => {
                let value = resolved.value.trim().to_ascii_lowercase();
                match property {
                    "direction" => value == "rtl",
                    _ => !matches!(value.as_str(), "horizontal-tb" | "initial" | "normal"),
                }
            }
            _ => false,
        };
        if exotic("direction") || exotic("writing-mode") {
            return true;
        }
        current = tree.parent_of(node);
    }
    false
}

/// 沿祖先链找最近一条 `text-align`——`text-align` 是可继承属性，WebView 下行内盒子用的
/// 就是它。默认 `left`。
fn inherited_text_align(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> String {
    let mut current = tree.parent_of(element);
    while let Some(node) = current {
        match resolved_property_with(project, tree, node, "text-align") {
            (Some(resolved), false) => return resolved.value.trim().to_ascii_lowercase(),
            (_, true) => return "left".to_string(),
            (None, false) => current = tree.parent_of(node),
        }
    }
    "left".to_string()
}

#[cfg(test)]
mod tests {
    use super::{ID, TextBoxProperty};
    use crate::rules::testutil::wxml::{Fixture, check_rule};

    fn count(wxml: &str, wxss: &str) -> usize {
        let fixture = Fixture::page("text-box", wxml, wxss);
        check_rule(&fixture, &TextBoxProperty)
            .iter()
            .filter(|diagnostic| diagnostic.rule == ID)
            .count()
    }

    #[test]
    fn width_on_text_is_reported() {
        assert_eq!(
            count(
                r#"<view><text class="t">说明文字</text></view>"#,
                ".t { width: 492rpx; font-size: 24rpx; }"
            ),
            1
        );
    }

    #[test]
    fn text_without_box_properties_is_skipped() {
        assert_eq!(
            count(
                r#"<view><text class="t">说明文字</text></view>"#,
                ".t { font-size: 24rpx; color: #666; }"
            ),
            0
        );
    }

    #[test]
    fn auto_width_is_skipped() {
        assert_eq!(
            count(
                r#"<view><text class="t">说明文字</text></view>"#,
                ".t { width: auto; }"
            ),
            0
        );
    }

    #[test]
    fn zero_vertical_spacing_is_skipped() {
        assert_eq!(
            count(
                r#"<view><text class="t">说明文字</text></view>"#,
                ".t { margin-top: 0; padding-bottom: 0rpx; }"
            ),
            0
        );
    }

    #[test]
    fn vertical_spacing_is_reported() {
        assert_eq!(
            count(
                r#"<view><text class="t">说明文字</text></view>"#,
                ".t { margin-top: 20rpx; }"
            ),
            1
        );
    }

    #[test]
    fn flex_child_is_skipped() {
        assert_eq!(
            count(
                r#"<view class="row"><text class="t">说明文字</text></view>"#,
                ".row { display: flex; } .t { width: 200rpx; }"
            ),
            0
        );
    }

    #[test]
    fn nested_text_is_skipped() {
        assert_eq!(
            count(
                r#"<view><text class="outer">外层<text class="t">内层</text></text></view>"#,
                ".t { width: 200rpx; margin-top: 20rpx; }"
            ),
            0
        );
    }

    #[test]
    fn explicit_block_is_skipped() {
        assert_eq!(
            count(
                r#"<view><text class="t">说明文字</text></view>"#,
                ".t { display: block; width: 200rpx; }"
            ),
            0
        );
    }

    #[test]
    fn span_box_property_is_reported() {
        assert_eq!(
            count(
                r#"<view><span class="t">说明文字</span></view>"#,
                ".t { width: 200rpx; }"
            ),
            1
        );
    }

    #[test]
    fn nested_span_is_skipped() {
        assert_eq!(
            count(
                r#"<view><span class="outer">外层<span class="t">内层</span></span></view>"#,
                ".t { width: 80px; }"
            ),
            0
        );
    }

    #[test]
    fn rtl_context_skips_text_align_but_not_box_properties() {
        assert_eq!(
            count(
                r#"<view class="row"><text class="t">说明文字</text></view>"#,
                ".row { direction: rtl; } .t { text-align: center; }"
            ),
            0
        );
        assert_eq!(
            count(
                r#"<view class="row"><text class="t">说明文字</text></view>"#,
                ".row { direction: rtl; } .t { width: 200rpx; }"
            ),
            1
        );
    }

    #[test]
    fn vertical_writing_mode_skips_text_align() {
        assert_eq!(
            count(
                r#"<view class="row"><text class="t">说明文字</text></view>"#,
                ".row { writing-mode: vertical-rl; } .t { text-align: center; }"
            ),
            0
        );
    }

    #[test]
    fn text_align_center_is_reported() {
        assert_eq!(
            count(
                r#"<view class="row"><text class="t">说明文字</text></view>"#,
                ".row { text-align: left; } .t { text-align: center; }"
            ),
            1
        );
    }

    #[test]
    fn text_align_matching_parent_is_skipped() {
        assert_eq!(
            count(
                r#"<view class="row"><text class="t">说明文字</text></view>"#,
                ".row { text-align: center; } .t { text-align: center; }"
            ),
            0
        );
    }
}
