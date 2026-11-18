//! `skyline/default-layout-drift`：工程没开 `defaultDisplayBlock` / `defaultContentBox`
//! 时，Skyline 的默认布局与盒模型仍然和 WebView 不一样。
//!
//! 这两个开关是**工程级**的迁移决定：`defaultDisplayBlock` 让 `display`（flex→block）、
//! `flex-direction`（column→row）、`align-items`（stretch→normal）对齐 WebView，
//! `defaultContentBox` 让 `box-sizing`（border-box→content-box）对齐 WebView。迁移中的
//! 工程整体还按 WebView 的默认值写样式，所以按工程报一条 warning，而不是逐个元素/逐条声明报；
//! 消息里附「受影响最多的 5 个文档」让这条工程级告警能落到具体页面上。
//!
//! 两个刻意的取舍：
//! - 工程里存在 Skyline 页面、或**根本没有声明 renderer**（工具按 Skyline 默认口径检查）时判。
//!   后者是「准备迁移」的工程：97 条 `flex-direction` 之类的告警全都由这一处配置引起，
//!   不在这里说一句，报告里就只剩逐条改样式的建议。显式声明 WebView 的工程不判；
//! - 布尔值只认 JSON `true`。字符串 `"true"` 按未开启处理（类型写错由
//!   `skyline/renderer-options` 负责），不在这条规则里再解析一遍真值。

use std::collections::HashMap;

use crate::input::json::JsonPositions;
use crate::matcher::Dialect;
use crate::project::Project;
use crate::rules::wxml::{cascade::resolved_property_with_dialect, for_each_element};
use crate::rules::{Rule, RuleMeta, Target};
use crate::{Diagnostic, Severity};

pub struct DefaultLayoutDrift;

impl DefaultLayoutDrift {
    pub const ID: &'static str = "skyline/default-layout-drift";
}

impl Rule for DefaultLayoutDrift {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "app.json 未开启 defaultDisplayBlock / defaultContentBox，Skyline 的默认布局与盒模型仍与 WebView 不同",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        if !super::applicable_project_level(project) {
            return Vec::new();
        }
        // 显式声明了 renderer 却没有任何 Skyline 页面：纯 WebView 工程，还没开始迁移。
        // 没声明 renderer 时工具按 Skyline 默认口径检查（报告末尾有那句 note），照报。
        if project.declares_renderer() && !project.pages.iter().any(|page| page.is_skyline) {
            return Vec::new();
        }
        // `rendererOptions` 或它下面 `skyline` 写成非对象时静默：那是
        // `skyline/renderer-options` 的职责，本规则不重复报同一份写坏的配置。
        if project
            .app_json
            .get("rendererOptions")
            .is_some_and(|options| {
                !options.is_object()
                    || options
                        .get("skyline")
                        .is_some_and(|skyline| !skyline.is_object())
            })
        {
            return Vec::new();
        }

        // 没有 `rendererOptions` 键时落在文件开头。
        let (line, column) = JsonPositions::new(&project.app_json_source)
            .key("rendererOptions")
            .unwrap_or((1, 1));
        let mut out = Vec::new();

        // 只判 app 级，不逐页合并页面级覆盖：页面级 `rendererOptions.skyline.*` 可以让个别
        // 页面换回 WebView 默认值，但把「app 级 + 每页覆盖」做成配置合并器会把这条简单规则
        // 复杂化。因此口径固定为「app 级未开启就报」，消息里也写「app 级未开启」——命中本身
        // 意味着这个工程还没做整体决定。
        if !project.app_display_block() {
            out.push(warning(
                "app.json",
                line,
                column,
                "/rendererOptions/skyline/defaultDisplayBlock",
                "app 级未开启 defaultDisplayBlock：Skyline 下 display 默认 flex、flex-direction 默认 column、align-items 默认 stretch，与 WebView 的 block 布局不同，子元素都按 flex item 计算（垂直外边距不再折叠，宽高与对齐语义都变）；此外 position 默认 relative（WebView 是 static），没有开关能对齐",
                "在 app.json 的 rendererOptions.skyline 里开启 defaultDisplayBlock: true 可让这三项对齐 WebView",
            ));
        }
        out.extend(content_box_drift(project, line, column));
        out
    }
}

/// `box-sizing` 的两端差异，按元素实测而不是按开关猜。
///
/// 判据是「同一份样式在两端解析出的有效值是否不同」：WebView 侧按规范默认
/// `content-box`，Skyline 侧按 `defaultContentBox` 决定默认值。这样两种工程形态都能
/// 判对——没有 reset 时逐个元素命中默认值差异；有 `* { box-sizing: border-box }` 这类
/// reset 时，WebView 全部变成 border-box，而 Skyline 会整条丢弃主体为空的选择器
/// （`style/rule_set.cc` 的 `ExtractRuleBySelector`），回落到默认值，此时若开了
/// `defaultContentBox` 反而是 Skyline 单方面变 content-box。
fn content_box_drift(project: &Project, line: u32, column: u32) -> Option<Diagnostic> {
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
        let webview = box_sizing(webview).unwrap_or("content-box");
        let skyline = box_sizing(skyline).unwrap_or(skyline_default);
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
    Some(warning(
        "app.json",
        line,
        column,
        "/rendererOptions/skyline/defaultContentBox",
        message,
        help,
    ))
}

/// 声明值归一；动态值（模板串）判不了。
fn box_sizing<'a>(
    resolved: Option<crate::rules::wxml::cascade::Resolved<'a>>,
) -> Option<&'static str> {
    match resolved?.value.trim().to_ascii_lowercase().as_str() {
        "content-box" => Some("content-box"),
        "border-box" => Some("border-box"),
        _ => None,
    }
}

fn warning(
    path: &str,
    line: u32,
    column: u32,
    pointer: &str,
    message: impl Into<String>,
    help: impl Into<String>,
) -> Diagnostic {
    Diagnostic::new(
        DefaultLayoutDrift::ID,
        Severity::Warning,
        message,
        path,
        line,
        column,
    )
    .help(help)
    // 开关缺失，没有可哈希的值；指针本身已能区分两条诊断。
    .json(crate::diag::JsonAnchor::new(pointer, None))
}

#[cfg(test)]
mod tests {
    use crate::Severity;
    use crate::project::Project;
    use crate::rules::Rule;
    use crate::rules::testutil::{Fixture, config};

    use super::DefaultLayoutDrift;

    fn project(name: &str, app: &str, page: &str) -> (Fixture, Project) {
        config::project(name, app, page)
    }

    #[test]
    fn reports_both_switches_when_none_is_enabled() {
        let (_f, project) = project(
            "drift-both-off",
            r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            "{}",
        );
        let diags = DefaultLayoutDrift.check(&project);
        assert_eq!(diags.len(), 2);
        assert!(diags.iter().all(|d| d.severity == Severity::Warning));
        assert!(diags.iter().all(|d| d.path == "app.json"));
        // 没有 rendererOptions 键：落在文件开头。
        assert!(diags.iter().all(|d| (d.line, d.column) == (1, 1)));
        assert!(diags[0].message.contains("defaultDisplayBlock"));
        assert!(diags[0].message.contains("flex-direction 默认 column"));
        assert!(diags[0].message.contains("align-items 默认 stretch"));
        // `position` 没有开关（Skyline 初值 relative、CSS 初值 static），并进第一条消息；
        // `text-align` 两边初值都是 start，不构成差异。
        assert!(diags[0].message.contains("position 默认 relative"));
        assert!(!diags[0].message.contains("text-align"));
        assert!(diags[1].message.contains("defaultContentBox"));
        assert!(diags[1].message.contains("border-box"));
    }

    #[test]
    fn reports_only_content_box_when_display_block_is_on() {
        let (_f, project) = project(
            "drift-display-block-on",
            r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"defaultDisplayBlock":true}}}"#,
            "{}",
        );
        let diags = DefaultLayoutDrift.check(&project);
        assert_eq!(diags.len(), 1);
        assert!(
            diags[0].message.contains("defaultContentBox"),
            "{}",
            diags[0].message
        );
    }

    #[test]
    fn page_level_override_does_not_silence_the_app_level_report() {
        // 页面级开了两个开关，但 app 级没开：按「app 级未开启就报」的口径仍然要报。
        let (_f, project) = project(
            "drift-page-level-on",
            r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            r#"{"rendererOptions":{"skyline":{"defaultDisplayBlock":true,"defaultContentBox":true}}}"#,
        );
        assert_eq!(DefaultLayoutDrift.check(&project).len(), 2);
    }

    #[test]
    fn accepts_webview_projects() {
        // 显式声明 WebView 的工程还没开始迁移，不报。
        let (_f, project) = project(
            "drift-webview",
            r#"{"pages":["pages/index/index"],"renderer":"webview"}"#,
            "{}",
        );
        assert!(DefaultLayoutDrift.check(&project).is_empty());
    }

    #[test]
    fn reports_when_the_project_declares_no_renderer() {
        // 没声明 renderer：工具按 Skyline 默认口径检查，这两个默认值差异同样成立，
        // 而且报告里 90% 的告警都由这一处配置引起，必须说出来。
        let (_f, project) = project(
            "drift-undeclared",
            r#"{"pages":["pages/index/index"]}"#,
            "{}",
        );
        assert_eq!(DefaultLayoutDrift.check(&project).len(), 2);
    }

    #[test]
    fn accepts_both_switches_on() {
        let (_f, project) = project(
            "drift-both-on",
            r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"defaultDisplayBlock":true,"defaultContentBox":true}}}"#,
            "{}",
        );
        assert!(DefaultLayoutDrift.check(&project).is_empty());
    }

    #[test]
    fn accepts_app_level_top_level_display_block() {
        // app 级顶层写法运行时也认（runtime/index.ts:142）；defaultContentBox 只有嵌套写法。
        let (_f, project) = project(
            "drift-app-top-level",
            r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"defaultDisplayBlock":true,"skyline":{"defaultContentBox":true}}}"#,
            "{}",
        );
        assert!(DefaultLayoutDrift.check(&project).is_empty());
    }

    #[test]
    fn ignores_a_non_object_renderer_options() {
        // 整块写坏由 skyline/renderer-options 报，这里不重复。
        let (_f, project) = project(
            "drift-not-object",
            r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":"on"}"#,
            "{}",
        );
        assert!(DefaultLayoutDrift.check(&project).is_empty());
    }

    #[test]
    fn ignores_a_non_object_skyline_layer() {
        // `rendererOptions.skyline` 写坏同样由 skyline/renderer-options 报，这里不重复。
        let (_f, project) = project(
            "drift-skyline-not-object",
            r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":"on"}}"#,
            "{}",
        );
        assert!(DefaultLayoutDrift.check(&project).is_empty());
    }

    #[test]
    fn string_booleans_are_truthy_like_the_runtime() {
        // 运行时是 `rendererOptions.skyline.defaultDisplayBlock || false`，字符串 "false"
        // 也是非空字符串 —— 同样算开启。类型写错由 skyline/renderer-options 负责报。
        let (_f, project) = project(
            "drift-string-bool",
            r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"defaultDisplayBlock":"true","defaultContentBox":"true"}}}"#,
            "{}",
        );
        assert!(DefaultLayoutDrift.check(&project).is_empty());
    }

    #[test]
    fn app_level_display_block_wins_over_the_nested_layer() {
        // `runtime/index.ts:141-144` 是 `顶层 || 嵌套`：顶层 true 就算开了。
        let (_f, project) = project(
            "drift-top-wins",
            r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"defaultDisplayBlock":true,"skyline":{"defaultDisplayBlock":false,"defaultContentBox":true}}}"#,
            "{}",
        );
        assert!(DefaultLayoutDrift.check(&project).is_empty());
    }

    /// `* { box-sizing: border-box }` 这类 reset：WebView 全变 border-box，Skyline 丢
    /// 主体为空的选择器后回落到默认 border-box，两端其实一致——不能报。
    #[test]
    fn universal_reset_silences_the_content_box_report() {
        let (_f, project) = reset_project(
            "drift-universal-reset",
            r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"defaultDisplayBlock":true}}}"#,
            "*{box-sizing:border-box}",
        );
        assert!(DefaultLayoutDrift.check(&project).is_empty());
    }

    /// 同上但开关已开：Skyline 默认变 content-box，而 `*` 的 border-box 进不了规则集，
    /// 这时两端反而不同。建议必须是「改选择器」，不能是「改开关」。
    #[test]
    fn universal_reset_with_the_switch_on_reports_the_other_direction() {
        let (_f, project) = reset_project(
            "drift-universal-reset-on",
            r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"defaultDisplayBlock":true,"defaultContentBox":true}}}"#,
            "*{box-sizing:border-box}",
        );
        let diags = DefaultLayoutDrift.check(&project);
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert!(diags[0].message.contains("Skyline content-box"));
        assert!(!diags[0].message.contains("未开启 defaultContentBox"));
        assert!(diags[0].help.as_deref().unwrap().contains("标签"));
    }

    /// 标签选择器的 reset 两端都生效，不构成差异。
    #[test]
    fn tag_reset_keeps_both_renderers_equal() {
        let (_f, project) = reset_project(
            "drift-tag-reset",
            r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"defaultDisplayBlock":true}}}"#,
            "view{box-sizing:border-box}",
        );
        assert!(DefaultLayoutDrift.check(&project).is_empty());
    }

    /// 类选择器同理：两端都进桶。
    #[test]
    fn class_reset_keeps_both_renderers_equal() {
        let (_f, project) = reset_project(
            "drift-class-reset",
            r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"defaultDisplayBlock":true}}}"#,
            ".page{box-sizing:border-box}",
        );
        assert!(DefaultLayoutDrift.check(&project).is_empty());
    }

    /// 工程级一条告警要能落到页面：消息里列出受影响最多的文档。
    #[test]
    fn message_lists_the_most_affected_pages() {
        let fixture = Fixture::new("drift-hot-pages");
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
        let project = fixture.project();
        let diags = DefaultLayoutDrift.check(&project);
        assert_eq!(diags.len(), 1);
        let message = &diags[0].message;
        assert!(message.contains("受影响最多的文档"), "{message}");
        // 元素多的页面排在前面。
        let big = message.find("pages/big/big").expect("缺少 big 页");
        let index = message.find("pages/index/index").expect("缺少 index 页");
        assert!(big < index, "{message}");
    }

    /// app.wxss 由用例给；页面 wxml 用 reset 能命中的写法。
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
}
