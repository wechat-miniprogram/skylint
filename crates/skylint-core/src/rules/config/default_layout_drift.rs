//! `skyline/default-layout-drift`：工程没开 `defaultDisplayBlock` / `defaultContentBox`
//! 时，Skyline 的默认布局与盒模型仍然和 WebView 不一样。
//!
//! 这两个开关是**工程级**的迁移决定：`defaultDisplayBlock` 让 `display`（flex→block）、
//! `flex-direction`（column→row）、`align-items`（stretch→normal）对齐 WebView，
//! `defaultContentBox` 让 `box-sizing`（border-box→content-box）对齐 WebView。迁移中的
//! 工程整体还按 WebView 的默认值写样式，所以按工程报一条 info，而不是逐个元素/逐条声明报。
//!
//! 两个刻意的取舍：
//! - 只有工程里存在 Skyline 页面时才判。纯 WebView 工程还没开始迁移，这条提示是噪音；
//! - 布尔值只认 JSON `true`。字符串 `"true"` 按未开启处理（类型写错由
//!   `skyline/renderer-options` 负责），不在这条规则里再解析一遍真值。

use crate::input::json::JsonPositions;
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

pub struct DefaultLayoutDrift;

impl DefaultLayoutDrift {
    pub const ID: &'static str = "skyline/default-layout-drift";
}

impl Rule for DefaultLayoutDrift {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Info,
            description: "app.json 未开启 defaultDisplayBlock / defaultContentBox，Skyline 的默认布局与盒模型仍与 WebView 不同",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        if !project.pages.iter().any(|page| page.is_skyline) {
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
            out.push(
                Diagnostic::new(
                    Self::ID,
                    Severity::Info,
                    "app 级未开启 defaultDisplayBlock：Skyline 下 display 默认 flex、flex-direction 默认 column、align-items 默认 stretch，与 WebView 的 block 布局不同，子元素都按 flex item 计算（垂直外边距不再折叠，宽高与对齐语义都变）；position 默认 relative、text-align 默认 start，这两项没有开关能对齐",
                    "app.json",
                    line,
                    column,
                )
                .help(
                    "在 app.json 的 rendererOptions.skyline 里开启 defaultDisplayBlock: true 可让这三项对齐 WebView",
                ),
            );
        }
        if !project.app_content_box() {
            out.push(
                Diagnostic::new(
                    Self::ID,
                    Severity::Info,
                    "app 级未开启 defaultContentBox：Skyline 下 box-sizing 默认 border-box（WebView 默认 content-box），显式声明 width 的元素实际内容盒会比 WebView 中小（少掉 padding 与 border）",
                    "app.json",
                    line,
                    column,
                )
                .help(
                    "开启 rendererOptions.skyline.defaultContentBox: true 对齐 WebView，或显式写 box-sizing: content-box",
                ),
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
    use crate::rules::wxss::testutil::Fixture;

    use super::DefaultLayoutDrift;

    fn project(name: &str, app: &str, page: &str) -> (Fixture, Project) {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write("app.json", app)
            .write("pages/index/index.json", page)
            .write("pages/index/index.wxml", "<view/>");
        let loaded = fixture.project();
        (fixture, loaded)
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
        assert!(diags.iter().all(|d| d.severity == Severity::Info));
        assert!(diags.iter().all(|d| d.path == "app.json"));
        // 没有 rendererOptions 键：落在文件开头。
        assert!(diags.iter().all(|d| (d.line, d.column) == (1, 1)));
        assert!(diags[0].message.contains("defaultDisplayBlock"));
        assert!(diags[0].message.contains("flex-direction 默认 column"));
        assert!(diags[0].message.contains("align-items 默认 stretch"));
        // position / text-align 没有开关，并进第一条消息，不再单独报。
        assert!(diags[0].message.contains("position"));
        assert!(diags[0].message.contains("text-align"));
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
        // 纯 WebView 工程还没开始迁移，不报。
        let (_f, project) = project("drift-webview", r#"{"pages":["pages/index/index"]}"#, "{}");
        assert!(DefaultLayoutDrift.check(&project).is_empty());
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
    fn string_booleans_count_as_off() {
        // 只认 JSON 布尔 true；类型写错由 skyline/renderer-options 报，这里按未开启处理。
        let (_f, project) = project(
            "drift-string-bool",
            r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"defaultDisplayBlock":"true","defaultContentBox":"true"}}}"#,
            "{}",
        );
        assert_eq!(DefaultLayoutDrift.check(&project).len(), 2);
    }
}
