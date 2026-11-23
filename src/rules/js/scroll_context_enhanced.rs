//! `ScrollViewContext` 的前置条件：`<scroll-view>` 必须开启 `enhanced`。
//!
//! 依据：`skyline-scroll-api/references/api/scroll-view-context.md`「仅在 scroll-view
//! 组件开启 `enhanced` 属性后生效」；框架侧 `ScrollViewContext.invoke()` 在
//! `enhanced !== true` 时只 `console.warn('此方法需要 scroll-view 开启 enhanced 属性！')`
//! 并 `return false`，方法体不执行。
//!
//! 判定口径：
//! - 只认 ScrollViewContext 独有的四个方法（`triggerRefresh` / `closeRefresh` /
//!   `triggerTwoLevel` / `closeTwoLevel`）。`scrollTo` / `scrollIntoView` 与
//!   DraggableSheetContext 同名，单独出现分不清是哪个 context。
//! - 同一文件还要有 `.node(...)`：context 只能从 `NodesRef.node` 取到。
//! - `enhanced` 按单元的渲染闭包找：页面自己的 WXML 与它引用到的组件 / 模板里，
//!   任意一个 `<scroll-view>` 开了 `enhanced` 就算通过——scroll-view 常被封装在组件里。
//!
//! ## 证据用例：为什么没有
//!
//! 本条**不可自动证**。沙箱复核（同一页面并排放未开 / 开启 `enhanced` 的两个
//! `<scroll-view>`，各自 `node().triggerRefresh({ duration: 300 })`）的结果是：
//!
//! - 两者返回值都是 `undefined`；
//! - 两者都没有出现 `此方法需要 scroll-view 开启 enhanced 属性！`；
//! - `node().scrollTo` 在两类 scroll-view 上都照常生效（未开 `enhanced` 也能滚）。
//!
//! 也就是说当前模拟器不实现「未开 enhanced 时只 warn 并 return false」这一步，
//! 取不到能区分两个分支的读数，硬写一条 warning 断言只会因为「本来就没有日志」
//! 而永远失败。需要在真实开发者工具 / 真机复核后再补。

use std::collections::HashSet;

use crate::diag::{Diagnostic, Severity};
use crate::input::js::{CallInfo, JsDoc};
use crate::project::{Project, Scope};
use crate::rules::wxml::{for_each_element, structure::find_attr};
use crate::rules::{Rule, RuleMeta, Target};

use super::{at_span, renders_in_skyline};

/// ScrollViewContext 独有、其它 context 没有的方法。
const METHODS: &[&str] = &[
    "triggerRefresh",
    "closeRefresh",
    "triggerTwoLevel",
    "closeTwoLevel",
];

/// 调用的方法名。
///
/// `res[0].node.triggerRefresh(..)` 这类链里带计算属性的写法，输入层的 callee 是空串
/// （整条链放弃），只能从调用区间取到 `(` 之前的文本再取最后一段。
fn method_tail<'a>(source: &'a str, call: &'a CallInfo) -> &'a str {
    let chain = if call.callee.is_empty() {
        let text = source
            .get(call.start as usize..call.end as usize)
            .unwrap_or_default();
        text.split('(').next().unwrap_or_default()
    } else {
        call.callee.as_str()
    };
    chain.rsplit('.').next().unwrap_or_default()
}

fn find_call(source: &str, doc: &JsDoc, methods: &[&str]) -> Option<(u32, u32)> {
    doc.calls()
        .iter()
        .find(|call| methods.contains(&method_tail(source, call)))
        .map(|call| (call.start, call.end))
}

pub struct ScrollContextEnhanced;

impl ScrollContextEnhanced {
    pub const ID: &'static str = "skyline/scroll-context-enhanced";
}

impl Rule for ScrollContextEnhanced {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "用 ScrollViewContext 控制滚动 / 刷新前，`<scroll-view>` 必须开启 `enhanced`",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut enhanced: HashSet<String> = HashSet::new();
        for_each_element(project, Target::Skyline, |site| {
            if site.element.tag == "scroll-view" && find_attr(site.element, "enhanced").is_some() {
                enhanced.insert(site.doc.path().to_string());
            }
        });
        let graph = project.graph();
        let mut out = Vec::new();
        for file in &project.scripts {
            if !renders_in_skyline(project, &file.scope) {
                continue;
            }
            let source = file.doc.source();
            let Some((start, end)) = find_call(source, &file.doc, METHODS) else {
                continue;
            };
            if find_call(source, &file.doc, &["node"]).is_none() {
                continue;
            }
            let unit = match &file.scope {
                Scope::Page(route) => route.as_str(),
                Scope::Component(path) => path.as_str(),
                Scope::Global | Scope::Loose(_) => continue,
            };
            let Some(wxml) = project.unit_wxml(unit) else {
                continue;
            };
            if graph
                .render_docs(wxml)
                .iter()
                .any(|doc| enhanced.contains(*doc))
            {
                continue;
            }
            out.push(at_span(
                Self::ID,
                Severity::Warning,
                file,
                start,
                end,
                "这个 scroll-view 没有开启 `enhanced`：`node()` 拿到的不是可用的 ScrollViewContext，该调用只会打一条 warning 后直接返回，滚动 / 刷新都不会执行",
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testutil::js::{Fixture, messages};

    use super::ScrollContextEnhanced;

    /// 一个页面 + 给定 wxml / js。
    fn page(name: &str, wxml: &str, js: &str) -> (Fixture, crate::project::Project) {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", wxml)
            .write("pages/index/index.js", js);
        let project = fixture.project();
        (fixture, project)
    }

    const REFRESH: &str = r#"
        Page({
          onReady() {
            wx.createSelectorQuery().select('#sv').node().exec((res) => {
              res[0].node.triggerRefresh({ duration: 300 });
            });
          },
        });
    "#;

    #[test]
    fn missing_enhanced_is_reported() {
        let (_f, project) = page(
            "enhanced-missing",
            r#"<scroll-view id="sv" type="list" scroll-y/>"#,
            REFRESH,
        );
        let diagnostics = ScrollContextEnhanced.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].severity, crate::Severity::Warning);
        assert!(diagnostics[0].message.contains("enhanced"));
    }

    #[test]
    fn enhanced_scroll_view_passes() {
        let (_f, project) = page(
            "enhanced-ok",
            r#"<scroll-view id="sv" type="list" scroll-y enhanced/>"#,
            REFRESH,
        );
        assert_eq!(
            messages(&ScrollContextEnhanced, &project),
            Vec::<String>::new()
        );
    }

    #[test]
    fn enhanced_in_a_child_component_passes() {
        let fixture = Fixture::new("enhanced-component");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                r#"{"usingComponents":{"list":"/components/list/list"}}"#,
            )
            .write("pages/index/index.wxml", r#"<list id="sv"/>"#)
            .write("pages/index/index.js", REFRESH)
            .write("components/list/list.json", r#"{"component":true}"#)
            .write(
                "components/list/list.wxml",
                r#"<scroll-view type="list" scroll-y enhanced/>"#,
            )
            .write("components/list/list.js", "Component({});");
        let project = fixture.project();
        assert_eq!(
            messages(&ScrollContextEnhanced, &project),
            Vec::<String>::new()
        );
    }

    #[test]
    fn scroll_to_alone_is_not_judged() {
        // `scrollTo` 与 DraggableSheetContext 同名，单独出现不判。
        let (_f, project) = page(
            "enhanced-scroll-to",
            r#"<scroll-view id="sv" type="list" scroll-y/>"#,
            r#"
            Page({
              onReady() {
                wx.createSelectorQuery().select('#sv').node().exec((res) => {
                  res[0].node.scrollTo({ top: 0 });
                });
              },
            });
            "#,
        );
        assert_eq!(
            messages(&ScrollContextEnhanced, &project),
            Vec::<String>::new()
        );
    }

    #[test]
    fn refresh_without_node_is_not_judged() {
        // 没有 `.node()` 说明拿不到 context，可能是别的对象上的同名方法。
        let (_f, project) = page(
            "enhanced-no-node",
            r#"<scroll-view id="sv" type="list" scroll-y/>"#,
            "Page({ onReady() { this.panel.triggerRefresh(); } });",
        );
        assert_eq!(
            messages(&ScrollContextEnhanced, &project),
            Vec::<String>::new()
        );
    }

    #[test]
    fn webview_page_is_skipped() {
        let fixture = Fixture::new("enhanced-webview");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                r#"{"renderer":"webview","navigationStyle":"custom"}"#,
            )
            .write(
                "pages/index/index.wxml",
                r#"<scroll-view id="sv" type="list" scroll-y/>"#,
            )
            .write("pages/index/index.js", REFRESH);
        let project = fixture.project();
        assert_eq!(
            messages(&ScrollContextEnhanced, &project),
            Vec::<String>::new()
        );
    }
}
