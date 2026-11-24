//! `webview/tabbar-renderer`：自定义 tabBar 声明 `renderer: "skyline"` 后，退回 WebView 会抛异常。
//!
//! tabBar 的渲染器与页面分开配置：`WXConfig.tabBar.renderer || res.renderer`
//! （SDK 路由实现）。页面回退时
//! `res.renderer` 已经变成 `"webview"`，所以没写 `tabBar.renderer` 的工程天然安全；
//! 但一旦在 `app.json` 里把 tabBar 自己钉成 `"skyline"`，WebView 适配器这条分支会
//! **直接抛异常**而不是降级：
//!
//! ```js
//! static getInstance(renderer: TabBarRenderer) {
//!   if (renderer !== 'webview') throw new Error(`Invalid tab-bar renderer: ${renderer}`)
//! ```
//! （glass-easel 适配层）
//!
//! 只有自定义 tabBar（`tabBar.custom: true`）才会走 `createTabBar`
//! （`onAppRoute.js:669-671` 的 `isTab && isCustomTabBar`），内置 tabBar 不受影响。
//!
//! 不可自动证：模拟器里只要 app.json 带自定义 tabBar，automator 的
//! `App.callFunction`（`EvidenceExpect::Runtime` / `Console` 取证都依赖它）就不再返回；
//! 非 `webview` 的 renderer 还会让首页路由直接抛
//! `Invalid tab-bar renderer`，页面不可用。因此本规则没有 `EvidenceCase`，只能保留覆盖率
//! 门禁里的白名单（理由见 `rules::tests::rules_have_cases`）。

use serde_json::Value;

use crate::input::json::JsonPositions;
use crate::project::Project;
use crate::rules::{Rule, RuleMeta, Target};
use crate::{Diagnostic, Severity};

use super::anchor;

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
            anchor(&["tabBar", "renderer"], tab_bar.get("renderer")),
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
    fn probe_fixture_is_reported() {
        // 与手工排查用的最小工程同形：页面在 tabBar.list 里，custom + renderer 都是
        // 触发条件；规则命中的是配置本身，不依赖页面内容。
        let diagnostics = check_with(
            "webview/tabbar-probe",
            r#"{"pages":["pages/index/index"],"renderer":"webview","componentFramework":"glass-easel","lazyCodeLoading":"requiredComponents","tabBar":{"custom":true,"renderer":"skyline","list":[{"pagePath":"pages/index/index","text":"Home"}]}}"#,
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
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
