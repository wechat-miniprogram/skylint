//! 文本与元素混排时 Skyline 会把文本当成独立 flex item，需用 `<text>` 包裹。

use std::collections::HashSet;
use std::fs;

use crate::diag::{Diagnostic, Severity};
use crate::input::wxml::{self, WxmlDoc, WxmlElement, WxmlNode};
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};

pub const ID: &str = "skyline/no-inline-text";

/// 文本与元素混排时唯一允许直接承载文本的标签。
///
/// 依据：`skyline-components` 的 MUST 规则「内联文本必须用 text 组件」与
/// 「text 组件内只能嵌套 text」；`span` 只是图文混排的内联容器，其中的文字
/// 示例仍然用 `<text>` 包裹，因此不在白名单内。
const TEXT_TAG: &str = "text";

pub struct NoInlineText;

impl Rule for NoInlineText {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Warning,
            description: "Skyline 下文本与元素混排会变成独立 flex item，需用 `<text>` 包裹",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let wxml_files = project
            .pages
            .iter()
            .filter_map(|page| page.files.wxml.as_deref())
            .chain(
                project
                    .components
                    .iter()
                    .filter_map(|component| component.files.wxml.as_deref()),
            )
            // 模板不属于任何单元，但同样可能把文字与元素混排。
            .chain(project.template_files.iter().map(String::as_str));

        let mut seen = HashSet::new();
        let mut diagnostics = Vec::new();
        for path in wxml_files {
            if !seen.insert(path) {
                continue;
            }
            // 单文件读取/解析失败只跳过该单元，绝不终止整次扫描。
            let Ok(source) = fs::read_to_string(project.abs(path)) else {
                continue;
            };
            let Ok(doc) = wxml::parse(path, &source) else {
                continue;
            };
            collect(&doc, &doc.nodes, path, &mut diagnostics);
        }
        diagnostics
    }
}

fn collect(doc: &WxmlDoc, nodes: &[WxmlNode], path: &str, out: &mut Vec<Diagnostic>) {
    for node in nodes {
        let WxmlNode::Element(element) = node else {
            continue;
        };
        check_element(doc, element, path, out);
        collect(doc, &element.children, path, out);
    }
}

/// 只有「文本与元素混排」在 Skyline 下会静默错位：纯文本子节点仍按 inline 流渲染。
/// `<text>` 内部允许文本与 `<text>` 子节点混排，整棵子树豁免。
fn check_element(doc: &WxmlDoc, element: &WxmlElement, path: &str, out: &mut Vec<Diagnostic>) {
    if element.tag == TEXT_TAG {
        return;
    }
    let has_element = element
        .children
        .iter()
        .any(|child| matches!(child, WxmlNode::Element(_)));
    if !has_element {
        return;
    }
    for text in element.children.iter().filter_map(|child| match child {
        WxmlNode::Text(text) if text.is_meaningful() => Some(text),
        _ => None,
    }) {
        let (line, column) = doc.position(text.start);
        let (end_line, end_column) = doc.position(text.end);
        out.push(
            Diagnostic::new(
                ID,
                Severity::Warning,
                format!(
                    "`<{}>` 内文本与元素混排，Skyline 会把文本当成独立 flex item，导致错位",
                    element.tag
                ),
                path,
                line,
                column,
            )
            .end(end_line, end_column)
            .help("把文本包进 `<text>`，或把整段文字移入独立的 `<text>` 子节点"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::testutil::{Fixture, check_rule, project};
    use super::NoInlineText;

    fn check(fixture: &Fixture) -> Vec<crate::Diagnostic> {
        check_rule(fixture, &NoInlineText)
    }

    // ---- positive ----

    #[test]
    fn text_and_view_mixed() {
        let diagnostics = check(&project("mixed", "<view>Hello <view>World</view></view>"));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (1, 7));
        assert_eq!(diagnostics[0].end_column, 12);
    }

    #[test]
    fn interpolation_and_text_child_mixed() {
        let diagnostics = check(&project("interp", "<view>{{a}}<text>b</text></view>"));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (1, 7));
    }

    #[test]
    fn text_and_image_mixed() {
        let diagnostics = check(&project("image", "<view>文字<image/></view>"));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (1, 7));
    }

    #[test]
    fn block_text_and_element_mixed_reports_text_position() {
        let diagnostics = check(&project(
            "block",
            "<view>\n  <block wx:for=\"{{list}}\">{{item}}<image/></block>\n</view>",
        ));
        assert_eq!(diagnostics.len(), 1);
        // 报在文本节点上，不是 block 上。
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (2, 28));
    }

    #[test]
    fn every_text_child_is_reported_separately() {
        let diagnostics = check(&project("multiple", "<view>a<text>b</text>c</view>"));
        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].column, 7);
        assert_eq!(diagnostics[1].column, 22);
    }

    #[test]
    fn text_after_element_is_reported() {
        let diagnostics = check(&project("nested", "<view><view/>尾部</view>"));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].column, 14);
    }

    #[test]
    fn mixed_text_with_surrounding_whitespace() {
        let diagnostics = check(&project("ws", "<view>\n  hello\n  <view/>\n</view>"));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (2, 3));
    }

    #[test]
    fn text_mixed_inside_wx_if() {
        let diagnostics = check(&project("if", "<view wx:if=\"{{ok}}\">yes<view/></view>"));
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn emoji_before_text_uses_character_column() {
        let diagnostics = check(&project("emoji", "<view>😀hi<image/></view>"));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (1, 7));
        assert_eq!(diagnostics[0].end_column, 10);
    }

    // ---- negative ----

    #[test]
    fn whitespace_only_is_ignored() {
        let diagnostics = check(&project("blank", "<view>\n  <view/>\n</view>"));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn pure_static_text_is_allowed() {
        let diagnostics = check(&project("pure-static", "<view>静态文本</view>"));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn pure_interpolation_text_is_allowed() {
        let diagnostics = check(&project("pure-interp", "<view>{{a}}</view>"));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn multiple_pure_text_nodes_are_allowed() {
        let diagnostics = check(&project("pure-multi", "<view>a{{b}}c</view>"));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn text_inside_text_is_allowed() {
        let diagnostics = check(&project("text", "<text>hello</text>"));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn nested_text_is_allowed() {
        let diagnostics = check(&project("text-nested", "<text>a<text>b</text></text>"));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn text_wrapped_in_view_is_allowed() {
        let diagnostics = check(&project("wrapped", "<view><text>hello</text></view>"));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn comments_are_ignored() {
        let diagnostics = check(&project("comment", "<view><!-- hi --></view>"));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn text_with_comment_is_not_mixed() {
        let diagnostics = check(&project("text-comment", "<view>hi<!-- c --></view>"));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn only_elements_are_allowed() {
        let diagnostics = check(&project("only-elements", "<view><view/><image/></view>"));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn text_element_with_element_child_is_exempt() {
        // `<text>` 内只能嵌 `<text>` 是另一条规则（text-only-children），这里不报。
        let diagnostics = check(&project("text-exempt", "<text>a<image/></text>"));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn self_closing_tags_are_ignored() {
        let diagnostics = check(&project("self-closing", "<view><image/></view>"));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn pure_text_inside_structural_wrappers_is_allowed() {
        let diagnostics = check(&project(
            "wrapper-ok",
            "<view wx:if=\"{{ok}}\">yes</view>\n<block wx:for=\"{{list}}\">{{item}}</block>",
        ));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn span_still_requires_text() {
        let diagnostics = check(&project("span", "<span><text>a</text><image/></span>"));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn webview_page_is_checked_too() {
        // 迁移工具的受众就是还没上 Skyline 的工程，未迁移的页面不能跳过。
        let fixture = Fixture::new("webview");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"webview"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write(
                "pages/index/index.wxml",
                "<view>hello <view>world</view></view>",
            );
        assert_eq!(check(&fixture).len(), 1);
    }

    // ---- resilience ----

    #[test]
    fn broken_file_does_not_stop_other_files() {
        let fixture = Fixture::new("broken");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/bad/bad","pages/good/good"],"renderer":"skyline"}"#,
            )
            .write("pages/bad/bad.json", "{}")
            .write("pages/bad/bad.wxml", "<view>{{ a</view>")
            .write("pages/good/good.json", "{}")
            .write("pages/good/good.wxml", "<view>hello<image/></view>");

        let diagnostics = check(&fixture);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].path, "pages/good/good.wxml");
    }
}
