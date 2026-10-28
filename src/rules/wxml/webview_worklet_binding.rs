//! WXML 的 `worklet:` 事件绑定在 WebView 下静默不触发。
//!
//! 模板编译把 `worklet:ongesture` 编成 `wl(...)`，只挂一个 lifetime 钩子；唯一消费方是
//! Skyline 组件的 `worklet-handler` behavior（`skyline/src/components/*/index.ts`），
//! WebView 侧的 `wx-component` 完全没有对应实现——既不报错也不退化成普通事件，
//! 回调永远不会被调用（`glass-easel-template-compiler` 的 proc_gen 只产出 `wl` 标记）。
//!
//! 两端兼容：同时挂普通 `bind:xxx`，或用 `wx:if="{{renderer === 'skyline'}}"` 把
//! worklet 绑定隔离在 Skyline 分支里。注意 `if (wx.worklet)` 不能当守卫——
//! 这个 API 在 WebView 下也存在（有 JS 模拟实现），只是 `worklet:` 绑定没消费方。

use crate::diag::{Diagnostic, Severity};
use crate::project::Project;
use crate::rules::{Rule, RuleMeta, Target};

use super::{span_diagnostic, visit, walk};

pub const ID: &str = "webview/worklet-binding";

pub struct WorkletBinding;

impl Rule for WorkletBinding {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Error,
            description: "`worklet:` 事件绑定在 WebView 下不触发回调，且不报错",
        }
    }

    fn target(&self) -> Target {
        Target::Webview
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit(project, |doc, out| {
            walk(doc, |element, _| {
                if element.skyline_only() {
                    return;
                }
                for attribute in &element.attributes {
                    if !attribute.name.starts_with("worklet:") {
                        continue;
                    }
                    out.push(
                        span_diagnostic(
                            ID,
                            doc,
                            attribute.start,
                            attribute.end,
                            doc.path(),
                            Severity::Error,
                            format!(
                                "`{}` 在 WebView 下没有消费方，回调不会触发也不会报错",
                                attribute.name
                            ),
                        )
                        .help("同时挂普通 `bind:` 事件，或用 `wx:if=\"{{renderer === 'skyline'}}\"` 隔离 worklet 绑定"),
                    );
                }
            });
        })
    }
}

#[cfg(test)]
mod tests {
    use super::WorkletBinding;
    use crate::rules::testutil::wxml::{check_rule, project};

    #[test]
    fn worklet_binding_is_reported() {
        let diagnostics = check_rule(
            &project(
                "webview/worklet",
                "<pan-gesture-handler worklet:ongesture=\"onPan\"/>",
            ),
            &WorkletBinding,
        );
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("worklet:ongesture"));
    }

    #[test]
    fn renderer_guard_silences() {
        let diagnostics = check_rule(
            &project(
                "webview/worklet-guard",
                "<view wx:if=\"{{renderer === 'skyline'}}\"><pan-gesture-handler worklet:ongesture=\"onPan\"/></view>",
            ),
            &WorkletBinding,
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn plain_binding_is_fine() {
        let diagnostics = check_rule(
            &project(
                "webview/worklet-none",
                "<view bind:tap=\"onTap\"><text>a</text></view>",
            ),
            &WorkletBinding,
        );
        assert!(diagnostics.is_empty());
    }
}
