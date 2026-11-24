//! `nested-scroll-*` 与 `scroll-view type="nested"` 在 WebView 下不联动。
//!
//! 嵌套滚动是 Skyline 新增能力（`skyline/shadow/components/nested_scroll_*`），
//! WebView 的 `wx-scroll-view` 从不读 `type`（WebView 组件实现），
//! `nested-scroll-header` / `nested-scroll-body` 是未注册标签 → 未知元素：
//! 子节点还在，但头部不会随主体滚动收起，而是常驻在顶部。
//!
//! 两端兼容：给整块加 `wx:if="{{renderer === 'skyline'}}"`，WebView 分支改用
//! 普通的 `scroll-view` + 固定头部（或用 `page-container`）。

use crate::diag::{Diagnostic, Severity};
use crate::project::Project;
use crate::rules::{Rule, RuleMeta, Target, VisualAction, VisualCase, VisualExpect};

use super::{element_diagnostic, for_each_element, structure::find_attr};

pub const ID: &str = "webview/nested-scroll";

/// 只存在于 Skyline 的嵌套滚动标签。
const TAGS: [&str; 2] = ["nested-scroll-header", "nested-scroll-body"];

/// 视觉用例：外层 `type="nested"` + `nested-scroll-*`，body 里再放一个 `scroll-view`，
/// 首行里放一块固定宽的目标块。
///
/// 静态布局两端一致（未注册标签在 glass-easel 下只影响包裹方式，子节点仍是块级），
/// 差异要靠同一串注入触摸触发：WebView 把 `type="nested"` 当普通 `scroll-view`，
/// 触点在内部 `scroll-view` 上时它会滚动，内容上移约 28.5px（DOM top 40 → 10，画面上
/// 被内层容器顶边 y=40 裁掉）；Skyline 的嵌套滚动
/// 原生容器不消费宿主注入的合成触摸，目标块 top 保持 40。目标块固定 160px 宽，
/// 避开 WebView `scroll-view` 的滚动条槽位（两端宽度都是 160）。
static VISUAL_CASES: &[VisualCase] = &[VisualCase {
    wxml: r#"<scroll-view class="outer" type="nested" scroll-y>
  <nested-scroll-header class="header"><view class="hmark">HEADER</view></nested-scroll-header>
  <nested-scroll-body class="body">
    <scroll-view class="inner" scroll-y>
      <view class="row"><view class="mark" id="target"></view></view>
      <view class="row">ROW 2</view>
      <view class="row">ROW 3</view>
      <view class="row">ROW 4</view>
      <view class="row">ROW 5</view>
      <view class="row">ROW 6</view>
    </scroll-view>
  </nested-scroll-body>
</scroll-view>"#,
    wxss: ".outer { height: 320px; background: #f4f6fb; }\n.hmark { width: 160px; height: 40px; background: #8a93a3; }\n.inner { height: 280px; background: #ffffff; }\n.row { height: 60px; background: #f4f6fb; }\n.mark { width: 160px; height: 24px; background: #c2402a; }\n",
    js: None,
    expect: VisualExpect::Differ,
    note: "在首行目标块上向上滑 30px：WebView 把 `type=\"nested\"` 当普通 scroll-view，内部滚动容器消费这串触摸，内容上移约 28.5px（DOM bbox：目标块 top 40 → 10，但 10 在内层滚动容器顶边 y=40 之上，被裁掉——WebView 截图里这块红色一个像素都看不到）；可见证据是 ROW 2–5 的文本从 103.5/163.5/223.5/283.5 上移到 75/135/195.5/255.5、ROW 6 在 315 露头、右侧出现滚动条。Skyline 的嵌套滚动原生容器不消费宿主注入的合成触摸，目标块 top 保持 40、ROW 2 仍在 103.5。只证明两端对同一份源码/同一串合成触摸反应不同，不等于真机上头部随主体收起的联动",
    target: "#target",
}];

pub struct NestedScroll;

impl Rule for NestedScroll {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Warning,
            description: "嵌套滚动只在 Skyline 下生效，WebView 下头部不联动、常驻顶部",
        }
    }

    fn target(&self) -> Target {
        Target::Webview
    }

    fn visual_cases(&self) -> &'static [VisualCase] {
        VISUAL_CASES
    }

    fn visual_actions(&self) -> &'static [VisualAction] {
        const ACTIONS: &[VisualAction] = &[VisualAction::Swipe("#target", 0, -30)];
        ACTIONS
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        for_each_element(project, self.target(), |site| {
            let (doc, element) = (site.doc, site.element);
            let tag = element.tag.to_ascii_lowercase();
            let nested_type = tag == "scroll-view"
                && find_attr(element, "type").and_then(|attr| attr.value.as_deref())
                    == Some("nested");
            if !nested_type && !TAGS.contains(&tag.as_str()) {
                return;
            }
            out.push(
                    element_diagnostic(
                        ID,
                        doc,
                        element,
                        doc.path(),
                        Severity::Warning,
                        format!(
                            "`{tag}` 的嵌套滚动是 Skyline 新增能力，WebView 下头部不会随主体联动"
                        ),
                    )
                    .help("给整块加 `wx:if=\"{{renderer === 'skyline'}}\"`，WebView 分支用普通 `scroll-view` + 固定头部"),
                );
        });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::NestedScroll;
    use crate::rules::testutil::wxml::{Fixture, check_rule, project};

    #[test]
    fn nested_type_is_reported() {
        let diagnostics = check_rule(
            &project(
                "webview/nested-type",
                "<scroll-view type=\"nested\" scroll-y><nested-scroll-header><view/></nested-scroll-header><nested-scroll-body><view/></nested-scroll-body></scroll-view>",
            ),
            &NestedScroll,
        );
        assert_eq!(diagnostics.len(), 3);
    }

    #[test]
    fn guarded_block_is_skipped() {
        let diagnostics = check_rule(
            &project(
                "webview/nested-guard",
                "<scroll-view wx:if=\"{{renderer === 'skyline'}}\" type=\"nested\"><nested-scroll-header><view/></nested-scroll-header></scroll-view>",
            ),
            &NestedScroll,
        );
        assert!(diagnostics.is_empty());
    }

    /// 视觉用例那份页面结构本身要真的命中本规则：`type="nested"` 与两个
    /// `nested-scroll-*` 标签各报一条。
    #[test]
    fn visual_case_is_reported() {
        let case = &super::VISUAL_CASES[0];
        let fixture = Fixture::page("webview/nested-visual", case.wxml, case.wxss);
        let diagnostics = check_rule(&fixture, &NestedScroll);
        assert_eq!(diagnostics.len(), 3, "{diagnostics:#?}");
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.rule == super::ID)
        );
    }

    #[test]
    fn plain_scroll_view_is_fine() {
        let diagnostics = check_rule(
            &project(
                "webview/nested-none",
                "<scroll-view scroll-y><view/></scroll-view>",
            ),
            &NestedScroll,
        );
        assert!(diagnostics.is_empty());
    }
}
