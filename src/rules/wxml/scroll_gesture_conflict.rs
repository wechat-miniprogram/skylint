//! 滚动容器与祖先的 `touchmove` 手势会同时生效。
//!
//! Skyline 的 `scroll-view` 滚动由原生滚动完成（`skyline/shadow/components/scroll_view_shadow_node.cc`
//! 创建 `ScrollView` widget），JS 的 `bind:touchmove` 是普通节点事件，两者**并行**——
//! 祖先在子元素滚动时会照常收到 `touchmove`。所以「半屏弹窗在下拉时收起」这类
//! 实现（面板绑 `touchmove` + 内容里放 `scroll-y` 列表）会在往回滚列表时误触发：
//! 列表在滚，面板同时被 `translate3d` 往下拖，抬手超过阈值就关闭。
//!
//! 判据静态可判定、修法唯一，所以收成规则：
//! 某个 `scroll-y` 的 `scroll-view` 的逻辑祖先链上（**跨 `<slot>` 投影**，
//! 见 [`crate::wxml_graph`]）有元素绑了 `bind:touchmove`，且两者之间没有
//! `catch:touchmove` 阻断冒泡。
//!
//! 修法：在列表的 `bind:scroll` 里按 `scrollTop` 门控手势开关（滚动离开顶部就
//! 关掉拖拽），或者把拖拽手势绑到不含滚动容器的区域（例如只绑标题条）。

use std::collections::HashSet;

use crate::diag::{Diagnostic, Severity};
use crate::project::Project;
use crate::rules::{Rule, RuleMeta, Target};
use crate::wxml_graph::{FlatElement, WxmlGraph};

use super::{ElementDoc, Reach, element_docs, walk_skyline};

pub const ID: &str = "skyline/scroll-gesture-conflict";

const MESSAGE: &str = "`scroll-view` 的滚动和祖先上的 `touchmove` 会同时生效：Skyline 的滚动是原生滚动，\
     触摸事件照常冒泡，列表往回拖时会误触发祖先的拖拽（例如半屏弹窗的下拉收起）";

const HELP: &str = "在列表的 `bind:scroll` 里按 `scrollTop` 门控手势开关（离开顶部就关掉拖拽），\
     或把拖拽手势绑到不包含滚动容器的区域；已经这样做了可以忽略，用\
     `<!-- skylint-disable-next-line skyline/scroll-gesture-conflict -->` 关掉这条提示";

pub struct ScrollGestureConflict;

impl Rule for ScrollGestureConflict {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Warning,
            description: "滚动容器上的滑动会冒泡触发祖先的 touchmove 手势（拖拽收起等）",
        }
    }

    fn target(&self) -> Target {
        Target::Skyline
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let graph = WxmlGraph::new(project);
        let mut out = Vec::new();
        let mut reported = HashSet::new();
        for ElementDoc { doc, reach } in element_docs(project) {
            if reach != Reach::Skyline {
                continue;
            }
            let path = doc.path().to_string();
            walk_skyline(&doc, |element, _| {
                if !is_vertical_scroll_view(element) {
                    return;
                }
                // 自己 catch 住 touchmove 就不会冒泡到祖先，冲突不存在。
                if element.attributes.iter().any(|attr| {
                    attr.name.eq_ignore_ascii_case("catch:touchmove")
                        || attr.name.eq_ignore_ascii_case("catchtouchmove")
                }) {
                    return;
                }
                let (line, _) = doc.position(element.tag_start);
                let scroll_view = format!("{path}:{line}");
                let chain = graph.ancestors(&path, element).nodes;
                // 调用方把拖拽开关做成动态绑定：里面有 scrollTop 门控，静态看不出来。
                if chain.iter().any(|(_, ancestor)| is_gated(ancestor)) {
                    return;
                }
                for (ancestor_path, ancestor) in chain {
                    if has_catch_touchmove(ancestor) {
                        break;
                    }
                    if !has_bind_touchmove(ancestor) {
                        continue;
                    }
                    if !reported.insert((ancestor_path.to_string(), ancestor.line, ancestor.column))
                    {
                        continue;
                    }
                    out.push(
                        Diagnostic::new(
                            ID,
                            Severity::Warning,
                            format!("{MESSAGE}（与 {scroll_view} 的 `scroll-view` 冲突）"),
                            ancestor_path,
                            ancestor.line,
                            ancestor.column,
                        )
                        .end(ancestor.end_line, ancestor.end_column)
                        .help(HELP),
                    );
                }
            });
        }
        out
    }
}

fn is_vertical_scroll_view(element: &crate::input::wxml::WxmlElement) -> bool {
    element.tag.eq_ignore_ascii_case("scroll-view")
        && element
            .attributes
            .iter()
            .any(|attr| attr.name.eq_ignore_ascii_case("scroll-y") && !is_false(attr))
}

fn is_false(attr: &crate::input::wxml::WxmlAttribute) -> bool {
    matches!(attr.value.as_deref(), Some("false") | Some("0"))
}

fn has_bind_touchmove(element: &FlatElement) -> bool {
    element.attr("bind:touchmove").is_some() || element.attr("bindtouchmove").is_some()
}

fn has_catch_touchmove(element: &FlatElement) -> bool {
    element.attr("catch:touchmove").is_some() || element.attr("catchtouchmove").is_some()
}

/// 使用方是否把拖拽开关做成了动态绑定（`dragHide="{{…}}"` 一类）。
///
/// 这条链的固定写法：面板绑 `touchmove`，开关由调用方传。调用方传动态值时，说明
/// 里面有 `scrollTop` 门控（`select-drama-dialog.ts` 那套），静态看不出门控逻辑，
/// 就不能报。属性名各组件不同（`dragHide` / `dragEnable` / `canDrag`），按「名字里
/// 带 drag 且是动态值」判，而不是认死一个。
fn is_gated(element: &FlatElement) -> bool {
    !crate::rules::wxml::is_builtin(&element.tag)
        && element.attributes.iter().any(|attr| {
            let name = attr.name.to_ascii_lowercase();
            // 事件绑定（`bind:dragmove` / `catchdragend`）不是开关，别把它当成门控。
            !name.starts_with("bind")
                && !name.starts_with("catch")
                && !name.starts_with("capture")
                && name.contains("drag")
                && !attr.is_static
        })
}

#[cfg(test)]
mod tests {
    use super::ScrollGestureConflict;
    use crate::rules::Rule;
    use crate::rules::testutil::Fixture;

    fn page(name: &str, wxml: &str, using: &str, js: &str) -> Fixture {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                &format!("{{\"usingComponents\":{{{using}}}}}"),
            )
            .write("pages/index/index.wxml", wxml)
            .write("pages/index/index.js", js);
        fixture
    }

    fn check(fixture: &Fixture) -> Vec<crate::Diagnostic> {
        ScrollGestureConflict.check(&fixture.project())
    }

    #[test]
    fn touchmove_ancestor_over_scroll_is_reported() {
        let fixture = page(
            "gesture-same-file",
            r#"<view bind:touchmove="onMove"><scroll-view scroll-y><view/></scroll-view></view>"#,
            "",
            "Page({ onMove() {} });",
        );
        let diagnostics = check(&fixture);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].rule, super::ID);
        assert!(diagnostics[0].message.contains("pages/index/index.wxml:1"));
    }

    /// 关键形态：拖拽绑在组件自己的面板上，滚动容器是通过 `<slot>` 传进来的。
    #[test]
    fn slot_projected_scroll_is_reported_on_the_component() {
        let fixture = page(
            "gesture-slot",
            r#"<panel><scroll-view scroll-y><view/></scroll-view></panel>"#,
            "\"panel\": \"/components/panel/panel\"",
            "Page({});",
        );
        fixture
            .write("components/panel/panel.json", r#"{"component":true}"#)
            .write("components/panel/panel.js", "Component({});")
            .write(
                "components/panel/panel.wxml",
                r#"<view class="panel" bind:touchmove="onMove"><slot/></view>"#,
            );
        let diagnostics = check(&fixture);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].path, "components/panel/panel.wxml");
        assert!(diagnostics[0].message.contains("pages/index/index.wxml:1"));
    }

    #[test]
    fn catch_touchmove_breaks_the_chain() {
        let fixture = page(
            "gesture-catch",
            r#"<view bind:touchmove="onMove"><scroll-view scroll-y catch:touchmove="noop"><view/></scroll-view></view>"#,
            "",
            "Page({ onMove() {}, noop() {} });",
        );
        assert!(check(&fixture).is_empty());
    }

    #[test]
    fn horizontal_only_scroll_is_fine() {
        let fixture = page(
            "gesture-horizontal",
            r#"<view bind:touchmove="onMove"><scroll-view scroll-x><view/></scroll-view></view>"#,
            "",
            "Page({ onMove() {} });",
        );
        assert!(check(&fixture).is_empty());
    }

    #[test]
    fn scroll_without_touchmove_ancestor_is_fine() {
        let fixture = page(
            "gesture-none",
            r#"<view bind:tap="onTap"><scroll-view scroll-y><view/></scroll-view></view>"#,
            "",
            "Page({ onTap() {} });",
        );
        assert!(check(&fixture).is_empty());
    }

    #[test]
    fn dynamic_drag_hide_means_it_is_gated() {
        let fixture = page(
            "gesture-gated",
            r#"<panel dragHide="{{dragHide}}"><scroll-view scroll-y><view/></scroll-view></panel>"#,
            "\"panel\": \"/components/panel/panel\"",
            "Page({});",
        );
        fixture
            .write("components/panel/panel.json", r#"{"component":true}"#)
            .write("components/panel/panel.js", "Component({});")
            .write(
                "components/panel/panel.wxml",
                r#"<view bind:touchmove="onMove"><slot/></view>"#,
            );
        assert!(check(&fixture).is_empty());
    }

    #[test]
    fn any_dynamic_drag_flag_counts_as_gated() {
        // 属性名各组件不同，按「名字带 drag + 动态值」判，不认死 `dragHide`。
        let fixture = page(
            "gesture-gated-renamed",
            r#"<panel dragEnable="{{canDrag}}"><scroll-view scroll-y><view/></scroll-view></panel>"#,
            "\"panel\": \"/components/panel/panel\"",
            "Page({});",
        );
        fixture
            .write("components/panel/panel.json", r#"{"component":true}"#)
            .write("components/panel/panel.js", "Component({});")
            .write(
                "components/panel/panel.wxml",
                r#"<view bind:touchmove="onMove"><slot/></view>"#,
            );
        assert!(check(&fixture).is_empty());
    }

    #[test]
    fn static_drag_hide_is_still_reported() {
        let fixture = page(
            "gesture-static-drag-hide",
            r#"<panel dragHide><scroll-view scroll-y><view/></scroll-view></panel>"#,
            "\"panel\": \"/components/panel/panel\"",
            "Page({});",
        );
        fixture
            .write("components/panel/panel.json", r#"{"component":true}"#)
            .write("components/panel/panel.js", "Component({});")
            .write(
                "components/panel/panel.wxml",
                r#"<view bind:touchmove="onMove"><slot/></view>"#,
            );
        assert_eq!(check(&fixture).len(), 1);
    }

    #[test]
    fn webview_page_is_not_reported() {
        let fixture = Fixture::new("gesture-webview");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", r#"{"renderer":"webview"}"#)
            .write(
                "pages/index/index.wxml",
                r#"<view bind:touchmove="onMove"><scroll-view scroll-y><view/></scroll-view></view>"#,
            )
            .write("pages/index/index.js", "Page({ onMove() {} });");
        assert!(check(&fixture).is_empty());
    }
}
