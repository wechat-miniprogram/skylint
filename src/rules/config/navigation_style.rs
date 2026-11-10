//! `skyline/navigation-style`：Skyline 页面的 `navigationStyle` 解析结果不是 `custom`
//! 时，开发者工具会拒绝编译（`minidev build` 不校验这条）。
//!
//! 判据来自 minidev `compiler-graph/src/json/mutate.rs:452-477`（`check_renderer`）：
//!
//! - 页面 json 显式写 `navigationStyle: "default"` → 直接报错；
//! - 否则解析链（页面 json → app.json 的 `window.navigationStyle`）不是 `custom`，
//!   且页面写了任意 `navigationBarTitleText` / `navigationBarTextStyle` /
//!   `navigationBarBackgroundColor` → 报错。
//!
//! 编译器随后会无条件把 Skyline 页的 `navigationStyle` 改写成 `custom`、缺省补
//! `disableScroll: true`，但那是构建产物，不改写会失败这件事仍然要提前告知。
//! 插件视图不判：插件页走 `compile_plugin_json`，不经过 `compile_page_json`。

use serde_json::Value;

use crate::input::json::JsonPositions;
use crate::project::Project;
use crate::rules::{Rule, RuleMeta, Target};
use crate::{Diagnostic, Severity};

pub struct NavigationStyle;

impl NavigationStyle {
    pub const ID: &'static str = "skyline/navigation-style";
}

/// 与 minidev 同序：诊断落在页面里写的第一个导航栏字段上。
const NAVIGATION_BAR_KEYS: [&str; 3] = [
    "navigationBarTitleText",
    "navigationBarTextStyle",
    "navigationBarBackgroundColor",
];

impl Rule for NavigationStyle {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Error,
            description: "Skyline 页面的 navigationStyle 必须解析为 custom，且不能同时写 navigationBar* 字段",
        }
    }

    fn target(&self) -> Target {
        Target::Skyline
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        if !super::applicable_project_level(project) {
            return Vec::new();
        }
        let mut out = Vec::new();
        for page in project.pages.iter().filter(|page| page.is_skyline) {
            let Some(json) = project.page_json.get(&page.route) else {
                continue;
            };
            let page_path = format!("{}.json", page.route);
            let source = project
                .page_json_source
                .get(&page.route)
                .map_or("", String::as_str);
            let style = json.get("navigationStyle").and_then(Value::as_str);

            if style == Some("default") {
                super::error(
                    &mut out,
                    Self::ID,
                    &page_path,
                    JsonPositions::new(source).key("navigationStyle"),
                    format!(
                        "页面 `{page_path}` 使用 Skyline 渲染，但 navigationStyle 显式写成 `\"default\"`，开发者工具会拒绝编译（`minidev build` 不校验这条）"
                    ),
                    "把页面 json 的 `navigationStyle` 改成 `\"custom\"`；Skyline 页面固定使用自定义导航栏，minidev 在编译前直接报错",
                );
                continue;
            }

            let is_custom = match style {
                Some(style) => style == "custom",
                None => {
                    project
                        .app_json
                        .get("window")
                        .and_then(|window| window.get("navigationStyle"))
                        .and_then(Value::as_str)
                        == Some("custom")
                }
            };
            if is_custom {
                continue;
            }
            let Some(key) = NAVIGATION_BAR_KEYS
                .iter()
                .copied()
                .find(|key| json.get(*key).is_some())
            else {
                continue;
            };

            let reason = match style {
                Some(style) => format!("页面 json 的 `navigationStyle` 是 `\"{style}\"`"),
                None => {
                    "页面 json 未写 `navigationStyle`，app.json 的 `window.navigationStyle` 也不是 `\"custom\"`"
                        .to_string()
                }
            };
            super::error(
                &mut out,
                Self::ID,
                &page_path,
                JsonPositions::new(source).key(key),
                format!(
                    "页面 `{page_path}` 使用 Skyline 渲染，写了 `{key}`，但{reason}，开发者工具会拒绝编译（`minidev build` 不校验这条）"
                ),
                "把页面 json 的 `navigationStyle` 写成 `\"custom\"`（或在 app.json 的 `window.navigationStyle` 统一设为 `\"custom\"`）；Skyline 页面固定使用自定义导航栏",
            );
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use crate::Severity;
    use crate::project::Project;
    use crate::rules::Rule;
    use crate::rules::testutil::Fixture;

    use super::NavigationStyle;

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
    fn reports_explicit_default_navigation_style() {
        let (_f, project) = project(
            "nav-default",
            r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            r#"{"navigationStyle":"default"}"#,
        );
        let diags = NavigationStyle.check(&project);
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(diags[0].path, "pages/index/index.json");
        assert!(diags[0].message.contains("default"), "{}", diags[0].message);
    }

    #[test]
    fn reports_navigation_bar_fields_without_custom() {
        let (_f, project) = project(
            "nav-fields",
            r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            "{\n  \"navigationBarTitleText\": \"首页\"\n}",
        );
        let diags = NavigationStyle.check(&project);
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!((diags[0].line, diags[0].column), (2, 3), "应指向字段本身");
    }

    /// app 级 window.navigationStyle 为 custom 时，页面只写 navigationBar* 不报。
    #[test]
    fn accepts_app_level_custom_with_page_navigation_bar_fields() {
        let (_f, project) = project(
            "nav-app-custom",
            r#"{"pages":["pages/index/index"],"renderer":"skyline","window":{"navigationStyle":"custom"}}"#,
            r#"{"navigationBarTitleText":"首页"}"#,
        );
        assert!(NavigationStyle.check(&project).is_empty());
    }

    #[test]
    fn accepts_page_level_custom_with_navigation_bar_fields() {
        let (_f, project) = project(
            "nav-page-custom",
            r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            r#"{"navigationStyle":"custom","navigationBarTitleText":"首页"}"#,
        );
        assert!(NavigationStyle.check(&project).is_empty());
    }

    /// 页面级覆盖 app 级：app 是 custom，页面显式 default 仍然报。
    #[test]
    fn page_level_default_overrides_app_custom() {
        let (_f, project) = project(
            "nav-page-override",
            r#"{"pages":["pages/index/index"],"renderer":"skyline","window":{"navigationStyle":"custom"}}"#,
            r#"{"navigationStyle":"default"}"#,
        );
        assert_eq!(NavigationStyle.check(&project).len(), 1);
    }

    /// 非 Skyline 页不校验。
    #[test]
    fn skips_webview_pages() {
        let (_f, project) = project(
            "nav-webview",
            r#"{"pages":["pages/index/index"],"renderer":"webview"}"#,
            r#"{"navigationStyle":"default"}"#,
        );
        assert!(NavigationStyle.check(&project).is_empty());
    }
}
