//! `skyline/page-scroll-api`：页面级滚动在 Skyline 下不存在，相关生命周期与配置静默失效。
//!
//! 证据链：
//!
//! - 引擎默认不给页面根建滚动容器：`shadow/context.cc` 的 `enablePageScroll` 才把
//!   根包进 `ScrollViewShadowNode`（`shadow_tree.cc:202-213`），默认分支是直接
//!   `root_container_->AppendChildImpl(root_)`——页面根不可滚。整页能不能滚由
//!   `scroll-view` 决定。
//! - `onPageScroll` / `onReachBottom` 的唯一生产者在 WebView 渲染层：`WAWebview.js`
//!   里 `(enablePageScroll || enablePullUpRefresh)` 时才挂 `window.onscroll` 并
//!   `publishPageEvent`。Skyline 渲染层 `WARenderContext.js` 没有任何对应实现。
//! - 因此页面 json 的 `enablePullDownRefresh` / `onReachBottomDistance` 也没有消费者：
//!   下拉与触底都发生在被 `scroll-view` 接管的内容区里。
//!
//! 迁移写法：整页内容搬进 `<scroll-view type="list">`，用 `bindscroll`（`e.detail.scrollTop`）、
//! `bindscrolltolower`、`refresher-enabled` + `refresher-triggered` 承接原逻辑。

use crate::input::json::JsonPositions;
use crate::project::Project;
use crate::rules::config::{anchor, warn};
use crate::rules::{Rule, RuleMeta, VisualCase, VisualExpect};
use crate::{Diagnostic, Severity};

use super::at_span;

/// 页面对象上的滚动生命周期：键名、问题的说法、替代写法。
struct Lifecycle {
    key: &'static str,
    message: &'static str,
    help: &'static str,
}

const LIFECYCLES: &[Lifecycle] = &[
    Lifecycle {
        key: "onPageScroll",
        message: "页面根在 Skyline 下不是滚动容器，`onPageScroll` 不会触发（事件只在 WebView 渲染层产生）",
        help: "把整页内容搬进 `<scroll-view type=\"list\">`，用 `bindscroll` 从 `e.detail.scrollTop` 取滚动位置",
    },
    Lifecycle {
        key: "onReachBottom",
        message: "页面根在 Skyline 下不滚动，`onReachBottom` 不会触发",
        help: "改用 `<scroll-view bindscrolltolower>`；分页距离用 `lower-threshold` 控制",
    },
    Lifecycle {
        key: "onPullDownRefresh",
        message: "下拉发生在页面容器上，Skyline 的页面根不可滚，`onPullDownRefresh` 不会触发",
        help: "改用 `<scroll-view refresher-enabled bindrefresherrefresh>`，用 `refresher-triggered` 收起",
    },
];

/// 页面 json 上的同类配置：写了也没有读者。
const PAGE_KEYS: &[(&str, &str, &str)] = &[
    (
        "enablePullDownRefresh",
        "页面根在 Skyline 下不可滚，`enablePullDownRefresh` 拉不出下拉区域",
        "下拉刷新改用 `<scroll-view refresher-enabled>`；这行配置可以删掉",
    ),
    (
        "onReachBottomDistance",
        "`onReachBottom` 在 Skyline 下不触发，触底距离配置也就没有读者",
        "触底距离改用 `<scroll-view lower-threshold>`；这行配置可以删掉",
    ),
];

/// 视觉用例：页面 JS 在 1.2s 后用 `wx.pageScrollTo` 把页面滚到 400。
///
/// 两端首帧一致（`#target` 都在 top 500）；动作之后 WebView 的页面根滚了 400，
/// Skyline 的页面根不是滚动容器，`#target` 留在原处。
static VISUAL_CASES: &[VisualCase] = &[VisualCase {
    wxml: r#"<view class="stage">
  <view class="marker" id="target">scrollTop = {{top}}</view>
  <view class="tail">页尾</view>
</view>"#,
    wxss: ".stage { position: relative; height: 1400px; background: #f4f6fb; }\n.marker { position: absolute; left: 20px; top: 500px; width: 240px; height: 96px; background: #2f6fed; color: #ffffff; text-align: center; line-height: 96px; font-size: 16px; border-radius: 8px; }\n.tail { position: absolute; left: 20px; top: 1200px; font-size: 14px; color: #888; }\n",
    js: Some(
        "Page({\n  data: { top: 0 },\n  onPageScroll(e) { this.setData({ top: e.scrollTop }); },\n  onReady() {\n    setTimeout(() => { wx.pageScrollTo({ scrollTop: 400, duration: 0 }); }, 1200);\n  },\n});\n",
    ),
    expect: VisualExpect::Differ,
    note: "两端首帧相同；1.2s 后 `wx.pageScrollTo(400)`：WebView 的页面根滚动 400（#target 从 top 500 到 100、文本变 400），Skyline 页面根不可滚，位置与文本都不变（同页的 `wx.pageScrollTo` 也会被 unsupported-api 报出）",
    target: "#target",
}];

pub struct PageScrollApi;

impl Rule for PageScrollApi {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "页面级滚动生命周期 / 配置在 Skyline 下不生效：页面根不是滚动容器，改用 scroll-view",
        }
    }

    fn visual_cases(&self) -> &'static [VisualCase] {
        VISUAL_CASES
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        check_lifecycles(project, &mut out);
        check_page_json(project, &mut out);
        out
    }
}

impl PageScrollApi {
    pub const ID: &'static str = "skyline/page-scroll-api";
}

fn check_lifecycles(project: &Project, out: &mut Vec<Diagnostic>) {
    for file in &project.scripts {
        if !super::renders_in_skyline(project, &file.scope) {
            continue;
        }
        for member in file.doc.members() {
            let Some(lifecycle) = LIFECYCLES.iter().find(|entry| entry.key == member.key) else {
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
                    lifecycle.message,
                )
                .help(lifecycle.help),
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
        for (key, message, help) in PAGE_KEYS {
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
                help,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testutil::check_rule;
    use crate::rules::testutil::js::{Fixture, single, single_with_app};

    use super::{PageScrollApi, VISUAL_CASES};

    /// 视觉用例的同一份 WXML/WXSS/JS 必须真的能触发本规则。
    #[test]
    fn visual_case_triggers_rule() {
        let case = &VISUAL_CASES[0];
        let fixture = Fixture::new("psa-visual");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", case.wxml)
            .write("pages/index/index.wxss", case.wxss)
            .write("pages/index/index.js", case.js.expect("视觉用例需要 JS"));
        let diagnostics = check_rule(&fixture, &PageScrollApi);
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.rule == PageScrollApi::ID),
            "视觉用例未触发规则：{diagnostics:#?}"
        );
    }

    #[test]
    fn page_scroll_lifecycles_are_reported() {
        let (_f, project) = single(
            "psa-lifecycles",
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
        let diagnostics = PageScrollApi.check(&project);
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

    /// 没有声明 Skyline 的页面（app 级 webview）不报。
    #[test]
    fn webview_page_is_not_reported() {
        let fixture = crate::rules::testutil::Fixture::new("psa-webview");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"webview"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", "<view/>")
            .write(
                "pages/index/index.js",
                "Page({ onPageScroll(e) {}, onReachBottom() {} });",
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
        let (_f, project) = single(
            "psa-throttle",
            "Page({\n  data: {},\n  onReachBottom: throttle(function () {}),\n});",
        );
        let diagnostics = PageScrollApi.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].line, 3, "{diagnostics:#?}");
    }
}
