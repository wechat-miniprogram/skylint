//! 静态写死 `renderer="skyline"`：回退到 WebView 时子组件仍走 Skyline 分支。
//!
//! 页面/组件把渲染后端当静态 prop 传给子组件（`renderer="skyline"`）后，子组件里
//! `wx:if="{{renderer === 'skyline'}}"` 这类分支在 WebView 下依然成立——它们渲染的是
//! 只有在 Skyline 里才正确/才存在的结构与样式（`<text>` 里塞组件、`max-lines` 截断等）。
//! 而真实的渲染后端是 `this.renderer`，只有 `renderer="{{renderer}}"` 才跟着变。
//!
//! 真实修复形态（`weapp-novel-plugin_1` 的 `feat-glass-easel-webview: 59a2d74f`）：
//! 6 处 `renderer="skyline"` 全部改成 `renderer="{{ renderer }}"`，同时页面
//! `data.renderer` 也改成默认 `'webview'`、`onLoad` 里用 `this.renderer` 覆盖。

use crate::diag::Severity;
use crate::project::Project;
use crate::rules::{Rule, RuleMeta, Target};

use super::{element_diagnostic, structure::find_attr, visit, walk};

pub const ID: &str = "webview/static-renderer";

pub struct StaticRenderer;

impl Rule for StaticRenderer {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Warning,
            description: "静态写死 renderer=\"skyline\"：回退 WebView 时子组件仍走 Skyline 分支",
        }
    }

    fn target(&self) -> Target {
        Target::Webview
    }

    fn check(&self, project: &Project) -> Vec<crate::Diagnostic> {
        visit(project, |doc, out| {
            walk(doc, |element, _| {
                if element.renderer_guarded() {
                    return;
                }
                let Some(attr) = find_attr(element, "renderer") else {
                    return;
                };
                if !attr.is_static || attr.value.as_deref() != Some("skyline") {
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
                            "`<{}>` 静态传入 `renderer=\"skyline\"`：回退到 WebView 时它仍按 Skyline 渲染，里面依赖 Skyline 的结构会失效",
                            element.tag
                        ),
                    )
                    .help("改成 `renderer=\"{{renderer}}\"`，让子组件拿到真实渲染后端；页面 data 里也要用 `this.renderer` 覆盖默认值"),
                );
            });
        })
    }
}

#[cfg(test)]
mod tests {
    use super::StaticRenderer;
    use crate::rules::testutil::wxml::{check_rule, project};

    #[test]
    fn static_skyline_prop_is_reported() {
        let diagnostics = check_rule(
            &project(
                "webview/static-renderer",
                "<view><seg-comment-badge renderer=\"skyline\" count=\"2\"/></view>",
            ),
            &StaticRenderer,
        );
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("seg-comment-badge"));
    }

    #[test]
    fn dynamic_renderer_prop_is_fine() {
        let diagnostics = check_rule(
            &project(
                "webview/static-renderer-dynamic",
                "<view><seg-comment-badge renderer=\"{{renderer}}\"/></view>",
            ),
            &StaticRenderer,
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn webview_literal_is_fine() {
        let diagnostics = check_rule(
            &project(
                "webview/static-renderer-webview",
                "<view><seg-comment-badge renderer=\"webview\"/></view>",
            ),
            &StaticRenderer,
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn inside_a_renderer_branch_it_is_intentional() {
        let diagnostics = check_rule(
            &project(
                "webview/static-renderer-guarded",
                "<view><seg-comment-badge wx:if=\"{{renderer === 'skyline'}}\" renderer=\"skyline\"/></view>",
            ),
            &StaticRenderer,
        );
        assert!(diagnostics.is_empty());
    }
}
