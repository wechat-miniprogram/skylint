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
//!
//! `convertRpxToVw` 是例外：它不在 `rendererOptions` 里读，而是 webview-engine 直接读
//! **app.json 顶层**（`wxapplib/src/webview-engine/src/index.ts:105`）。

use serde_json::{Map, Value};

use crate::input::json::JsonPositions;
use crate::project::Project;
use crate::rules::{Rule, RuleMeta, Target};
use crate::{Diagnostic, Severity};

use super::warn;

pub struct RendererOptions;

impl RendererOptions {
    pub const ID: &'static str = "skyline/renderer-options";
}

/// 已知键的期望类型（来源：`skyline-config/references/app-config.md` 的配置项表，
/// 与运行时的取值方式一一对应）。每个键在 app.json 与页面 json 里都合法：
/// 布局类的 6 个键由 Skyline 运行时读（`runtime/index.ts:424-449`），版本区间与
/// `disableABTest` 由客户端选渲染器时读，官方文档也写明「app.json 或 page.json」
/// （`skyline-overview/references/migration/release.md:105-145`）。
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
    ("tagNameStyleIsolation", Kind::Isolation),
    ("keyframeStyleIsolation", Kind::Isolation),
    ("disableABTest", Kind::Bool),
    ("sdkVersionBegin", Kind::Text),
    ("sdkVersionEnd", Kind::Text),
    ("iosVersionBegin", Kind::Text),
    ("iosVersionEnd", Kind::Text),
    ("androidVersionBegin", Kind::Text),
    ("androidVersionEnd", Kind::Text),
    ("ohosVersionBegin", Kind::Text),
    ("ohosVersionEnd", Kind::Text),
];

/// app 级 `rendererOptions` 顶层**不生效**的键：运行时只读
/// `rendererOptions.skyline.*`（`runtime/index.ts:126-151`）。
/// `defaultDisplayBlock` 不在此列——运行时两种写法都认。
/// app 级 `rendererOptions` 顶层**会**被读取的键。
///
/// 运行时只认这两个（`runtime/index.ts:139` 的 `?.defaultDisplayBlock`、`:126`
/// 起的 `?.skyline?.`）加上 `__useTextStyle`（`typings/wxConfig.d.ts:92`）。
const APP_TOP_READ_KEYS: &[&str] = &["defaultDisplayBlock", "__useTextStyle"];

const SKYLINE_ONLY_TOP: &[&str] = &[
    "defaultContentBox",
    "enableImagePreload",
    "enableScrollViewAutoSize",
    "tagNameStyleIsolation",
    "keyframeStyleIsolation",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    App,
    Page,
}

impl Rule for RendererOptions {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: RendererOptions::ID,
            severity: Severity::Warning,
            description: "rendererOptions 的写法或类型不会被运行时读取，配置静默失效",
        }
    }

    fn target(&self) -> Target {
        // `rendererOptions.skyline.*` 只有 Skyline 运行时读取；对齐开关对 WebView 无意义。
        Target::Skyline
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        if !super::applicable(project) {
            return Vec::new();
        }
        let mut out = Vec::new();
        check_file(
            &project.app_json_path,
            &project.app_json_source,
            &project.app_json,
            Level::App,
            // plugin.json 有自己的键集（pages 是 map、publicComponents 等），
            // 套 app.json 的键表会造误报，插件视图只做已知键的类型 / 层级检查。
            !project.plugin_view,
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
                true,
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
    known_keys_only: bool,
    out: &mut Vec<Diagnostic>,
) {
    let positions = JsonPositions::new(source);
    // app.json 顶层的 convertRpxToVw 是唯一生效的写法，顺带校验类型。
    if level == Level::App
        && let Some(value) = json.get("convertRpxToVw")
        && !value.is_boolean()
    {
        warn(
            out,
            RendererOptions::ID,
            path,
            positions.key("convertRpxToVw"),
            format!(
                "app.json 顶层的 convertRpxToVw 期望 boolean，实际是 {value}：运行时按真值判断，字符串或数字可能被误当成开启"
            ),
            "写成 JSON 布尔值 true / false，不要加引号或写数字",
        );
    }

    // `renderer` 只认字面量 skyline / webview：编译器与加载器都是精确比较
    // （`miniprogram-compiler-rs crates/graph/src/json/mutate.rs:425-441`），
    // 写成 `Skyline` 既不算 Skyline 也不算 opt-out，页面会按默认处理。
    if level == Level::App
        && let Some(value) = json.get("renderer")
    {
        let recognized = value.as_str() == Some("skyline") || value.as_str() == Some("webview");
        if !recognized {
            warn(
                out,
                RendererOptions::ID,
                path,
                positions.key("renderer"),
                format!("`renderer` 的取值 {value} 不会被识别，只认 \"skyline\" / \"webview\""),
                "写成小写的精确值：\"skyline\" 或 \"webview\"",
            );
        }
    }

    let Some(options) = json.get("rendererOptions") else {
        return;
    };
    let Some(options) = options.as_object() else {
        warn(
            out,
            RendererOptions::ID,
            path,
            positions.key("rendererOptions"),
            "rendererOptions 不是对象，整块配置不生效".to_string(),
            "rendererOptions 的每个配置项都是键值对，写成对象；开关放在 rendererOptions.skyline 下",
        );
        return;
    };

    // convertRpxToVw 在 rendererOptions 任意层级都不会被读取。
    check_convert_rpx_to_vw(path, &positions, options, out);

    match options.get("skyline") {
        None => {}
        Some(Value::Object(skyline)) => {
            check_version_pairs(path, &positions, skyline, out);
            check_skyline(path, &positions, skyline, known_keys_only, out)
        }
        Some(_) => warn(
            out,
            RendererOptions::ID,
            path,
            positions.key("skyline"),
            "rendererOptions.skyline 不是对象，整块配置不生效".to_string(),
            "开关应写成 rendererOptions.skyline 下的键值对",
        ),
    }

    // 页面级只读 rendererOptions.skyline.*；写在顶层的开关是死配置。
    // app 级顶层只有 defaultDisplayBlock 生效，其余已知开关都在 skyline 层。
    for key in options.keys() {
        if key == "skyline" || key == "convertRpxToVw" {
            continue;
        }
        if level == Level::Page {
            // 页面级只读 rendererOptions.skyline.*，顶层任何键（含不认识的）都没有读者。
            warn(
                out,
                RendererOptions::ID,
                path,
                positions.key(key),
                format!(
                    "页面级只读 rendererOptions.skyline.*，`{key}` 写在 rendererOptions 顶层不生效"
                ),
                "把该开关移进 rendererOptions.skyline 下",
            );
        } else if APP_TOP_READ_KEYS.contains(&key.as_str()) {
            // 运行时明确读取这两个键。
        } else if SKYLINE_ONLY_TOP.contains(&key.as_str()) {
            warn(
                out,
                RendererOptions::ID,
                path,
                positions.key(key),
                format!(
                    "app 级 `{key}` 写在 rendererOptions 顶层不会生效，运行时只读 rendererOptions.skyline.{key}"
                ),
                "把该配置移进 rendererOptions.skyline 下",
            );
        } else {
            if !known_keys_only {
                continue;
            }
            warn(
                out,
                RendererOptions::ID,
                path,
                positions.key(key),
                format!(
                    "{path} 的 `rendererOptions.{key}` 没有读者：app 级只读 rendererOptions.defaultDisplayBlock 与 rendererOptions.skyline.*"
                ),
                "把开关移进 rendererOptions.skyline 下；已知配置项见 skyline-config/references/app-config.md",
            );
        }
    }
}

/// `convertRpxToVw` 只在 app.json 顶层被读取；出现在 `rendererOptions` 任意层级都无效。
fn check_convert_rpx_to_vw(
    path: &str,
    positions: &JsonPositions,
    options: &Map<String, Value>,
    out: &mut Vec<Diagnostic>,
) {
    if options.contains_key("convertRpxToVw") {
        warn(
            out,
            RendererOptions::ID,
            path,
            positions.key("convertRpxToVw"),
            "`convertRpxToVw` 写在 rendererOptions 里不会生效，运行时只读 app.json 顶层"
                .to_string(),
            "把 convertRpxToVw 移到 app.json 顶层（与 pages 同级），不要放进 rendererOptions",
        );
    }
    for child in options.values() {
        walk_convert_rpx_to_vw(path, positions, child, out);
    }
}

fn walk_convert_rpx_to_vw(
    path: &str,
    positions: &JsonPositions,
    value: &Value,
    out: &mut Vec<Diagnostic>,
) {
    match value {
        Value::Object(map) => check_convert_rpx_to_vw(path, positions, map, out),
        Value::Array(items) => {
            for item in items {
                walk_convert_rpx_to_vw(path, positions, item, out);
            }
        }
        _ => {}
    }
}

/// 版本区间：`xxxVersionEnd` 必须不小于 `xxxVersionBegin`，否则整个版本段都不生效
/// （官方文档「`xxxVersionEnd` 填最大值，否则新版本不生效」）。
const VERSION_PAIRS: &[(&str, &str)] = &[
    ("sdkVersionBegin", "sdkVersionEnd"),
    ("iosVersionBegin", "iosVersionEnd"),
    ("androidVersionBegin", "androidVersionEnd"),
    ("ohosVersionBegin", "ohosVersionEnd"),
];

fn check_version_pairs(
    path: &str,
    positions: &JsonPositions,
    skyline: &Map<String, Value>,
    out: &mut Vec<Diagnostic>,
) {
    for (begin_key, end_key) in VERSION_PAIRS {
        let pair = (
            skyline.get(*begin_key).and_then(Value::as_str),
            skyline.get(*end_key).and_then(Value::as_str),
        );
        let (Some(begin), Some(end)) = pair else {
            continue;
        };
        let (Some(begin_value), Some(end_value)) = (version_tuple(begin), version_tuple(end))
        else {
            continue;
        };
        if begin_value > end_value {
            warn(
                out,
                RendererOptions::ID,
                path,
                positions.key(end_key),
                format!(
                    "`{begin_key}: \"{begin}\"` 大于 `{end_key}: \"{end}\"`，版本区间为空，Skyline 在这个区间里不会启用"
                ),
                "把 xxxVersionEnd 写成最大值（如 \"15.255.255\"），或让 begin 不大于 end",
            );
        }
    }
}

/// `3.0.1` → `(3, 0, 1)`；解析失败（含 `*` / 空串）返回 `None`，不判。
fn version_tuple(text: &str) -> Option<Vec<u32>> {
    text.split('.')
        .map(|part| part.parse::<u32>().ok())
        .collect()
}

fn check_skyline(
    path: &str,
    positions: &JsonPositions,
    skyline: &Map<String, Value>,
    known_keys_only: bool,
    out: &mut Vec<Diagnostic>,
) {
    for (key, value) in skyline {
        if key == "convertRpxToVw" {
            continue;
        }
        let Some(kind) = kind_of(key) else {
            if !known_keys_only {
                continue;
            }
            warn(
                out,
                RendererOptions::ID,
                path,
                positions.key(key),
                format!("rendererOptions.skyline 下的 `{key}` 不是已知配置项，运行时不会读取"),
                "已知配置项见 skyline-config/references/app-config.md 的 rendererOptions 表；未知键多半是拼写错误或多层级",
            );
            continue;
        };
        // 页面级与 app 级都读：布局类的 6 个键由运行时读，版本区间由客户端选渲染器时读。
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
            format!("rendererOptions.skyline.{key} 期望版本号字符串，实际是 {value}"),
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
    warn(
        out,
        RendererOptions::ID,
        path,
        positions.key(key),
        message,
        help,
    );
}

fn kind_of(key: &str) -> Option<Kind> {
    KNOWN
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, kind)| *kind)
}

#[cfg(test)]
mod tests {
    use crate::Severity;
    use crate::project::Project;
    use crate::rules::Rule;
    use crate::rules::testutil::Fixture;

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
            &app_with(r#"{"skyline":{"defaultDisplayBlock":"true","enableImagePreload":1}}"#),
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
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("enableImagePreload"))
        );
    }

    #[test]
    fn accepts_app_level_convert_rpx_to_vw() {
        let (_f, project) = project(
            "renderer-options-rpx-to-vw-top",
            r#"{"pages":["pages/index/index"],"renderer":"skyline","convertRpxToVw":true}"#,
            "{}",
        );
        assert!(RendererOptions.check(&project).is_empty());
    }

    #[test]
    fn reports_convert_rpx_to_vw_inside_renderer_options() {
        // app 级与页面级、顶层与 skyline 层：运行时都不读。
        let (_f, project) = project(
            "renderer-options-rpx-to-vw-nested",
            r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"convertRpxToVw":true,"skyline":{"convertRpxToVw":false}}}"#,
            r#"{"rendererOptions":{"skyline":{"convertRpxToVw":true}}}"#,
        );
        let diags = RendererOptions.check(&project);
        assert_eq!(diags.len(), 3, "{diags:?}");
        assert!(
            diags.iter().all(
                |d| d.message.contains("convertRpxToVw") && d.message.contains("app.json 顶层")
            ),
            "{diags:?}"
        );
        assert_eq!(
            diags
                .iter()
                .filter(|d| d.path == "pages/index/index.json")
                .count(),
            1
        );
    }

    #[test]
    fn reports_non_boolean_app_level_convert_rpx_to_vw() {
        let (_f, project) = project(
            "renderer-options-rpx-to-vw-type",
            r#"{"pages":["pages/index/index"],"renderer":"skyline","convertRpxToVw":"true"}"#,
            "{}",
        );
        let diags = RendererOptions.check(&project);
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert!(diags[0].message.contains("convertRpxToVw"));
        assert!(
            diags[0].message.contains("期望 boolean"),
            "{}",
            diags[0].message
        );
    }

    #[test]
    fn reports_unknown_app_level_top_keys() {
        // 运行时只读 rendererOptions.defaultDisplayBlock 与 rendererOptions.skyline.*，
        // 其余顶层键（含拼错的）静默失效。
        let (_f, project) = project(
            "renderer-options-app-top-unknown",
            r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"defaultDispayBlock":true,"__useTextStyle":true}}"#,
            "{}",
        );
        let diags = RendererOptions.check(&project);
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert!(diags[0].message.contains("defaultDispayBlock"), "{diags:?}");
        assert!(diags[0].message.contains("没有读者"), "{diags:?}");
    }

    #[test]
    fn reports_unknown_page_level_top_keys() {
        // 页面级只读 rendererOptions.skyline.*，顶层写什么都不生效。
        let (_f, project) = project(
            "renderer-options-page-top-unknown",
            r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            r#"{"rendererOptions":{"whatever":true}}"#,
        );
        let diags = RendererOptions.check(&project);
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].path, "pages/index/index.json");
        assert!(diags[0].message.contains("whatever"), "{diags:?}");
    }

    #[test]
    fn reports_app_level_top_keys_that_only_work_under_skyline() {
        let (_f, project) = project(
            "renderer-options-app-top-skyline-only",
            r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"defaultContentBox":true,"enableImagePreload":true,"enableScrollViewAutoSize":true,"tagNameStyleIsolation":"isolated","keyframeStyleIsolation":"isolated"}}"#,
            "{}",
        );
        let diags = RendererOptions.check(&project);
        assert_eq!(diags.len(), 5, "{diags:?}");
        for key in [
            "defaultContentBox",
            "enableImagePreload",
            "enableScrollViewAutoSize",
            "tagNameStyleIsolation",
            "keyframeStyleIsolation",
        ] {
            assert!(
                diags
                    .iter()
                    .any(|d| d.message.contains(key)
                        && d.message.contains("rendererOptions.skyline")),
                "应报 {key}: {diags:?}"
            );
        }
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

    /// 页面级与 app 级都读：版本区间与 `disableABTest` 由客户端选渲染器时读
    /// （官方文档写明「app.json 或 page.json」），不能当死配置报。
    #[test]
    fn page_level_version_ranges_are_legal() {
        let (_f, project) = project(
            "renderer-options-page-version-ranges",
            r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            r#"{"rendererOptions":{"skyline":{"disableABTest":true,"iosVersionBegin":"15.255.254","iosVersionEnd":"15.255.255","ohosVersionBegin":"8.0.18","ohosVersionEnd":"15.255.255"}}}"#,
        );
        assert!(RendererOptions.check(&project).is_empty());
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
            r#"{"pages":["pages/index/index"],"renderer":"skyline","convertRpxToVw":false,"rendererOptions":{"skyline":{"defaultDisplayBlock":true,"defaultContentBox":false,"tagNameStyleIsolation":"legacy","keyframeStyleIsolation":"isolated","enableScrollViewAutoSize":true,"enableImagePreload":false,"disableABTest":true,"sdkVersionBegin":"3.0.0","sdkVersionEnd":"15.255.255","iosVersionBegin":"10.0.0","iosVersionEnd":"15.255.255","androidVersionBegin":"5.0.0","androidVersionEnd":"15.255.255"}}}"#,
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

    #[test]
    fn reports_unrecognized_renderer_value() {
        // 编译器与加载器都是精确比较，`Skyline` 既不算 Skyline 也不算 opt-out。
        let (_f, project) = project(
            "renderer-case",
            r#"{"pages":["pages/index/index"],"renderer":"Skyline"}"#,
            "{}",
        );
        let diags = RendererOptions.check(&project);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert!(diags[0].message.contains("renderer"));
        assert_eq!((diags[0].line, diags[0].column), (1, 32));
    }

    #[test]
    fn reports_empty_version_window() {
        let (_f, project) = project(
            "renderer-version-window",
            &app_with(
                r#"{"skyline":{"sdkVersionBegin":"3.14.2","sdkVersionEnd":"3.3.3","iosVersionBegin":"8.0.40","iosVersionEnd":"8.0.39"}}"#,
            ),
            "{}",
        );
        let diags = RendererOptions.check(&project);
        assert_eq!(diags.len(), 2, "{diags:#?}");
        assert!(diags.iter().all(|d| d.message.contains("版本区间为空")));
    }

    #[test]
    fn accepts_documented_version_window_and_ohos_keys() {
        let (_f, project) = project(
            "renderer-version-ok",
            &app_with(
                r#"{"skyline":{"sdkVersionBegin":"3.0.1","sdkVersionEnd":"15.255.255","ohosVersionBegin":"1.0.5","ohosVersionEnd":"15.255.255"}}"#,
            ),
            "{}",
        );
        assert!(RendererOptions.check(&project).is_empty());
    }

    #[test]
    fn unparsable_versions_are_not_compared() {
        let (_f, project) = project(
            "renderer-version-odd",
            &app_with(r#"{"skyline":{"sdkVersionBegin":"3.0.x","sdkVersionEnd":"3.0.1"}}"#),
            "{}",
        );
        assert!(RendererOptions.check(&project).is_empty());
    }
}
