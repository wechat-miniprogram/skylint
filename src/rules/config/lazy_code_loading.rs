//! `skyline/lazy-code-loading`：页面 json **显式**写 `renderer: "skyline"`，而 app.json
//! 的 `lazyCodeLoading` 不是 `"requiredComponents"`，开发者工具会拒绝编译（`minidev build` 不校验这条）。
//!
//! 判据来自 minidev `compiler-graph/src/json/mutate.rs:441-450`（`check_renderer`）：
//! 只认页面 json 自己写的 `renderer: "skyline"`——app 级 `renderer` 让页面继承
//! Skyline 时这条不报。放在这里而不是只放 app 级，是因为同一个工程可以只给个别
//! 页面开 Skyline，而 `lazyCodeLoading` 是 app 级唯一开关。
//!
//! 插件视图不判：插件页走 `compile_plugin_json`，不经过 `compile_page_json`，
//! 这条校验在插件视图里不会让构建失败。

use serde_json::Value;

use crate::input::json::JsonPositions;
use crate::project::Project;
use crate::rules::{Rule, RuleMeta, Target};
use crate::{Diagnostic, Severity};

pub struct LazyCodeLoading;

impl Rule for LazyCodeLoading {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: "skyline/lazy-code-loading",
            severity: Severity::Error,
            description: "页面显式声明 Skyline 渲染时，app.json 必须设置 lazyCodeLoading: \"requiredComponents\"",
        }
    }

    fn target(&self) -> Target {
        // 这是 Skyline 构建期的硬性前提，不是 WebView 回退场景。
        Target::Skyline
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        if !super::applicable_project_level(project) {
            return Vec::new();
        }
        if project
            .app_json
            .get("lazyCodeLoading")
            .and_then(Value::as_str)
            == Some("requiredComponents")
        {
            return Vec::new();
        }

        // `check_renderer` 只看页面 json 的显式声明，app 级 renderer 不算。
        let pages: Vec<&str> = project
            .pages
            .iter()
            .filter(|page| {
                project
                    .page_json
                    .get(&page.route)
                    .and_then(|json| json.get("renderer"))
                    .and_then(Value::as_str)
                    == Some("skyline")
            })
            .map(|page| page.route.as_str())
            .collect();
        if pages.is_empty() {
            return Vec::new();
        }

        let mut out = Vec::new();
        // app.json 写了这个键（值不对）时只报一次：修法只有一处，逐页重复没有意义。
        if project.app_json.contains_key("lazyCodeLoading") {
            super::error(
                &mut out,
                Self::ID,
                &project.app_json_path,
                JsonPositions::new(&project.app_json_source).key("lazyCodeLoading"),
                format!(
                    "{}显式使用 Skyline 渲染，但 app.json 的 lazyCodeLoading 不是 \"requiredComponents\"，开发者工具会拒绝编译（`minidev build` 不校验这条）",
                    page_list(&pages)
                ),
                "在 app.json 顶层写 \"lazyCodeLoading\": \"requiredComponents\"；页面显式 renderer: \"skyline\" 时 minidev 会在编译前校验这一项",
            );
            return out;
        }

        // 没有键就没有位置，逐页落在触发它的 `renderer` 上并点名 app.json。
        for route in pages {
            super::error(
                &mut out,
                Self::ID,
                &format!("{route}.json"),
                JsonPositions::new(
                    project
                        .page_json_source
                        .get(route)
                        .map_or("", String::as_str),
                )
                .key("renderer"),
                format!(
                    "页面 `{route}.json` 显式声明 `renderer: \"skyline\"`，但 app.json 未设置 `lazyCodeLoading: \"requiredComponents\"`，开发者工具会拒绝编译（`minidev build` 不校验这条）"
                ),
                "在 app.json 顶层补 \"lazyCodeLoading\": \"requiredComponents\"；这是 app 级开关，一个工程只需要写一次",
            );
        }
        out
    }
}

impl LazyCodeLoading {
    pub const ID: &'static str = "skyline/lazy-code-loading";
}

/// 「页面 `a`、`b`」；页面多时只列前三个并带上总数。
fn page_list(routes: &[&str]) -> String {
    let head = &routes[..routes.len().min(3)];
    if routes.len() > 3 {
        format!("页面 `{}` 等 {} 个页面", head.join("`、`"), routes.len())
    } else {
        format!("页面 `{}`", head.join("`、`"))
    }
}

#[cfg(test)]
mod tests {
    use crate::Severity;
    use crate::project::Project;
    use crate::rules::Rule;
    use crate::rules::testutil::Fixture;

    use super::LazyCodeLoading;

    fn project(name: &str, app: &str, page: &str) -> (Fixture, Project) {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write("app.json", app)
            .write("pages/index/index.json", page)
            .write("pages/index/index.wxml", "<view/>");
        let project = fixture.project();
        (fixture, project)
    }

    #[test]
    fn reports_explicit_skyline_page_without_required_components() {
        let (_f, project) = project(
            "lazy-explicit",
            r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            r#"{"renderer":"skyline"}"#,
        );
        let diags = LazyCodeLoading.check(&project);
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(diags[0].path, "pages/index/index.json");
        assert!(diags[0].message.contains("app.json"));
    }

    #[test]
    fn accepts_required_components() {
        let (_f, project) = project(
            "lazy-ok",
            r#"{"pages":["pages/index/index"],"renderer":"skyline","lazyCodeLoading":"requiredComponents"}"#,
            r#"{"renderer":"skyline"}"#,
        );
        assert!(LazyCodeLoading.check(&project).is_empty());
    }

    /// app 级 `renderer: "skyline"` 只让页面继承渲染器，这条校验不触发。
    #[test]
    fn ignores_pages_inheriting_skyline_from_app() {
        let (_f, project) = project(
            "lazy-inherited",
            r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            "{}",
        );
        assert!(LazyCodeLoading.check(&project).is_empty());
    }

    /// 页面级 `renderer` 优先于 app 级：app 是 webview，页面显式 skyline 仍然要求。
    #[test]
    fn page_level_skyline_overrides_app_webview() {
        let (_f, project) = project(
            "lazy-page-override",
            r#"{"pages":["pages/index/index"],"renderer":"webview"}"#,
            r#"{"renderer":"skyline"}"#,
        );
        assert_eq!(LazyCodeLoading.check(&project).len(), 1);
    }

    #[test]
    fn page_level_webview_opts_out_even_with_skyline_app() {
        let (_f, project) = project(
            "lazy-page-webview",
            r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            r#"{"renderer":"webview"}"#,
        );
        assert!(LazyCodeLoading.check(&project).is_empty());
    }

    #[test]
    fn reports_wrong_value_once_at_app_json() {
        let (_f, project) = project(
            "lazy-wrong-value",
            "{\n  \"pages\": [\"pages/index/index\"],\n  \"lazyCodeLoading\": \"all\"\n}",
            r#"{"renderer":"skyline"}"#,
        );
        let diags = LazyCodeLoading.check(&project);
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].path, "app.json");
        assert_eq!((diags[0].line, diags[0].column), (3, 3), "应指向键本身");
    }

    /// 插件页不经过 `compile_page_json`，这条校验在插件视图里不会让构建失败。
    #[test]
    fn skips_plugin_view() {
        let fixture = Fixture::new("lazy-plugin");
        fixture
            .write(
                "project.config.json",
                r#"{"miniprogramRoot":"","pluginRoot":"plugin"}"#,
            )
            .write("app.json", r#"{"pages":["pages/index/index"]}"#)
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", "<view/>")
            .write("plugin/plugin.json", r#"{"pages":{"a":"pages/a/a"}}"#)
            .write("plugin/pages/a/a.json", r#"{"renderer":"skyline"}"#)
            .write("plugin/pages/a/a.wxml", "<view/>");
        let views = Project::load_views(fixture.root()).unwrap();
        let plugin = views.iter().find(|view| view.plugin_view).unwrap();
        assert!(LazyCodeLoading.check(plugin).is_empty());
    }
}
