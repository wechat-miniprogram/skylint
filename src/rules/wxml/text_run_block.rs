//! 模板定义里并排的文本级元素在 Skyline 下各自占一行。
//!
//! WebView 里 `<text>` 是 inline（`wxapplib/src/wx-component/src/wx-text/wx-text.less`
//! 只设 cursor，没有 `display`，自定义元素按浏览器默认的 `inline`；对比
//! `wx-view.less` 显式写了 `display: block`），所以同一个块级容器里的几个 `<text>`
//! 走同一个行内流。Skyline 里 `<text>` 是 `TextShadowNode`，只有父级也是文本节点时
//! 才降级成 `TextSpan` 参与同一段落（`shadow/components/text_shadow_node.cc:149-158`）；
//! 父级不是 text 时它是根文本节点，`display` 取 CSS 初值
//! （`css/css_properties/display.cc:43-50`：`defaultDisplayBlock` 为 true 时 `block`，
//! 否则 `flex`）——两种都是块级盒子，兄弟各占一行。
//!
//! 为什么单独一条规则：`inline-run` 的判据在模板定义里不成立。模板文件不是单元，
//! `StyleSheet::scopes` 只记单元，`sheet_covers` 对模板路径恒为假，样式求值会退化成
//! 「什么都没写」，所以 `inline-run` 直接跳过模板（cinema 价格区、`_movie-list` 评分串
//! 这几处真差异都落在模板里，正是这个盲区）。这里用 [`Project::template_scopes`]（模板被
//! 哪些单元 `<import>` / `<include>` 进来）求样式，把模板这一半补上；非模板文件仍由
//! `inline-run` 负责，两条规则按文档类型分工，同一处不会报两次。
//!
//! 不报的情形：父级本身是 `text` / `span` / `navigator`（共用一个段落）、父级是 flex
//! 容器（两种渲染器都按 flex item 排）、`scroll-view scroll-x`（Skyline 下渲染成行向
//! flex）、成员显式写了 `display`（作者已经指定）、成员脱离文档流或带纵向间距（作者
//! 已经表达「各占一行」）、以及这一段里有裸文本（归 `no-inline-text`）。

use crate::diag::{Diagnostic, Severity};
use crate::input::wxml::{WxmlDoc, WxmlElement, WxmlNode};
use crate::matcher::Tree;
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};

use super::branch::exclusive;
use super::structure::attr_is_off;
use super::{
    element_diagnostic, template_cascade_entries, template_resolved_property, visit, walk_skyline,
};

pub const ID: &str = "skyline/text-run-block";

/// TextShadowNode：它们内部的子节点本来就共用一个段落，不产生块级盒子。
const TEXT_HOSTS: [&str; 3] = ["text", "span", "navigator"];

/// 靠默认 layout 参与行内流的成员。`image` 不在内：把图片塞进 `<text>` 会丢节点，
/// 修法不唯一，含图片的并列串由 `inline-run` 按 flex 修法报。
const TEXT_LEVEL: [&str; 2] = ["text", "span"];

pub struct TextRunBlock;

impl Rule for TextRunBlock {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Warning,
            description: "模板定义里并排的 `<text>` 在 Skyline 下会各自占一行",
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
    // 普通 WXML 由 `inline-run` 负责；两条规则判据同源，这里只补模板那一半。
    if !element.in_template() {
        return;
    }
    if TEXT_HOSTS.contains(&element.tag.as_str()) {
        return;
    }
    // 内联 `style` 是动态值时父级 display 未知，宁可漏报。
    if has_dynamic_style(element) {
        return;
    }
    // 横向滚动的 scroll-view 在 Skyline 下是行向 flex 容器（`horizontal-scroll-enable-flex`），
    // 子节点本来就是 flex item，与 WebView 一致。
    if element.tag == "scroll-view" && !attr_is_off(element, "scroll-x") {
        return;
    }
    // 父级是 flex 容器时（行向同行、列向同列）两种渲染器排法一致。
    if is_flex_container(project, tree, element) {
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
            // 裸文本与元素混排归 `no-inline-text`，这里不重复报。
            WxmlNode::Text(text) if text.is_meaningful() => has_bare_text = true,
            WxmlNode::Element(child) if is_text_member(project, tree, child) => run.push(child),
            _ => flush(doc, &mut run, &mut has_bare_text, out),
        }
    }
    flush(doc, &mut run, &mut has_bare_text, out);
}

/// 文本级成员：标签在内，且靠**默认 layout** 参与行内流。
///
/// 显式写了 `display` 的不算：作者已经指定了排法（`inline` / `inline-block` 同行，
/// `block` 各行），不需要工具再判。绝对定位、占满整行、带纵向外边距的成员也不是
/// 行内内容，判据与 `inline_run::is_inline_member` 一致。
fn is_text_member(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> bool {
    TEXT_LEVEL.contains(&element.tag.as_str())
        && template_cascade_entries(project, tree, element, "display").is_empty()
        && !has_dynamic_style(element)
        && !renderer_conditioned(element)
        && !is_out_of_flow(project, tree, element)
        && !stacks_vertically(project, tree, element)
}

/// 元素的某个属性引用了 `renderer`：这个元素在两个渲染器下的样式可能不同。
fn renderer_conditioned(element: &WxmlElement) -> bool {
    element
        .attributes
        .iter()
        .any(|attr| attr.references_renderer)
}

fn is_flex_container(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> bool {
    template_resolved_property(project, tree, element, "display").is_some_and(|resolved| {
        let value = resolved.value.trim();
        value.eq_ignore_ascii_case("flex") || value.eq_ignore_ascii_case("inline-flex")
    })
}

fn is_out_of_flow(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> bool {
    template_resolved_property(project, tree, element, "position").is_some_and(|resolved| {
        matches!(
            resolved.value.trim().to_ascii_lowercase().as_str(),
            "fixed" | "absolute"
        )
    })
}

fn stacks_vertically(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> bool {
    ["width", "margin-top", "margin-bottom"]
        .iter()
        .filter_map(|property| template_resolved_property(project, tree, element, property))
        .any(|resolved| {
            let value = resolved.value.trim();
            !matches!(value, "0" | "0px" | "0rpx" | "auto" | "none")
        })
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
    let mut coexists = true;
    for (index, left) in run.iter().enumerate() {
        if run[index + 1..].iter().any(|right| exclusive(left, right)) {
            coexists = false;
        }
    }
    if run.len() >= 2 && !*has_bare_text && coexists {
        out.push(
            element_diagnostic(
                ID,
                doc,
                run[0],
                doc.path(),
                Severity::Warning,
                format!(
                    "模板里 {} 个文本级元素并列：WebView 下排在同一行，Skyline 下各自占一行",
                    run.len()
                ),
            )
            .help("把这一串 `<text>` 包进同一个 `<text>`（保留各自的 class）；也可以在父级写 `display: flex; flex-direction: row; align-items: baseline`，两种写法都能让它们同行"),
        );
    }
    run.clear();
    *has_bare_text = false;
}

#[cfg(test)]
mod tests {
    use super::TextRunBlock;
    use crate::rules::testutil::wxml::{Fixture, check_rule, project};

    fn check(fixture: &Fixture) -> Vec<crate::Diagnostic> {
        check_rule(fixture, &TextRunBlock)
    }

    /// 模板定义体：cinema 价格区的真实写法（`tpls/cinema-new/_cinema-item.wxml`）。
    fn template(body: &str) -> Fixture {
        let fixture = Fixture::new("text-run/template");
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
    fn span_counts_as_text_level() {
        let diagnostics = check(&template("<view><text>a</text><span>b</span></view>"));
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn non_template_document_is_left_to_inline_run() {
        let diagnostics = check(&project(
            "text-run/plain",
            "<view><text>a</text><text>b</text></view>",
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn text_host_parent_shares_one_paragraph() {
        let diagnostics = check(&template("<text><text>a</text><text>b</text></text>"));
        assert!(diagnostics.is_empty());
    }

    /// 内联 `<template name>` 写在单元自身（页面 / 组件）里时，样式按该单元的作用域求值。
    #[test]
    fn inline_template_in_unit_resolves_unit_styles() {
        let fixture = Fixture::wxml(
            "text-run/inline-template",
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
    fn renderer_conditional_parent_is_not_judged() {
        // recent-read：WebView 分支靠 `book-info-webview` 把 title/author 排成上下，
        // Skyline 分支靠默认值就成立，两端一致。
        let fixture = Fixture::wxml(
            "text-run/renderer-class",
            "<template name=\"t\"><view class=\"book-info {{ renderer === 'skyline' ? '' : 'book-info-webview' }}\"><text class=\"book-title\">a</text><text class=\"book-author\">b</text></view></template>",
        );
        let diagnostics = check(&fixture);
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn flex_parent_has_no_difference() {
        let fixture = template_with_style(
            "<view class=\"price\"><text>a</text><text>b</text></view>",
            ".price { display: flex; }",
        );
        assert!(check(&fixture).is_empty());
    }

    #[test]
    fn explicit_display_is_intentional() {
        let fixture = template_with_style(
            "<view><text class=\"run\">a</text><text class=\"run\">b</text></view>",
            ".run { display: inline; }",
        );
        assert!(check(&fixture).is_empty());
    }

    #[test]
    fn bare_text_is_left_to_no_inline_text() {
        let diagnostics = check(&template("<view>说明<text>a</text><text>b</text></view>"));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn horizontal_scroll_view_renders_as_row_flex() {
        let diagnostics = check(&template(
            "<scroll-view scroll-x><text>a</text><text>b</text></scroll-view>",
        ));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn wx_if_branches_never_coexist() {
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
}
