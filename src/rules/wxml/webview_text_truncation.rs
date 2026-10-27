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
use crate::input::wxml::{WxmlDoc, WxmlElement};
use crate::project::Project;
use crate::rules::{Rule, RuleMeta, Target};

use super::{element_diagnostic, sheet_covers, structure::find_attr, visit, walk};

pub const ID: &str = "webview/text-truncation";

/// WebView 分支常用的截断兜底标记。
const CLAMP_FALLBACK: &str = "-webkit-line-clamp";

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
            walk(doc, |element, _| {
                if !element.tag.eq_ignore_ascii_case("text") || element.renderer_guarded() {
                    return;
                }
                let attributes: Vec<&str> = ["max-lines", "overflow"]
                    .into_iter()
                    .filter(|name| find_attr(element, name).is_some())
                    .collect();
                if attributes.is_empty() || has_clamp_fallback(project, doc, element) {
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

/// 元素上是否已经有一处 `-webkit-line-clamp` 兜底（样式表按 class 文本命中的，
/// 或内联 `style` 里写的）。
fn has_clamp_fallback(project: &Project, doc: &WxmlDoc, element: &WxmlElement) -> bool {
    if find_attr(element, "style").is_some_and(|attr| {
        attr.value
            .as_deref()
            .is_some_and(|style| style.contains(CLAMP_FALLBACK))
    }) {
        return true;
    }
    let classes = classes_of(element);
    if classes.is_empty() {
        return false;
    }
    project.styles.iter().any(|sheet| {
        sheet_covers(sheet, project, doc.path())
            && sheet.doc.rules.iter().any(|rule| {
                rule.declarations
                    .iter()
                    .any(|decl| decl.name.eq_ignore_ascii_case(CLAMP_FALLBACK))
                    && rule.selectors.iter().any(|selector| {
                        classes
                            .iter()
                            .any(|class| selector.text.contains(class.as_str()))
                    })
            })
    })
}

/// 元素 `class` 里静态可知的词。
fn classes_of(element: &WxmlElement) -> Vec<String> {
    let Some(attribute) = find_attr(element, "class") else {
        return Vec::new();
    };
    let mut classes: Vec<String> = attribute
        .class_tokens
        .as_ref()
        .map(|tokens| tokens.definite.clone())
        .unwrap_or_default();
    classes.extend(
        attribute
            .value
            .as_deref()
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_string),
    );
    classes
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
            ".webview .desc { display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; overflow: hidden; }",
        );
        assert!(check(&fixture).is_empty());
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
