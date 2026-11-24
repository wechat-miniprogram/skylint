//! Skyline 没有**元素级行内格式化上下文（IFC）**：WebView 会把裸文本与行内元素
//! 排进同一个行盒，Skyline 只有 flex / block 容器与 TextShadowNode 内部的行内流。
//! 下面三种写法都依赖前者，迁到 Skyline 后会静默变成块级堆叠：
//!
//! 1. **裸文本与元素混排**：WebView 把裸文本排进容器行内流（`文案<image/>` 同一行），
//!    Skyline 的裸文本是独立块级项，元素另起一行；
//! 2. **非文本元素静态 `display: inline` / `inline-block`**：WebView 按行内级盒排
//!    （收缩到内容宽、可与兄弟并排），Skyline 忽略声明、仍按块级盒排；
//! 3. **≥2 个没写 `display` 的文本级元素并列**：WebView 下 `<text>` 是 inline、
//!    `<image>` / `<span>` 是 inline-block，兄弟共享一个行内流；Skyline 下各自占一行。
//!
//! `display: inline-block` 的口径（实测，同页两端各跑一次）：WebView 真的按行内块排——
//! 盒子收缩到内容宽、与兄弟同排；Skyline 把它当块级，盒子撑满父宽、独占一行。
//! 所以它和 `inline` 一样必须覆盖；「两端都按块级」是错误口径。
//!
//! 根因相同，修法也只有一条：把要并排的内容放进 `display: flex; flex-direction: row`
//! 的容器；如果本来就要各占一行，就给两端都显式写 `display: block`，让两端一致。
//! **不要建议「包进 `<text>`」**：Skyline 里 `<text>` 作为 `<view>` 的子项仍是块级项，
//! 包完还是各自换行，只是把形态 1 变成了形态 3。
//!
//! 想要「一段共享排版的文本流」只能让 TextShadowNode 当最近容器：文本子节点变
//! `TextSpan`、非文本子节点变 `WidgetSpan`，全部 `BuildInlineSpan` 进同一个段落
//! （引擎按节点类型分派、文本与 inline 元素各自成段）。
//!
//! 不报的情况：父级是 flex（两端都按 flex item 排）；元素已显式写非行内的
//! `display`（形态 3 的成员）；脱离文档流；作者已用固定宽度 / 纵向 margin 表达
//! 「各占一行」；成员分属互斥 `wx:if` 分支。
//!
//! 模板定义体与 `<include>` 片段同样适用：样式按引用它们的单元求值，祖先链按
//! **真实实例化路径**匹配（[`super::cascade_entries`] 是唯一口径），所以模板里
//! 「两串 `<text>` 靠默认布局并排」与普通页面是同一个现象，只报一次。判不了时
//! （一条实例化路径都没有，或选择器超出受限文法）按「不知道」处理：不报。

use crate::diag::{Diagnostic, Severity};
use crate::input::wxml::{WxmlDoc, WxmlElement, WxmlNode};
use crate::matcher::Tree;
use crate::project::Project;
use crate::rules::{Rule, RuleMeta, VisualCase, VisualExpect};
use crate::wxml_graph::SlotHost;

use super::branch::{exclusive, exclusive_branches};
use super::structure::attr_is_off;
use super::{
    ElementSite, cascade_entries_with, element_diagnostic, flex_container, for_each_element,
    is_flex_value, resolved_property, resolved_property_with, span_diagnostic,
};

pub const ID: &str = "skyline/inline-run";

/// 可以作为行内流成员的标签：WebView 下 `text` 是 inline，`image` / `span` 是 inline-block。
const INLINE_TAGS: [&str; 3] = ["text", "image", "span"];

/// TextShadowNode：它们内部的子节点本来就共用一个段落。
const TEXT_HOSTS: [&str; 3] = ["text", "span", "navigator"];

/// 三种形态共用的修法：显式 flex，或两端都写成块级。
const HELP: &str = "把要并排的内容放进 `display: flex; flex-direction: row` 的容器；如果本来就要各占一行，给两端都显式写 `display: block`。不要靠包 `<text>` 修：Skyline 下它仍是块级项，会各自换行";

pub struct InlineRun;

/// 视觉用例：父级是普通 `<view>`，两个 `<text>` 并列。
///
/// WebView 的两个 `<text>` 都是 inline，走同一个行内流（父级高一行）；
/// Skyline 默认 block，各自占一行（父级高两行）。`#target` 的 height 直接不同。
static VISUAL_CASES: &[VisualCase] = &[
    VisualCase {
        wxml: r#"<view class="row" id="target"><text class="tag">标签一</text><text class="tag">标签二</text></view>"#,
        wxss: ".row { padding: 10px; background: #f4f4f4; }\n.tag { font-size: 16px; background: #ccd; }\n",
        js: None,
        expect: VisualExpect::Differ,
        note: "并列的两个 `<text>`：WebView 排在同一行（父级高一行），Skyline 各自占一行（父级高两行）；`.tag` 的底色把这件事画得更清楚：Skyline 每行都撑满父宽（10–364.5），WebView 只包住文字（10–105.5）",
        target: "#target",
    },
    // 形态 1：裸文本与 `<image>`（WebView 的 inline-block）混排。整页模式。
    VisualCase {
        wxml: r#"<view class="tip">限时优惠<image class="icon" id="target"/></view>"#,
        wxss: ".tip { padding: 10px; font-size: 16px; }\n.icon { width: 60px; height: 60px; background: #cde; }\n",
        js: None,
        expect: VisualExpect::Differ,
        note: "WebView 把裸文本与 <image> 排进同一个行盒：图片紧接文字右侧（x 74 CSS），行盒被 60px 图片撑高，文字按基线落到行盒底部、比 Skyline 低约 21.5 CSS；Skyline 没有元素级行内流，裸文本是独立块级项，图片另起一行（y 从 10 到 32 CSS）。整页模式，证据强度弱于元素模式（无 bbox 兜底）",
        target: "",
    },
    // 形态 2：两个静态 `display: inline` 的非文本元素。整页模式。
    VisualCase {
        wxml: r#"<view class="row"><view class="a" id="target">甲</view><view class="b">乙</view></view>"#,
        wxss: ".row { padding: 10px; }\n.a { display: inline; background: #cde; }\n.b { display: inline; background: #edc; }\n",
        js: None,
        expect: VisualExpect::Differ,
        note: "WebView 的 `display: inline` 让 `.a`、`.b` 都收缩成文字宽并排在同一行；Skyline 忽略它，两个都撑满父宽、各占一行。规则对 `.a`、`.b` 各报一条，属预期。整页模式，证据强度弱于元素模式",
        target: "",
    },
    // 形态 2 的 `inline-block` 版：与上一条同构，只换声明。整页模式。
    VisualCase {
        wxml: r#"<view class="row"><view class="a" id="target">甲</view><view class="b">乙</view></view>"#,
        wxss: ".row { padding: 10px; }\n.a { display: inline-block; background: #cde; }\n.b { display: inline-block; background: #edc; }\n",
        js: None,
        expect: VisualExpect::Differ,
        note: "`display: inline-block`：WebView 真按行内块排——`.a`、`.b` 各自收缩到文字宽并排在同一行；Skyline 忽略声明、仍按块级盒排，两个都撑满父宽、各占一行。规则对 `.a`、`.b` 各报一条，属预期。整页模式，证据强度弱于元素模式（无 bbox 兜底）",
        target: "",
    },
];

impl Rule for InlineRun {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Warning,
            description: "Skyline 没有元素级行内格式化上下文：裸文本与元素混排、非文本元素上的 `display: inline` / `inline-block`、并列的文本级元素都会变成块级堆叠",
        }
    }

    fn visual_cases(&self) -> &'static [VisualCase] {
        VISUAL_CASES
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        for_each_element(project, self.target(), |site| {
            // 形态 2 只看元素自己的声明，与它有几个兄弟无关。
            check_inline_display(site, &mut out);
            check_element(site, &mut out);
        });
        out
    }
}

fn check_element(site: &ElementSite<'_, '_>, out: &mut Vec<Diagnostic>) {
    let (project, doc, tree, element, ancestors) = (
        site.project,
        site.doc,
        site.tree,
        site.element,
        site.ancestors,
    );
    // 文档既不是单元、也不在任何单元的模板闭包里：没有可求值的作用域，样式与
    // 实例化处的祖先都判不了，整篇跳过（不对无名文档做推断）。
    if project.scope_of(doc.path()).is_none() && !project.template_scopes.contains_key(doc.path()) {
        return;
    }
    if TEXT_HOSTS.contains(&element.tag.as_str()) {
        return;
    }
    // 子节点被投影进组件的 `<text>`（或没有匹配的槽位、不渲染）时，这些内容运行时
    // 在 TextShadowNode 里，两端都按行内流排；按本元素判定会误报。槽位解析在图上做。
    match site.projected_parent() {
        Some(SlotHost::NotRendered) => return,
        Some(SlotHost::Element(tag)) if TEXT_HOSTS.contains(&tag) => return,
        _ => {}
    }
    // 内联 `style` 是动态值时 display 未知，宁可漏报。
    if has_dynamic_style(element) {
        return;
    }
    // 横向滚动的 scroll-view 在 Skyline 下是行向 flex 容器
    // （`component-attribute`），子节点本来就是 flex item，与 WebView 一致。
    if element.tag == "scroll-view" && !attr_is_off(element, "scroll-x") {
        return;
    }
    // 结构上凑不出差异：形态 1 要裸文本 + 元素，形态 3 要 ≥2 个文本级元素。
    let element_children: Vec<&WxmlElement> = element
        .children
        .iter()
        .filter_map(|child| match child {
            WxmlNode::Element(child) => Some(child),
            _ => None,
        })
        .collect();
    let has_meaningful_text = element
        .children
        .iter()
        .any(|child| matches!(child, WxmlNode::Text(text) if text.is_meaningful()));
    let inline_candidates = element_children
        .iter()
        .filter(|child| INLINE_TAGS.contains(&child.tag.as_str()))
        .count();
    if !(has_meaningful_text && !element_children.is_empty()) && inline_candidates < 2 {
        return;
    }
    // 看不见的容器里「同行还是分行」没有视觉差异：`display: none` 让整棵子树不渲染；
    // `visibility: hidden` 且被移出视口时也一样（离屏预热容器）。
    if is_invisible(project, tree, element, ancestors) {
        return;
    }
    // 模板体 / 片段里父级的 `display` 判不了时（没有实例化路径，或选择器超出
    // 受限文法），无法断言「不是 flex」：`None` 直接跳过。
    let Some(display) = cascade_entries_with(project, tree, element, "display") else {
        return;
    };
    // 父级是 flex 容器时（行向同行、列向同列）两种渲染器排法一致。
    if display
        .last()
        .is_some_and(|entry| is_flex_value(&entry.value))
    {
        return;
    }
    // 父级的 class / style 跟着 `renderer` 变时，WebView 那一支可能已经写了
    // `display: flex; flex-direction: column`（把两行文本排成上下）——那种写法在
    // Skyline 下靠默认值就成立，不是差异。我们只求得着 Skyline 一支，判不了另一支。
    if varies_by_renderer(element) {
        return;
    }
    // 形态 1：裸文本与元素混排。报出后不再叠加形态 3，同一根因只留一条。
    if report_mixed_text(project, tree, element, doc, out) {
        return;
    }
    let mut run: Vec<&WxmlElement> = Vec::new();
    for child in &element.children {
        match child {
            WxmlNode::Element(child) if is_inline_member(project, tree, child) => {
                run.push(child);
            }
            _ => flush(doc, &mut run, out),
        }
    }
    flush(doc, &mut run, out);
}

/// 形态 2：非文本元素自己静态声明了 `display: inline` / `inline-block`。
///
/// Skyline 对元素级 `display` 只认 block / flex 一类，`inline` 与 `inline-block` 都
/// 退回块级盒；WebView 两种都按行内级盒排（实测 `inline-block` 收缩到内容宽，
/// 在 Skyline 里却撑满父宽）。TextShadowNode 宿主、flex 子项与脱流元素除外。
fn check_inline_display(site: &ElementSite<'_, '_>, out: &mut Vec<Diagnostic>) {
    let (project, doc, tree, element) = (site.project, site.doc, site.tree, site.element);
    if TEXT_HOSTS.contains(&element.tag.as_str()) || has_dynamic_style(element) {
        return;
    }
    let Some(value) = static_inline_display(project, tree, element) else {
        return;
    };
    if super::is_out_of_flow(project, tree, element)
        || is_invisible(project, tree, element, site.ancestors)
    {
        return;
    }
    let Some(parent) = tree.parent_of(element) else {
        return;
    };
    if TEXT_HOSTS.contains(&parent.tag.as_str()) || flex_container(project, tree, parent) {
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
                "`<{}>` 上静态 `display: {value}` 在 Skyline 下不生效：WebView 按行内级盒排（收缩到内容宽、可与兄弟并排），Skyline 仍按块级盒（独占一行、撑满父宽）",
                element.tag
            ),
        )
        .help(HELP),
    );
}

/// 元素自己静态声明的行内级 `display`；判不了或不是行内级时返回 `None`。
fn static_inline_display(
    project: &Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
) -> Option<String> {
    match resolved_property_with(project, tree, element, "display") {
        (Some(resolved), false) => {
            let value = resolved.value.trim().to_ascii_lowercase();
            matches!(value.as_str(), "inline" | "inline-block").then_some(value)
        }
        _ => None,
    }
}

/// 形态 1：裸文本与元素混排（「裸文本 + 单个元素」也算，比如客服电话那一行）。
///
/// 报在文本节点上：要改的是这段文本所在的排版容器。分属互斥 `wx:if` 分支的文本与
/// 元素不会同时渲染，不算混排。只要报出至少一条就返回 `true`，调用方据此跳过形态 3。
fn report_mixed_text(
    project: &Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
    doc: &WxmlDoc,
    out: &mut Vec<Diagnostic>,
) -> bool {
    let elements: Vec<&WxmlElement> = element
        .children
        .iter()
        .filter_map(|child| match child {
            WxmlNode::Element(child) => Some(child),
            _ => None,
        })
        .filter(|child| !super::is_out_of_flow(project, tree, child))
        .collect();
    if elements.is_empty() {
        return false;
    }
    let mut reported = false;
    for text in element.children.iter().filter_map(|child| match child {
        WxmlNode::Text(text) if text.is_meaningful() => Some(text),
        _ => None,
    }) {
        if elements
            .iter()
            .all(|element| exclusive_branches(&text.branches, &element.branches))
        {
            continue;
        }
        out.push(
            span_diagnostic(
                ID,
                doc,
                text.start,
                text.end,
                doc.path(),
                Severity::Warning,
                format!(
                    "`<{}>` 内裸文本与元素混排：WebView 把裸文本排进元素级行内流，Skyline 没有元素级行内格式化上下文，裸文本会成为独立块级项",
                    element.tag
                ),
            )
            .help(HELP),
        );
        reported = true;
    }
    reported
}

/// 这个元素在级联后确定不渲染：自己或任一祖先 `display: none`。
///
/// 只认这一种：`visibility: hidden` 常被当作动画初始态，运行时会变可见；离屏 / 1px
/// 盒子也可能被脚本挪回来。这些情况下「判不了」就按会渲染处理——报出来让人确认，
/// 比默默漏掉一个真差异好。
///
/// 链上任何节点带动态 `style` 时同样判不了：运行时的 `display` 可能覆盖静态声明。
pub(crate) fn is_invisible(
    project: &Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
    ancestors: &[&WxmlElement],
) -> bool {
    let chain: Vec<&WxmlElement> = std::iter::once(element)
        .chain(ancestors.iter().copied())
        .collect();
    if chain.iter().any(|node| has_dynamic_style(node)) {
        return false;
    }
    chain.iter().any(|node| {
        super::resolved_property(project, tree, node, "display")
            .is_some_and(|resolved| resolved.value.trim().eq_ignore_ascii_case("none"))
    })
}

/// 元素是否自己声明了纵向占位：固定宽度或纵向 margin。
///
/// 作者这么写就已经表达了「各占一行」，并列关系不是无意中形成的。`auto` / `0` /
/// `none` 不算（那些不产生纵向块）。
pub(crate) fn stacks_vertically(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> bool {
    ["width", "margin-top", "margin-bottom"]
        .iter()
        .filter_map(|property| resolved_property(project, tree, element, property))
        .any(|resolved| {
            let value = resolved.value.trim();
            !matches!(value, "0" | "0px" | "0rpx" | "auto" | "none")
        })
}

/// 文本级成员：标签在内，且靠**默认 layout** 参与行内流。
///
/// 显式写 inline / inline-block 的不算：判据只看「没写 `display`、靠两端默认值参与
/// 行内流」的成员；非文本元素的显式声明由 [`check_inline_display`] 单独处理。
/// 绝对定位、占满整行、带纵向外边距的成员也不是行内内容：前两者两端都脱离行内流，
/// 后者作者已经用纵向间距表达「各占一行」。
pub(crate) fn is_inline_member(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> bool {
    if !INLINE_TAGS.contains(&element.tag.as_str()) {
        return false;
    }
    let Some(display) = cascade_entries_with(project, tree, element, "display") else {
        return false;
    };
    display.is_empty()
        && !has_dynamic_style(element)
        && !varies_by_renderer(element)
        && !super::is_out_of_flow(project, tree, element)
        && !stacks_vertically(project, tree, element)
}

/// 元素的某个属性引用了 `renderer`：这个元素在两个渲染器下的样式可能不同。
///
/// 判据是「属性表达式里出现 `renderer` 这个数据字段」——包括 `item.renderer`
/// 这类同名字段（宁可按受影响处理）。
fn varies_by_renderer(element: &WxmlElement) -> bool {
    element
        .attributes
        .iter()
        .any(|attr| attr.references_renderer)
}

fn has_dynamic_style(element: &WxmlElement) -> bool {
    element
        .attributes
        .iter()
        .any(|attr| attr.name.eq_ignore_ascii_case("style") && !attr.is_static)
}

/// 一段并列的文本级成员；分属互斥 `wx:if` 分支的成员不会同时渲染，整段跳过。
fn flush(doc: &WxmlDoc, run: &mut Vec<&WxmlElement>, out: &mut Vec<Diagnostic>) {
    // 任一两个成员分属互斥分支就无法证明它们同时渲染，整段跳过。
    let mut coexists = true;
    for (index, left) in run.iter().enumerate() {
        if run[index + 1..].iter().any(|right| exclusive(left, right)) {
            coexists = false;
        }
    }
    if run.len() >= 2 && coexists {
        out.push(
            element_diagnostic(
                ID,
                doc,
                run[0],
                doc.path(),
                Severity::Warning,
                format!(
                    "{} 个文本级元素并列且都没写 `display`：WebView 按 inline / inline-block 排在同一行，Skyline 下各自占一行",
                    run.len()
                ),
            )
            .help(HELP),
        );
    }
    run.clear();
}

#[cfg(test)]
mod tests {
    use super::InlineRun;
    use crate::rules::testutil::wxml::{Fixture, check_rule, project};

    fn check(fixture: &Fixture) -> Vec<crate::Diagnostic> {
        check_rule(fixture, &InlineRun)
    }

    #[test]
    fn adjacent_text_siblings() {
        let diagnostics = check(&project(
            "inline-run/text",
            "<view><text>a</text><text>b</text></view>",
        ));
        assert_eq!(diagnostics.len(), 1);
    }

    /// 视觉用例同源的页面：两个并列 `<text>` 在两端行数不同。
    #[test]
    fn visual_case_adjacent_texts_are_reported() {
        let fixture = Fixture::page(
            "inline-run/visual",
            r#"<view class="row" id="target"><text class="tag">标签一</text><text class="tag">标签二</text></view>"#,
            ".row { padding: 10px; }\n.tag { font-size: 16px; }\n",
        );
        let diagnostics = check(&fixture);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("并列"));
    }

    /// 三种形态的视觉用例都必须真的能触发本规则。
    #[test]
    fn every_visual_case_triggers_the_rule() {
        for case in super::VISUAL_CASES {
            let fixture = Fixture::page("inline-run/visual", case.wxml, case.wxss);
            let diagnostics = check(&fixture);
            assert!(!diagnostics.is_empty(), "视觉用例未触发规则：{}", case.wxml);
        }
    }

    // ---- 三种形态 ----

    /// 形态 1：裸文本 + **单个** 元素（客服电话那一行）也要报。
    #[test]
    fn bare_text_with_a_single_element_is_reported() {
        let fixture = Fixture::wxml(
            "inline-run/mixed-single",
            r#"<view class="memo">客服电话：<text class="phone">1010-5335</text></view>"#,
        );
        let diagnostics = check(&fixture);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("裸文本与元素混排"));
    }

    /// 形态 2：非文本元素上的静态 `display: inline`。
    #[test]
    fn static_inline_display_is_reported() {
        let fixture = Fixture::wxml(
            "inline-run/inline-decl",
            r#"<view class="row"><view class="a">甲</view></view>"#,
        );
        fixture.write("pages/index/index.wxss", ".a { display: inline; }");
        let diagnostics = check(&fixture);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(
            diagnostics[0].message.contains("display: inline"),
            "{diagnostics:#?}"
        );
    }

    /// 形态 2：`display: inline-block` 在 WebView 是行内块，Skyline 也按块级 —— 必须覆盖。
    #[test]
    fn static_inline_block_display_is_reported() {
        let fixture = Fixture::wxml(
            "inline-run/inline-block",
            r#"<view class="row"><view class="a">甲</view></view>"#,
        );
        fixture.write("pages/index/index.wxss", ".a { display: inline-block; }");
        let diagnostics = check(&fixture);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(
            diagnostics[0].message.contains("display: inline-block"),
            "{diagnostics:#?}"
        );
    }

    /// 把裸文本包进 `<text>` 仍然是两个块级项 —— 规则必须继续报，不能给错误建议。
    #[test]
    fn wrapping_bare_text_in_a_text_tag_is_still_reported() {
        let diagnostics = check(&project(
            "inline-run/wrapped-text",
            "<view><text>说明</text><text>1010-5335</text></view>",
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("都没写 `display`"));
    }

    /// 形态 2 的视觉用例：两个 `display: inline` 的 `<view>` 各报一条。
    #[test]
    fn visual_case_inline_display_reports_each_element() {
        let case = &super::VISUAL_CASES[2];
        let fixture = Fixture::page("inline-run/visual-inline", case.wxml, case.wxss);
        assert_eq!(check(&fixture).len(), 2);
    }

    /// 形态 2 的 `inline-block` 视觉用例与 `inline` 版同源：两个 `<view>` 各报一条。
    #[test]
    fn visual_case_inline_block_reports_each_element() {
        let case = &super::VISUAL_CASES[3];
        let fixture = Fixture::page("inline-run/visual-inline-block", case.wxml, case.wxss);
        let diagnostics = check(&fixture);
        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.message.contains("display: inline-block")),
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn text_and_image_siblings() {
        let diagnostics = check(&project(
            "inline-run/image",
            "<view><text>a</text><image src=\"x\"/></view>",
        ));
        assert_eq!(diagnostics.len(), 1);
    }

    /// 含 `image` 时唯一可行的修法是 flex：`<text>` 装不下图片节点。
    #[test]
    fn image_member_gets_the_flex_help() {
        let diagnostics = check(&project(
            "inline-run/image-help",
            "<view><text>a</text><image src=\"x\"/></view>",
        ));
        assert_eq!(diagnostics.len(), 1);
        let help = diagnostics[0].help.as_deref().unwrap_or_default();
        assert!(help.contains("display: flex"), "{help}");
        assert!(!help.contains("包进同一个"), "{help}");
    }

    /// 三种形态用同一条修法：显式 flex，或两端都写成块级；不再建议包 `<text>`。
    #[test]
    fn text_only_members_get_the_flex_help() {
        let diagnostics = check(&project(
            "inline-run/text-help",
            "<view><text>a</text><span>b</span></view>",
        ));
        assert_eq!(diagnostics.len(), 1);
        let help = diagnostics[0].help.as_deref().unwrap_or_default();
        assert!(help.contains("display: flex"), "{help}");
        assert!(help.contains("display: block"), "{help}");
        assert!(!help.contains("包进同一个"), "{help}");
        assert!(help.contains("不要靠包 `<text>` 修"), "{help}");
    }

    #[test]
    fn flex_parent_has_no_difference() {
        let fixture = project(
            "inline-run/flex",
            "<view class=\"row\"><text>a</text><text>b</text></view>",
        );
        fixture.write("pages/index/index.wxss", ".row { display: flex; }");
        assert!(check(&fixture).is_empty());
    }

    #[test]
    fn text_parent_shares_one_paragraph() {
        let diagnostics = check(&project(
            "inline-run/span",
            "<span><text>a</text><image src=\"x\"/></span>",
        ));
        assert!(diagnostics.is_empty());
    }

    /// 片段被投影进组件的 `<text>`：运行时文本落在 TextShadowNode 里，两端都不换行。
    #[test]
    fn text_projected_into_a_text_slot_is_exempt() {
        let fixture = Fixture::new("inline-run/slot-text");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                r#"{"usingComponents":{"pill":"/components/pill/pill"}}"#,
            )
            .write(
                "pages/index/index.wxml",
                r#"<pill>共 {{n}} 条<image src="x"/></pill>"#,
            )
            .write("pages/index/index.js", "Page({});")
            .write("components/pill/pill.json", r#"{"component":true}"#)
            .write("components/pill/pill.wxml", "<text><slot/></text>")
            .write("components/pill/pill.js", "Component({});");
        assert!(check(&fixture).is_empty());
    }

    #[test]
    fn bare_text_mixed_with_element_is_reported() {
        let diagnostics = check(&project(
            "inline-run/bare",
            "<view>说明<text>a</text><text>b</text></view>",
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(
            diagnostics[0].message.contains("裸文本"),
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn explicit_block_display_is_intentional() {
        let fixture = project(
            "inline-run/block",
            "<view><text class=\"stack\">a</text><text class=\"stack\">b</text></view>",
        );
        fixture.write("pages/index/index.wxss", ".stack { display: block; }");
        assert!(check(&fixture).is_empty());
    }

    #[test]
    fn nested_for_and_else_chain() {
        let source = concat!(
            "<view wx:for=\"{{list}}\">",
            "<view class=\"cnt\">",
            "<image wx:if=\"{{a}}\"/>",
            "<block wx:else><image wx:if=\"{{b}}\"/><image wx:else/></block>",
            "</view></view>"
        );
        let diagnostics = check(&project("inline-run/nested-for", source));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn wx_if_branches_never_coexist() {
        let diagnostics = check(&project(
            "inline-run/branch",
            "<view><text wx:if=\"{{a}}\">a</text><text wx:else>b</text></view>",
        ));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn else_on_block_keeps_the_chain() {
        let diagnostics = check(&project(
            "inline-run/block-branch",
            "<view><image wx:if=\"{{a}}\"/><block wx:else><image wx:if=\"{{b}}\"/><image wx:else/></block></view>",
        ));
        assert!(diagnostics.is_empty());
    }

    /// 父级的 class 跟着 `renderer` 变时不判：WebView 那一支可能已经排成上下。
    #[test]
    fn renderer_conditional_parent_is_not_judged() {
        // recent-read：WebView 分支靠 `book-info-webview` 把 title/author 排成上下，
        // Skyline 分支靠默认值就成立，两端一致。
        let fixture = Fixture::wxml(
            "inline-run/renderer-class",
            "<view class=\"book-info {{ renderer === 'skyline' ? '' : 'book-info-webview' }}\"><text class=\"book-title\">a</text><text class=\"book-author\">b</text></view>",
        );
        let diagnostics = check(&fixture);
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// 横向滚动的 scroll-view 在 Skyline 下是行向 flex，与 WebView 一致。
    #[test]
    fn horizontal_scroll_view_renders_as_row_flex() {
        let diagnostics = check(&project(
            "inline-run/scroll-x",
            "<scroll-view scroll-x><text>a</text><text>b</text></scroll-view>",
        ));
        assert!(diagnostics.is_empty());
    }

    // ---- 模板定义体：样式按引用它的单元求值 ----

    /// 模板定义体：某价格区模板的真实写法。
    fn template(body: &str) -> Fixture {
        let fixture = Fixture::new("inline-run/template");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write(
                "tpls/t.wxml",
                &format!("<template name=\"cinema\">{body}</template>"),
            )
            .write(
                "pages/index/index.wxml",
                r#"<import src="/tpls/t.wxml"/><template is="cinema"/>"#,
            );
        fixture
    }

    /// 模板体 + 引用它的页面样式（模板的样式只能从引用它的单元进来）。
    fn template_with_style(body: &str, wxss: &str) -> Fixture {
        let fixture = template(body);
        fixture.write("pages/index/index.wxss", wxss);
        fixture
    }

    #[test]
    fn adjacent_texts_in_template_are_reported() {
        let fixture = template(
            "<view class=\"price\"><text class=\"num\">45</text><text>元</text><text class=\"num-unit\">起</text></view>",
        );
        let diagnostics = check(&fixture);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].rule, super::ID);
    }

    #[test]
    fn span_counts_as_text_level_in_template() {
        let diagnostics = check(&template("<view><text>a</text><span>b</span></view>"));
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn template_text_and_image_pair_gets_the_flex_help() {
        let diagnostics = check(&template("<view><text>a</text><image src=\"x\"/></view>"));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        let help = diagnostics[0].help.as_deref().unwrap_or_default();
        assert!(help.contains("display: flex"), "{help}");
    }

    #[test]
    fn template_text_host_parent_shares_one_paragraph() {
        let diagnostics = check(&template("<text><text>a</text><text>b</text></text>"));
        assert!(diagnostics.is_empty());
    }

    /// 内联 `<template name>` 写在单元自身（页面 / 组件）里时，样式按该单元的作用域求值。
    #[test]
    fn inline_template_in_unit_resolves_unit_styles() {
        let fixture = Fixture::wxml(
            "inline-run/inline-template",
            "<template name=\"t\"><view class=\"row\"><text>a</text><text>b</text></view></template>",
        );
        fixture.write(
            "pages/index/index.wxss",
            ".row { display: flex; flex-direction: row; align-items: baseline; }",
        );
        let diagnostics = check(&fixture);
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn template_flex_parent_has_no_difference() {
        let fixture = template_with_style(
            "<view class=\"price\"><text>a</text><text>b</text></view>",
            ".price { display: flex; }",
        );
        assert!(check(&fixture).is_empty());
    }

    #[test]
    fn template_explicit_display_is_intentional() {
        let fixture = template_with_style(
            "<view><text class=\"run\">a</text><text class=\"run\">b</text></view>",
            ".run { display: inline; }",
        );
        assert!(check(&fixture).is_empty());
    }

    #[test]
    fn template_bare_text_is_reported_as_mixed() {
        let diagnostics = check(&template("<view>说明<text>a</text><text>b</text></view>"));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("裸文本"));
    }

    #[test]
    fn template_wx_if_branches_never_coexist() {
        let diagnostics = check(&template(
            "<view><text wx:if=\"{{a}}\">a</text><text wx:else>b</text></view>",
        ));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn single_text_is_not_a_run() {
        let diagnostics = check(&template("<view><text>a</text></view>"));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn view_between_texts_breaks_the_run() {
        let diagnostics = check(&template(
            "<view><text>a</text><view/><text>b</text></view>",
        ));
        assert!(diagnostics.is_empty());
    }

    /// 模板里「祖先在调用处」的选择器：`.outer` 写在页面里包住 `<template is>`，
    /// 真实实例化路径是 `.outer > .inner`，`display: flex` 真的命中 ⇒ 不报。
    #[test]
    fn template_flex_on_usage_ancestor_is_not_reported() {
        let fixture = template_with_style(
            "<view class=\"inner\"><text>a</text><text>b</text></view>",
            ".outer .inner { display: flex; align-items: center; }",
        );
        fixture.write(
            "pages/index/index.wxml",
            r#"<import src="/tpls/t.wxml"/><view class="outer"><template is="cinema"/></view>"#,
        );
        let diagnostics = check(&fixture);
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// 同一个模板被两个单元引用，只有其中一个带 `.outer`：存在即成立 ⇒ 不报。
    #[test]
    fn template_flex_on_one_of_two_units_is_not_reported() {
        let fixture = template("<view class=\"inner\"><text>a</text><text>b</text></view>");
        fixture
            .write(
                "pages/index/index.wxml",
                r#"<import src="/tpls/t.wxml"/><view class="outer"><template is="cinema"/></view>"#,
            )
            .write("pages/other/other.json", "{}")
            .write(
                "pages/other/other.wxml",
                r#"<import src="/tpls/t.wxml"/><template is="cinema"/>"#,
            )
            .write(
                "app.json",
                r#"{"pages":["pages/index/index","pages/other/other"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.wxss", ".outer .inner { display: flex; }");
        let diagnostics = check(&fixture);
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// `.outer` 谁都没有：一条路径都不命中 ⇒ 照报（不是无条件豁免）。
    #[test]
    fn template_flex_missing_everywhere_is_reported() {
        let fixture = template_with_style(
            "<view class=\"inner\"><text>a</text><text>b</text></view>",
            ".outer .inner { display: flex; align-items: center; }",
        );
        fixture.write(
            "pages/index/index.wxml",
            r#"<import src="/tpls/t.wxml"/><view class="wrap"><template is="cinema"/></view>"#,
        );
        let diagnostics = check(&fixture);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    /// `visibility: hidden` 可能是动画初始态，运行时会变可见：照报，让人确认。
    #[test]
    fn visibility_hidden_container_is_still_reported() {
        let diagnostics = check(&project(
            "inline-run/hidden",
            "<view style=\"position: absolute; top: -9999px; visibility: hidden\"><view class=\"inner\"><text>a</text><text>b</text></view></view>",
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    /// 祖先带动态 `style`：运行时的 `display` 可能变化，判不了就报。
    #[test]
    fn dynamic_ancestor_style_is_reported() {
        let diagnostics = check(&project(
            "inline-run/dynamic-style",
            "<view style=\"display: {{hidden ? 'none' : 'block'}}\"><view class=\"inner\"><text>a</text><text>b</text></view></view>",
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    /// `visibility: hidden` 但没离屏（大量元素拿它当动画初始态）照报，不滤掉真差异。
    #[test]
    fn hidden_but_onscreen_container_is_reported() {
        let fixture = project(
            "inline-run/hidden-onscreen",
            "<view class=\"bubble\"><text>a</text><text>b</text></view>",
        );
        fixture.write(
            "pages/index/index.wxss",
            ".bubble { visibility: hidden; position: absolute; left: 0; }",
        );
        let diagnostics = check(&fixture);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    /// 1px 离屏预热容器也可能是脚本挪回来的：照报（只有 `display: none` 才跳过）。
    #[test]
    fn offscreen_one_pixel_container_is_still_reported() {
        let diagnostics = check(&project(
            "inline-run/preload",
            "<view style=\"position: fixed; top: -9999px; left: -9999px; width: 1px; height: 1px; overflow: hidden; visibility: hidden\"><image src=\"a\"/><image src=\"b\"/></view>",
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    /// 子节点用 `visibility: visible` 覆盖祖先的 hidden。
    #[test]
    fn visible_child_overrides_hidden_ancestor() {
        let fixture = project(
            "inline-run/visible-child",
            "<view style=\"position: absolute; top: -9999px; visibility: hidden\"><view class=\"inner\"><text>a</text><text>b</text></view></view>",
        );
        fixture.write("pages/index/index.wxss", ".inner { visibility: visible; }");
        let diagnostics = check(&fixture);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    /// 离屏预热容器：`display: none` 写在 host 自己身上时不报。
    #[test]
    fn display_none_host_is_not_reported() {
        let diagnostics = check(&project(
            "inline-run/display-none",
            "<view class=\"inner\" style=\"display: none\"><text>a</text><text>b</text></view>",
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// 同字段不同字面量的两个独立 `wx:if`：只有一个会渲染，不算并列。
    #[test]
    fn same_field_literal_branches_never_coexist() {
        let source = concat!(
            "<view class=\"masks\">",
            "<image wx:if=\"{{mode === 0}}\"/>",
            "<image wx:if=\"{{mode === 1}}\"/>",
            "<image wx:if=\"{{mode === 2}}\"/>",
            "</view>"
        );
        let diagnostics = check(&project("inline-run/mask-branches", source));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// 模板被 import 了但从来没实例化（没有实例化路径）：判不了，不报。
    #[test]
    fn template_never_instantiated_is_skipped() {
        let fixture = template("<view class=\"inner\"><text>a</text><text>b</text></view>");
        // 只 import，不放 `<template is>`：模板体永远不会渲染。
        fixture.write(
            "pages/index/index.wxml",
            r#"<import src="/tpls/t.wxml"/><view class="wrap"/>"#,
        );
        // 复合选择器 + 没有实例化路径 ⇒ 判不了（不是「算出 flex」）。
        fixture.write("pages/index/index.wxss", ".outer .inner { display: flex; }");
        let diagnostics = check(&fixture);
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// 既不是单元、也不在 `template_scopes` 里的文档整篇跳过：没有作用域可求值，
    /// 也不该退化成「没写样式」去推断。
    #[test]
    fn unnamed_document_is_skipped() {
        let fixture = project("inline-run/unnamed", "<view/>");
        fixture.write(
            "tpls/stray.wxml",
            "<view><text>a</text><text>b</text></view>",
        );
        let out = check(&fixture);
        assert!(out.is_empty(), "{out:#?}");
    }
}
