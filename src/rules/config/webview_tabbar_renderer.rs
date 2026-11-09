//! `webview/tabbar-renderer`：自定义 tabBar 声明 `renderer: "skyline"` 后，退回 WebView 会抛异常。
//!
//! tabBar 的渲染器与页面分开配置：`WXConfig.tabBar.renderer || res.renderer`
//! （`wxapplib/src/sdk/src/appservice/app/onAppRoute.js:672`）。页面回退时
//! `res.renderer` 已经变成 `"webview"`，所以没写 `tabBar.renderer` 的工程天然安全；
//! 但一旦在 `app.json` 里把 tabBar 自己钉成 `"skyline"`，WebView 适配器这条分支会
//! **直接抛异常**而不是降级：
//!
//! ```js
//! static getInstance(renderer: TabBarRenderer) {
//!   if (renderer !== 'webview') throw new Error(`Invalid tab-bar renderer: ${renderer}`)
//! ```
//! （`glass-easel-adapter/src/render/webview.ts:1071-1075`）
//!
//! 只有自定义 tabBar（`tabBar.custom: true`）才会走 `createTabBar`
//! （`onAppRoute.js:669-671` 的 `isTab && isCustomTabBar`），内置 tabBar 不受影响。

use serde_json::Value;

use crate::input::json::JsonPositions;
use crate::project::Project;
use crate::rules::{Rule, RuleMeta, Target};
use crate::{Diagnostic, Severity};

pub struct TabBarRenderer;

impl Rule for TabBarRenderer {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: "webview/tabbar-renderer",
            severity: Severity::Error,
            description: "自定义 tabBar 声明 renderer: \"skyline\"，退回 WebView 时抛异常",
        }
    }

    fn target(&self) -> Target {
        Target::Webview
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        if !super::applicable(project) {
            return Vec::new();
        }
        let Some(Value::Object(tab_bar)) = project.app_json.get("tabBar") else {
            return Vec::new();
        };
        let custom = tab_bar.get("custom").and_then(Value::as_bool) == Some(true);
        // 适配器要求 `renderer` 恰好是 `webview`：其他任何值（含大小写写错）都会
        // 落到 `WebviewAdapterTabBar.getInstance` 的抛异常分支。
        let pinned = tab_bar
            .get("renderer")
            .and_then(Value::as_str)
            .filter(|renderer| !renderer.eq_ignore_ascii_case("webview"));
        let Some(pinned) = pinned else {
            return Vec::new();
        };
        if !custom {
            return Vec::new();
        }
        let mut out = Vec::new();
        super::error(
            &mut out,
            "webview/tabbar-renderer",
            &project.app_json_path,
            JsonPositions::new(&project.app_json_source).key("tabBar"),
            format!(
                "自定义 tabBar 钉死 `renderer: \"{pinned}\"`：退回 WebView 时 `WebviewAdapterTabBar.getInstance` 会抛 `Invalid tab-bar renderer`，tabBar 整体不可用"
            ),
            "删掉 `tabBar.renderer`，让它随页面渲染器走；只有确定不会回退时才保留该键",
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use crate::Diagnostic;
    use crate::project::Project;
    use crate::rules::Rule;
    use crate::rules::testutil::Fixture;

    use super::TabBarRenderer;

    fn project(name: &str, app: &str) -> (Fixture, Project) {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write("app.json", app)
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", "<view/>");
        let project = fixture.project();
        (fixture, project)
    }

    fn check_with(name: &str, app: &str) -> Vec<Diagnostic> {
        let (_fixture, project) = project(name, app);
        TabBarRenderer.check(&project)
    }

    #[test]
    fn skyline_custom_tabbar_is_reported() {
        let diagnostics = check_with(
            "webview/tabbar",
            r#"{"pages":["pages/index/index"],"renderer":"skyline","tabBar":{"custom":true,"renderer":"skyline","list":[]}}"#,
        );
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("Invalid tab-bar renderer"));
    }

    #[test]
    fn builtin_tabbar_is_fine() {
        let diagnostics = check_with(
            "webview/tabbar-builtin",
            r#"{"pages":["pages/index/index"],"renderer":"skyline","tabBar":{"renderer":"skyline","list":[]}}"#,
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn webview_tabbar_is_fine() {
        let diagnostics = check_with(
            "webview/tabbar-webview",
            r#"{"pages":["pages/index/index"],"renderer":"skyline","tabBar":{"custom":true,"list":[]}}"#,
        );
        assert!(diagnostics.is_empty());
    }
}
