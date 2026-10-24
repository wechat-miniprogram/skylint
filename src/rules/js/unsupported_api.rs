//! Skyline 不支持的 JS API / 页面配置项。
//!
//! 条目直接写在 [`ENTRIES`] 里，Rust 侧只做两种匹配：`Call` 命中 callee 精确相等的调用；
//! `PluginCall` 同上（callee 等于目标或以 `目标.` 开头），但只在插件工程里判。
//! 加条目只改 [`ENTRIES`]。
//!
//! 证据：
//! - `skyline-overview/references/migration/compatibility.md:198-205`（Q: 不支持 animate
//!   动画接口，给了 worklet 替代写法）；
//! - `skyline-overview/SKILL.md:100`（不支持 animate 接口）；
//! - `skyline-route/references/api/router-api.md:9`（所有 `wx.router` 方法均不支持小程序插件）。
//!
//! 注释与字符串里的 `this.animate` 不在 AST 的调用节点上，天然不会命中。

use crate::Severity;
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};

use super::{Callee, at_span, is_plugin};

#[derive(Debug, Clone, Copy)]
enum Entry {
    /// 调用点：callee 精确等于目标文本。
    Call {
        callee: &'static str,
        message: &'static str,
    },
    /// 调用点：callee 等于目标名或以 `目标名.` 开头，且只在插件工程里判。
    ///
    /// 插件限制原文：`skyline-route/references/api/router-api.md:9`「所有 wx.router
    /// 方法均不支持小程序插件」。插件页内调这些接口不会报错，只是静默无效。
    PluginCall {
        prefix: &'static str,
        message: &'static str,
    },
}

const ENTRIES: &[Entry] = &[
    Entry::Call {
        callee: "this.animate",
        message: "Skyline 不支持 `this.animate`，动画不会执行；改用 worklet（wx.worklet.shared + applyAnimatedStyle）",
    },
    Entry::PluginCall {
        prefix: "wx.router",
        message: "小程序插件不支持 wx.router.*（自定义路由），插件页内调用无效",
    },
];

impl Entry {
    fn message(&self) -> &'static str {
        match self {
            Self::Call { message, .. } | Self::PluginCall { message, .. } => message,
        }
    }
}

pub struct UnsupportedApi;

impl UnsupportedApi {
    pub const ID: &'static str = "skyline/unsupported-api";
}

impl Rule for UnsupportedApi {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "Skyline 不支持的 JS API（如 this.animate、插件内的 wx.router.*）",
        }
    }

    fn check(&self, project: &Project) -> Vec<crate::Diagnostic> {
        let plugin = is_plugin(project);
        let mut out = Vec::new();
        for file in &project.scripts {
            for call in file.doc.calls() {
                for entry in ENTRIES {
                    let span = match entry {
                        Entry::Call { callee, .. } => Callee::Exact(callee)
                            .matches(&call.callee)
                            .then_some((call.start, call.end)),
                        Entry::PluginCall { prefix, .. } => (plugin
                            && Callee::Namespace(prefix).matches(&call.callee))
                        .then_some((call.start, call.end)),
                    };
                    let Some((start, end)) = span else { continue };
                    out.push(at_span(
                        Self::ID,
                        Severity::Warning,
                        file,
                        start,
                        end,
                        entry.message(),
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

    use super::UnsupportedApi;
    use crate::rules::testutil::js::{Fixture, messages, plugin, single, single_with_app};

    /// 单页工程 + 一个额外的脚本文件（用于验证全域扫描的边界）。
    fn with_script(name: &str, rel: &str, js: &str) -> (Fixture, crate::project::Project) {
        let f = Fixture::new(name);
        f.write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", "<view/>")
            .write("pages/index/index.js", "Page({});")
            .write(rel, js);
        let project = f.project();
        (f, project)
    }

    #[test]
    fn utility_scripts_outside_units_are_scanned() {
        let (_f, project) = with_script(
            "unsupported-loose-util",
            "utils/helper.ts",
            "export const jump = () => this.animate('.x', [], 100);",
        );
        let diagnostics = UnsupportedApi.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].path, "utils/helper.ts");
    }

    #[test]
    fn type_declarations_are_not_scanned() {
        let (_f, project) = with_script(
            "unsupported-dts",
            "utils/types.d.ts",
            "export declare const jump: () => this.animate('.x', [], 100);\n",
        );
        assert_eq!(messages(&UnsupportedApi, &project), Vec::<String>::new());
    }

    #[test]
    fn npm_build_output_is_not_scanned() {
        let (_f, project) = with_script(
            "unsupported-npm",
            "miniprogram_npm/foo/index.js",
            "this.animate('.x', [], 100);",
        );
        assert_eq!(messages(&UnsupportedApi, &project), Vec::<String>::new());
    }

    #[test]
    fn unsupported_calls_are_reported() {
        let (_f, project) = single(
            "unsupported-positive",
            "this.animate('.a', [], 1000);\nPage({\n  data: {},\n});\nthis.animate('.b', [], 200);",
        );
        let diagnostics = UnsupportedApi.check(&project);
        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
        assert!(
            diagnostics
                .iter()
                .all(|d| d.severity == crate::Severity::Warning)
        );
        assert_eq!(diagnostics[0].path, "pages/index/index.js");
        assert!(diagnostics[0].message.contains("this.animate"));
        assert_eq!(diagnostics[1].message, diagnostics[0].message);
    }

    /// `onPageScroll` 由 `feature_flag.GetEnablePageScroll()` 支持，不再是无效配置。
    #[test]
    fn page_on_page_scroll_is_supported() {
        let (_f, project) = single("unsupported-page-scroll", "Page({ onPageScroll() {} });");
        assert_eq!(messages(&UnsupportedApi, &project), Vec::<String>::new());
    }

    #[test]
    fn app_js_is_scanned_too() {
        let (_f, project) = single_with_app(
            "unsupported-app",
            "this.animate('.b', [], 1);",
            Some("this.animate('.a', [], 1);"),
        );
        let diagnostics = UnsupportedApi.check(&project);
        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].path, "app.js");
        assert_eq!(diagnostics[1].path, "pages/index/index.js");
    }

    #[test]
    fn non_calls_and_non_page_options_are_ignored() {
        let (_f, project) = single(
            "unsupported-negative",
            r#"
            // this.animate('.a', [], 1);
            const text = 'this.animate';
            page.onPageScroll = () => {};
            wx.Page({ onPageScroll() {} });
            this.animateLater('.b', [], 1);
            Page({ onLoad() {} });
            "#,
        );
        assert_eq!(messages(&UnsupportedApi, &project), Vec::<String>::new());
    }

    #[test]
    fn non_literal_page_argument_is_ignored() {
        let (_f, project) = single("unsupported-dynamic", "Page(config);");
        assert_eq!(messages(&UnsupportedApi, &project), Vec::<String>::new());
    }

    #[test]
    fn router_calls_are_reported_only_in_plugin_projects() {
        let (_f, plugin_project) = plugin(
            "unsupported-plugin-router",
            "wx.router.navigateTo({ url: 'x' });\nwx.router.addRouteBuilder('a', b);",
        );
        let diagnostics = UnsupportedApi.check(&plugin_project);
        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
        assert!(diagnostics.iter().all(|d| d.message.contains("wx.router")));

        let (_n, normal) = single(
            "unsupported-normal-router",
            "wx.router.navigateTo({ url: 'x' });",
        );
        assert_eq!(messages(&UnsupportedApi, &normal), Vec::<String>::new());
    }

    #[test]
    fn plugin_prefix_does_not_match_similar_names() {
        let (_f, project) = plugin(
            "unsupported-plugin-prefix",
            "wx.routers.navigateTo({});\nmywx.router.x();\nwx.routerX.y();",
        );
        assert_eq!(messages(&UnsupportedApi, &project), Vec::<String>::new());
    }
}
