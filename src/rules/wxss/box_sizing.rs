//! `box-sizing` 两端不一致：声明级与默认值级两种形态都在这条规则里。
//!
//! **声明级**：`box-sizing` 在 Skyline 下只接受 `content-box` / `border-box`。
//! `padding-box` / `margin-box` 在 WebView 同样非法（MDN formal syntax 只有
//! `content-box | border-box`），两端都丢弃该声明，差异只在**回落值**：未开
//! `defaultContentBox` 时 Skyline 回落 `border-box`（`skyline/css/css_properties/box_sizing.cc`
//! 的 ApplyInitial），WebView 回落 `content-box`；开了开关后两端都回落
//! `content-box`。`initial` / `unset` / `revert` 等全局关键字走同一条 ApplyInitial
//! 路径，同样会得到 Skyline 的 border-box。
//!
//! **默认值级**：元素根本没写 `box-sizing` 时，两端默认值也不同（WebView 按规范
//! `content-box`，Skyline 按 `defaultContentBox` 取）。这一层按元素实测、在
//! `app.json` 上报工程级一条，消息里附受影响最多的文档。
//!
//! 两层共用一个 id：它们的修法是同一个（显式写 `box-sizing` 或开开关），只是定位
//! 粒度不同——声明级能指到行，默认值级只能指到工程 + 热点页面。

use std::collections::HashMap;

use crate::input::json::JsonPositions;
use crate::matcher::Dialect;
use crate::project::Project;
use crate::{Diagnostic, Severity};

use super::value_util::{at_value_with, is_dynamic};
use super::visit_declarations;
use crate::rules::wxml::{
    cascade::resolved_property_with_dialect, element_diagnostic, for_each_element,
};
use crate::rules::{Rule, RuleMeta, Target};

pub struct BoxSizing;

impl BoxSizing {
    pub const ID: &'static str = "skyline/box-sizing";
}

impl Rule for BoxSizing {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Error,
            description: "box-sizing 两端不一致：非法取值/initial 的回落值不同，或未开 defaultContentBox 时默认值不同",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut out = visit_declarations(project, |sheet, _rule, decl, out| {
            let doc = &sheet.doc;
            if !decl.name.eq_ignore_ascii_case("box-sizing") {
                return;
            }
            let value = decl.value.trim();
            if value.is_empty() {
                return;
            }
            let lower = value.to_ascii_lowercase();
            // 变量 / 插值无法静态判定，交给运行时（$var、@var、#{} 同样算）。
            if is_dynamic(&decl.tokens) {
                return;
            }
            if is_box_sizing_value(&lower) {
                // `initial` / `unset` / `revert` 落到 ApplyInitial：只有开启
                // defaultContentBox 才是 content-box。
                if is_global_value(&lower) && !project.app_content_box() {
                    out.push(at_value_with(
                        Self::ID,
                        doc,
                        decl,
                        Severity::Warning,
                        format!("`box-sizing: {value}` 在 Skyline 下会解析成 border-box（WebView 是 content-box）"),
                        "显式写 content-box 或 border-box：未开启 defaultContentBox 时 Skyline 的初始值是 border-box",
                    ));
                }
                return;
            }
            // 非法值两端都会丢弃，差异只在回落值；开了 defaultContentBox 后两端都
            // 回落 content-box，没有可见差异，不报。
            if project.app_content_box() {
                return;
            }
            out.push(at_value_with(
                Self::ID,
                doc,
                decl,
                Severity::Error,
                format!("`box-sizing: {value}` 在 Skyline 与 WebView 下都不是合法值，该声明会被忽略；Skyline 回落 border-box，WebView 回落 content-box"),
                "Skyline 只接受 content-box / border-box；未开启 defaultContentBox 时它的回落值是 border-box（WebView 是 content-box），请显式声明并复核元素尺寸",
            ));
        });

        out.extend(default_drift(project));
        drift_elements(project, &mut out);
        out
    }
}

/// 默认值级差异里**真的会改变布局**的那部分，逐元素指到 WXML。
///
/// 工程级一条告警只说「多少元素不同」，落到代码还得自己找。真正有可见后果的是
/// 「某一轴写了尺寸 + 同一轴有非零 padding」的组合：border-box 会把 padding 挤进
/// 尺寸里（`width:750rpx; padding:0 36rpx` 就是 `address/pages/index-v3` 骨架屏
/// 那一次）。这类点比全部漂移元素少一个量级，按元素报得动。
fn drift_elements(project: &Project, out: &mut Vec<Diagnostic>) {
    if project.app_content_box() {
        return;
    }
    for_each_element(project, Target::Skyline, |site| {
        let drift = |dialect| {
            resolved_property_with_dialect(
                site.project,
                site.tree,
                site.element,
                "box-sizing",
                dialect,
            )
        };
        let (webview, webview_undecidable) = drift(Dialect::WebView);
        if webview_undecidable {
            return;
        }
        let (skyline, skyline_undecidable) = drift(Dialect::Skyline);
        if skyline_undecidable {
            return;
        }
        if resolved_box_sizing(webview).unwrap_or("content-box") != "content-box"
            || resolved_box_sizing(skyline).unwrap_or("border-box") != "border-box"
        {
            return;
        }

        let axis = sized_axis_with_padding(site.project, site.tree, site.element);
        let Some((size_property, size_value, padding)) = axis else {
            return;
        };
        out.push(
            element_diagnostic(
                BoxSizing::ID,
                site.doc,
                site.element,
                site.doc.path(),
                Severity::Info,
                format!(
                    "`<{}>` 写了 `{size_property}: {size_value}` 和同轴 padding `{padding}`：Skyline（border-box）把这些 padding 算进尺寸里，内容区比 WebView 小",
                    site.element.tag
                ),
            )
            .help("显式写 `box-sizing: content-box`，把尺寸挪到外层元素，或开启 rendererOptions.skyline.defaultContentBox"),
        );
    });
}

/// 找出「某轴有确定尺寸且有同轴非零 padding」的那一轴。
fn sized_axis_with_padding(
    project: &Project,
    tree: &crate::matcher::Tree<'_>,
    element: &crate::input::wxml::WxmlElement,
) -> Option<(&'static str, String, String)> {
    for (size_property, padding_properties) in [
        (
            "width",
            ["padding-left", "padding-right", "padding"].as_slice(),
        ),
        (
            "height",
            ["padding-top", "padding-bottom", "padding"].as_slice(),
        ),
    ] {
        let Some(size) = definite_value(project, tree, element, size_property) else {
            continue;
        };
        for padding_property in padding_properties {
            if let Some(padding) = non_zero_value(project, tree, element, padding_property) {
                return Some((size_property, size, padding));
            }
        }
    }
    None
}

/// 写了且不是「按内容 / 按父级」的尺寸值。
fn definite_value(
    project: &Project,
    tree: &crate::matcher::Tree<'_>,
    element: &crate::input::wxml::WxmlElement,
    property: &str,
) -> Option<String> {
    let (resolved, undecidable) =
        resolved_property_with_dialect(project, tree, element, property, Dialect::Skyline);
    if undecidable {
        return None;
    }
    let value = resolved?.value.trim().to_ascii_lowercase();
    let content_sized = matches!(
        value.as_str(),
        "" | "auto"
            | "none"
            | "inherit"
            | "initial"
            | "unset"
            | "revert"
            | "min-content"
            | "max-content"
            | "fit-content"
    );
    (!content_sized).then_some(value)
}

/// 写了且不为零的 padding 值。
fn non_zero_value(
    project: &Project,
    tree: &crate::matcher::Tree<'_>,
    element: &crate::input::wxml::WxmlElement,
    property: &str,
) -> Option<String> {
    let (resolved, undecidable) =
        resolved_property_with_dialect(project, tree, element, property, Dialect::Skyline);
    if undecidable {
        return None;
    }
    let value = resolved?.value.trim().to_ascii_lowercase();
    if matches!(value.as_str(), "inherit" | "initial" | "unset" | "revert") {
        return None;
    }
    let all_zero = value
        .split_whitespace()
        .all(crate::rules::wxml::style::is_zero);
    (!all_zero && !value.is_empty()).then_some(value)
}

/// 默认值级的差异：元素没写 `box-sizing` 时，两端默认值不同。
///
/// 按元素实测而不是按开关猜：WebView 侧按规范默认 `content-box`，Skyline 侧按
/// `defaultContentBox` 决定默认值。这样两种工程形态都能判对——没有 reset 时逐个元素
/// 命中默认值差异；有 `* { box-sizing: border-box }` 这类 reset 时，WebView 全部变成
/// border-box，而 Skyline 会整条丢弃主体为空的选择器（`style/rule_set.cc` 的
/// `ExtractRuleBySelector`），回落到默认值，此时若开了 `defaultContentBox` 反而是
/// Skyline 单方面变 content-box。
fn default_drift(project: &Project) -> Option<Diagnostic> {
    // 与 `skyline/default-layout-drift` 共用同一道闸门：单文件 / 纯 WebView 工程 /
    // 写坏的 `rendererOptions` 都不报工程级诊断。
    if !crate::rules::config::applicable_renderer_options(project) {
        return None;
    }
    let skyline_default = if project.app_content_box() {
        "content-box"
    } else {
        "border-box"
    };
    // 两个方向的计数分开记：前者是「开关没开」，后者是「reset 的选择器在 Skyline 不生效」，
    // 修法完全相反，混成一个数就没法给建议。
    let mut open_content_box = 0usize;
    let mut unreachable_reset = 0usize;
    // 工程级一条告警定位不到页面；按文档聚合出「哪些页面受影响最多」，让这条告警可操作。
    let mut pages: HashMap<String, usize> = HashMap::new();
    for_each_element(project, Target::Skyline, |site| {
        let (webview, webview_undecidable) = resolved_property_with_dialect(
            site.project,
            site.tree,
            site.element,
            "box-sizing",
            Dialect::WebView,
        );
        if webview_undecidable {
            return;
        }
        let (skyline, skyline_undecidable) = resolved_property_with_dialect(
            site.project,
            site.tree,
            site.element,
            "box-sizing",
            Dialect::Skyline,
        );
        if skyline_undecidable {
            return;
        }
        let webview = resolved_box_sizing(webview).unwrap_or("content-box");
        let skyline = resolved_box_sizing(skyline).unwrap_or(skyline_default);
        match (webview, skyline) {
            ("content-box", "border-box") => {
                open_content_box += 1;
                *pages.entry(site.doc.path().to_string()).or_default() += 1;
            }
            ("border-box", "content-box") => unreachable_reset += 1,
            _ => {}
        }
    });

    // 只看前 5 个：这类差异往往是「几个公共组件 × 页面数」，列多了没有信息量。
    let mut hottest: Vec<(String, usize)> = pages.into_iter().collect();
    hottest.sort_by(|(a_path, a), (b_path, b)| b.cmp(a).then_with(|| a_path.cmp(b_path)));
    let hot_list = hottest
        .iter()
        .take(5)
        .map(|(path, count)| format!("`{path}`（{count} 个）"))
        .collect::<Vec<_>>()
        .join("、");

    let (message, help) = match (open_content_box, unreachable_reset) {
        (0, 0) => return None,
        (count, 0) => (
            format!(
                "app 级未开启 defaultContentBox：{count} 个元素两端解析出的 box-sizing 不同（WebView content-box / Skyline border-box），显式声明尺寸的元素实际内容盒比 WebView 小，少掉的正是同轴 padding 与 border；受影响最多的文档：{hot_list}"
            ),
            "开启 rendererOptions.skyline.defaultContentBox: true 对齐 WebView；不想动全局开关时逐元素显式写 box-sizing: content-box",
        ),
        (0, count) => (
            format!(
                "{count} 个元素的 box-sizing 两端解析结果不同（WebView border-box / Skyline content-box）：WebView 侧的值来自不会进入 Skyline 规则集的选择器（`*`、仅属性选择器），Skyline 回落到默认值"
            ),
            "把这类选择器的主体改成标签（如 view、text）或逐元素显式声明 box-sizing；主体为空的选择器整条规则在 Skyline 不生效，只改开关无法对齐",
        ),
        (open, unreachable) => (
            format!(
                "盒模型默认值两端不一致：{open} 个元素 WebView content-box / Skyline border-box，另有 {unreachable} 个元素相反（Skyline 丢弃了 `*` 或仅属性选择器后回落到默认值）；受影响最多的文档：{hot_list}"
            ),
            "先统一默认值：无 reset 的元素开 defaultContentBox；由 `*` 之类选择器提供的 border-box 改成标签选择器再开开关，否则两边只能对齐一侧",
        ),
    };

    // 没有 `rendererOptions` 键时落在文件开头。
    let (line, column) = JsonPositions::new(&project.app_json_source)
        .key("rendererOptions")
        .unwrap_or((1, 1));
    Some(
        Diagnostic::new(
            BoxSizing::ID,
            Severity::Warning,
            message,
            "app.json",
            line,
            column,
        )
        .help(help)
        // 开关缺失，没有可哈希的值；指针本身已能区分诊断。
        .json(crate::diag::JsonAnchor::new(
            "/rendererOptions/skyline/defaultContentBox",
            None,
        )),
    )
}

/// 类名字符串归一；动态值（模板串）判不了。
fn resolved_box_sizing<'a>(
    resolved: Option<crate::rules::wxml::cascade::Resolved<'a>>,
) -> Option<&'static str> {
    match resolved?.value.trim().to_ascii_lowercase().as_str() {
        "content-box" => Some("content-box"),
        "border-box" => Some("border-box"),
        _ => None,
    }
}

/// 全局关键字：它们的天空值是 `ApplyInitial` 的结果。
fn is_global_value(value: &str) -> bool {
    matches!(value, "initial" | "unset" | "revert" | "revert-layer")
}

fn is_box_sizing_value(value: &str) -> bool {
    matches!(
        value,
        "content-box" | "border-box" | "inherit" | "initial" | "unset" | "revert" | "revert-layer"
    )
}

#[cfg(test)]
mod tests {
    use crate::rules::testutil::wxss::{Fixture, messages, page};
    use crate::{Project, rules::Rule};

    use super::BoxSizing;

    /// 声明级诊断（`app.json` 上的工程级默认值告警不算）。
    fn declarations(project: &Project) -> Vec<String> {
        BoxSizing
            .check(project)
            .into_iter()
            .filter(|diagnostic| diagnostic.path != "app.json")
            .map(|diagnostic| diagnostic.message)
            .collect()
    }

    #[test]
    fn box_sizing_reports_unsupported_values() {
        let (_f, project) = page(
            "box-sizing-positive",
            r#"<view class="a"/><view class="b"/><view class="c"/>"#,
            r#"
            .a { box-sizing: padding-box; }
            .b { box-sizing: padding-box !important; }
            .c { box-sizing: margin-box; }
            "#,
        );
        let diags: Vec<_> = BoxSizing
            .check(&project)
            .into_iter()
            .filter(|diagnostic| diagnostic.path != "app.json")
            .collect();
        assert_eq!(diags.len(), 3);
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Error));
        assert!(diags[0].message.contains("padding-box"));
        assert!(diags[0].message.contains("content-box"));
        assert!(diags[0].help.as_deref().unwrap().contains("border-box"));
    }

    #[test]
    fn box_sizing_accepts_supported_values_and_keywords() {
        let (_f, project) = page(
            "box-sizing-negative",
            r#"<view class="a"/><view class="b"/><view class="c"/><view class="d"/><view class="f"/>"#,
            r#"
            .a { box-sizing: content-box; }
            .b { box-sizing: border-box; }
            .c { box-sizing: inherit; }
            .d { box-sizing: var(--box); }
            .f { padding-box: 1px; }
            "#,
        );
        assert_eq!(declarations(&project), Vec::<String>::new());
    }

    /// 未开启 defaultContentBox 时 `initial` / `unset` 解析成 border-box，与 WebView 不同。
    #[test]
    fn initial_resolves_to_border_box_without_the_flag() {
        let (_f, project) = page(
            "box-sizing-initial",
            r#"<view class="a"/><view class="b"/>"#,
            r#"
            .a { box-sizing: initial; }
            .b { box-sizing: unset; }
            "#,
        );
        let diags: Vec<_> = BoxSizing
            .check(&project)
            .into_iter()
            .filter(|diagnostic| diagnostic.path != "app.json")
            .collect();
        assert_eq!(diags.len(), 2, "{diags:#?}");
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Warning));
        assert!(diags[0].message.contains("border-box"));
    }

    /// 非法值两端都会丢弃，差异只在回落值：开了 defaultContentBox 后两端都回落
    /// content-box，没有可见差异，不报。
    #[test]
    fn invalid_values_are_silent_with_default_content_box() {
        let f = Fixture::new("box-sizing-content-box");
        f.write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"defaultContentBox":true}}}"#,
            )
            .write("pages/index/index.json", "{}")
            .write(
                "pages/index/index.wxml",
                r#"<view class="a"/><view class="b"/>"#,
            )
            .write(
                "pages/index/index.wxss",
                r#"
                .a { box-sizing: padding-box; }
                .b { box-sizing: margin-box; }
                "#,
            );
        assert_eq!(declarations(&f.project()), Vec::<String>::new());
    }

    /// app.wxss 由用例给；页面 wxml 用能命中 reset 的写法。
    fn reset_project(name: &str, app: &str, app_wxss: &str) -> (Fixture, Project) {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write("app.json", app)
            .write("app.wxss", app_wxss)
            .write("pages/index/index.json", "{}")
            .write(
                "pages/index/index.wxml",
                r#"<view class="page"><view class="page"/></view>"#,
            );
        let loaded = fixture.project();
        (fixture, loaded)
    }

    /// 元素没写 `box-sizing` 时两端默认值不同，报一条工程级。
    #[test]
    fn default_value_drift_is_reported() {
        let (_f, project) = reset_project(
            "box-sizing-drift",
            r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"defaultDisplayBlock":true}}}"#,
            "",
        );
        let diags = BoxSizing.check(&project);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert_eq!(diags[0].path, "app.json");
        assert_eq!(diags[0].severity, crate::Severity::Warning);
        assert!(diags[0].message.contains("defaultContentBox"));
        assert!(diags[0].message.contains("受影响最多的文档"));
    }

    /// `* { box-sizing: border-box }` 这类 reset：WebView 全变 border-box，Skyline 丢
    /// 主体为空的选择器后回落到默认 border-box，两端其实一致——不能报。
    #[test]
    fn universal_reset_silences_the_default_drift() {
        let (_f, project) = reset_project(
            "box-sizing-universal-reset",
            r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"defaultDisplayBlock":true}}}"#,
            "*{box-sizing:border-box}",
        );
        // 两端都回落到 border-box：既没有声明级命中，也没有默认值差异。
        assert_eq!(messages(&BoxSizing, &project), Vec::<String>::new());
    }

    /// 同上但开关已开：Skyline 默认变 content-box，而 `*` 的 border-box 进不了规则集，
    /// 这时两端反而不同。建议必须是「改选择器」，不能是「改开关」。
    #[test]
    fn universal_reset_with_the_switch_on_reports_the_other_direction() {
        let (_f, project) = reset_project(
            "box-sizing-universal-reset-on",
            r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"defaultDisplayBlock":true,"defaultContentBox":true}}}"#,
            "*{box-sizing:border-box}",
        );
        let diags = BoxSizing.check(&project);
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert!(diags[0].message.contains("Skyline content-box"));
        assert!(!diags[0].message.contains("未开启 defaultContentBox"));
        assert!(diags[0].help.as_deref().unwrap().contains("标签"));
    }

    /// 标签 / 类选择器的 reset 两端都生效，不构成差异。
    #[test]
    fn reachable_resets_keep_both_renderers_equal() {
        for app_wxss in [
            "view{box-sizing:border-box}",
            ".page{box-sizing:border-box}",
        ] {
            let (_f, project) = reset_project(
                "box-sizing-reachable-reset",
                r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"defaultDisplayBlock":true}}}"#,
                app_wxss,
            );
            assert_eq!(
                messages(&BoxSizing, &project),
                Vec::<String>::new(),
                "{app_wxss}"
            );
        }
    }

    /// 工程级一条告警要能落到页面：消息里列出受影响最多的文档。
    #[test]
    fn message_lists_the_most_affected_pages() {
        let fixture = Fixture::new("box-sizing-hot-pages");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index","pages/big/big"],"renderer":"skyline","rendererOptions":{"skyline":{"defaultDisplayBlock":true}}}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", r#"<view><view/></view>"#)
            .write("pages/big/big.json", "{}")
            .write(
                "pages/big/big.wxml",
                r#"<view><view/><view/><view/><view/></view>"#,
            );
        let diags = BoxSizing.check(&fixture.project());
        assert_eq!(diags.len(), 1);
        let message = &diags[0].message;
        assert!(message.contains("受影响最多的文档"), "{message}");
        // 元素多的页面排在前面。
        let big = message.find("pages/big/big").expect("缺少 big 页");
        let index = message.find("pages/index/index").expect("缺少 index 页");
        assert!(big < index, "{message}");
    }

    /// 预处理器变量不是「不受支持的值」，必须全静默（SCSS / Less 工程上 error 级误报）。
    #[test]
    fn preprocessor_variables_are_not_reported() {
        let (_f, project) = page(
            "box-sizing-dynamic",
            r#"<view class="a"/><view class="b"/><view class="c"/><view class="d"/>"#,
            r#"
            .a { box-sizing: $box; }
            .b { box-sizing: @box; }
            .c { box-sizing: #{$box}; }
            .d { box-sizing: calc(var(--x)); }
            "#,
        );
        assert_eq!(declarations(&project), Vec::<String>::new());
    }

    /// 「某轴写了尺寸 + 同轴非零 padding」的漂移元素要单独指到 WXML——工程级那条只说总数。
    fn element_hits(app_json: &str, wxml: &str, wxss: &str) -> Vec<String> {
        let fixture = Fixture::new("box-sizing-elements");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write("app.json", app_json)
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", wxml)
            .write("pages/index/index.wxss", wxss);
        BoxSizing
            .check(&fixture.project())
            .into_iter()
            .filter(|diagnostic| diagnostic.path.ends_with(".wxml"))
            .map(|diagnostic| diagnostic.message)
            .collect()
    }

    const SKYLINE_APP: &str = r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#;

    #[test]
    fn sized_element_with_matching_padding_is_reported() {
        let hits = element_hits(
            SKYLINE_APP,
            r#"<view class="a">内容</view>"#,
            r#".a { width: 750rpx; padding: 0 36rpx; }"#,
        );
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(hits[0].contains("width: 750rpx"), "{}", hits[0]);
    }

    #[test]
    fn vertical_axis_padding_is_reported() {
        let hits = element_hits(
            SKYLINE_APP,
            r#"<view class="a">内容</view>"#,
            r#".a { height: 82rpx; padding-top: 5rpx; }"#,
        );
        assert_eq!(hits.len(), 1, "{hits:?}");
    }

    #[test]
    fn content_sized_element_is_not_reported() {
        let hits = element_hits(
            SKYLINE_APP,
            r#"<view class="a">内容</view>"#,
            r#".a { width: auto; padding: 0 36rpx; }"#,
        );
        assert!(hits.is_empty(), "{hits:?}");
    }

    #[test]
    fn zero_padding_is_not_reported() {
        let hits = element_hits(
            SKYLINE_APP,
            r#"<view class="a">内容</view>"#,
            r#".a { width: 750rpx; padding: 0; }"#,
        );
        assert!(hits.is_empty(), "{hits:?}");
    }

    #[test]
    fn cross_axis_padding_is_not_reported() {
        // 宽 + 垂直 padding 不会挤宽内容区。
        let hits = element_hits(
            SKYLINE_APP,
            r#"<view class="a">内容</view>"#,
            r#".a { width: 750rpx; padding-top: 8rpx; }"#,
        );
        assert!(hits.is_empty(), "{hits:?}");
    }

    #[test]
    fn explicit_content_box_is_not_reported() {
        let hits = element_hits(
            SKYLINE_APP,
            r#"<view class="a">内容</view>"#,
            r#".a { box-sizing: content-box; width: 750rpx; padding: 0 36rpx; }"#,
        );
        assert!(hits.is_empty(), "{hits:?}");
    }
}
