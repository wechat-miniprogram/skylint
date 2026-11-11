//! 并列的文本级元素在 WebView 下排在同一行，在 Skyline 下各自换行。
//!
//! WebView 里 `<text>` 是 inline（`wxapplib/src/wx-component/src/wx-text/wx-text.less`
//! 只设 cursor，没有 `display`，自定义元素按浏览器默认的 `inline`；对比 `wx-view.less`
//! 显式写了 `display: block`），`<image>` / `<span>` 是 inline-block，block 容器里几个
//! 并列的文本级元素走**同一个行内流**；Skyline 里它们默认是 block（`display.cc:43-50`：
//! 默认 block 或 flex），各自占一行。
//!
//! 想要「一段共享排版的文本流」只能让 TextShadowNode 当最近容器：文本子节点变
//! `TextSpan`、非文本子节点变 `WidgetSpan`，全部 `BuildInlineSpan` 进同一个段落
//! （`shadow_node.cc:205-209`、`text_span.cc:280-285`、`text.cc:188-201`）。
//! 只想「横向排列」就显式 `display: flex`——两种渲染器都按 flex 排，天然一致。
//!
//! 裸文本与元素混排归 `no-inline-text`（「裸文本 + **单个** `<text>` 兄弟」也在
//! 那里，比如 `<view>客服电话：<text>1010-5335</text></view>`）；这里只补「没有
//! 裸文本」的那一半：兄弟全是被当成 inline 的元素。父级是 flex/grid 时不报（两边
//! 都按 flex item 排，无差异）；元素显式写了非 inline 的 `display` 时也不报
//! （作者已经明确要块级）。
//!
//! 模板定义体与 `<include>` 片段同样适用：样式按引用它们的单元求值
//! （[`super::cascade_entries`] 是唯一口径），所以模板里「两串 `<text>` 靠默认布局
//! 并排」与普通页面是同一个现象，只报一次。

use crate::diag::{Diagnostic, Severity};
use crate::input::wxml::{WxmlDoc, WxmlElement, WxmlNode};
use crate::matcher::Tree;
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};

use super::branch::exclusive;
use super::structure::attr_is_off;
use super::{display_values, element_diagnostic, flex_container, visit, walk_skyline};

pub const ID: &str = "skyline/inline-run";

/// 可以作为行内流成员的标签：WebView 下 `text` 是 inline，`image` / `span` 是 inline-block。
const INLINE_TAGS: [&str; 3] = ["text", "image", "span"];

/// TextShadowNode：它们内部的子节点本来就共用一个段落。
const TEXT_HOSTS: [&str; 3] = ["text", "span", "navigator"];

pub struct InlineRun;

impl Rule for InlineRun {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Warning,
            description: "并列的文本级元素在 WebView 下排在同一行，在 Skyline 下各自占一行",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit(project, |doc, out| {
            let tree = Tree::new(doc);
            walk_skyline(doc, |element, _ancestors| {
                check_element(project, doc, &tree, element, out);
            });
        })
    }
}

fn check_element(
    project: &Project,
    doc: &WxmlDoc,
    tree: &Tree<'_>,
    element: &WxmlElement,
    out: &mut Vec<Diagnostic>,
) {
    // 文档既不是单元、也不在任何单元的模板闭包里：没有可求值的作用域，样式与
    // 实例化处的祖先都判不了，整篇跳过（不对无名文档做推断）。
    if project.scope_of(doc.path()).is_none() && !project.template_scopes.contains_key(doc.path()) {
        return;
    }
    if TEXT_HOSTS.contains(&element.tag.as_str()) {
        return;
    }
    // 内联 `style` 是动态值时 display 未知，宁可漏报。
    if has_dynamic_style(element) {
        return;
    }
    // 横向滚动的 scroll-view 在 Skyline 下是行向 flex 容器
    // （`horizontal-scroll-enable-flex`），子节点本来就是 flex item，与 WebView 一致。
    if element.tag == "scroll-view" && !attr_is_off(element, "scroll-x") {
        return;
    }
    // 父级是 flex 容器时（行向同行、列向同列）两种渲染器排法一致。
    if flex_container(project, tree, element) {
        return;
    }
    // 父级的 class / style 跟着 `renderer` 变时，WebView 那一支可能已经写了
    // `display: flex; flex-direction: column`（把两行文本排成上下）——那种写法在
    // Skyline 下靠默认值就成立，不是差异。我们只求得着 Skyline 一支，判不了另一支。
    if renderer_conditioned(element) {
        return;
    }
    let mut run: Vec<&WxmlElement> = Vec::new();
    let mut has_bare_text = false;
    for child in &element.children {
        match child {
            // 裸文本本身就是行内内容，不打断序列；但它出现时整段交给 `no-inline-text`。
            WxmlNode::Text(text) if text.is_meaningful() => has_bare_text = true,
            WxmlNode::Element(child) if is_inline_member(project, tree, child) => {
                run.push(child);
            }
            _ => flush(doc, &mut run, &mut has_bare_text, out),
        }
    }
    flush(doc, &mut run, &mut has_bare_text, out);
}

/// 文本级成员：标签在内，且靠**默认 layout** 参与行内流。
///
/// 显式写 inline / inline-block 的不算：两种渲染器都会把它们排进行内行，
/// 不写才依赖默认值（Skyline 默认 block / flex，WebView 默认 inline）。
/// 绝对定位、占满整行、带纵向外边距的成员也不是行内内容：前两者两端都脱离
/// 行内流，后者作者已经用纵向间距表达「各占一行」。
pub(crate) fn is_inline_member(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> bool {
    INLINE_TAGS.contains(&element.tag.as_str())
        && display_values(project, tree, element).is_empty()
        && !has_dynamic_style(element)
        && !renderer_conditioned(element)
        && !super::is_out_of_flow(project, tree, element)
        && !super::stacks_vertically(project, tree, element)
}

/// 元素的某个属性引用了 `renderer`：这个元素在两个渲染器下的样式可能不同。
fn renderer_conditioned(element: &WxmlElement) -> bool {
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
fn flush(
    doc: &WxmlDoc,
    run: &mut Vec<&WxmlElement>,
    has_bare_text: &mut bool,
    out: &mut Vec<Diagnostic>,
) {
    // 任一两个成员分属互斥分支就无法证明它们同时渲染，整段跳过。
    let mut coexists = true;
    for (index, left) in run.iter().enumerate() {
        if run[index + 1..].iter().any(|right| exclusive(left, right)) {
            coexists = false;
        }
    }
    if run.len() >= 2 && !*has_bare_text && coexists {
        // `image` 塞不进 `<text>`（WebView 只保留文本与嵌套 text，节点会丢），
        // 所以含图片时只有 flex 一种修法。
        let help = if run.iter().any(|element| element.tag == "image") {
            "父级写 `display: flex; flex-direction: row` 让它们横向排列；不要把 `image` 塞进 `<text>`——WebView 只保留文本与嵌套 text，节点会丢"
        } else {
            "把这一串包进同一个 `<text>`（保留各自的 class）"
        };
        out.push(
            element_diagnostic(
                ID,
                doc,
                run[0],
                doc.path(),
                Severity::Warning,
                format!(
                    "{} 个文本级元素并列：WebView 下排在同一行，Skyline 下各自占一行",
                    run.len()
                ),
            )
            .help(help),
        );
    }
    run.clear();
    *has_bare_text = false;
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

    /// 全是文本级标签时唯一修法是包进同一个 `<text>`。
    #[test]
    fn text_only_members_get_the_text_help() {
        let diagnostics = check(&project(
            "inline-run/text-help",
            "<view><text>a</text><span>b</span></view>",
        ));
        assert_eq!(diagnostics.len(), 1);
        let help = diagnostics[0].help.as_deref().unwrap_or_default();
        assert!(help.contains("包进同一个 `<text>`"), "{help}");
        assert!(!help.contains("display: flex"), "{help}");
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

    #[test]
    fn bare_text_is_left_to_no_inline_text() {
        let diagnostics = check(&project(
            "inline-run/bare",
            "<view>说明<text>a</text><text>b</text></view>",
        ));
        assert!(diagnostics.is_empty());
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

    /// 模板定义体：cinema 价格区的真实写法（`tpls/cinema-new/_cinema-item.wxml`）。
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
    fn template_bare_text_is_left_to_no_inline_text() {
        let diagnostics = check(&template("<view>说明<text>a</text><text>b</text></view>"));
        assert!(diagnostics.is_empty());
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

    /// 既不是单元、也不在 `template_scopes` 里的文档整篇跳过：没有作用域可求值，
    /// 也不该退化成「没写样式」去推断。
    #[test]
    fn unnamed_document_is_skipped() {
        let fixture = project("inline-run/unnamed", "<view/>");
        fixture.write(
            "tpls/stray.wxml",
            "<view><text>a</text><text>b</text></view>",
        );
        let project = fixture.project();
        let doc = crate::rules::wxml::load_doc(&project, "tpls/stray.wxml").unwrap();
        let tree = crate::matcher::Tree::new(&doc);
        let mut out = Vec::new();
        for element in doc.elements() {
            super::check_element(&project, &doc, &tree, element, &mut out);
        }
        assert!(out.is_empty(), "{out:#?}");
    }
}
