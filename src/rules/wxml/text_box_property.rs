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
//! 显式 `line-height` 也是同一类：Skyline 的块级 `<text>` 用自己的行高撑行盒；
//! WebView 的行盒取「父级块的行高」与自身声明的**较大者**，声明更矮时被父级撑大。
//! 所以只有声明值小于父级行盒时两端才有可见差别（同一张截图对比，单位设备 px）。
//! 下面五个案例取自同一次实测：A–D 是四个页面，E 是一个被页面复用的组件。
//!
//! - 页面 A：某段说明文字声明 33rpx，相邻行 pitch 两端 35 / 46；按钮文字声明
//!   40rpx，按钮内字形中心 756.5 / 760.5。
//! - 页面 B：详情文字声明 36rpx，行距 36 / 45。
//! - 页面 C：提示文字声明 36rpx，按钮下沿到字形顶部 39 / 47。
//! - 页面 D：某段文字声明 54rpx，大于父级行盒 45——两端灰色带内的字形偏移都是 16，
//!   完全一致；显式 `display: block` 的另一段文字声明 34rpx，pitch 34.5 / 34。
//! - 组件 E：某元素声明 32rpx，父级 `line-height: 1` 按默认 16px 字号折算也是
//!   32rpx——相等，两端一致。
//!
//! 不报的情况（实测两端一致，报出来是噪声）：
//!
//! - 嵌套在 `text` / `span` 里的 `<text>`：两端都是 inline；
//! - 作为 flex 子项：两端都按 flex item 处理，宽度声明两边都生效；
//! - 自身显式 `display: block` / `inline-block` / `flex` / `grid`：两端都按块级盒排；
//! - 取值等于初始值（`width: auto`、`max-width: none`、间距为 0、`line-height: normal`）；
//! - `line-height` 不小于父级块的行盒（数字 / 百分比按声明处字号折算，`normal` 按实测
//!   1.4 × 字号，差不到 1 CSS px 不算）：此时 WebView 也按声明值排；
//! - 模板定义体与 `<include>` 片段的**根节点**：实例化处的父级不可知，宁可漏报；
//!   片段里更深的元素按图上的实例化路径补全祖先，照常判定。

use crate::diag::{Diagnostic, Severity};
use crate::input::wxml::WxmlElement;
use crate::matcher::Tree;
use crate::project::Project;
use crate::rules::wxml::style::{
    has_exotic_flow, inherited_text_align, inline_box_in_skyline, is_initial_value, is_zero,
};
use crate::rules::wxml::{
    ElementSite, element_diagnostic, flex_container, for_each_element, resolved_property,
    resolved_property_with,
};
use crate::rules::{Rule, RuleMeta, VisualCase, VisualExpect};

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

pub struct TextBoxProperty;

/// 视觉用例：同一段 WXML/WXSS 在两端各渲一次，看 `#target` 的几何与像素。
///
/// 正例复刻真实工程实测：WebView 里 `<text>` 是 inline（宽度只包文字、`margin-top`
/// 不参与布局）；Skyline 里是块级盒（占满父宽、`margin-top: 8px` 生效）。
///
/// 负例有两个：按原方案只删 `margin-top` 的对照实测不成立（两端仍是块级盒 / inline 盒，
/// 宽度差 275 CSS px）；两端都显式 `display: block` 的对照才是有效阴性对照。
static VISUAL_CASES: &[VisualCase] = &[
    VisualCase {
        wxml: r#"<view class="wrap">
  <text class="name" id="target">流浪地球3</text>
  <view class="after">下一块</view>
</view>"#,
        wxss: ".wrap { padding: 20px; }\n.name { display: inline; margin-top: 8px; font-size: 13px; line-height: 18px; }\n.after { background: #eee; }\n",
        js: None,
        expect: VisualExpect::Differ,
        note: "WebView 的 <text> 是 inline（宽度只包文字、margin-top 不生效）；Skyline 是块级盒（占满父宽、margin-top 生效）",
        target: "#target",
    },
    VisualCase {
        wxml: r#"<view class="wrap">
  <text class="name" id="target">流浪地球3</text>
  <view class="after">下一块</view>
</view>"#,
        wxss: ".wrap { padding: 20px; }\n.name { display: inline; font-size: 13px; line-height: 18px; }\n.after { background: #eee; }\n",
        js: None,
        expect: VisualExpect::Same,
        note: "按原方案只删 margin-top：期望一致，但 Skyline 仍是块级盒、WebView 仍是 inline，宽度必然不同（实测不成立）",
        target: "#target",
    },
    // 上面那条按「只删 margin-top」写的阴性对照实测不成立：Skyline 的 `<text>` 是块级盒
    // （宽 335），WebView 是 inline（宽 59.8），宽度本身就不同。这条把两端都显式设成
    // block，才是真正同构的阴性对照：bbox 与像素都应一致。
    VisualCase {
        wxml: r#"<view class="wrap">
  <text class="name" id="target">流浪地球3</text>
  <view class="after">下一块</view>
</view>"#,
        wxss: ".wrap { padding: 20px; }\n.name { display: block; font-size: 13px; line-height: 18px; }\n.after { background: #eee; }\n",
        js: None,
        expect: VisualExpect::Same,
        note: "两端都把 <text> 显式设为 block 且去掉 margin-top：bbox 与像素应一致（有效阴性对照）",
        target: "#target",
    },
];

impl Rule for TextBoxProperty {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Warning,
            description: "顶层 `<text>` / `<span>` 上写了宽高、垂直间距或小于父级行高的 `line-height`：Skyline 按块级盒应用，WebView 的 inline 盒排版不同",
        }
    }

    fn visual_cases(&self) -> &'static [VisualCase] {
        VISUAL_CASES
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
    // 嵌套在文本元素里的 `<text>`：两端都是 inline。
    if site
        .ancestors
        .iter()
        .any(|ancestor| TEXT_CONTAINERS.contains(&ancestor.tag.as_str()))
    {
        return;
    }
    // 被 `<import>` / `<include>` 引入的片段：本文件之外还有实例化处的祖先。
    // 根节点的父级静态不可知（是不是 flex、有没有嵌在 `<text>` 里都看不出来），
    // 只判更深的元素；它们跨文件的祖先用图上的实例化路径补齐。
    if doc.fragment {
        if site.ancestors.is_empty() {
            return;
        }
        let chain = site.graph.ancestors(doc.path(), element);
        // 图上放不下这个元素（动态实例化 / 未渲染）：跨文件的祖先同样不可知。
        if chain.open
            || chain.nodes.is_empty()
            || chain
                .nodes
                .iter()
                .any(|(_, ancestor)| TEXT_CONTAINERS.contains(&ancestor.tag.as_str()))
        {
            return;
        }
    }
    if let Some(parent) = tree.parent_of(element)
        && flex_container(project, tree, parent)
    {
        return;
    }
    if !inline_box_in_skyline(project, tree, element) {
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
    let (line_height, undecidable) = resolved_property_with(project, tree, element, "line-height");
    if undecidable {
        return;
    }
    if let Some(resolved) = line_height
        && meaningful_line_height(&resolved.value)
        && line_height_dominates(project, tree, element, &resolved.value)
    {
        hits.push(format!("`line-height: {}`", resolved.value.trim()));
    }
    if let Some(value) = misaligned_text_align(project, tree, element) {
        hits.push(format!("`text-align: {value}`"));
    }
    if hits.is_empty() {
        return;
    }

    out.push(
        element_diagnostic(
            ID,
            doc,
            element,
            doc.path(),
            Severity::Warning,
            format!(
                "`<{}>` 上的这些声明两端语义不同：Skyline 把它当块级盒，WebView 当 inline 盒",
                element.tag
            ),
        )
        .help(format!("这里见到的是 {}。{HELP}", hits.join("、"))),
    );
}

/// 显式 `line-height` 是否会让两端的行盒不同。
///
/// `normal` 两端都由字体度量决定；`inherit` / `unset` 与零值同样看不出差别。
fn meaningful_line_height(value: &str) -> bool {
    let value = value.trim();
    !matches!(
        value.to_ascii_lowercase().as_str(),
        "normal" | "inherit" | "unset"
    ) && !is_zero(value)
}

/// `line-height` 判定的换算基准：750rpx = 375 CSS px（设计稿惯例）。
const RPX_PER_CSS_PX: f64 = 2.0;
/// `page` 没写 `font-size` 时 WeChat 的默认字号。
const DEFAULT_FONT_SIZE: f64 = 16.0;
/// `normal` 行高：WeChat WebView 默认字体实测 22.4px / 16px = 1.4。
const NORMAL_LINE_HEIGHT: f64 = 1.4;
/// 行盒差不到 1 CSS px 时半行距位移小于 0.5px，肉眼看不出来。
const VISIBLE_LINE_HEIGHT_GAP: f64 = 1.0;

/// 元素声明的 `line-height` 是否比父级块的行盒更矮。
///
/// WebView 的行盒取「父级块的行高」与自身声明的**较大者**：声明不小于父级行高时
/// 两端都按声明值排，没有可见差别；只有更矮时才被父级撑大。实测（同一张截图对比）：
///
/// - 页面 A：某段说明文字声明 33rpx，相邻行
///   pitch 两端 35 / 46 设备 px；按钮文字声明 40rpx，按钮内字形中心 756.5 / 760.5。
/// - 页面 B：详情文字声明 36rpx，行距 36 / 45。
/// - 页面 C：某段提示文字声明 36rpx，按钮下沿到
///   字形顶部 39 / 47（父级行盒 45 没变，字形在 45 的行盒里比在 36 的行盒里低约 4.5）。
/// - 页面 D：某段文字声明 54rpx，大于父级行盒 45——两端字形位置一致
///   （灰色带内偏移 16 / 16），所以声明值不小于父级行高的一律不报。
fn line_height_dominates(
    project: &Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
    declared: &str,
) -> bool {
    let Some(font_size) = font_size_in_css_px(project, tree, element) else {
        return false;
    };
    let Some(declared) = line_height_in_css_px(declared, font_size) else {
        return false;
    };
    let Some(parent) = parent_line_height_in_css_px(project, tree, element) else {
        return false;
    };
    declared + VISIBLE_LINE_HEIGHT_GAP < parent
}

/// `font-size` 的有效值（CSS px）：沿祖先链找第一个声明，缺省 16px。
///
/// 百分比 / 关键字字号要先解析上一级字号，静态算不准就不判（`None`）。
/// 片段 / 模板定义体的祖先链在实例化处被截断：到顶都没找到声明时不能用默认值猜。
fn font_size_in_css_px(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> Option<f64> {
    let mut current = Some(element);
    while let Some(node) = current {
        if let Some(resolved) = resolved_property(project, tree, node, "font-size") {
            let value = resolved.value.trim().to_ascii_lowercase();
            if !matches!(value.as_str(), "inherit" | "unset" | "revert") {
                return length_in_css_px(&value);
            }
        }
        current = tree.parent_of(node);
    }
    if tree.ancestors_unknown(element) {
        return None;
    }
    Some(DEFAULT_FONT_SIZE)
}

/// 父级块的行盒高度（CSS px）。
///
/// 从直接父级向上找第一个 `line-height` 声明：数字 / 百分比按**声明处**的字号折算，
/// `normal` 按默认字体度量折算；整条链都没写就是父级字号的 `normal`。
fn parent_line_height_in_css_px(
    project: &Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
) -> Option<f64> {
    let Some(parent) = tree.parent_of(element) else {
        // 直属于页面根：行盒来自 `page` 的默认字号。
        return Some(NORMAL_LINE_HEIGHT * DEFAULT_FONT_SIZE);
    };
    let mut current = Some(parent);
    while let Some(node) = current {
        if let Some(resolved) = resolved_property(project, tree, node, "line-height") {
            let value = resolved.value.trim().to_ascii_lowercase();
            match value.as_str() {
                // 继承逐级向下传递的是计算值：接着往上找真正的声明处。
                "inherit" | "unset" | "revert" => {}
                "normal" => {
                    return Some(NORMAL_LINE_HEIGHT * font_size_in_css_px(project, tree, node)?);
                }
                _ => {
                    return line_height_in_css_px(
                        &value,
                        font_size_in_css_px(project, tree, node)?,
                    );
                }
            }
        }
        current = tree.parent_of(node);
    }
    if tree.ancestors_unknown(element) {
        // 片段 / 模板：实例化处的祖先可能在调用处写了行高，不拿默认值猜。
        return None;
    }
    Some(NORMAL_LINE_HEIGHT * font_size_in_css_px(project, tree, parent)?)
}

/// `px` / `rpx` 长度 → CSS px。其它单位算不准，返回 `None`。
fn length_in_css_px(value: &str) -> Option<f64> {
    let (number, unit) = split_length(value)?;
    match unit.as_str() {
        "px" => Some(number),
        "rpx" => Some(number / RPX_PER_CSS_PX),
        _ => None,
    }
}

/// `line-height` 取值 → CSS px：长度直接换算，数字 / 百分比按元素字号折算。
fn line_height_in_css_px(value: &str, font_size: f64) -> Option<f64> {
    let value = value.trim();
    let lower = value.to_ascii_lowercase();
    if matches!(lower.as_str(), "normal" | "inherit" | "unset") {
        return None;
    }
    if let Some(percentage) = lower.strip_suffix('%') {
        return percentage
            .trim()
            .parse::<f64>()
            .ok()
            .map(|value| value / 100.0 * font_size);
    }
    let (number, unit) = split_length(value)?;
    match unit.as_str() {
        "px" => Some(number),
        "rpx" => Some(number / RPX_PER_CSS_PX),
        "" | "em" => Some(number * font_size),
        _ => None,
    }
}

/// 拆「数字 + 单位」（小写）。
fn split_length(value: &str) -> Option<(f64, String)> {
    let value = value.trim();
    let end = value
        .find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+')))
        .unwrap_or(value.len());
    let number = value[..end].parse::<f64>().ok()?;
    Some((number, value[end..].trim().to_ascii_lowercase()))
}

/// 取值是不是「写了就会改变渲染」。
fn is_meaningful(property: &str, value: &str) -> bool {
    if is_initial_value(value) {
        return false;
    }
    match property {
        // 垂直间距为 0 时两端都看不出变化。
        "margin-top" | "margin-bottom" | "padding-top" | "padding-bottom" => !is_zero(value),
        _ => true,
    }
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

    /// 被 `<import>` 引入的模板片段，样式由引用它的页面 `@import` 进来。
    fn template_count(name: &str, page_wxml: &str, template_wxml: &str, wxss: &str) -> usize {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", page_wxml)
            .write("pages/index/index.wxss", r#"@import "/tpls/list.wxss";"#)
            .write("tpls/list.wxml", template_wxml)
            .write("tpls/list.wxss", wxss);
        check_rule(&fixture, &TextBoxProperty)
            .iter()
            .filter(|diagnostic| diagnostic.rule == ID)
            .count()
    }

    /// 真实漏报：某模板文件里 `<text class="name">` 的后代选择器声明写在
    /// 片段自己的 WXSS 里，片段整体被跳过时一条都不报。
    #[test]
    fn descendant_selector_in_imported_template_is_reported() {
        assert_eq!(
            template_count(
                "text-box-template-descendant",
                r#"<import src="/tpls/list.wxml"/><view class="list"><template is="list"/></view>"#,
                r#"<template name="list"><view class="movie-item"><text class="name mt-text-ellipsis">{{ nm }}</text></view></template>"#,
                ".movie-item .name { margin-top: 8px; }",
            ),
            1
        );
    }

    /// 片段根节点的父级在实例化处，是不是 flex 子项判不了，宁可漏报。
    #[test]
    fn fragment_root_is_skipped() {
        assert_eq!(
            template_count(
                "text-box-fragment-root",
                r#"<import src="/tpls/list.wxml"/><view class="list"><template is="list"/></view>"#,
                r#"<template name="list"><text class="name">说明文字</text></template>"#,
                ".name { margin-top: 8px; }",
            ),
            0
        );
    }

    /// 模板被实例化在外层 `<text>` 里：片段里的 `<text>` 两端都成了 inline。
    #[test]
    fn template_instantiated_inside_text_is_skipped() {
        assert_eq!(
            template_count(
                "text-box-template-in-text",
                r#"<import src="/tpls/list.wxml"/><text class="outer"><template is="list"/></text>"#,
                r#"<template name="list"><view class="movie-item"><text class="name">说明文字</text></view></template>"#,
                ".movie-item .name { margin-top: 8px; }",
            ),
            0
        );
    }

    /// 片段的父级行盒可能在实例化处（调用方样式表）声明，静态判不了就不报行高。
    #[test]
    fn line_height_in_fragment_without_parent_declaration_is_skipped() {
        assert_eq!(
            template_count(
                "text-box-template-line-height",
                r#"<import src="/tpls/list.wxml"/><view class="list"><template is="list"/></view>"#,
                r#"<template name="list"><view class="item"><text class="t">说明文字</text></view></template>"#,
                ".item .t { font-size: 24rpx; line-height: 36rpx; }",
            ),
            0
        );
    }

    /// 真实漏报：某页面里 `<text class="text-detail">` 写了 `line-height: 36rpx`。
    /// 直接父级是普通 `<view>`，外层 flex 容器不算 flex 子项。
    #[test]
    fn explicit_line_height_is_reported() {
        assert_eq!(
            count(
                r#"<view class="setting-item"><view><text class="text-detail">关闭后无法收到个性化内容推荐</text></view></view>"#,
                ".setting-item { display: flex; } .text-detail { font-size: 24rpx; line-height: 36rpx; }"
            ),
            1
        );
    }

    #[test]
    fn normal_line_height_is_skipped() {
        assert_eq!(
            count(
                r#"<view><text class="t">说明文字</text></view>"#,
                ".t { line-height: normal; }"
            ),
            0
        );
    }

    #[test]
    fn zero_line_height_is_skipped() {
        assert_eq!(
            count(
                r#"<view><text class="t">说明文字</text></view>"#,
                ".t { line-height: 0rpx; }"
            ),
            0
        );
    }

    #[test]
    fn line_height_on_block_text_is_skipped() {
        assert_eq!(
            count(
                r#"<view><text class="t">说明文字</text></view>"#,
                ".t { display: block; line-height: 36rpx; }"
            ),
            0
        );
    }

    #[test]
    fn line_height_on_nested_text_is_skipped() {
        assert_eq!(
            count(
                r#"<view><text class="outer">外层<text class="t">内层</text></text></view>"#,
                ".t { line-height: 36rpx; }"
            ),
            0
        );
    }

    /// 同一个元素的垂直间距与行高合成一条诊断，不重复报。
    #[test]
    fn line_height_and_box_property_report_once() {
        assert_eq!(
            count(
                r#"<view><text class="t">说明文字</text></view>"#,
                ".t { margin-top: 8px; line-height: 36rpx; }"
            ),
            1
        );
    }

    /// 实测（页面 D）：声明 54rpx 大于父级行盒，两端字形位置一致。
    #[test]
    fn line_height_not_below_parent_line_box_is_skipped() {
        assert_eq!(
            count(
                r#"<view><text class="t">说明文字</text></view>"#,
                ".t { line-height: 54rpx; }"
            ),
            0
        );
    }

    /// 父级也声明了同样的行高：WebView 的行盒就是这个值，两端一致。
    #[test]
    fn line_height_equal_to_parent_is_skipped() {
        assert_eq!(
            count(
                r#"<view class="p"><text class="t">说明文字</text></view>"#,
                ".p { line-height: 24rpx; } .t { line-height: 24rpx; }"
            ),
            0
        );
    }

    /// 父级行高更矮：WebView 会被声明值撑大，两端都按声明排。
    #[test]
    fn line_height_above_parent_is_skipped() {
        assert_eq!(
            count(
                r#"<view class="p"><text class="t">说明文字</text></view>"#,
                ".p { line-height: 28rpx; } .t { line-height: 46rpx; }"
            ),
            0
        );
    }

    /// `line-height: 1` 的父级（16px 行盒）与 32rpx（16px）相等：组件 E 属于这种。
    #[test]
    fn unitless_parent_line_height_equal_to_declaration_is_skipped() {
        assert_eq!(
            count(
                r#"<view class="p"><text class="t">说明文字</text></view>"#,
                ".p { line-height: 1; } .t { font-size: 22rpx; line-height: 32rpx; }"
            ),
            0
        );
    }

    /// 页面 B 的真实结构：父级链上有 `line-height: inherit`，要落到默认 normal。
    #[test]
    fn line_height_below_inherited_parent_is_reported() {
        assert_eq!(
            count(
                r#"<view class="setting-page"><view class="setting-list"><view class="form-view"><view class="setting-item"><view><text class="text-detail">关闭后无法收到个性化内容推荐</text></view></view></view></view></view>"#,
                ".setting-item { display: flex; line-height: inherit; } .text-detail { font-size: 24rpx; line-height: 36rpx; }"
            ),
            1
        );
    }

    /// 无单位 / 百分比行高按元素字号折算后仍小于父级行盒时照报。
    #[test]
    fn unitless_line_height_below_parent_is_reported() {
        assert_eq!(
            count(
                r#"<view><text class="t">说明文字</text></view>"#,
                ".t { font-size: 14px; line-height: 1.4; }"
            ),
            1
        );
    }
}
