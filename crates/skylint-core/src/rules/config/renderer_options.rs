//! `skyline/renderer-options`：`rendererOptions` 的写法 / 类型不会被运行时读取。
//!
//! 这条规则存在的原因是构建行为的不对称：页面 json 有 schema 校验，`minidev
//! build` 会对页面里的 `rendererOptions.skyline.defaultDisplayBlock: "yes"` 报错；
//! 同一份配置写进 `app.json` 则完全静默通过（实测 app.json 的 `renderer: 123`、
//! 未知顶层键、`tagNameStyleIsolation: "bogus"` 都不报）。
//!
//! 运行时（`wxapplib/src/skyline/src/runtime/index.ts`）只按自己认的键和类型取值：
//! 布尔开关是 `|| false`，隔离级别是 `=== 'isolated' ? Isolated : Legacy`，页面级
//! 只读 `rendererOptions.skyline.*`。所以类型不符、层级写错都不会崩，只会静默
//! 回落默认值——因此这里全部是 warning 而不是 error。

use serde_json::{Map, Value};

use crate::input::json::JsonPositions;
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

pub struct RendererOptions;

impl RendererOptions {
    pub const ID: &'static str = "skyline/renderer-options";
}

/// 已知键的期望类型（来源：`skyline-config/references/app-config.md` 的配置项表，
/// 与运行时的取值方式一一对应）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// `boolean`：运行时 `|| false`，非布尔值静默变成默认值。
    Bool,
    /// `"isolated"` / `"legacy"`：运行时只判断是否等于 `"isolated"`。
    Isolation,
    /// 版本边界：字符串，运行时按字符串比较。
    Text,
}

const KNOWN: &[(&str, Kind)] = &[
    ("defaultDisplayBlock", Kind::Bool),
    ("defaultContentBox", Kind::Bool),
    ("enableScrollViewAutoSize", Kind::Bool),
    ("enableImagePreload", Kind::Bool),
    ("convertRpxToVw", Kind::Bool),
    ("disableABTest", Kind::Bool),
    ("tagNameStyleIsolation", Kind::Isolation),
    ("keyframeStyleIsolation", Kind::Isolation),
    ("sdkVersionBegin", Kind::Text),
    ("sdkVersionEnd", Kind::Text),
    ("iosVersionBegin", Kind::Text),
    ("iosVersionEnd", Kind::Text),
    ("androidVersionBegin", Kind::Text),
    ("androidVersionEnd", Kind::Text),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    App,
    Page,
}

impl Rule for RendererOptions {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "rendererOptions 的写法或类型不会被运行时读取，配置静默失效",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        check_file(
            "app.json",
            &project.app_json_source,
            &project.app_json,
            Level::App,
            &mut out,
        );
        for (route, json) in &project.page_json {
            // 配置写错就是写错，不按「这页是不是 Skyline」过滤。
            let source = project
                .page_json_source
                .get(route)
                .map_or("", String::as_str);
            check_file(
                &format!("{route}.json"),
                source,
                json,
                Level::Page,
                &mut out,
            );
        }
        out
    }
}

fn check_file(
    path: &str,
    source: &str,
    json: &Map<String, Value>,
    level: Level,
    out: &mut Vec<Diagnostic>,
) {
    let Some(options) = json.get("rendererOptions") else {
        return;
    };
    let positions = JsonPositions::new(source);
    let Some(options) = options.as_object() else {
        warn(
            out,
            path,
            positions.key("rendererOptions"),
            "rendererOptions 不是对象，整块配置不生效".to_string(),
            "rendererOptions 的每个配置项都是键值对，写成对象；开关放在 rendererOptions.skyline 下",
        );
        return;
    };

    match options.get("skyline") {
        None => {}
        Some(Value::Object(skyline)) => check_skyline(path, &positions, skyline, out),
        Some(_) => warn(
            out,
            path,
            positions.key("skyline"),
            "rendererOptions.skyline 不是对象，整块配置不生效".to_string(),
            "开关应写成 rendererOptions.skyline 下的键值对",
        ),
    }

    // 页面级只读 rendererOptions.skyline.*；写在顶层的开关是死配置。
    // app 级顶层的 defaultDisplayBlock 是运行时明确支持的写法（另一条规则/视图也依赖它），
    // 其余顶层键不在本规则范围内。
    if level == Level::Page {
        for key in options.keys() {
            if key == "skyline" || kind_of(key).is_none() {
                continue;
            }
            warn(
                out,
                path,
                positions.key(key),
                format!(
                    "页面级只读 rendererOptions.skyline.*，`{key}` 写在 rendererOptions 顶层不生效"
                ),
                "把该开关移进 rendererOptions.skyline 下",
            );
        }
    }
}

fn check_skyline(
    path: &str,
    positions: &JsonPositions,
    skyline: &Map<String, Value>,
    out: &mut Vec<Diagnostic>,
) {
    for (key, value) in skyline {
        let Some(kind) = kind_of(key) else {
            warn(
                out,
                path,
                positions.key(key),
                format!("rendererOptions.skyline 下的 `{key}` 不是已知配置项，运行时不会读取"),
                "已知配置项见 skyline-config/references/app-config.md 的 rendererOptions 表；未知键多半是拼写错误或多层级",
            );
            continue;
        };
        check_value(path, positions, key, value, kind, out);
    }
}

fn check_value(
    path: &str,
    positions: &JsonPositions,
    key: &str,
    value: &Value,
    kind: Kind,
    out: &mut Vec<Diagnostic>,
) {
    let valid = match kind {
        Kind::Bool => value.is_boolean(),
        Kind::Isolation => matches!(value.as_str(), Some("isolated" | "legacy")),
        Kind::Text => value.is_string(),
    };
    if valid {
        return;
    }

    let (message, help) = match kind {
        Kind::Bool => (
            format!(
                "rendererOptions.skyline.{key} 期望 boolean，实际是 {value}：运行时不做类型校验（app 级按 `|| false` 取真值，页面级要求 `typeof === 'boolean'`），开关可能被当成真值或直接失效"
            ),
            "写成 JSON 布尔值 true / false，不要加引号或写数字",
        ),
        Kind::Text => (
            format!(
                "rendererOptions.skyline.{key} 期望版本号字符串，实际是 {value}"
            ),
            "写成字符串，例如 \"3.0.0\"",
        ),
        Kind::Isolation => match value.as_str() {
            Some("") => (
                format!(
                    "rendererOptions.skyline.{key} 是空字符串：运行时先做真值判断，空串会落回默认值 \"isolated\""
                ),
                "要显式关闭隔离请写 \"legacy\"，不要写空串",
            ),
            Some(other) => (
                format!(
                    "rendererOptions.skyline.{key} 的值 \"{other}\" 不是 \"isolated\"/\"legacy\"：运行时只判断是否等于 \"isolated\"，其他任何值都按 \"legacy\" 处理"
                ),
                "可选值只有 \"isolated\" 与 \"legacy\"，注意不要带多余空白",
            ),
            None => (
                format!(
                    "rendererOptions.skyline.{key} 期望 \"isolated\" 或 \"legacy\"，实际是 {value}：类型不符，运行时按默认值处理"
                ),
                "可选值只有 \"isolated\" 与 \"legacy\"",
            ),
        },
    };
    warn(out, path, positions.key(key), message, help);
}

fn kind_of(key: &str) -> Option<Kind> {
    KNOWN
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, kind)| *kind)
}

fn warn(
    out: &mut Vec<Diagnostic>,
    path: &str,
    at: Option<(u32, u32)>,
    message: String,
    help: &str,
) {
    let (line, column) = at.unwrap_or((1, 1));
    out.push(
        Diagnostic::new(
            RendererOptions::ID,
            Severity::Warning,
            message,
            path,
            line,
            column,
        )
        .help(help),
    );
}

#[cfg(test)]
mod tests {
    use crate::Severity;
    use crate::project::Project;
    use crate::rules::Rule;
    use crate::rules::wxss::testutil::Fixture;

    use super::RendererOptions;

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

    fn app_with(options: &str) -> String {
        format!(
            r#"{{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{options}}}"#
        )
    }

    #[test]
    fn reports_invalid_isolation_values() {
        let (_f, project) = project(
            "renderer-options-isolation",
            &app_with(
                r#"{"skyline":{"tagNameStyleIsolation":"legacy ","keyframeStyleIsolation":"bogus"}}"#,
            ),
            "{}",
        );
        let diags = RendererOptions.check(&project);
        assert_eq!(diags.len(), 2);
        assert!(diags.iter().all(|d| d.severity == Severity::Warning));
        let tag = diags
            .iter()
            .find(|d| d.message.contains("tagNameStyleIsolation"))
            .expect("应报 tagNameStyleIsolation");
        assert!(tag.message.contains("\"legacy \""), "{}", tag.message);
        assert!(tag.message.contains("\"legacy\""), "{}", tag.message);
        let keyframe = diags
            .iter()
            .find(|d| d.message.contains("keyframeStyleIsolation"))
            .expect("应报 keyframeStyleIsolation");
        assert!(
            keyframe.message.contains("按 \"legacy\" 处理"),
            "{}",
            keyframe.message
        );
    }

    #[test]
    fn reports_string_instead_of_boolean() {
        let (_f, project) = project(
            "renderer-options-boolean",
            &app_with(r#"{"skyline":{"defaultDisplayBlock":"true","convertRpxToVw":1}}"#),
            "{}",
        );
        let diags = RendererOptions.check(&project);
        assert_eq!(diags.len(), 2);
        assert!(
            diags.iter().all(|d| d.message.contains("期望 boolean")),
            "{diags:?}"
        );
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("defaultDisplayBlock"))
        );
        assert!(diags.iter().any(|d| d.message.contains("convertRpxToVw")));
    }

    #[test]
    fn reports_unknown_key_under_skyline() {
        let (_f, project) = project(
            "renderer-options-unknown",
            &app_with(r#"{"skyline":{"foo":true}}"#),
            "{}",
        );
        let diags = RendererOptions.check(&project);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("`foo`"), "{}", diags[0].message);
        assert!(diags[0].message.contains("不会读取"));
    }

    #[test]
    fn reports_page_level_switch_outside_skyline() {
        let (_f, project) = project(
            "renderer-options-page-top",
            r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            r#"{"rendererOptions":{"defaultDisplayBlock":true}}"#,
        );
        let diags = RendererOptions.check(&project);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].path, "pages/index/index.json");
        assert!(diags[0].message.contains("rendererOptions.skyline"));
        assert!(diags[0].message.contains("defaultDisplayBlock"));
    }

    #[test]
    fn reports_non_object_blocks() {
        let (_f, first) = project(
            "renderer-options-not-object",
            r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":true}"#,
            "{}",
        );
        let diags = RendererOptions.check(&first);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("rendererOptions 不是对象"));

        let (_f2, second) = project(
            "renderer-options-skyline-not-object",
            &app_with(r#"{"skyline":"on"}"#),
            "{}",
        );
        let diags = RendererOptions.check(&second);
        assert_eq!(diags.len(), 1);
        assert!(
            diags[0]
                .message
                .contains("rendererOptions.skyline 不是对象"),
            "{}",
            diags[0].message
        );
    }

    #[test]
    fn points_at_the_offending_key() {
        let app = "{\n  \"pages\": [\"pages/index/index\"],\n  \"renderer\": \"skyline\",\n  \"rendererOptions\": {\n    \"skyline\": {\n      \"defaultDisplayBlock\": \"yes\"\n    }\n  }\n}";
        let (_f, project) = project("renderer-options-position", app, "{}");
        let diags = RendererOptions.check(&project);
        assert_eq!(diags.len(), 1);
        assert_eq!((diags[0].line, diags[0].column), (6, 7));
    }

    #[test]
    fn checks_pages_that_are_not_skyline() {
        let (_f, project) = project(
            "renderer-options-webview",
            r#"{"pages":["pages/index/index"],"renderer":"webview"}"#,
            r#"{"rendererOptions":{"skyline":{"tagNameStyleIsolation":"bogus"}}}"#,
        );
        assert_eq!(RendererOptions.check(&project).len(), 1);
    }

    #[test]
    fn accepts_valid_configuration() {
        let (_f, project) = project(
            "renderer-options-valid",
            &app_with(
                r#"{"skyline":{"defaultDisplayBlock":true,"defaultContentBox":false,"tagNameStyleIsolation":"legacy","keyframeStyleIsolation":"isolated","enableScrollViewAutoSize":true,"enableImagePreload":false,"convertRpxToVw":false,"disableABTest":true,"sdkVersionBegin":"3.0.0","sdkVersionEnd":"15.255.255","iosVersionBegin":"10.0.0","iosVersionEnd":"15.255.255","androidVersionBegin":"5.0.0","androidVersionEnd":"15.255.255"}}"#,
            ),
            r#"{"rendererOptions":{"skyline":{"defaultDisplayBlock":false}}}"#,
        );
        assert!(RendererOptions.check(&project).is_empty());
    }

    #[test]
    fn ignores_missing_renderer_options() {
        let (_f, project) = project(
            "renderer-options-missing",
            r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            "{}",
        );
        assert!(RendererOptions.check(&project).is_empty());
    }

    #[test]
    fn accepts_app_level_top_level_default_display_block() {
        // app 级非嵌套写法：运行时明确读取 rendererOptions.defaultDisplayBlock。
        let (_f, project) = project(
            "renderer-options-app-top",
            r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"defaultDisplayBlock":true}}"#,
            "{}",
        );
        assert!(RendererOptions.check(&project).is_empty());
    }
}
