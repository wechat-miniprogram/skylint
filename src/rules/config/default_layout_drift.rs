//! `skyline/default-layout-drift`：工程没开 `defaultDisplayBlock` 时，Skyline 的默认布局
//! 仍然和 WebView 不一样。
//!
//! 这个开关是**工程级**的迁移决定：`defaultDisplayBlock` 让 `display`（flex→block）、
//! `flex-direction`（column→row）、`align-items`（stretch→normal）对齐 WebView。迁移中的
//! 工程整体还按 WebView 的默认值写样式，所以按工程报一条 warning，而不是逐个元素报。
//!
//! `box-sizing` 的默认值差异不在本规则里：它和「声明了非法值 / `initial` 的回落值不同」
//! 是同一个修法与同一个属性，统一在 `skyline/box-sizing` 报（那里同样带热点文档列表）。
//!
//! 一个刻意的取舍：
//! - 工程里存在 Skyline 页面、或**根本没有声明 renderer**（工具按 Skyline 默认口径检查）时判。
//!   后者是「准备迁移」的工程：97 条 `flex-direction` 之类的告警全都由这一处配置引起，
//!   不在这里说一句，报告里就只剩逐条改样式的建议。显式声明 WebView 的工程不判；
//! - 布尔值只认 JSON `true`。字符串 `"true"` 按未开启处理（类型写错由
//!   `skyline/renderer-options` 负责），不在这条规则里再解析一遍真值。

use crate::input::json::JsonPositions;
use crate::project::Project;
use crate::rules::{Rule, RuleMeta, VisualCase, VisualExpect};
use crate::{Diagnostic, Severity};

pub struct DefaultLayoutDrift;

impl DefaultLayoutDrift {
    pub const ID: &'static str = "skyline/default-layout-drift";
}

/// 视觉用例：app.json 没开 `defaultDisplayBlock`，相邻块的垂直外边距行为不同。
///
/// WebView 里两个兄弟块处于普通块级布局，`margin-bottom: 20px` 与 `margin-top: 30px`
/// 折叠成 30px；Skyline 的 `display: flex; flex-direction: column` 不折叠，间距 50px。
static VISUAL_CASES: &[VisualCase] = &[VisualCase {
    wxml: r#"<view class="page">
  <view class="first"></view>
  <view class="second" id="target"></view>
</view>"#,
    wxss: ".page { padding: 20px; background: #f5f5f5; }\n.first { height: 40px; margin-bottom: 20px; background: #e54d42; }\n.second { height: 40px; margin-top: 30px; background: #1e88e5; }\n",
    js: None,
    expect: VisualExpect::Differ,
    note: "未开 defaultDisplayBlock：WebView 折叠相邻外边距，间距 30px；Skyline 是 flex column，不折叠，间距 50px，第二个块下移 20px",
    target: "#target",
}];

impl Rule for DefaultLayoutDrift {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "app.json 未开启 defaultDisplayBlock / defaultContentBox，Skyline 的默认布局与盒模型仍与 WebView 不同",
        }
    }

    fn visual_cases(&self) -> &'static [VisualCase] {
        VISUAL_CASES
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        if !super::applicable_renderer_options(project) {
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
        out
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

    use super::{DefaultLayoutDrift, VISUAL_CASES};
    use crate::rules::testutil::check_rule;

    fn project(name: &str, app: &str, page: &str) -> (Fixture, Project) {
        config::project(name, app, page)
    }

    /// 视觉用例的同一份 WXML/WXSS（页面级配置）必须真的能触发本规则。
    #[test]
    fn visual_case_triggers_rule() {
        let case = &VISUAL_CASES[0];
        let fixture = Fixture::new("drift-visual");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", case.wxml)
            .write("pages/index/index.wxss", case.wxss);
        let diagnostics = check_rule(&fixture, &DefaultLayoutDrift);
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.rule == DefaultLayoutDrift::ID),
            "视觉用例未触发规则：{diagnostics:#?}"
        );
    }

    #[test]
    fn reports_display_switch_when_none_is_enabled() {
        let (_f, project) = project(
            "drift-both-off",
            r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            "{}",
        );
        let diags = DefaultLayoutDrift.check(&project);
        assert_eq!(diags.len(), 1);
        assert!(diags.iter().all(|d| d.severity == Severity::Warning));
        assert!(diags.iter().all(|d| d.path == "app.json"));
        // 没有 rendererOptions 键：落在文件开头。
        assert!(diags.iter().all(|d| (d.line, d.column) == (1, 1)));
        assert!(diags[0].message.contains("defaultDisplayBlock"));
        assert!(diags[0].message.contains("flex-direction 默认 column"));
        assert!(diags[0].message.contains("align-items 默认 stretch"));
        // `position` 没有开关（Skyline 初值 relative、CSS 初值 static），并进这条消息；
        // `text-align` 两边初值都是 start，不构成差异。
        assert!(diags[0].message.contains("position 默认 relative"));
        assert!(!diags[0].message.contains("text-align"));
        // `box-sizing` 的默认值差异归 `skyline/box-sizing`。
        assert!(!diags[0].message.contains("defaultContentBox"));
    }

    #[test]
    fn accepts_display_block_only() {
        let (_f, project) = project(
            "drift-display-block-on",
            r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"defaultDisplayBlock":true}}}"#,
            "{}",
        );
        // box-sizing 的默认值差异已划归 `skyline/box-sizing`，这条规则只管 display 一族。
        assert!(DefaultLayoutDrift.check(&project).is_empty());
    }

    #[test]
    fn page_level_override_does_not_silence_the_app_level_report() {
        // 页面级开了两个开关，但 app 级没开：按「app 级未开启就报」的口径仍然要报。
        let (_f, project) = project(
            "drift-page-level-on",
            r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            r#"{"rendererOptions":{"skyline":{"defaultDisplayBlock":true,"defaultContentBox":true}}}"#,
        );
        assert_eq!(DefaultLayoutDrift.check(&project).len(), 1);
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
        // 没声明 renderer：工具按 Skyline 默认口径检查，这个默认值差异同样成立，
        // 而且报告里 90% 的告警都由这一处配置引起，必须说出来。
        let (_f, project) = project(
            "drift-undeclared",
            r#"{"pages":["pages/index/index"]}"#,
            "{}",
        );
        assert_eq!(DefaultLayoutDrift.check(&project).len(), 1);
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
}
