//! `skyline/wxs-sync-rect`：WXS 里同步调用 `getBoundingClientRect()` 在 Skyline 下恒为全 0。
//!
//! WXS 的矩形查询最终落到 glass-easel-adapter 的两条分支
//! （`glass-easel-adapter/src/events/wxs/base.ts:72-93`）：
//!
//! - WebView（`BackendMode.Domlike`）：`e.getBoundingClientRect?.()`，DOM 同步返回；
//! - Skyline：`be.getBoundingClientRect(cb)`，引擎侧只有回调式接口
//!   （`skyline/src/native/native_global.ts:402`），因为布局在 UI 线程、JS 在逻辑线程。
//!
//! 于是 `:201-229` 在不传回调时只能返回 `defaultBoundingClientRect`（全 0）——调用方拿着
//! 0 继续算，既不报错也不生效。判据只有一种写法：`.getBoundingClientRect()` 不传实参。
//! 修法唯一：改成回调形式（需要同步值的场景改为回调里算、把结果缓存）。

use crate::Severity;
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};

use super::{at_span, renders_in_skyline};

const MESSAGE: &str = "WXS 里同步调用 `getBoundingClientRect()` 在 Skyline 下恒返回全 0（引擎只提供回调式矩形查询）；改成 `getBoundingClientRect(rect => …)`，需要同步值时把结果缓存下来";

const HELP: &str = "WebView 的 DOM 可以同步取矩形，Skyline 的测量要跨线程，只能回调；缓存一次结果再在触摸回调里使用";

pub struct WxsSyncRect;

impl WxsSyncRect {
    pub const ID: &'static str = "skyline/wxs-sync-rect";
}

impl Rule for WxsSyncRect {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "WXS 里同步调用 `getBoundingClientRect()` 在 Skyline 下拿到的是全 0",
        }
    }

    fn check(&self, project: &Project) -> Vec<crate::Diagnostic> {
        let mut out = Vec::new();
        for file in &project.scripts {
            if !file.path.ends_with(".wxs") || !renders_in_skyline(project, &file.scope) {
                continue;
            }
            for call in file.doc.calls() {
                if !call.callee.ends_with(".getBoundingClientRect") || !call.args.is_empty() {
                    continue;
                }
                out.push(
                    at_span(
                        Self::ID,
                        Severity::Warning,
                        file,
                        call.start,
                        call.end,
                        MESSAGE,
                    )
                    .help(HELP),
                );
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testutil::js::Fixture;

    use super::WxsSyncRect;

    fn fixture(name: &str, renderer: &str, wxs: &str) -> Fixture {
        let f = Fixture::new(name);
        f.write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                &format!(r#"{{"pages":["pages/index/index"],"renderer":"{renderer}"}}"#),
            )
            .write("pages/index/index.json", "{}")
            .write(
                "pages/index/index.wxml",
                r#"<wxs module="m" src="./index.wxs"/><view bind:touchmove="{{m.move}}"/>"#,
            )
            .write("pages/index/index.js", "Page({});")
            .write("pages/index/index.wxs", wxs);
        f
    }

    #[test]
    fn sync_call_is_reported() {
        let f = fixture(
            "wxs-sync-rect",
            "skyline",
            "function move(evt, owner) { var w = owner.getBoundingClientRect().width; }",
        );
        let diagnostics = WxsSyncRect.check(&f.project());
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].path, "pages/index/index.wxs");
    }

    #[test]
    fn callback_form_is_not_reported() {
        let f = fixture(
            "wxs-sync-rect-cb",
            "skyline",
            "function move(evt, owner) { owner.getBoundingClientRect(function (rect) { var w = rect.width; }); }",
        );
        assert!(WxsSyncRect.check(&f.project()).is_empty());
    }

    /// WebView 页里同步形式是好的（DOM 同步返回），不能报。
    #[test]
    fn webview_page_is_not_reported() {
        let f = fixture(
            "wxs-sync-rect-webview",
            "webview",
            "function move(evt, owner) { var w = owner.getBoundingClientRect().width; }",
        );
        assert!(WxsSyncRect.check(&f.project()).is_empty());
    }

    /// 普通 `.js` 文件里的同名调用不是 WXS：那是 `wx.createSelectorQuery` 之外的写法，
    /// 与这条规则的机制无关，不参与。
    #[test]
    fn non_wxs_scripts_are_ignored() {
        let f = fixture(
            "wxs-sync-rect-js",
            "skyline",
            "function move(evt, owner) { var w = owner.getBoundingClientRect().width; }",
        );
        f.write(
            "pages/index/index.js",
            "Page({ a() { const w = this.getBoundingClientRect().width; } });",
        );
        assert_eq!(WxsSyncRect.check(&f.project()).len(), 1);
    }
}
