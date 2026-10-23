//! worklet 的调用边界：哪些 API 只能在 worklet 内调，哪些不能在 worklet 内调。
//!
//! 三条独立判定：
//! - `scrollViewContext.scrollTo` 不在 worklet 内（台账 #159）；
//! - worklet 内调用 `wx.*`（台账 #155）；
//! - 插件工程里的 `scrollViewContext.scrollTo`（台账 #171）。
//!
//! `worklet_scope` 由 `input/js.rs` 的访问者栈给出，取**最近**的外层函数：
//! 普通函数里嵌一个 worklet 函数，内层调用算 worklet；worklet 里再嵌普通函数，
//! 内层调用回到普通线程。
//!
//! 证据：
//! - `skyline-worklet/references/base/scroll-view-context.md:7`（「提供在 worklet
//!   函数内操作」）、`skyline-scroll-api/SKILL.md:109`（逻辑线程调用不生效）；
//! - `skyline-worklet/references/core/worklet-overview.md`（双线程，worklet 在 UI 线程）；
//!   `skyline-worklet/SKILL.md:132`（worklet 内不能直接调用 `wx.*`）；
//! - 插件限制：`skyline-scroll-api/references/api/worklet-scroll-context.md:3`
//!   （「小程序插件：**不支持**」）、`:96`，`skyline-scroll-api/SKILL.md:110`。

use crate::project::Project;
use crate::rules::{Rule, RuleMeta};
use crate::Severity;

use super::{Callee, at_span, is_plugin};

/// 只在 UI 线程（worklet）可用的方法。按段匹配：`worklet.scrollViewContext.scrollTo`
/// 算，`myscrollViewContext.scrollTo` 不算。
const SCROLL_TO: Callee = Callee::Suffix("scrollViewContext.scrollTo");

pub struct WorkletApiScope;

impl WorkletApiScope {
    pub const ID: &'static str = "skyline/worklet-api-scope";
}

impl Rule for WorkletApiScope {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Error,
            description: "scrollViewContext.scrollTo 必须在 worklet 内调用，worklet 内不能调用 wx.*",
        }
    }

    fn check(&self, project: &Project) -> Vec<crate::Diagnostic> {
        let plugin = is_plugin(project);
        let mut out = Vec::new();
        for file in &project.scripts {
            for call in file.doc.calls() {
                let scroll_to = SCROLL_TO.matches(&call.callee);
                if !scroll_to && call.worklet_scope && call.callee.starts_with("wx.") {
                    out.push(at_span(
                        Self::ID,
                        Severity::Error,
                        file,
                        call.start,
                        call.end,
                        format!(
                            "worklet 函数运行在 UI 线程，不能调用 `wx.*`（此时是 {}）",
                            call.callee
                        ),
                    ));
                }
                if !scroll_to {
                    continue;
                }
                // 插件里这个 API 整体不可用，比「不在 worklet 内」更根本，只报一条。
                if plugin {
                    out.push(at_span(
                        Self::ID,
                        Severity::Error,
                        file,
                        call.start,
                        call.end,
                        "插件内不支持 worklet.scrollViewContext.scrollTo",
                    ));
                } else if !call.worklet_scope {
                    out.push(at_span(
                        Self::ID,
                        Severity::Error,
                        file,
                        call.start,
                        call.end,
                        "`scrollViewContext.scrollTo` 只能在 worklet 函数内调用，当前在普通 JS 线程调用不会生效",
                    ));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;

    use crate::rules::testutil::js::{messages, plugin, single};
    use super::WorkletApiScope;

    const SCROLL_TO_OUTSIDE: &str =
        "function scroll() { scrollViewContext.scrollTo(this.ref.value, { top: 10 }); }";

    #[test]
    fn scroll_to_outside_worklet_is_reported() {
        let (_f, project) = single("scope-outside", SCROLL_TO_OUTSIDE);
        let diagnostics = WorkletApiScope.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].severity, crate::Severity::Error);
        assert!(diagnostics[0].message.contains("只能在 worklet 函数内调用"));
    }

    #[test]
    fn scroll_to_inside_worklet_is_accepted() {
        let (_f, project) = single(
            "scope-inside",
            "function scroll() { 'worklet'; scrollViewContext.scrollTo(this.ref.value, { top: 1 }); }",
        );
        assert_eq!(messages(&WorkletApiScope, &project), Vec::<String>::new());
    }

    #[test]
    fn scroll_to_through_optional_member_is_still_matched() {
        let (_f, project) = single(
            "scope-optional",
            "const fn = () => { worklet.scrollViewContext?.scrollTo(ref, { top: 1 }); };",
        );
        // 可选链的静态部分仍可解析，链尾照常命中；不在 worklet 内 → 仍报。
        assert_eq!(messages(&WorkletApiScope, &project).len(), 1);
    }

    #[test]
    fn wx_calls_inside_worklet_are_reported() {
        let (_f, project) = single(
            "scope-wx-inside",
            r#"
            function request() {
              'worklet';
              wx.request({ url: 'https://x' });
            }
            function shared() {
              'worklet';
              const value = wx.worklet.shared(0);
            }
            "#,
        );
        let diagnostics = WorkletApiScope.check(&project);
        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("wx.request"));
        // `wx.worklet.*` 的读取发生在 worklet 外，函数体内直接调也算违规。
        assert!(diagnostics[1].message.contains("wx.worklet.shared"));
    }

    #[test]
    fn wx_calls_outside_worklet_are_accepted() {
        let (_f, project) = single(
            "scope-wx-outside",
            r#"
            const value = wx.worklet.shared(0);
            wx.request({ url: 'https://x' });
            function normal() { wx.showToast({ title: 'x' }); }
            "#,
        );
        assert_eq!(messages(&WorkletApiScope, &project), Vec::<String>::new());
    }

    #[test]
    fn nested_worklet_uses_the_nearest_scope() {
        let (_f, project) = single(
            "scope-nested",
            r#"
            function outer() {
              scrollViewContext.scrollTo(ref, { top: 1 });
              const animated = () => {
                'worklet';
                scrollViewContext.scrollTo(ref, { top: 2 });
                const normal = () => {
                  scrollViewContext.scrollTo(ref, { top: 3 });
                };
              };
            }
            "#,
        );
        let diagnostics = WorkletApiScope.check(&project);
        // 外层普通函数、worklet 内嵌普通箭头各一条；worklet 内那条不报。
        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].line, 3);
        assert_eq!(diagnostics[1].line, 8);
    }

    /// 链尾两段分别精确匹配：同名前缀不得误命中。
    #[test]
    fn similarly_named_receiver_is_not_matched() {
        let (_f, project) = single(
            "scope-similar",
            "function scroll() { myscrollViewContext.scrollTo(ref, { top: 1 }); }",
        );
        assert_eq!(messages(&WorkletApiScope, &project), Vec::<String>::new());
    }

    /// 插件里这个 API 整体不可用：同一处只报一条。
    #[test]
    fn plugin_reports_one_diagnostic_per_call() {
        let (_f, project) = plugin("scope-plugin-one", SCROLL_TO_OUTSIDE);
        assert_eq!(messages(&WorkletApiScope, &project).len(), 1);
    }

    #[test]
    fn plugin_project_reports_the_plugin_restriction() {
        let (_f, project) = plugin("scope-plugin", SCROLL_TO_OUTSIDE);
        let plugin_messages = messages(&WorkletApiScope, &project);
        assert!(
            plugin_messages
                .iter()
                .any(|m| m == "插件内不支持 worklet.scrollViewContext.scrollTo"),
            "{plugin_messages:?}"
        );
        // 同一份代码在普通工程里只有 #159 一条。
        let (_normal, normal) = single("scope-normal", SCROLL_TO_OUTSIDE);
        assert_eq!(
            messages(&WorkletApiScope, &normal),
            ["`scrollViewContext.scrollTo` 只能在 worklet 函数内调用，当前在普通 JS 线程调用不会生效"]
        );
    }

    #[test]
    fn plugin_project_still_checks_scope_and_wx() {
        let (_f, project) = plugin(
            "scope-plugin-wx",
            "function f() { 'worklet'; wx.request({}); }",
        );
        let messages = messages(&WorkletApiScope, &project);
        assert_eq!(messages.len(), 1, "{messages:?}");
        assert!(messages[0].contains("wx.request"));
    }
}
