//! 并列的文本级元素在 WebView 下排在同一行，在 Skyline 下各自换行。
//!
//! WebView 里 `<text>` 是 inline、`<image>` 是 inline-block，block 容器里几个并列的
//! 文本级元素走**同一个行内流**；Skyline 里它们默认是 block（`display.cc:43-50`：
//! 默认 block 或 flex），各自占一行。
//!
//! 想要「一段共享排版的文本流」只能让 TextShadowNode 当最近容器：文本子节点变
//! `TextSpan`、非文本子节点变 `WidgetSpan`，全部 `BuildInlineSpan` 进同一个段落
//! （`shadow_node.cc:205-209`、`text_span.cc:280-285`、`text.cc:188-201`）。
//! 只想「横向排列」就显式 `display: flex`——两种渲染器都按 flex 排，天然一致。
//!
//! 裸文本与元素混排归 `no-inline-text`，这里只补「没有裸文本」的那一半：兄弟全是
//! 被当成 inline 的元素。父级是 flex/grid 时不报（两边都按 flex item 排，无差异）；
//! 元素显式写了非 inline 的 `display` 时也不报（作者已经明确要块级）。

use crate::diag::{Diagnostic, Severity};
use crate::input::wxml::{WxmlDoc, WxmlElement, WxmlNode};
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};

use crate::matcher::Tree;

use super::{display_values, element_diagnostic, is_flex_value, visit, walk};

pub const ID: &str = "skyline/inline-run";

/// 可以作为行内流成员的标签：WebView 下 `text` 是 inline，`image` / `span` 是 inline-block。
const INLINE_TAGS: [&str; 3] = ["text", "image", "span"];

/// TextShadowNode：它们内部的子节点本来就共用一个段落。
const TEXT_TAGS: [&str; 3] = ["text", "span", "navigator"];

pub struct InlineRun;

impl Rule for InlineRun {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Info,
            description: "WebView 下并排的文本级元素在 Skyline 下会各自换行",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit(project, |doc, out| {
            let tree = Tree::new(doc);
            walk(doc, |element, _ancestors| {
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
    if TEXT_TAGS.contains(&element.tag.as_str()) {
        return;
    }
    // `<template>` 定义里的元素在实例化时才接上外层祖先，样式表里的后代选择器
    // 在这里一律匹配不到（`display_values` 会返回空）——与其误报，不如不判。
    if element.in_template || doc.fragment {
        return;
    }
    // 内联 `style` 是动态值时 display 未知，宁可漏报。
    if has_dynamic_style(element) {
        return;
    }
    // 父级是 flex/grid 时两边都把子节点当 flex/grid item，没有差异。
    if display_values(project, tree, element)
        .iter()
        .any(|value| is_flex_value(value))
    {
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
fn is_inline_member(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> bool {
    INLINE_TAGS.contains(&element.tag.as_str())
        && display_values(project, tree, element).is_empty()
        && !has_dynamic_style(element)
        && !is_out_of_flow(project, tree, element)
        && !stacks_vertically(project, tree, element)
}

fn is_out_of_flow(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> bool {
    super::resolved_property(project, tree, element, "position").is_some_and(|resolved| {
        matches!(
            resolved.value.trim().to_ascii_lowercase().as_str(),
            "fixed" | "absolute"
        )
    })
}

fn stacks_vertically(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> bool {
    ["width", "margin-top", "margin-bottom"]
        .iter()
        .filter_map(|property| super::resolved_property(project, tree, element, property))
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

/// 两个元素是否分属同一个 `wx:if` 的不同分支（不会同时渲染）。
fn exclusive(left: &WxmlElement, right: &WxmlElement) -> bool {
    left.branches.iter().any(|a| {
        right
            .branches
            .iter()
            .any(|b| a.group == b.group && a.arm != b.arm)
    })
}

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
        out.push(
            element_diagnostic(
                ID,
                doc,
                run[0],
                doc.path(),
                Severity::Info,
                format!(
                    "{} 个文本级元素并列：WebView 下排在同一行，Skyline 下各自换行",
                    run.len()
                ),
            )
            .help("需要共享排版的一段文本时用 `<text>` / `<span>` 当父节点；只需要横向排列时写 `display: flex; flex-direction: row`"),
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
}
