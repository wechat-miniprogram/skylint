//! `skyline/config-ignored`：配置写在不被读取的位置 / 类型上，静默无效。
//!
//! 两条约束都是「写了但没有读者」，构建链路完全静默：
//!
//! - `disableScroll` 是页面级配置（`skyline-config/references/page-config.md:49`：
//!   「此配置只在页面 json 中有效，无法在 `app.json` 中设置」）。写进 app.json 的
//!   `window` 不会被任何页面读取，minidev 也不校验 app.json（实测静默通过）。
//! - `setting.skylineRenderEnable` 的类型是 Boolean（`skyline-config/references/
//!   project-config.md:18-29`，开发者工具里开启 Skyline 渲染调试）。开发者工具按
//!   真值判断，字符串 `"false"` 在 JS 里是真值，反而会把调试渲染器打开。
//!
//! 两条都是 warning：静默失效不会让构建失败，只是实际行为与配置意图不符。

use serde_json::Value;

use crate::input::json::JsonPositions;
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

use super::warn;

pub struct ConfigIgnored;

impl ConfigIgnored {
    pub const ID: &'static str = "skyline/config-ignored";
}

impl Rule for ConfigIgnored {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ConfigIgnored::ID,
            severity: Severity::Warning,
            description: "配置写在不被读取的位置或类型上，静默无效",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        check_app_window(project, &mut out);
        check_project_setting(project, &mut out);
        out
    }
}

/// app.json 的 `window` 是全局窗口配置，页面级开关写在这里没有读者。
fn check_app_window(project: &Project, out: &mut Vec<Diagnostic>) {
    let Some(window) = project.app_json.get("window").and_then(Value::as_object) else {
        return;
    };
    if !window.contains_key("disableScroll") {
        return;
    }
    warn(
        out,
        ConfigIgnored::ID,
        "app.json",
        JsonPositions::new(&project.app_json_source).key("disableScroll"),
        "app.json 的 `window.disableScroll` 不生效，`disableScroll` 只能写在页面 json".to_string(),
        "Skyline 页面默认不滚动（`check_renderer` 会把 skyline 页的 `disableScroll` 改成 true）；需要关滚动的页面，把 `disableScroll: true` 写在该页的 json 里",
    );
}

/// `setting` 里的开关由开发者工具按真值读取，类型不符比缺失更危险。
/// 开发者工具把个人配置写在 project.private.config.json（常被 gitignore），两份都要查。
fn check_project_setting(project: &Project, out: &mut Vec<Diagnostic>) {
    for (path, config, source) in [
        (
            "project.config.json",
            &project.project_config,
            &project.project_config_source,
        ),
        (
            "project.private.config.json",
            &project.project_private_config,
            &project.project_private_config_source,
        ),
    ] {
        let Some(value) = config
            .get("setting")
            .and_then(Value::as_object)
            .and_then(|setting| setting.get("skylineRenderEnable"))
        else {
            continue;
        };
        if value.is_boolean() {
            continue;
        }
        warn(
            out,
            ConfigIgnored::ID,
            path,
            JsonPositions::new(source).key("skylineRenderEnable"),
            format!(
                "`setting.skylineRenderEnable` 应为布尔值，当前是{}；开发者工具按真值判断，字符串 \"false\" 反而会开启",
                type_name(value)
            ),
            "写成 JSON 布尔值 true / false，不要加引号或写数字",
        );
    }
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "空值",
        Value::Bool(_) => "布尔值",
        Value::Number(_) => "数字",
        Value::String(_) => "字符串",
        Value::Array(_) => "数组",
        Value::Object(_) => "对象",
    }
}

#[cfg(test)]
mod tests {
    use crate::Severity;
    use crate::project::Project;
    use crate::rules::Rule;
    use crate::rules::testutil::Fixture;

    use super::ConfigIgnored;

    fn project(name: &str, app: &str, config: &str) -> (Fixture, Project) {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", config)
            .write("app.json", app)
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", "<view/>");
        let project = fixture.project();
        (fixture, project)
    }

    fn app_with_window(window: &str) -> String {
        format!(r#"{{"pages":["pages/index/index"],"window":{window}}}"#)
    }

    fn config_with_setting(value: &str) -> String {
        format!(r#"{{"miniprogramRoot":"","setting":{{"skylineRenderEnable":{value}}}}}"#)
    }

    #[test]
    fn reports_disable_scroll_under_app_window() {
        let (_f, project) = project(
            "config-ignored-disable-scroll",
            &app_with_window(r#"{"navigationStyle":"custom","disableScroll":true}"#),
            r#"{"miniprogramRoot":""}"#,
        );
        let diags = ConfigIgnored.check(&project);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, Severity::Warning);
        assert_eq!(diags[0].path, "app.json");
        assert!(
            diags[0].message.contains("`window.disableScroll`"),
            "{}",
            diags[0].message
        );
        assert!(
            diags[0].message.contains("只能写在页面 json"),
            "{}",
            diags[0].message
        );
    }

    #[test]
    fn points_at_disable_scroll_key() {
        let app = "{\n  \"pages\": [\"pages/index/index\"],\n  \"window\": {\n    \"navigationStyle\": \"custom\",\n    \"disableScroll\": true\n  }\n}";
        let (_f, project) = project(
            "config-ignored-disable-scroll-position",
            app,
            r#"{"miniprogramRoot":""}"#,
        );
        let diags = ConfigIgnored.check(&project);
        assert_eq!(diags.len(), 1);
        assert_eq!((diags[0].line, diags[0].column), (5, 5), "应指向键本身");
    }

    #[test]
    fn accepts_disable_scroll_outside_window() {
        // disableScroll 不在 window 里：本规则不管，避免与其它位置判定重复。
        let (_f, project) = project(
            "config-ignored-disable-scroll-top",
            r#"{"pages":["pages/index/index"],"disableScroll":true}"#,
            r#"{"miniprogramRoot":""}"#,
        );
        assert!(ConfigIgnored.check(&project).is_empty());
    }

    #[test]
    fn accepts_app_window_without_disable_scroll() {
        let (_f, project) = project(
            "config-ignored-window-clean",
            &app_with_window(r##"{"navigationStyle":"custom","backgroundColor":"#ffffff"}"##),
            r#"{"miniprogramRoot":""}"#,
        );
        assert!(ConfigIgnored.check(&project).is_empty());
    }

    #[test]
    fn reports_non_boolean_skyline_render_enable() {
        for (name, value, kind) in [
            ("string", "\"yes\"", "字符串"),
            ("string-false", "\"false\"", "字符串"),
            ("number", "0", "数字"),
        ] {
            let (_f, project) = project(
                &format!("config-ignored-render-enable-{name}"),
                r#"{"pages":["pages/index/index"]}"#,
                &config_with_setting(value),
            );
            let diags = ConfigIgnored.check(&project);
            assert_eq!(diags.len(), 1, "{value} 应报");
            assert_eq!(diags[0].severity, Severity::Warning);
            assert_eq!(diags[0].path, "project.config.json");
            assert!(
                diags[0].message.contains("`setting.skylineRenderEnable`"),
                "{}",
                diags[0].message
            );
            assert!(
                diags[0].message.contains(&format!("当前是{kind}")),
                "{}",
                diags[0].message
            );
        }
    }

    #[test]
    fn reports_string_false_as_truthy_wording() {
        let (_f, project) = project(
            "config-ignored-render-enable-truthy",
            r#"{"pages":["pages/index/index"]}"#,
            &config_with_setting("\"false\""),
        );
        let diags = ConfigIgnored.check(&project);
        assert_eq!(diags.len(), 1);
        assert!(
            diags[0].message.contains("按真值判断"),
            "{}",
            diags[0].message
        );
        assert!(
            diags[0].message.contains("反而会开启"),
            "{}",
            diags[0].message
        );
    }

    #[test]
    fn points_at_skyline_render_enable_key() {
        let config = "{\n  \"miniprogramRoot\": \"\",\n  \"setting\": {\n    \"skylineRenderEnable\": \"true\"\n  }\n}";
        let (_f, project) = project(
            "config-ignored-render-enable-position",
            r#"{"pages":["pages/index/index"]}"#,
            config,
        );
        let diags = ConfigIgnored.check(&project);
        assert_eq!(diags.len(), 1);
        assert_eq!((diags[0].line, diags[0].column), (4, 5), "应指向键本身");
    }

    /// 开发者工具把个人设置写在 project.private.config.json（常被 gitignore），
    /// 只查 project.config.json 会漏掉真实生效的那份。
    #[test]
    fn reports_non_boolean_value_in_private_config() {
        let (fixture, _) = project(
            "config-ignored-private",
            r#"{"pages":["pages/index/index"]}"#,
            r#"{"miniprogramRoot":""}"#,
        );
        fixture.write(
            "project.private.config.json",
            "{\n  \"setting\": {\n    \"skylineRenderEnable\": \"yes\"\n  }\n}",
        );
        let project = fixture.project();
        let diags = ConfigIgnored.check(&project);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert_eq!(diags[0].path, "project.private.config.json");
        assert_eq!((diags[0].line, diags[0].column), (3, 5), "应指向键本身");
    }

    #[test]
    fn accepts_boolean_skyline_render_enable() {
        for (name, value) in [("true", "true"), ("false", "false")] {
            let (_f, project) = project(
                &format!("config-ignored-render-enable-{name}"),
                r#"{"pages":["pages/index/index"]}"#,
                &config_with_setting(value),
            );
            assert!(ConfigIgnored.check(&project).is_empty(), "{value} 不应判定");
        }
    }

    #[test]
    fn accepts_missing_or_non_object_setting() {
        for (name, config) in [
            ("missing", r#"{"miniprogramRoot":""}"#),
            ("empty-setting", r#"{"miniprogramRoot":"","setting":{}}"#),
            (
                "setting-not-object",
                r#"{"miniprogramRoot":"","setting":true}"#,
            ),
        ] {
            let (_f, project) = project(
                &format!("config-ignored-render-enable-{name}"),
                r#"{"pages":["pages/index/index"]}"#,
                config,
            );
            assert!(
                ConfigIgnored.check(&project).is_empty(),
                "{config} 不应判定"
            );
        }
    }

    #[test]
    fn reports_both_constraints_independently() {
        let (_f, project) = project(
            "config-ignored-both",
            &app_with_window(r#"{"disableScroll":true}"#),
            &config_with_setting("\"yes\""),
        );
        let diags = ConfigIgnored.check(&project);
        assert_eq!(diags.len(), 2);
        assert!(diags.iter().any(|d| d.path == "app.json"));
        assert!(diags.iter().any(|d| d.path == "project.config.json"));
    }
}
