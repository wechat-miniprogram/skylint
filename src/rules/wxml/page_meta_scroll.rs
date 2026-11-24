//! `skyline/page-meta-scroll`：`<page-meta>` 的页面滚动绑定在 Skyline 下没有实现。
//!
//! 证据：Skyline 的 `page-meta` 组件里，把 `scroll-top` 转成滚动的整段被注释掉了
//! （`wx-page-meta` 的 `_applyUpdates`：`if (this._dirtyFields.scrollTop) { … }`
//! 连同 `__webviewEngine__.onPageScrollTo(...)` 整块是注释），同一文件里
//! `background-text-style` / `background-color` / `page-orientation` 都正常派发。WebView
//! 版的同名组件走的是 `onPageScrollTo`，这是 Skyline 渲染层唯一的消费者。
//!
//! 因此：
//! - `scroll-top`（含 `scroll-duration`）不产生任何滚动，页面也不会因此改变位置；
//! - `bindscrolldone` 只在上面那条路径走完才触发，同样不会响。
//!
//! `scroll-duration` 单独出现时没有意义（它只影响 `scroll-top` 的动画时长），
//! 不单独报。

use crate::Diagnostic;
use crate::diag::Severity;
use crate::input::wxml::WxmlElement;
use crate::project::Project;
use crate::rules::wxml::structure::find_attr;
use crate::rules::wxml::{for_each_element, span_diagnostic};
use crate::rules::{Rule, RuleMeta, VisualCase, VisualExpect};

const TAG: &str = "page-meta";

/// 滚动结束事件；WXML 里三种前缀写法都要认。
const SCROLL_DONE: &[&str] = &["bindscrolldone", "bind:scrolldone", "catchscrolldone"];

pub struct PageMetaScroll;

/// 视觉用例：`scroll-top` 写在模板里，首帧就生效。
///
/// WebView 的 `<page-meta>` 走 `onPageScrollTo`：页高 1200px（6×200）减视口 667px，
/// `scroll-top=600` 被钳到最大值 533，`#target` 顶部落在 67px，不是「视口顶部」。
/// Skyline 的这条路径被注释掉了，滚动偏移仍为 0；同时页面根把 6 个 200px 块压成
/// 111.17px，`#target` 顶部在 333.5px。99.97% 的像素差异同时含「压扁」与「滚动」两个效应。
static VISUAL_CASES: &[VisualCase] = &[VisualCase {
    wxml: r#"<page-meta scroll-top="600"/>
<view class="block a">一</view>
<view class="block b">二</view>
<view class="block c">三</view>
<view class="block d" id="target">四</view>
<view class="block e">五</view>
<view class="block f">六</view>"#,
    wxss: ".block { height: 200px; font-size: 20px; }\n.a { background: #eee; }\n.b { background: #ddd; }\n.c { background: #ccc; }\n.d { background: #f66; }\n.e { background: #bbb; }\n.f { background: #aaa; }\n",
    js: None,
    expect: VisualExpect::Differ,
    note: "首帧 `<page-meta scroll-top=\"600\">`：WebView 滚到页高上限 533px，`#target` 顶部在 67px；Skyline 不滚（偏移 0），页面根把 6 个 200px 块压成 111.17px，`#target` 顶部在 333.5px——99.97% 的差异同时来自「压扁」和「滚动」",
    target: "#target",
}];

impl PageMetaScroll {
    pub const ID: &'static str = "skyline/page-meta-scroll";

    const HELP: &'static str = "整页滚动改用 `<scroll-view scroll-top>` 或 `ScrollViewContext.scrollTo`（先 `createSelectorQuery().select().ref()` 拿引用）";
}

impl Rule for PageMetaScroll {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "`<page-meta>` 的 scroll-top / scrolldone 在 Skyline 下没有实现：页面根不是滚动容器",
        }
    }

    fn visual_cases(&self) -> &'static [VisualCase] {
        VISUAL_CASES
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        for_each_element(project, self.target(), |site| {
            let element: &WxmlElement = site.element;
            if element.tag != TAG {
                return;
            }
            let scroll_top = find_attr(element, "scroll-top");
            let scroll_done = SCROLL_DONE.iter().find_map(|name| find_attr(element, name));
            // 两个绑定指向同一件事，`scroll-top` 在就只报它，避免同一行报两条。
            let (attr, message) = match (scroll_top, scroll_done) {
                (Some(attr), _) => (
                    attr,
                    "Skyline 的 `<page-meta>` 没有实现 `scroll-top`：页面根不是滚动容器，这个绑定不会滚动页面",
                ),
                (None, Some(attr)) => (
                    attr,
                    "`scrolldone` 只在 `<page-meta scroll-top>` 滚动结束时触发；Skyline 下这条路径没有实现，事件不会触发",
                ),
                (None, None) => return,
            };
            out.push(
                span_diagnostic(
                    Self::ID,
                    site.doc,
                    attr.start,
                    attr.end,
                    site.doc.path(),
                    Severity::Warning,
                    message,
                )
                .help(site.reach.help(Self::HELP)),
            );
        });
        out
    }
}

#[cfg(test)]
mod tests {
    use crate::rules::testutil::wxml::{Fixture, check_rule};

    use super::PageMetaScroll;

    fn check(fixture: &Fixture) -> Vec<crate::Diagnostic> {
        check_rule(fixture, &PageMetaScroll)
    }

    /// 视觉用例同源的页面：静态 `scroll-top` 首帧就应该有差异。
    #[test]
    fn visual_case_scroll_top_is_reported() {
        let fixture = Fixture::page(
            "pms-visual",
            r#"<page-meta scroll-top="600"/>
<view class="block">一</view>
<view class="block d" id="target">四</view>"#,
            ".block { height: 200px; }\n",
        );
        let diagnostics = check(&fixture);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("scroll-top"));
    }

    #[test]
    fn scroll_top_is_reported() {
        let fixture = Fixture::wxml(
            "pms-top",
            r#"<page-meta scroll-top="{{top}}" scroll-duration="300"/>"#,
        );
        let diagnostics = check(&fixture);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(
            diagnostics[0].message.contains("scroll-top"),
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn scroll_done_alone_is_reported() {
        for (name, attr) in [
            ("pms-done", "bindscrolldone"),
            ("pms-done-colon", "bind:scrolldone"),
        ] {
            let fixture = Fixture::wxml(name, &format!("<page-meta {attr}=\"onDone\"/>"));
            let diagnostics = check(&fixture);
            assert_eq!(diagnostics.len(), 1, "{name}: {diagnostics:#?}");
            assert!(diagnostics[0].message.contains("scrolldone"), "{name}");
        }
    }

    /// 两个绑定同时写只报一条（报 `scroll-top`，根因在那里）。
    #[test]
    fn both_bindings_report_once() {
        let fixture = Fixture::wxml(
            "pms-both",
            r#"<page-meta scroll-top="{{top}}" bindscrolldone="onDone"/>"#,
        );
        assert_eq!(check(&fixture).len(), 1, "{:#?}", check(&fixture));
    }

    /// 其它属性（背景色、字号、方向）Skyline 都实现了，不能误报。
    #[test]
    fn other_page_meta_attributes_are_fine() {
        let fixture = Fixture::wxml(
            "pms-other",
            r##"<page-meta background-color="#fff" page-font-size="16px" page-orientation="landscape"/>"##,
        );
        assert!(check(&fixture).is_empty(), "{:#?}", check(&fixture));
    }
}
