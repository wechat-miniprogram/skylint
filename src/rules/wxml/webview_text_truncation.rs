//! `<text>` 的截断属性在 WebView 下完全无效。
//!
//! `max-lines` / `overflow` 是 TextShadowNode 自己的属性
//! （`skyline/shadow/components/text_shadow_node.cc:305-306`），WebView 的
//! `wx-text` 只声明 `selectable / user-select / decode / space`
//! （`wxapplib/src/wx-component/src/wx-text/wx-text.js:131-173`），既不改样式也不报错：
//! `max-lines` 被丢掉、`overflow` 不是 HTML 表现属性 → 长文本原样铺开，多行截断与
//! 单行省略都不发生。
//!
//! 两端兼容写法：保留属性给 Skyline，另加 WebView 分支的 `-webkit-line-clamp`
//! （`display: -webkit-box` + `-webkit-box-orient: vertical` + `overflow: hidden`）。
//! Skyline 不支持这个 `display`，正好互相忽略。同一元素上已有这类兜底时不再报。

use crate::diag::{Diagnostic, Severity};
use crate::input::wxml::WxmlElement;
use crate::matcher::Tree;
use crate::project::Project;
use crate::rules::Rule;
use crate::rules::RuleMeta;
use crate::rules::Target;
use crate::rules::wxml::resolved_property;

use super::{ancestry, element_diagnostic, structure::find_attr, visit, walk};

pub const ID: &str = "webview/text-truncation";

pub struct TextTruncation;

impl Rule for TextTruncation {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Warning,
            description: "`<text>` 的 max-lines / overflow 在 WebView 下被忽略，截断与省略失效",
        }
    }

    fn target(&self) -> Target {
        Target::Webview
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        ancestry::with_views(project, |views| {
            visit(project, |doc, out| {
                let tree = Tree::new(doc);
                walk(doc, |element, ancestors| {
                    if !element.tag.eq_ignore_ascii_case("text") {
                        return;
                    }
                    if element.skyline_only() {
                        return;
                    }
                    // 只看真正起截断作用的写法：`overflow="visible"`、`max-lines="0"`
                    // 在两个渲染器下表现一致，报出来是噪音。
                    let attributes = truncating_attributes(project, &tree, element);
                    if attributes.is_empty()
                        || ancestry::webview_truncates(views, &tree, element, ancestors)
                            != Some(false)
                    {
                        return;
                    }
                    let written = attributes
                        .iter()
                        .map(|name| format!("`{name}`"))
                        .collect::<Vec<_>>()
                        .join(" / ");
                    out.push(
                    element_diagnostic(
                        ID,
                        doc,
                        element,
                        doc.path(),
                        Severity::Warning,
                        format!("{written} 只在 Skyline 的 `<text>` 上生效，WebView 下被忽略：文本不再截断"),
                    )
                    .help(
                        "保留属性给 Skyline，另加 WebView 分支：多行用 `display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: N; overflow: hidden`，单行用 `white-space: nowrap; overflow: hidden; text-overflow: ellipsis`",
                    ),
                );
                });
            })
        })
    }
}

/// 元素上真正会起截断作用的属性。
///
/// `max-lines="0"` / `overflow="visible"` / `overflow="scroll"` / `overflow="hidden"`
/// 在两个渲染器下表现一致，不算截断。动态值（`max-lines="{{n}}"`）按可能截断处理。
///
/// `overflow` 单独出现时还要看 `white-space`：Skyline 默认允许折行，只有元素不换行
/// （`nowrap`）时 `clip` / `ellipsis` 才有可裁掉的内容。转换器也把 `hidden` 当
/// `visible` 处理（`text_shadow_node.cc:266-285`），故它一律不算。
fn truncating_attributes(
    project: &Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
) -> Vec<&'static str> {
    let mut out = Vec::new();
    let max_lines = find_attr(element, "max-lines").is_some_and(|attr| {
        match attr.value.as_deref() {
            Some(value) => value.trim().parse::<u32>().is_ok_and(|lines| lines > 0),
            // 动态取值：可能是任何行数。
            None => true,
        }
    });
    if max_lines {
        out.push("max-lines");
    }
    if let Some(attr) = find_attr(element, "overflow") {
        let meaningful = match attr.value.as_deref() {
            Some(value) => matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "ellipsis" | "clip"
            ),
            None => true,
        };
        let nowrap = resolved_property(project, tree, element, "white-space")
            .is_some_and(|resolved| resolved.value.trim().eq_ignore_ascii_case("nowrap"));
        if meaningful && (max_lines || nowrap) {
            out.push("overflow");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::TextTruncation;
    use crate::rules::testutil::wxml::{Fixture, check_rule, project};

    fn check(fixture: &Fixture) -> Vec<crate::Diagnostic> {
        check_rule(fixture, &TextTruncation)
    }

    #[test]
    fn max_lines_is_reported() {
        let diagnostics = check(&project(
            "webview/max-lines",
            "<view><text max-lines=\"2\">a</text></view>",
        ));
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("max-lines"));
    }

    #[test]
    fn nowrap_overflow_is_reported() {
        let fixture = Fixture::page(
            "webview/overflow-nowrap",
            "<view><text class=\"desc\" overflow=\"ellipsis\">a</text></view>",
            ".desc { white-space: nowrap; }",
        );
        let diagnostics = check(&fixture);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(diagnostics[0].message.contains("overflow"));
    }

    /// Skyline 默认允许折行，`overflow="ellipsis"` 单独出现时没有可裁的内容，
    /// WebView 不认这个属性也不产生差异。
    #[test]
    fn overflow_without_nowrap_is_fine() {
        let diagnostics = check(&project(
            "webview/overflow-wrap",
            "<view><text overflow=\"ellipsis\">a</text></view>",
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn overflow_hidden_is_fine() {
        // 转换器把 `hidden` 落到 `kVisible`：两个渲染器都不截断。
        let diagnostics = check(&project(
            "webview/overflow-hidden",
            "<view><text overflow=\"hidden\">a</text><text class=\"desc\" overflow=\"hidden\">b</text></view>",
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn max_lines_with_overflow_reports_both() {
        let fixture = Fixture::page(
            "webview/max-lines-overflow",
            "<view><text max-lines=\"2\" overflow=\"ellipsis\">a</text></view>",
            "text { white-space: normal; }",
        );
        let diagnostics = check(&fixture);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(diagnostics[0].message.contains("max-lines"));
        assert!(diagnostics[0].message.contains("overflow"));
    }

    #[test]
    fn plain_text_is_fine() {
        let diagnostics = check(&project("webview/plain", "<view><text>a</text></view>"));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn clamp_fallback_silences() {
        let fixture = project(
            "webview/clamp",
            "<view><text class=\"desc\" max-lines=\"2\">a</text></view>",
        );
        fixture.write(
            "pages/index/index.wxss",
            ".desc { display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; overflow: hidden; }",
        );
        assert!(check(&fixture).is_empty());
    }

    #[test]
    fn single_line_fallback_silences() {
        let fixture = project(
            "webview/single-line",
            "<view><text class=\"desc\" overflow=\"ellipsis\">a</text></view>",
        );
        fixture.write(
            "pages/index/index.wxss",
            ".desc { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }",
        );
        assert!(check(&fixture).is_empty());
    }

    #[test]
    fn similar_class_name_does_not_silence() {
        // 旧实现用类名字串包含：`.subtitle` 会把 `class="title"` 误消音。
        let fixture = project(
            "webview/prefix-class",
            "<view><text class=\"title\" max-lines=\"2\">a</text></view>",
        );
        fixture.write(
            "pages/index/index.wxss",
            ".subtitle { -webkit-line-clamp: 2; }",
        );
        assert_eq!(check(&fixture).len(), 1);
    }

    #[test]
    fn non_truncating_values_are_fine() {
        let diagnostics = check(&project(
            "webview/visible",
            "<view><text overflow=\"visible\">a</text><text max-lines=\"0\">b</text></view>",
        ));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn renderer_guard_silences() {
        let diagnostics = check(&project(
            "webview/guard",
            "<view><text wx:if=\"{{renderer === 'skyline'}}\" max-lines=\"2\">a</text></view>",
        ));
        assert!(diagnostics.is_empty());
    }

    /// 模板定义体的真实祖先在实例化处：`<view class="clamp">` 截断时不再误报。
    #[test]
    fn template_body_sees_instantiation_ancestor_clamp() {
        let fixture = Fixture::new("webview/template-ancestor");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}""#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write(
                "pages/index/index.wxml",
                r#"<import src="/tpls/t"/><view class="clamp"><template is="t"/></view>"#,
            )
            .write(
                "pages/index/index.wxss",
                ".clamp { display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; overflow: hidden; }",
            )
            .write(
                "tpls/t.wxml",
                r#"<template name="t"><text max-lines="2">a</text></template>"#,
            );
        let diagnostics = check(&fixture);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }
}
