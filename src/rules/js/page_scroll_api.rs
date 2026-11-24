//! `skyline/page-scroll-api`：页面级滚动在 Skyline 下不存在。两层根因，必须分开说。
//!
//! **第一层：页面根默认不是滚动容器，页面级订阅也只存在于 WebView 渲染层。**
//!
//! - 引擎默认不给页面根建滚动容器：`shadow/context.cc` 的 `enablePageScroll` 才把
//!   根包进 `ScrollViewShadowNode`（`shadow_tree.cc:202-213`），默认分支是直接
//!   `root_container_->AppendChildImpl(root_)`。整页能不能滚由 `scroll-view` 决定。
//!   这个开关是引擎内部的 feature flag，工程配置里没有对应键（`rendererOptions.skyline`
//!   的已知键表里没有它），所以本工具按默认（关闭）判定。
//! - `onPageScroll` / `onReachBottom` 的唯一生产者在 WebView 渲染层：`WAWebview.js`
//!   里 `(enablePageScroll || enablePullUpRefresh)` 时才挂 `window.onscroll` 并
//!   `publishPageEvent`。Skyline 渲染层 `WARenderContext.js` 没有任何对应实现。
//! - 因此页面 json 的 `enablePullDownRefresh` / `onReachBottomDistance` 也没有消费者：
//!   下拉与触底都发生在被 `scroll-view` 接管的内容区里。
//! - `wx.pageScrollTo` 作用于同一个页面根：没有滚动容器就没有消费者。
//!
//! **第二层：`<page-meta>` 的滚动接线在 Skyline 里被注释掉。**
//!
//! Skyline 的 `page-meta` 组件里，把 `scroll-top` 转成滚动的整段是注释
//! （`wx-page-meta` 的 `_applyUpdates`：`if (this._dirtyFields.scrollTop) { … }`
//! 连同 `__webviewEngine__.onPageScrollTo(...)` 整块被注释），同一文件里
//! `background-*` / `page-orientation` 都正常派发。`scroll-into-view` 走的是同一条
//! 页面滚动接线，`bindscrolldone` 也只在这条链路走完才触发——组件根本不派发 scroll。
//! 这是组件实现缺口，与「页面根能不能滚」是两件事：即使页面根可滚，这里也不会动。
//!
//! 两层分开写 message 的原因：`page-meta` 那段是基础库的注释，哪天放出来就恢复了；
//! 页面级订阅则不会跟着恢复。合并成一句「页面根不可滚」会让第二层的用户去改错地方。
//!
//! 迁移写法：整页内容搬进 `<scroll-view type="list">`，用 `bindscroll`
//! （`e.detail.scrollTop`）、`bindscrolltolower`、`refresher-enabled` + `refresher-triggered`
//! 承接原逻辑；编程式滚动用 `ScrollViewContext.scrollTo`（先
//! `createSelectorQuery().select().ref()` 拿引用）。

use crate::input::json::JsonPositions;
use crate::input::wxml::WxmlElement;
use crate::project::Project;
use crate::rules::config::{anchor, warn};
use crate::rules::wxml::structure::find_attr;
use crate::rules::wxml::{for_each_element, span_diagnostic};
use crate::rules::{Rule, RuleMeta, VisualCase, VisualExpect};
use crate::{Diagnostic, Severity};

use super::{Callee, at_span, renders_in_skyline};

/// 页面级滚动的唯一迁移路径：所有站点共用同一段 help。
const HELP: &str = "把整页内容搬进 `<scroll-view type=\"list\">`：`bindscroll`（`e.detail.scrollTop`）、`bindscrolltolower`（`lower-threshold`）、`refresher-enabled` + `refresher-triggered` 承接原逻辑；编程式滚动用 `ScrollViewContext.scrollTo`（先 `createSelectorQuery().select().ref()` 拿引用）";

/// 页面对象上的滚动生命周期：键名与问题的说法。
const LIFECYCLES: &[(&str, &str)] = &[
    (
        "onPageScroll",
        "页面根在 Skyline 下默认不是滚动容器，`onPageScroll` 不会触发（事件只在 WebView 渲染层产生）",
    ),
    (
        "onReachBottom",
        "页面根在 Skyline 下默认不滚动，`onReachBottom` 不会触发",
    ),
    (
        "onPullDownRefresh",
        "下拉发生在页面容器上，Skyline 的页面根默认不可滚，`onPullDownRefresh` 不会触发",
    ),
];

/// 页面 json 上的同类配置：写了也没有读者。
const PAGE_KEYS: &[(&str, &str)] = &[
    (
        "enablePullDownRefresh",
        "页面根在 Skyline 下默认不可滚，`enablePullDownRefresh` 拉不出下拉区域",
    ),
    (
        "onReachBottomDistance",
        "`onReachBottom` 在 Skyline 下不触发，触底距离配置也就没有读者",
    ),
];

/// `<page-meta>` 上控制页面滚动的属性：属性名与各自的说法。
///
/// 三个绑定指向同一处被注释掉的接线，`scroll-top` 在就只报它，避免同一行报两条。
const PAGE_META_SCROLL: &[(&str, &str)] = &[
    (
        "scroll-top",
        "Skyline 的 `<page-meta>` 没有把 `scroll-top` 接到滚动上（这段接线被注释掉，组件不派发 scroll），这个绑定不会滚动页面",
    ),
    (
        "scroll-into-view",
        "Skyline 的 `<page-meta>` 没有把 `scroll-into-view` 接到滚动上（与 `scroll-top` 共用那段被注释掉的接线），页面不会滚到目标元素",
    ),
];

/// 滚动结束事件；WXML 里三种前缀写法都要认。
const SCROLL_DONE: &[&str] = &["bindscrolldone", "bind:scrolldone", "catchscrolldone"];

/// `scrolldone` 的根因说明：它依赖的正是上面那段被注释掉的滚动链路。
const SCROLL_DONE_MESSAGE: &str = "`scrolldone` 只在 `<page-meta>` 完成页面滚动后派发；Skyline 的这段接线被注释掉，组件不派发 scroll，事件不会触发";

/// 页面根滚动的编程式入口：与页面级订阅同源，没有可滚的根就没有消费者。
const PAGE_SCROLL_TO: Callee = Callee::Exact("wx.pageScrollTo");
const PAGE_SCROLL_TO_MESSAGE: &str = "页面级滚动在 Skyline 下没有实现：引擎默认不给页面根包滚动容器，`wx.pageScrollTo` 找不到可滚的根，滚动不会发生";

/// 视觉用例一：页面 JS 在 1.2s 后用 `wx.pageScrollTo` 把页面滚到 400。
///
/// 两端首帧一致（`#target` 都在 top 500）；动作之后 WebView 的页面根滚了 400，
/// Skyline 的页面根不是滚动容器，`#target` 留在原处。
///
/// 视觉用例二（`<page-meta scroll-top>`）：`scroll-top` 写在模板里，首帧就生效。
/// WebView 的 `<page-meta>` 走 `onPageScrollTo`：页高 1200px（6×200）减视口 667px，
/// `scroll-top=600` 被钳到最大值 533，`#target` 顶部落在 67px，不是「视口顶部」。
/// Skyline 的这条路径被注释掉了，滚动偏移仍为 0；同时页面根把 6 个 200px 块压成
/// 111.17px，`#target` 顶部在 333.5px。99.97% 的像素差异同时含「压扁」与「滚动」两个效应。
static VISUAL_CASES: &[VisualCase] = &[
    VisualCase {
        wxml: r#"<view class="stage">
  <view class="marker" id="target">scrollTop = {{top}}</view>
  <view class="tail">页尾</view>
</view>"#,
        wxss: ".stage { position: relative; height: 1400px; background: #f4f6fb; }\n.marker { position: absolute; left: 20px; top: 500px; width: 240px; height: 96px; background: #2f6fed; color: #ffffff; text-align: center; line-height: 96px; font-size: 16px; border-radius: 8px; }\n.tail { position: absolute; left: 20px; top: 1200px; font-size: 14px; color: #888; }\n",
        js: Some(
            "Page({\n  data: { top: 0 },\n  onPageScroll(e) { this.setData({ top: e.scrollTop }); },\n  onReady() {\n    setTimeout(() => { wx.pageScrollTo({ scrollTop: 400, duration: 0 }); }, 1200);\n  },\n});\n",
        ),
        expect: VisualExpect::Differ,
        note: "首帧两端一致（#target 都在 top 500）；1.2s 后 `wx.pageScrollTo(400)`：WebView 页面根滚 400（#target 从 500 到 100、文本变 400、右侧出现滚动条），Skyline 侧 #target 仍在 500、文本仍 0——截图上表现为「滚动完全没发生」。页面根不可滚这一层是引擎默认（`enablePageScroll` 未开）的结论，不是截图直接测到的；同页的 `wx.pageScrollTo` 由本规则报出",
        target: "#target",
    },
    VisualCase {
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
        note: "首帧 `<page-meta scroll-top=\"600\">`：WebView 滚到页高上限 533px，`#target` 顶部在 67px；Skyline 不滚（偏移 0），页面根是 `height:100%` 的 flex 容器、子项默认 `flex-shrink:1`，6 个 200px 块各被压到 667/6 = 111.17px，`#target` 顶部在 333.5px——这正是「根没有滚动容器」的直接证据；WebView 侧块宽 360（滚动条槽占 15px）、Skyline 侧 375。99.97% 的差异同时来自「压扁」和「滚动」",
        target: "#target",
    },
];

pub struct PageScrollApi;

impl Rule for PageScrollApi {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "页面级滚动在 Skyline 下不存在：页面根默认不建滚动容器、页面级订阅只在 WebView 渲染层，`<page-meta>` 的滚动接线也被注释掉——整页改用 scroll-view",
        }
    }

    fn visual_cases(&self) -> &'static [VisualCase] {
        VISUAL_CASES
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        check_page_meta(project, self.target(), &mut out);
        check_lifecycles(project, &mut out);
        check_page_scroll_to(project, &mut out);
        check_page_json(project, &mut out);
        out
    }
}

impl PageScrollApi {
    pub const ID: &'static str = "skyline/page-scroll-api";
}

/// `<page-meta>` 的滚动绑定：组件不派发 scroll，与页面根能不能滚无关。
fn check_page_meta(project: &Project, target: crate::rules::Target, out: &mut Vec<Diagnostic>) {
    for_each_element(project, target, |site| {
        let element: &WxmlElement = site.element;
        if element.tag != "page-meta" {
            return;
        }
        let scroll = PAGE_META_SCROLL
            .iter()
            .find_map(|(name, message)| find_attr(element, name).map(|attr| (attr, *message)));
        let scroll_done = SCROLL_DONE.iter().find_map(|name| find_attr(element, name));
        // 三个绑定指向同一处被注释掉的接线，只报第一条。
        let (attr, message) = match (scroll, scroll_done) {
            (Some((attr, message)), _) => (attr, message),
            (None, Some(attr)) => (attr, SCROLL_DONE_MESSAGE),
            (None, None) => return,
        };
        out.push(
            span_diagnostic(
                PageScrollApi::ID,
                site.doc,
                attr.start,
                attr.end,
                site.doc.path(),
                Severity::Warning,
                message,
            )
            .help(site.reach.help(HELP)),
        );
    });
}

fn check_lifecycles(project: &Project, out: &mut Vec<Diagnostic>) {
    for file in &project.scripts {
        if !renders_in_skyline(project, &file.scope) {
            continue;
        }
        for member in file.doc.members() {
            let Some((_, message)) = LIFECYCLES.iter().find(|(key, _)| *key == member.key) else {
                continue;
            };
            // 位置取属性自身的范围：`onReachBottom: throttle(fn)` 这类值不是函数
            // 字面量时也能落到声明行。
            out.push(
                at_span(
                    PageScrollApi::ID,
                    Severity::Warning,
                    file,
                    member.start,
                    member.end,
                    *message,
                )
                .help(HELP),
            );
        }
    }
}

/// `wx.pageScrollTo` 从 `unsupported-api` 迁来：它的根因是页面根不可滚，
/// 不是「某个 API 不受支持」。
fn check_page_scroll_to(project: &Project, out: &mut Vec<Diagnostic>) {
    for file in &project.scripts {
        if !renders_in_skyline(project, &file.scope) {
            continue;
        }
        for call in file.doc.calls() {
            // 只在 WebView 分支里执行的调用：Skyline 根本不走这段。
            if call.webview_only || !PAGE_SCROLL_TO.matches(&call.callee) {
                continue;
            }
            out.push(
                at_span(
                    PageScrollApi::ID,
                    Severity::Warning,
                    file,
                    call.start,
                    call.end,
                    PAGE_SCROLL_TO_MESSAGE,
                )
                .help(HELP),
            );
        }
    }
}

fn check_page_json(project: &Project, out: &mut Vec<Diagnostic>) {
    for page in &project.pages {
        if !page.is_skyline || page.opted_out {
            continue;
        }
        let Some(json) = project.page_json.get(&page.route) else {
            continue;
        };
        let source = project
            .page_json_source
            .get(&page.route)
            .map_or("", String::as_str);
        let positions = JsonPositions::new(source);
        for (key, message) in PAGE_KEYS {
            let Some(value) = json.get(*key) else {
                continue;
            };
            // `enablePullDownRefresh: false` 是「主动关掉」，没有配置意图可违反。
            if value.as_bool() == Some(false) {
                continue;
            }
            warn(
                out,
                PageScrollApi::ID,
                &format!("{}.json", page.route),
                positions.key(key),
                anchor(&[key], Some(value)),
                (*message).to_string(),
                HELP,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testutil::check_rule;
    use crate::rules::testutil::js::{Fixture, single, single_with_app};
    use crate::rules::testutil::wxml;

    use super::{PageScrollApi, VISUAL_CASES};

    fn check_js(js: &str) -> Vec<crate::Diagnostic> {
        let (_f, project) = single("psa-js", js);
        PageScrollApi.check(&project)
    }

    fn check_wxml(wxml: &str) -> Vec<crate::Diagnostic> {
        check_rule(&wxml::project("psa-wxml", wxml), &PageScrollApi)
    }

    /// 视觉用例的同一份 WXML/WXSS/JS 必须真的能触发本规则。
    #[test]
    fn visual_cases_trigger_rule() {
        for (index, case) in VISUAL_CASES.iter().enumerate() {
            let fixture = Fixture::new(&format!("psa-visual-{index}"));
            fixture
                .write("project.config.json", r#"{"miniprogramRoot":""}"#)
                .write(
                    "app.json",
                    r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
                )
                .write("pages/index/index.json", "{}")
                .write("pages/index/index.wxml", case.wxml)
                .write("pages/index/index.wxss", case.wxss)
                .write("pages/index/index.js", case.js.unwrap_or("Page({})\n"));
            let diagnostics = check_rule(&fixture, &PageScrollApi);
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.rule == PageScrollApi::ID),
                "第 {} 条视觉用例未触发规则：{diagnostics:#?}",
                index + 1
            );
        }
    }

    #[test]
    fn page_scroll_lifecycles_are_reported() {
        let diagnostics = check_js(
            r#"
            Page({
              data: { list: [] },
              onPageScroll(e) { this.setData({ top: e.scrollTop }); },
              onReachBottom() { this.loadMore(); },
              onPullDownRefresh() { this.refresh(); },
              onLoad() {},
            });
            "#,
        );
        assert_eq!(diagnostics.len(), 3, "{diagnostics:#?}");
        let messages = diagnostics
            .iter()
            .map(|d| d.message.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(messages.contains("onPageScroll"), "{messages}");
        assert!(messages.contains("onReachBottom"), "{messages}");
        assert!(messages.contains("onPullDownRefresh"), "{messages}");
        // 定位落在方法所在行（模板字符串首行是空行，所以是 4/5/6）。
        assert_eq!(
            diagnostics.iter().map(|d| d.line).collect::<Vec<_>>(),
            vec![4, 5, 6],
            "{diagnostics:#?}"
        );
    }

    /// 页面显式声明 `renderer: "webview"` 时整页不报：那是「这页不迁 Skyline」的决定。
    #[test]
    fn webview_page_is_not_reported() {
        let fixture = crate::rules::testutil::Fixture::new("psa-webview");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", r#"{"renderer":"webview"}"#)
            .write("pages/index/index.wxml", "<page-meta scroll-top=\"1\"/>")
            .write(
                "pages/index/index.js",
                "Page({ onPageScroll(e) {}, onReachBottom() { wx.pageScrollTo({ scrollTop: 1 }); } });",
            );
        let diagnostics = PageScrollApi.check(&fixture.project());
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// 页面 json 的下拉 / 触底配置同样没有读者；显式关闭不报。
    #[test]
    fn page_config_keys_are_reported() {
        let fixture = crate::rules::testutil::Fixture::new("psa-json");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/a/a","pages/b/b"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/a/a.json",
                r#"{"enablePullDownRefresh":true,"onReachBottomDistance":80}"#,
            )
            .write("pages/a/a.wxml", "<view/>")
            .write("pages/b/b.json", r#"{"enablePullDownRefresh":false}"#)
            .write("pages/b/b.wxml", "<view/>");
        let diagnostics = PageScrollApi.check(&fixture.project());
        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
        assert!(
            diagnostics.iter().all(|d| d.path == "pages/a/a.json"),
            "{diagnostics:#?}"
        );
    }

    /// `app.js` 里定义的普通方法（不在 `Page({...})` 根对象上）不报。
    #[test]
    fn helper_named_like_lifecycle_is_not_reported() {
        let (_f, project) = single_with_app(
            "psa-helper",
            "function onPageScroll() {}\nPage({ onLoad() { onPageScroll(); } });",
            Some("App({ onLaunch() {} });"),
        );
        let diagnostics = PageScrollApi.check(&project);
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// 值不是函数字面量（`throttle(...)`）时也落在属性所在行，不退回文件首。
    #[test]
    fn wrapped_lifecycle_lands_on_its_own_line() {
        let diagnostics =
            check_js("Page({\n  data: {},\n  onReachBottom: throttle(function () {}),\n});");
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].line, 3, "{diagnostics:#?}");
    }

    /// `<page-meta scroll-top>` 的根因是「组件接线被注释」，
    /// 不能再说成「页面根不是滚动容器」。
    #[test]
    fn scroll_top_on_page_meta_is_reported_with_its_own_root_cause() {
        let diagnostics = check_wxml(r#"<page-meta scroll-top="600"/>"#);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        let message = &diagnostics[0].message;
        assert!(message.contains("scroll-top"), "{message}");
        assert!(message.contains("被注释"), "{message}");
        assert!(!message.contains("页面根"), "{message}");
    }

    #[test]
    fn scroll_into_view_on_page_meta_is_reported() {
        let diagnostics = check_wxml(r#"<page-meta scroll-into-view="target"/>"#);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(
            diagnostics[0].message.contains("scroll-into-view"),
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn scroll_done_alone_on_page_meta_is_reported() {
        for (name, attr) in [
            ("bindscrolldone", "bindscrolldone"),
            ("bind:scrolldone", "bind:scrolldone"),
            ("catchscrolldone", "catchscrolldone"),
        ] {
            let diagnostics = check_wxml(&format!("<page-meta {attr}=\"onDone\"/>"));
            assert_eq!(diagnostics.len(), 1, "{name}: {diagnostics:#?}");
            assert!(diagnostics[0].message.contains("scrolldone"), "{name}");
        }
    }

    /// 多个绑定指向同一处接线：只报一条（报 `scroll-top`，根因在那里）。
    #[test]
    fn page_meta_bindings_report_once() {
        let diagnostics = check_wxml(
            r#"<page-meta scroll-top="{{top}}" scroll-into-view="target" bindscrolldone="onDone"/>"#,
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(
            diagnostics[0].message.contains("scroll-top"),
            "{diagnostics:#?}"
        );
    }

    /// 其它属性（背景色、字号、方向）Skyline 都实现了，不能误报。
    #[test]
    fn other_page_meta_attributes_are_fine() {
        let diagnostics = check_wxml(
            r##"<page-meta background-color="#fff" page-font-size="16px" page-orientation="landscape"/>"##,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// `wx.pageScrollTo` 由本规则负责，且只报一次。
    #[test]
    fn page_scroll_to_is_reported() {
        let diagnostics =
            check_js("Page({ onReady() { wx.pageScrollTo({ scrollTop: 400, duration: 0 }); } });");
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        let message = &diagnostics[0].message;
        assert!(message.contains("wx.pageScrollTo"), "{message}");
        assert!(message.contains("页面根"), "{message}");
    }

    /// 迁移后 `unsupported-api` 不再报这一条（避免同一个 API 双报）。
    #[test]
    fn page_scroll_to_is_not_reported_by_unsupported_api() {
        let (_f, project) = single(
            "psa-unsupported",
            "Page({ onReady() { wx.pageScrollTo({ scrollTop: 400 }); } });",
        );
        let unsupported = crate::rules::js::UnsupportedApi.check(&project);
        assert!(
            unsupported
                .iter()
                .all(|diagnostic| !diagnostic.message.contains("wx.pageScrollTo")),
            "{unsupported:#?}"
        );
    }

    /// WebView 分支里的调用在 Skyline 下不执行，报了是误报。
    #[test]
    fn page_scroll_to_in_webview_branch_is_not_reported() {
        let diagnostics = check_js(
            "Page({ open() { if (this.renderer === 'webview') { wx.pageScrollTo({ scrollTop: 1 }); } } });",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// `enablePageScroll` 是引擎内部开关，工程配置里没有这个键：
    /// 写进 `rendererOptions` 也改变不了页面根，本规则照报。
    #[test]
    fn page_root_scroll_flag_is_not_a_project_config_key() {
        let fixture = Fixture::new("psa-enable-page-scroll");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"enablePageScroll":true}}}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", "<view/>")
            .write(
                "pages/index/index.js",
                "Page({ onPageScroll() { wx.pageScrollTo({ scrollTop: 1 }); } });",
            );
        let diagnostics = PageScrollApi.check(&fixture.project());
        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
    }
}
