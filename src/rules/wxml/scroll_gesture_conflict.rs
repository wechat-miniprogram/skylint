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
use crate::rules::{Rule, RuleMeta, Target, VisualAction, VisualCase, VisualExpect};
use crate::wxml_graph::FlatElement;

use super::{Reach, for_each_element};

pub const ID: &str = "skyline/scroll-gesture-conflict";

const MESSAGE: &str = "`scroll-view` 的滚动和祖先上的 `touchmove` 会同时生效：Skyline 的滚动是原生滚动，\
     触摸事件照常冒泡，列表往回拖时会误触发祖先的拖拽（例如半屏弹窗的下拉收起）";

const HELP: &str = "在列表的 `bind:scroll` 里按 `scrollTop` 门控手势开关（离开顶部就关掉拖拽），\
     或把拖拽手势绑到不包含滚动容器的区域；已经这样做了可以忽略，用\
     `<!-- skylint-disable-next-line skyline/scroll-gesture-conflict -->` 关掉这条提示";

/// 视觉用例：阴性对照 + 真实 Differ。
///
/// 规则主张的差异在**触摸事件的冒泡路径**上（Skyline 原生滚动与 `touchmove` 并行），
/// 而不是静态渲染：同一结构画出来一样。第二条注入一次纵向滑动：同一串触摸打在列表
/// 内部，WebView 的滚动容器会滚动并把它吃掉，Skyline 下事件冒泡到祖先——用祖先里的
/// 指示条把「有没有收到触摸」画成终态。
///
/// 实测口径（2026-11 复核，见 `docs/RULE-VISUAL.md` 的「注入的手势不等于真机手势」）：
/// 宿主注入的合成 touch 序列不被 Skyline 的原生滚动控件消费（列表 `top` 保持 0），
/// 所以 Differ 证明的是「两端对同一串合成触摸的反应不同」，不等于真机上「原生滚动与
/// JS `touchmove` 并行」的手感；真机结论仍需单独验证。
///
/// 第一条 `Same` 的前置条件修正为「内容真的溢出」：容器 96px、单块内容 300px；注入点
/// 落在容器 `.list` 上，本模拟器两端都不启动原生滚动，内容又是一整块纯色，所以终态一致。
/// 它仍不是机制级对照：合成触摸下两端 `scrollTop` 都为 0，截图只能证明「溢出本身不产生
/// 可见差异」；本模拟器里「溢出 + scroll-view 的真阴性对照」不可能成立（见
/// `docs/RULE-VISUAL.md` 的「注入的手势不等于真机手势」）。
static VISUAL_CASES: &[VisualCase] = &[
    VisualCase {
        wxml: r#"<view class="sheet" bind:touchmove="onMove">
  <scroll-view class="list" scroll-y show-scrollbar="{{false}}">
    <view class="row" id="target"></view>
  </scroll-view>
</view>"#,
        // 全部涂同色，内容块给定宽 200px：本模拟器里 WebView 不实现
        // `show-scrollbar=false`，溢出的 scroll-view 会把内容宽度压窄 15px（原生滚动条槽位，
        // 实测 bbox 311 vs 296），目标用固定宽就避开槽位，也不让滑块进入比对区域。
        wxss: ".sheet { padding: 12px; background-color: #c2402a; }\n.list { height: 96px; background-color: #c2402a; }\n.row { width: 200px; height: 300px; background-color: #c2402a; }\n",
        js: Some("Page({ onMove() {} });\n"),
        expect: VisualExpect::Same,
        note: "结构命中规则（祖先 bind:touchmove + 子 scroll-y），内容 300px 真的溢出 96px 容器；滑动打在容器 `.list` 上，本模拟器两端都不启动原生滚动，单块纯色下终态一致（目标块固定宽 200px，避开 WebView 的滚动条槽位）。只证明「溢出本身不产生可见差异」，不证明两端滚动行为相同。Same 只在 `#target` 内成立——整页仍有 5760 device px 差异，是 WebView 独有的 15×96 CSS 滚动条，在元素比对区之外",
        target: "#target",
    },
    VisualCase {
        wxml: r#"<view class="sheet" id="target" bind:touchmove="onMove">
  <view class="banner {{moved ? 'hit' : ''}}">
    <text>moves={{moves}} top={{top}}</text>
  </view>
  <scroll-view class="list" id="list" scroll-y bindscroll="onScroll">
    <view class="row">第 1 行</view>
    <view class="row">第 2 行</view>
    <view class="row">第 3 行</view>
    <view class="row">第 4 行</view>
    <view class="row">第 5 行</view>
    <view class="row">第 6 行</view>
    <view class="row">第 7 行</view>
    <view class="row">第 8 行</view>
    <view class="row">第 9 行</view>
    <view class="row">第 10 行</view>
    <view class="row">第 11 行</view>
    <view class="row">第 12 行</view>
  </scroll-view>
</view>"#,
        wxss: ".sheet { padding: 12px; }\n.banner { height: 64px; line-height: 64px; text-align: center; color: #ffffff; background-color: #8a93a3; font-size: 14px; }\n.banner.hit { background-color: #c2402a; }\n.list { height: 144px; background-color: #f4f6fb; }\n.row { height: 24px; line-height: 24px; font-size: 13px; color: #333; }\n",
        js: Some(
            "Page({\n  data: { moves: 0, top: 0, moved: false },\n  onMove() { this.setData({ moves: this.data.moves + 1, moved: true }); },\n  onScroll(e) { this.setData({ top: Math.round(e.detail.scrollTop) }); },\n});\n",
        ),
        expect: VisualExpect::Differ,
        note: "在列表第一行上向上滑：Skyline 侧列表 top 保持 0（宿主注入的合成触摸不驱动原生滚动），只把 touchmove 冒泡给祖先（moves=6、指示条变红）；WebView 的滚动容器消费同一串触摸并阻止冒泡（列表滚到 100、top=100、祖先 moves=0）。仅证明合成触摸不被原生滚动消费，不代表真机「原生滚动 + touchmove 并行」行为",
        target: "#target",
    },
];

/// 两条用例的注入点不同（实测确定，不能共用）：
/// - `Same`：300px 内容块溢出 96px 容器，滑动打在容器 `.list` 上时两端都不启动原生
///   滚动，目标 bbox 不动，阴性对照成立；打在 `.row` 上 WebView 会真的滚动（目标 bbox
///   上移 120 CSS），`Same` 直接失败。
/// - `Differ`：12 行 24px 内容，滑动必须打在滚动内容（`.row`）上，WebView 的原生滚动
///   才会启动；打在 `.list` 容器上时本模拟器的 WebView 不滚，两端读数退化成一样。
pub struct ScrollGestureConflict;

impl Rule for ScrollGestureConflict {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Warning,
            description: "滚动容器上的滑动会冒泡触发祖先的 touchmove 手势（拖拽收起等）",
        }
    }

    fn visual_cases(&self) -> &'static [VisualCase] {
        VISUAL_CASES
    }

    fn visual_case_actions(&self, index: usize) -> &'static [VisualAction] {
        const SAME: &[VisualAction] = &[VisualAction::Swipe(".list", 0, -120)];
        const DIFFER: &[VisualAction] = &[VisualAction::Swipe(".row", 0, -120)];
        match index {
            0 => SAME,
            _ => DIFFER,
        }
    }

    fn target(&self) -> Target {
        Target::Skyline
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        let mut reported = HashSet::new();
        for_each_element(project, self.target(), |site| {
            if site.reach != Reach::Skyline {
                return;
            }
            let (doc, element) = (site.doc, site.element);
            let path = doc.path().to_string();
            {
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
                let chain = site.graph.ancestors(&path, element).nodes;
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
            };
        });
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

    /// 视觉用例那份页面结构本身要真的命中本规则（虽然它期望两端一致）。
    #[test]
    fn visual_cases_trigger_the_rule() {
        for (index, case) in super::VISUAL_CASES.iter().enumerate() {
            let fixture = Fixture::page(&format!("gesture-visual-{index}"), case.wxml, case.wxss);
            if let Some(js) = case.js {
                fixture.write("pages/index/index.js", js);
            }
            let diagnostics = check(&fixture);
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.rule == super::ID),
                "case {index}: {diagnostics:#?}"
            );
        }
    }

    /// 两条用例的注入点必须不同：`Same` 打在容器上（本模拟器的 WebView 不启动滚动，
    /// 目标 bbox 不动），`Differ` 打在滚动内容上（WebView 必须真的滚起来）。
    #[test]
    fn visual_cases_use_different_swipe_targets() {
        use crate::rules::VisualAction;
        let selector = |index: usize| {
            ScrollGestureConflict
                .visual_case_actions(index)
                .iter()
                .find_map(|action| match action {
                    VisualAction::Swipe(selector, ..) => Some(*selector),
                    _ => None,
                })
                .expect("用例应当注入一次 Swipe")
        };
        assert_eq!(selector(0), ".list");
        assert_eq!(selector(1), ".row");
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
