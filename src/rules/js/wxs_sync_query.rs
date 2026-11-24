//! `skyline/wxs-sync-query`：WXS 里同步形式的布局查询在 Skyline 下拿不到值。
//!
//! 两个方法，同一个成因：WXS 的节点描述符照着 DOM 提供了「同步 + 回调」双签名，
//! 而 Skyline 的布局在 UI 线程，引擎侧只有回调接口。
//!
//! WXS 的矩形查询最终落到适配层的两条分支
//! （glass-easel 适配层）：
//!
//! - WebView（`BackendMode.Domlike`）：`e.getBoundingClientRect?.()`，DOM 同步返回；
//! - Skyline：`be.getBoundingClientRect(cb)`，引擎侧只有回调式接口
//!   （Skyline 原生接口实现），因为布局在 UI 线程、JS 在逻辑线程。
//!
//! - `getBoundingClientRect()`：适配层在不传回调时只返回默认矩形（全 0）——
//!   调用方拿着 0 继续算，既不报错也不生效；
//! - `getComputedStyle(fields)`：适配层只有回调分支、没有返回分支，不传回调直接
//!   得到 `undefined`。
//!
//! 判据都只有一种写法：`getBoundingClientRect()` 不传实参、`getComputedStyle()` 缺第二个回调。
//! 引擎判的是 `typeof cb !== 'function'`，所以 `undefined` / `null` / `{}` / `[]` 等
//! 非函数实参同样等于没传。
//!
//! 覆盖面：`<wxs src>` 引入的外部模块（含只在 `<import>` 模板里引用的），以及内联
//! `<wxs module>` 的模块体；后者的行列落回所在 WXML。
//! 修法唯一：改成回调形式（需要同步值的场景改为回调里算、把结果缓存）。

use crate::project::Project;
use crate::rules::{Rule, RuleMeta, VisualAction, VisualCase, VisualExpect, VisualFile};
use crate::{Diagnostic, Severity};

use super::{at_span, renders_in_skyline};

const RECT_MESSAGE: &str = "WXS 里同步调用 `getBoundingClientRect()` 在 Skyline 下恒返回全 0（引擎只提供回调式矩形查询）；改成 `getBoundingClientRect(rect => …)`，需要同步值时把结果缓存下来";

const STYLE_MESSAGE: &str = "WXS 里同步调用 `getComputedStyle()` 在 Skyline 下返回 `undefined`（引擎只提供回调式样式查询）；改成 `getComputedStyle(fields, style => …)`";

const HELP: &str = "WebView 的 DOM 可以同步取矩形与计算样式，Skyline 的查询要跨线程、只能回调；回调里算好再缓存，在触摸回调里读缓存";

/// 视觉用例的 WXS 模块：在事件处理器的第二参数上做同步矩形查询，按结果改背景。
///
/// 用真实的 `.wxs` 文件而不是内联模块：构建产物里也是同一份脚本。
static VISUAL_FILES: &[VisualFile] = &[VisualFile {
    path: "probe.wxs",
    content: r#"function onTap(evt, owner) {
  var rect = owner.getBoundingClientRect()
  var wide = !!rect && rect.width > 100
  owner.setStyle(
    wide
      ? 'background-color: #1b8a5a;'
      : 'background-color: #c2402a;'
  )
}

module.exports = { onTap: onTap }
"#,
}];

static VISUAL_ACTIONS: &[VisualAction] = &[VisualAction::Tap("#target")];

static VISUAL_CASES: &[VisualCase] = &[VisualCase {
    wxml: r#"<wxs src="./probe.wxs" module="m"></wxs>
<view class="card" id="target" bind:tap="{{m.onTap}}">
  <text class="title">WXS 同步矩形查询</text>
  <text class="hint">点击后按 rect.width 改背景</text>
</view>"#,
    wxss: ".card { box-sizing: border-box; width: 240px; height: 180px; margin: 60px auto; padding: 20px; background-color: #6b7a90; border-radius: 12px; }\n.title { display: block; color: #ffffff; font-size: 18px; }\n.hint { display: block; margin-top: 14px; color: #e4e9f0; font-size: 14px; }\n",
    js: Some("Page({});\n"),
    expect: VisualExpect::Differ,
    note: "点击后 WXS 在 `ownerInstance`（页面根）上做同步矩形查询并按 `rect.width` 改背景：WebView 拿到真实宽度、整个视口变绿；Skyline 的同步查询恒为空（全 0）走兜底，只有页面根（高 300px）变红、下方仍是白底",
    target: "",
}];

pub struct WxsSyncQuery;

impl WxsSyncQuery {
    pub const ID: &'static str = "skyline/wxs-sync-query";
}

impl Rule for WxsSyncQuery {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "WXS 里同步形式的 `getBoundingClientRect()` / `getComputedStyle()` 在 Skyline 下拿不到值",
        }
    }

    fn visual_cases(&self) -> &'static [VisualCase] {
        VISUAL_CASES
    }

    fn visual_files(&self) -> &'static [VisualFile] {
        VISUAL_FILES
    }

    fn visual_actions(&self) -> &'static [VisualAction] {
        VISUAL_ACTIONS
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        for file in &project.scripts {
            if !file.path.ends_with(".wxs") || !renders_in_skyline(project, &file.scope) {
                continue;
            }
            for call in file.doc.calls() {
                let Some(message) = sync_query(file.doc.source(), call) else {
                    continue;
                };
                out.push(
                    at_span(
                        Self::ID,
                        Severity::Warning,
                        file,
                        call.start,
                        call.end,
                        message,
                    )
                    .help(HELP),
                );
            }
        }
        for module in &project.inline_wxs {
            if !module
                .scopes
                .iter()
                .any(|scope| renders_in_skyline(project, scope))
            {
                continue;
            }
            for call in module.doc.calls() {
                let Some(message) = sync_query(module.doc.source(), call) else {
                    continue;
                };
                let (line, column) = module.position(call.start);
                let (end_line, end_column) = module.position(call.end);
                out.push(
                    Diagnostic::new(
                        Self::ID,
                        Severity::Warning,
                        message,
                        &module.path,
                        line,
                        column,
                    )
                    .end(end_line, end_column)
                    .help(HELP),
                );
            }
        }
        out
    }
}

/// 这条调用是不是同步形式；是则返回对应文案。
fn sync_query(source: &str, call: &crate::input::js::CallInfo) -> Option<&'static str> {
    if call.callee.ends_with(".getBoundingClientRect") && missing_rect_callback(source, call) {
        Some(RECT_MESSAGE)
    } else if call.callee.ends_with(".getComputedStyle") && missing_callback(source, call) {
        Some(STYLE_MESSAGE)
    } else {
        None
    }
}

/// 第二个实参是不是真的回调。
///
/// 只看参数个数会把 `getComputedStyle(fields, undefined)` 当成有回调（引擎里同样拿不到值），
/// 所以显式排除那些「写了等于没写」的实参；变量/箭头函数/函数名一律当作有回调。
/// `getBoundingClientRect` 的回调是第一个实参；不传、或传 `undefined` / `null`
/// 都走「没有回调」的分支（适配层用 `typeof cb !== 'function'` 判）。
fn missing_rect_callback(source: &str, call: &crate::input::js::CallInfo) -> bool {
    match call.args.first() {
        None => true,
        Some(arg) => is_no_value(
            source
                .get(arg.start as usize..arg.end as usize)
                .unwrap_or_default(),
        ),
    }
}

/// `getComputedStyle(fields, cb?)` 的回调是第二个实参。
fn missing_callback(source: &str, call: &crate::input::js::CallInfo) -> bool {
    missing_arg(source, call, 1)
}

fn is_no_value(text: &str) -> bool {
    matches!(
        text.trim(),
        "undefined" | "null" | "void 0" | "false" | "0" | "''" | "\"\"" | "{}" | "[]"
    )
}

fn missing_arg(source: &str, call: &crate::input::js::CallInfo, index: usize) -> bool {
    let Some(arg) = call.args.get(index) else {
        return true;
    };
    is_no_value(
        source
            .get(arg.start as usize..arg.end as usize)
            .unwrap_or_default(),
    )
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testutil::js::Fixture;

    use super::{VISUAL_CASES, VISUAL_FILES, WxsSyncQuery};

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

    /// 视觉用例的同一份 WXML 与 `.wxs` 文件必须真的能触发本规则。
    #[test]
    fn visual_case_triggers_the_rule() {
        let case = &VISUAL_CASES[0];
        let f = Fixture::new("wxs-visual");
        f.write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", case.wxml)
            .write("pages/index/index.js", case.js.unwrap_or("Page({});"));
        for file in VISUAL_FILES {
            f.write(&format!("pages/index/{}", file.path), file.content);
        }
        let diagnostics = WxsSyncQuery.check(&f.project());
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.rule == WxsSyncQuery::ID),
            "视觉用例未触发规则：{diagnostics:#?}"
        );
    }

    #[test]
    fn sync_call_is_reported() {
        let f = fixture(
            "wxs-sync-rect",
            "skyline",
            "function move(evt, owner) { var w = owner.getBoundingClientRect().width; }",
        );
        let diagnostics = WxsSyncQuery.check(&f.project());
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].path, "pages/index/index.wxs");
    }

    #[test]
    fn sync_computed_style_is_reported() {
        let f = fixture(
            "wxs-sync-style",
            "skyline",
            "function move(evt, owner) { var s = owner.getComputedStyle(['width']) }",
        );
        let diagnostics = WxsSyncQuery.check(&f.project());
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("getComputedStyle"));
    }

    #[test]
    fn literal_second_argument_is_not_a_callback() {
        // 写了 `undefined` 等于没写回调：引擎里同样拿不到值。
        let f = fixture(
            "wxs-sync-style-undefined",
            "skyline",
            "function move(evt, owner) { owner.getComputedStyle(['width'], undefined) }",
        );
        let diagnostics = WxsSyncQuery.check(&f.project());
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    #[test]
    fn empty_object_and_array_arguments_are_not_callbacks() {
        // 引擎判 `typeof cb !== 'function'`：`{}` / `[]` 和 `undefined` 一样等于没写。
        for (name, wxs) in [
            (
                "wxs-sync-style-object",
                "function move(evt, owner) { owner.getComputedStyle(['width'], {}) }",
            ),
            (
                "wxs-sync-style-array",
                "function move(evt, owner) { owner.getComputedStyle(['width'], []) }",
            ),
            (
                "wxs-sync-rect-object",
                "function move(evt, owner) { owner.getBoundingClientRect({}) }",
            ),
        ] {
            let f = fixture(name, "skyline", wxs);
            assert_eq!(WxsSyncQuery.check(&f.project()).len(), 1, "{name}");
        }
    }

    fn inline_fixture(name: &str, renderer: &str) -> Fixture {
        let f = Fixture::new(name);
        f.write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                &format!(r#"{{"pages":["pages/index/index"],"renderer":"{renderer}"}}"#),
            )
            .write("pages/index/index.json", "{}")
            .write(
                "pages/index/index.wxml",
                "<view/>\n<wxs module=\"m\">\nfunction move(evt, owner) { owner.getBoundingClientRect() }\n</wxs>\n<view bind:touchmove=\"{{m.move}}\"/>\n",
            )
            .write("pages/index/index.js", "Page({});");
        f
    }

    #[test]
    fn inline_module_is_reported_on_the_wxml() {
        let f = inline_fixture("wxs-inline", "skyline");
        let diagnostics = WxsSyncQuery.check(&f.project());
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].path, "pages/index/index.wxml");
        // 模块体第 2 行落在 WXML 第 3 行（模块体从第 2 行的 `<wxs>` 后开始）。
        assert_eq!(diagnostics[0].line, 3);
    }

    #[test]
    fn inline_module_in_a_webview_page_is_ignored() {
        let f = inline_fixture("wxs-inline-webview", "webview");
        assert!(WxsSyncQuery.check(&f.project()).is_empty());
    }

    fn template_fixture(name: &str, renderer: &str) -> Fixture {
        let f = Fixture::new(name);
        f.write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                &format!(r#"{{"pages":["pages/index/index"],"renderer":"{renderer}"}}"#),
            )
            .write("pages/index/index.json", "{}")
            .write(
                "pages/index/index.wxml",
                r#"<import src="../../templates/probe.wxml"/><view/>"#,
            )
            .write("pages/index/index.js", "Page({});")
            .write(
                "templates/probe.wxml",
                r#"<wxs src="./probe.wxs" module="m"/><template name="x"><view bind:touchmove="{{m.move}}"/></template>"#,
            )
            .write(
                "templates/probe.wxs",
                "function move(evt, owner) { owner.getBoundingClientRect() }",
            );
        f
    }

    #[test]
    fn inline_module_in_an_imported_template_is_visible() {
        let f = Fixture::new("wxs-inline-template");
        f.write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write(
                "pages/index/index.wxml",
                r#"<import src="../../templates/probe.wxml"/><view/>"#,
            )
            .write("pages/index/index.js", "Page({});")
            .write(
                "templates/probe.wxml",
                "<wxs module=\"m\">\nfunction move(evt, owner) { owner.getBoundingClientRect() }\n</wxs>\n<template name=\"x\"><view bind:touchmove=\"{{m.move}}\"/></template>",
            );
        let diagnostics = WxsSyncQuery.check(&f.project());
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].path, "templates/probe.wxml");
        assert_eq!(diagnostics[0].line, 2);
    }

    #[test]
    fn wxs_referenced_only_from_an_imported_template_is_visible() {
        let f = template_fixture("wxs-template", "skyline");
        let diagnostics = WxsSyncQuery.check(&f.project());
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].path, "templates/probe.wxs");
    }

    #[test]
    fn template_imported_by_a_webview_page_is_ignored() {
        let f = template_fixture("wxs-template-webview", "webview");
        assert!(WxsSyncQuery.check(&f.project()).is_empty());
    }

    #[test]
    fn computed_style_callback_form_is_not_reported() {
        let f = fixture(
            "wxs-sync-style-cb",
            "skyline",
            "function move(evt, owner) { owner.getComputedStyle(['width'], function (s) { var w = s.width }) }",
        );
        assert!(WxsSyncQuery.check(&f.project()).is_empty());
    }

    #[test]
    fn callback_form_is_not_reported() {
        let f = fixture(
            "wxs-sync-rect-cb",
            "skyline",
            "function move(evt, owner) { owner.getBoundingClientRect(function (rect) { var w = rect.width; }); }",
        );
        assert!(WxsSyncQuery.check(&f.project()).is_empty());
    }

    /// WebView 页里同步形式是好的（DOM 同步返回），不能报。
    #[test]
    fn webview_page_is_not_reported() {
        let f = fixture(
            "wxs-sync-rect-webview",
            "webview",
            "function move(evt, owner) { var w = owner.getBoundingClientRect().width; }",
        );
        assert!(WxsSyncQuery.check(&f.project()).is_empty());
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
        assert_eq!(WxsSyncQuery.check(&f.project()).len(), 1);
    }
}
