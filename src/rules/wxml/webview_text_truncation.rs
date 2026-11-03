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
use crate::rules::wxml::webview_truncates;

use super::{element_diagnostic, structure::find_attr, visit, walk};

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
                let attributes = truncating_attributes(element);
                if attributes.is_empty() || webview_truncates(project, &tree, element, ancestors) {
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
    }
}

/// 元素上真正会起截断作用的属性。
///
/// `max-lines="0"` / `overflow="visible"` / `overflow="scroll"` 在两个渲染器下
/// 表现一致，不算截断。动态值（`max-lines="{{n}}"`）按可能截断处理。
fn truncating_attributes(element: &WxmlElement) -> Vec<&'static str> {
    let mut out = Vec::new();
    if let Some(attr) = find_attr(element, "max-lines") {
        let meaningful = match attr.value.as_deref() {
            Some(value) => value.trim().parse::<u32>().is_ok_and(|lines| lines > 0),
            // 动态取值：可能是任何行数。
            None => true,
        };
        if meaningful {
            out.push("max-lines");
        }
    }
    if let Some(attr) = find_attr(element, "overflow") {
        let meaningful = match attr.value.as_deref() {
            Some(value) => matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "ellipsis" | "hidden" | "clip"
            ),
            None => true,
        };
        if meaningful {
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
    fn overflow_is_reported() {
        let diagnostics = check(&project(
            "webview/overflow",
            "<view><text overflow=\"ellipsis\">a</text></view>",
        ));
        assert_eq!(diagnostics.len(), 1);
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
}
