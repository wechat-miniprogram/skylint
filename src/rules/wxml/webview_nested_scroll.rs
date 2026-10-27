//! `nested-scroll-*` 与 `scroll-view type="nested"` 在 WebView 下不联动。
//!
//! 嵌套滚动是 Skyline 新增能力（`skyline/shadow/components/nested_scroll_*`），
//! WebView 的 `wx-scroll-view` 从不读 `type`（`wxapplib/src/wx-component/src/wx-scroll-view/wx-scroll-view.js:239`），
//! `nested-scroll-header` / `nested-scroll-body` 是未注册标签 → 未知元素：
//! 子节点还在，但头部不会随主体滚动收起，而是常驻在顶部。
//!
//! 两端兼容：给整块加 `wx:if="{{renderer === 'skyline'}}"`，WebView 分支改用
//! 普通的 `scroll-view` + 固定头部（或用 `page-container`）。

use crate::diag::{Diagnostic, Severity};
use crate::project::Project;
use crate::rules::{Rule, RuleMeta, Target};

use super::{element_diagnostic, structure::find_attr, visit, walk};

pub const ID: &str = "webview/nested-scroll";

/// 只存在于 Skyline 的嵌套滚动标签。
const TAGS: [&str; 2] = ["nested-scroll-header", "nested-scroll-body"];

pub struct NestedScroll;

impl Rule for NestedScroll {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Warning,
            description: "嵌套滚动只在 Skyline 下生效，WebView 下头部不联动、常驻顶部",
        }
    }

    fn target(&self) -> Target {
        Target::Webview
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit(project, |doc, out| {
            walk(doc, |element, _| {
                if element.renderer_guarded() {
                    return;
                }
                let tag = element.tag.to_ascii_lowercase();
                let nested_type = tag == "scroll-view"
                    && find_attr(element, "type").and_then(|attr| attr.value.as_deref())
                        == Some("nested");
                if !nested_type && !TAGS.contains(&tag.as_str()) {
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
                            "`{tag}` 的嵌套滚动是 Skyline 新增能力，WebView 下头部不会随主体联动"
                        ),
                    )
                    .help("给整块加 `wx:if=\"{{renderer === 'skyline'}}\"`，WebView 分支用普通 `scroll-view` + 固定头部"),
                );
            });
        })
    }
}

#[cfg(test)]
mod tests {
    use super::NestedScroll;
    use crate::rules::testutil::wxml::{check_rule, project};

    #[test]
    fn nested_type_is_reported() {
        let diagnostics = check_rule(
            &project(
                "webview/nested-type",
                "<scroll-view type=\"nested\" scroll-y><nested-scroll-header><view/></nested-scroll-header><nested-scroll-body><view/></nested-scroll-body></scroll-view>",
            ),
            &NestedScroll,
        );
        assert_eq!(diagnostics.len(), 3);
    }

    #[test]
    fn guarded_block_is_skipped() {
        let diagnostics = check_rule(
            &project(
                "webview/nested-guard",
                "<scroll-view wx:if=\"{{renderer === 'skyline'}}\" type=\"nested\"><nested-scroll-header><view/></nested-scroll-header></scroll-view>",
            ),
            &NestedScroll,
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn plain_scroll_view_is_fine() {
        let diagnostics = check_rule(
            &project(
                "webview/nested-none",
                "<scroll-view scroll-y><view/></scroll-view>",
            ),
            &NestedScroll,
        );
        assert!(diagnostics.is_empty());
    }
}
