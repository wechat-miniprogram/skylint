//! Skyline 不支持的 JS API / 页面配置项。
//!
//! 条目直接写在 [`ENTRIES`] 里，Rust 侧只做两种匹配：`Call` 命中 callee 精确相等的调用；
//! `PluginCall` 同上（callee 等于目标或以 `目标.` 开头），但只在插件工程里判。
//! 加条目只改 [`ENTRIES`]。
//!
//! 证据：
//! - 兼容性文档（Q: 不支持 animate
//!   动画接口，给了 worklet 替代写法）；
//! - 总览的速查表同样把它列为不支持；
//! - 路由 API 文档（所有 `wx.router` 方法均不支持小程序插件）。
//!
//! 注释与字符串里的 `this.animate` 不在 AST 的调用节点上，天然不会命中。
//!
//! 页面级滚动的 API（`wx.pageScrollTo`）不在这里：它的根因是页面根默认不可滚，
//! 与「某个 API 不受支持」不是一回事，由 `skyline/page-scroll-api` 统一负责。

use crate::Severity;
use crate::project::Project;
use crate::rules::{Rule, RuleMeta, VisualCase, VisualComponent, VisualExpect};

use super::{Callee, at_span, in_plugin, renders_in_skyline};

#[derive(Debug, Clone, Copy)]
enum Entry {
    /// 调用点：callee 精确等于目标文本。
    Call {
        callee: &'static str,
        message: &'static str,
        /// 只在 Skyline 渲染的单元里才失效。
        skyline_only: bool,
    },
    /// 调用点：callee 等于目标名或以 `目标名.` 开头，且只在插件工程里判。
    ///
    /// 插件限制原文：`skyline-route/references/api/router-api.md:9`「所有 wx.router
    /// 方法均不支持小程序插件」。插件页内调这些接口不会报错，只是静默无效。
    PluginCall {
        prefix: &'static str,
        message: &'static str,
    },
}

const ENTRIES: &[Entry] = &[
    Entry::Call {
        callee: "this.animate",
        message: "Skyline 不支持 `this.animate`，动画不会执行；改用 worklet（wx.worklet.shared + applyAnimatedStyle）",
        skyline_only: true,
    },
    Entry::PluginCall {
        prefix: "wx.router",
        message: "小程序插件不支持 wx.router.*（自定义路由），插件页内调用无效",
    },
    Entry::Call {
        callee: "wx.createVideoContext",
        message: "Skyline 下 `wx.createVideoContext` 必须把组件实例作为第二个参数传入（`wx.createVideoContext('id', this)`）；省略 `this` 时拿不到 nodeId，context 可能定位不到节点。建议改用 `createSelectorQuery().select('#id').context()`",
        skyline_only: true,
    },
    Entry::Call {
        callee: "wx.createMapContext",
        message: "Skyline 下 `wx.createMapContext` 必须把组件实例作为第二个参数传入（`wx.createMapContext('id', this)`）；省略 `this` 时拿不到 nodeId，context 可能定位不到节点。建议改用 `createSelectorQuery().select('#id').context()`",
        skyline_only: true,
    },
    Entry::Call {
        callee: "wx.createLivePlayerContext",
        message: "Skyline 下 `wx.createLivePlayerContext` 必须把组件实例作为第二个参数传入（`wx.createLivePlayerContext('id', this)`）；省略 `this` 时拿不到 nodeId，context 可能定位不到节点。建议改用 `createSelectorQuery().select('#id').context()`",
        skyline_only: true,
    },
    Entry::Call {
        callee: "wx.createCameraContext",
        message: "`wx.createCameraContext` 已不再维护，Skyline 下暂无 selectorQuery 等价物；插件工程里 pluginId 两侧写法还不一致，需实测确认",
        skyline_only: true,
    },
    Entry::Call {
        callee: "wx.createLivePusherContext",
        message: "`wx.createLivePusherContext` 已不再维护，Skyline 下暂无 selectorQuery 等价物；插件工程里 pluginId 两侧写法还不一致，需实测确认",
        skyline_only: true,
    },
    Entry::Call {
        callee: "wx.createAudioContext",
        message: "`wx.createAudioContext` 已不再维护；Skyline 没有 `<audio>` 组件，改用 `wx.createInnerAudioContext()`",
        skyline_only: true,
    },
];

/// 视觉用例：组件 `ready` 里调 `this.animate`，动画回调里再 `setData` 换状态。
///
/// WebView 执行动画并在回调里把方块换成蓝色「已执行」；Skyline 不执行 `this.animate`，
/// 回调也不触发，方块保持红色原文。组件内部节点从页面层查不到，只能整页比对。
static VISUAL_CASES: &[VisualCase] = &[VisualCase {
    wxml: r#"<view class="wrap">
  <anim-box/>
  <view class="hint">页面内容</view>
</view>"#,
    wxss: ".wrap { padding: 20px; }\n.hint { margin-top: 16px; font-size: 14px; color: #666666; }\n",
    js: None,
    expect: VisualExpect::Differ,
    note: "组件 ready 里的 `this.animate`：WebView 执行动画并在回调里 setData 成蓝色「已执行」，Skyline 不执行动画、回调不触发，方块保持红色原文（整页模式，证据强度弱于元素模式）",
    target: "",
}];

/// 动画载体组件：所有用例共用，动画只在 `ready` 里跑一次。
static VISUAL_COMPONENTS: &[VisualComponent] = &[VisualComponent {
    name: "anim-box",
    wxml: r#"<view class="box {{done ? 'done' : ''}}" id="box">{{done ? '已执行' : 'ANIM'}}</view>"#,
    wxss: ".box { width: 220px; height: 88px; background-color: #d33; color: #ffffff; text-align: center; line-height: 88px; font-size: 16px; }\n.box.done { background-color: #2f6fed; }\n",
    js: Some(
        "Component({\n  data: { done: false },\n  lifetimes: {\n    ready() {\n      const finish = () => this.setData({ done: true });\n      if (typeof this.animate === 'function') {\n        this.animate('#box', [\n          { opacity: 1, translateX: 0 },\n          { opacity: 0.2, translateX: 180 },\n        ], 500, finish);\n      }\n    },\n  },\n});\n",
    ),
}];

impl Entry {
    fn message(&self) -> &'static str {
        match self {
            Self::Call { message, .. } | Self::PluginCall { message, .. } => message,
        }
    }
}

pub struct UnsupportedApi;

impl UnsupportedApi {
    pub const ID: &'static str = "skyline/unsupported-api";
}

impl Rule for UnsupportedApi {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "Skyline 不支持的 JS API（如 this.animate、插件内的 wx.router.*）",
        }
    }

    fn visual_cases(&self) -> &'static [VisualCase] {
        VISUAL_CASES
    }

    fn visual_components(&self) -> &'static [VisualComponent] {
        VISUAL_COMPONENTS
    }

    fn check(&self, project: &Project) -> Vec<crate::Diagnostic> {
        let mut out = Vec::new();
        for file in &project.scripts {
            for call in file.doc.calls() {
                // 只在 WebView 分支里执行的调用：Skyline 根本不走这段，报了是误报。
                if call.webview_only {
                    continue;
                }
                for entry in ENTRIES {
                    // `this.animate` 只在 Skyline 下失效：纯 WebView 页面照常可用。
                    let span = match entry {
                        Entry::Call {
                            callee,
                            skyline_only,
                            ..
                        } => (Callee::Exact(callee).matches(&call.callee)
                            && !shadows_with_own_method(&file.doc, callee)
                            && (!skyline_only || renders_in_skyline(project, &file.scope)))
                        .then_some((call.start, call.end)),
                        Entry::PluginCall { prefix, .. } => (in_plugin(project, &file.path)
                            && Callee::Namespace(prefix).matches(&call.callee))
                        .then_some((call.start, call.end)),
                    };
                    let Some((start, end)) = span else { continue };
                    out.push(at_span(
                        Self::ID,
                        Severity::Warning,
                        file,
                        start,
                        end,
                        entry.message(),
                    ));
                }
            }
        }
        out
    }
}

/// `this.xxx(...)` 是不是调本单元自己声明的方法（`methods: { xxx() {} }`、`this.xxx = ...`）。
///
/// 内置的 `this.animate` 同名方法会被单元自己定义的同名方法遮住（组件里 `methods: { animate() {} }`
/// 写好后调 `this.animate(...)` 就是调自己那个），这种调用不受 Skyline 限制，报了是误报。
fn shadows_with_own_method(doc: &crate::input::js::JsDoc, callee: &str) -> bool {
    let Some(name) = callee.strip_prefix("this.") else {
        return false;
    };
    if name.contains('.') {
        return false;
    }
    doc.members().iter().any(|member| member.key == name)
        || doc.object_fns().iter().any(|function| function.key == name)
        || doc.this_binds().iter().any(|binding| binding.key == name)
}

#[cfg(test)]
mod tests {

    use crate::rules::Rule;
    use crate::rules::testutil::check_rule;

    use super::{UnsupportedApi, VISUAL_CASES};
    use crate::rules::testutil::js::{Fixture, messages, plugin, single, single_with_app};

    /// 视觉用例的同一份 WXML/WXSS/JS（含规则级组件夹具）必须真的能触发本规则。
    #[test]
    fn visual_case_triggers_rule() {
        let case = &VISUAL_CASES[0];
        let using = UnsupportedApi
            .visual_components()
            .iter()
            .map(|component| {
                format!(
                    r#""{}":"/components/{}/index""#,
                    component.name, component.name
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let fixture = Fixture::new("unsupported-visual");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                &format!(r#"{{"usingComponents":{{{using}}}}}"#),
            )
            .write("pages/index/index.wxml", case.wxml)
            .write("pages/index/index.wxss", case.wxss)
            .write("pages/index/index.js", case.js.unwrap_or("Page({})"));
        for component in UnsupportedApi.visual_components() {
            let dir = format!("components/{}", component.name);
            fixture
                .write(&format!("{dir}/index.wxml"), component.wxml)
                .write(&format!("{dir}/index.wxss"), component.wxss)
                .write(
                    &format!("{dir}/index.js"),
                    component.js.unwrap_or("Component({})"),
                )
                .write(&format!("{dir}/index.json"), r#"{"component":true}"#);
        }
        let diagnostics = check_rule(&fixture, &UnsupportedApi);
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.rule == UnsupportedApi::ID),
            "视觉用例未触发规则：{diagnostics:#?}"
        );
    }

    /// 单页工程 + 一个额外的脚本文件（用于验证全域扫描的边界）。
    fn with_script(name: &str, rel: &str, js: &str) -> (Fixture, crate::project::Project) {
        let f = Fixture::new(name);
        f.write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", "<view/>")
            .write("pages/index/index.js", "Page({});")
            .write(rel, js);
        let project = f.project();
        (f, project)
    }

    /// 插件工程的 `miniprogramRoot` 是宿主示例小程序，宿主代码用 `wx.router` 没问题。
    #[test]
    fn host_pages_in_a_plugin_project_are_not_restricted() {
        let f = Fixture::new("unsupported-plugin-host");
        f.write(
            "project.config.json",
            r#"{"miniprogramRoot":"dist/miniprogram/","pluginRoot":"dist/plugin/","compileType":"plugin"}"#,
        )
        .write(
            "dist/miniprogram/app.json",
            r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
        )
        .write("dist/miniprogram/pages/index/index.json", "{}")
        .write("dist/miniprogram/pages/index/index.wxml", "<view/>")
        .write(
            "dist/miniprogram/pages/index/index.js",
            "Page({ go() { wx.router.navigateTo({ url: '/x' }); } });",
        );
        assert!(UnsupportedApi.check(&f.project()).is_empty());
    }

    /// 没有单元归属的脚本（`utils/`、`app.js`）静态判断不了会不会跑在 Skyline 下，
    /// 按「不是」处理：`this.animate` 在 WebView 里是好的，报了就是误报。
    #[test]
    fn scripts_without_a_unit_are_not_judged() {
        let (_f, project) = with_script(
            "unsupported-loose-util",
            "utils/helper.ts",
            "export const jump = this.animate('.x', [], 100);",
        );
        assert!(UnsupportedApi.check(&project).is_empty());
    }

    #[test]
    fn type_declarations_are_not_scanned() {
        let (_f, project) = with_script(
            "unsupported-dts",
            "utils/types.d.ts",
            "export declare const jump: () => this.animate('.x', [], 100);\n",
        );
        assert_eq!(messages(&UnsupportedApi, &project), Vec::<String>::new());
    }

    #[test]
    fn npm_build_output_is_not_scanned() {
        let (_f, project) = with_script(
            "unsupported-npm",
            "miniprogram_npm/foo/index.js",
            "this.animate('.x', [], 100);",
        );
        assert_eq!(messages(&UnsupportedApi, &project), Vec::<String>::new());
    }

    #[test]
    fn unsupported_calls_are_reported() {
        let (_f, project) = single(
            "unsupported-positive",
            "this.animate('.a', [], 1000);\nPage({\n  data: {},\n});\nthis.animate('.b', [], 200);",
        );
        let diagnostics = UnsupportedApi.check(&project);
        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
        assert!(
            diagnostics
                .iter()
                .all(|d| d.severity == crate::Severity::Warning)
        );
        assert_eq!(diagnostics[0].path, "pages/index/index.js");
        assert!(diagnostics[0].message.contains("this.animate"));
        assert_eq!(diagnostics[1].message, diagnostics[0].message);
    }

    /// `onPageScroll` 不是本规则的职责：页面级滚动的整体缺失由 `page-scroll-api` 报。
    #[test]
    fn page_on_page_scroll_is_not_this_rules_business() {
        let (_f, project) = single("unsupported-page-scroll", "Page({ onPageScroll() {} });");
        assert_eq!(messages(&UnsupportedApi, &project), Vec::<String>::new());
    }

    /// `wx.pageScrollTo` 已迁到 `page-scroll-api`：这里不能再报，否则同一个 API 双报。
    #[test]
    fn page_scroll_to_belongs_to_page_scroll_api() {
        let (_f, project) = single(
            "unsupported-page-scroll-to",
            "Page({ onReady() { wx.pageScrollTo({ scrollTop: 400 }); } });",
        );
        assert_eq!(messages(&UnsupportedApi, &project), Vec::<String>::new());
    }

    /// `app.js` 没有单元归属，不判；页面脚本里的调用照报。
    #[test]
    fn app_js_is_not_judged_but_page_scripts_are() {
        let (_f, project) = single_with_app(
            "unsupported-app",
            "this.animate('.b', [], 1);",
            Some("this.animate('.a', [], 1);"),
        );
        let diagnostics = UnsupportedApi.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].path, "pages/index/index.js");
    }

    /// 单元自己定义了同名方法（`methods: { animate() {} }`）时，`this.animate(...)` 调的是它，
    /// 不是内置动画 API，报出来是误报。
    #[test]
    fn own_method_with_the_same_name_shadows_the_builtin() {
        let f = Fixture::new("unsupported-shadowed");
        f.write(
            "project.config.json",
            r#"{"miniprogramRoot":"","compileType":"miniprogram"}"#,
        )
        .write(
            "app.json",
            r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
        )
        .write("pages/index/index.json", "{}")
        .write("pages/index/index.wxml", "<view/>")
        .write(
            "pages/index/index.js",
            r#"Component({ methods: { animate(name) { this[name](); }, open() { this.animate('showIn'); } } });"#,
        );
        assert!(UnsupportedApi.check(&f.project()).is_empty());
    }

    /// 老写法把方法直接挂在 `Page` / `Component` 选项对象上，同样遮住同名内置 API。
    #[test]
    fn top_level_methods_shadow_the_builtin() {
        for js in [
            "Page({ animate(name) {}, open() { this.animate('showIn'); } });",
            "Component({ animate(name) {}, open() { this.animate('showIn'); } });",
        ] {
            let (_f, project) = single("unsupported-shadowed-top-level", js);
            assert_eq!(
                messages(&UnsupportedApi, &project),
                Vec::<String>::new(),
                "{js}"
            );
        }
    }

    /// `key: function () {}` 与对象方法简写是同一种方法表，两种都要认。
    #[test]
    fn function_property_methods_shadow_the_builtin() {
        let (_f, project) = single(
            "unsupported-shadowed-fn-property",
            "Component({ methods: { animate: function (name) { this[name](); }, open: function () { this.animate('showIn'); } } });",
        );
        assert_eq!(messages(&UnsupportedApi, &project), Vec::<String>::new());
    }

    /// 运行时挂到 `this` 上的同名方法同样遮住内置 API。
    #[test]
    fn this_binding_shadows_the_builtin() {
        let (_f, project) = single(
            "unsupported-shadowed-this-bind",
            "Page({ onLoad() { this.animate = function (name) { this[name](); }; this.animate('showIn'); } });",
        );
        assert_eq!(messages(&UnsupportedApi, &project), Vec::<String>::new());
    }

    /// 只在 WebView 分支里调的 API 不报：`renderer` 直接比较的 then / else 都要认。
    #[test]
    fn webview_only_branches_are_skipped() {
        for js in [
            "Page({ open() { if (this.renderer === 'webview') { this.animate('showIn'); } } });",
            "Page({ open() { if (getApp().renderer !== 'skyline') { this.animate('showIn'); } } });",
            "Page({ open() { if (this.renderer === 'skyline') { return 1; } else { this.animate('showIn'); } } });",
        ] {
            let (_f, project) = single("unsupported-webview-branch", js);
            assert_eq!(
                messages(&UnsupportedApi, &project),
                Vec::<String>::new(),
                "{js}"
            );
        }
    }

    /// 反过来：Skyline 分支里的调用仍然要报。
    #[test]
    fn skyline_branch_is_still_reported() {
        let (_f, project) = single(
            "unsupported-skyline-branch",
            "Page({ open() { if (this.renderer === 'skyline') { this.animate('showIn'); } } });",
        );
        assert_eq!(messages(&UnsupportedApi, &project).len(), 1);
    }

    /// 渲染器比较先存进本地常量、再用常量做条件时，同样能辨认 WebView 分支。
    #[test]
    fn renderer_aliases_guard_webview_branches() {
        for js in [
            "Page({ open() { const isSkyline = this.renderer === 'skyline'; if (isSkyline) {} else { this.animate('showIn'); } } });",
            "Page({ open() { const isWebview = app.renderer === 'webview'; if (isWebview) { this.animate('showIn'); } } });",
            "Page({ open() { const isSkyline = this.renderer === 'skyline'; if (!isSkyline) { this.animate('showIn'); } } });",
        ] {
            let (_f, project) = single("unsupported-webview-alias", js);
            assert_eq!(
                messages(&UnsupportedApi, &project),
                Vec::<String>::new(),
                "{js}"
            );
        }
    }

    /// 别名落在 Skyline 分支上、或初始化值判不出来时不能当守卫，照常报。
    #[test]
    fn aliases_without_a_webview_branch_are_still_reported() {
        for js in [
            "Page({ open() { const isSkyline = this.renderer === 'skyline'; if (isSkyline) { this.animate('showIn'); } } });",
            "Page({ open() { const flag = foo(); if (flag) { this.animate('showIn'); } } });",
        ] {
            let (_f, project) = single("unsupported-alias-not-webview", js);
            assert_eq!(messages(&UnsupportedApi, &project).len(), 1, "{js}");
        }
    }

    /// 别名只在定义它的函数作用域内有效：另一个函数里的同名条件不沿用。
    #[test]
    fn renderer_alias_does_not_cross_function_scopes() {
        let (_f, project) = single(
            "unsupported-alias-scope",
            "Page({ a() { const isSkyline = this.renderer === 'skyline'; }, b() { if (isSkyline) { this.animate('showIn'); } } });",
        );
        assert_eq!(messages(&UnsupportedApi, &project).len(), 1);
    }

    /// `Behavior` 的方法表与 `Page` / `Component` 同形。
    #[test]
    fn behavior_methods_shadow_the_builtin() {
        let (_f, project) = single(
            "unsupported-shadowed-behavior",
            "Behavior({ methods: { animate(name) { this[name](); }, open() { this.animate('showIn'); } } });",
        );
        assert_eq!(messages(&UnsupportedApi, &project), Vec::<String>::new());
    }

    /// 同名方法定义在别的文件里不算遮住：过滤只看同一文件。
    #[test]
    fn method_in_another_file_does_not_shadow() {
        let f = Fixture::new("unsupported-shadowed-cross-file");
        f.write(
            "project.config.json",
            r#"{"miniprogramRoot":"","compileType":"miniprogram"}"#,
        )
        .write(
            "app.json",
            r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
        )
        .write("pages/index/index.json", "{}")
        .write("pages/index/index.wxml", "<view/>")
        .write(
            "pages/index/index.js",
            "Page({ open() { this.animate('showIn'); } });",
        )
        .write(
            "pages/other/other.js",
            "Component({ methods: { animate() {} } });",
        );
        let diagnostics = UnsupportedApi.check(&f.project());
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].path, "pages/index/index.js");
    }

    /// 纯 WebView 页面照常支持 `this.animate`，不能报。
    #[test]
    fn webview_page_is_not_reported() {
        let f = Fixture::new("unsupported-webview");
        f.write(
            "project.config.json",
            r#"{"miniprogramRoot":"","compileType":"miniprogram"}"#,
        )
        .write(
            "app.json",
            r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
        )
        .write("pages/index/index.json", r#"{"renderer":"webview"}"#)
        .write("pages/index/index.wxml", "<web-view/>")
        .write(
            "pages/index/index.js",
            "Page({ onLoad() { this.animate('.a', [], 1); } });",
        );
        assert!(UnsupportedApi.check(&f.project()).is_empty());
    }

    #[test]
    fn non_calls_and_non_page_options_are_ignored() {
        let (_f, project) = single(
            "unsupported-negative",
            r#"
            // this.animate('.a', [], 1);
            const text = 'this.animate';
            page.onPageScroll = () => {};
            wx.Page({ onPageScroll() {} });
            this.animateLater('.b', [], 1);
            Page({ onLoad() {} });
            "#,
        );
        assert_eq!(messages(&UnsupportedApi, &project), Vec::<String>::new());
    }

    #[test]
    fn non_literal_page_argument_is_ignored() {
        let (_f, project) = single("unsupported-dynamic", "Page(config);");
        assert_eq!(messages(&UnsupportedApi, &project), Vec::<String>::new());
    }

    #[test]
    fn router_calls_are_reported_only_in_plugin_projects() {
        let (_f, plugin_project) = plugin(
            "unsupported-plugin-router",
            "wx.router.navigateTo({ url: 'x' });\nwx.router.addRouteBuilder('a', b);",
        );
        let diagnostics = UnsupportedApi.check(&plugin_project);
        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
        assert!(diagnostics.iter().all(|d| d.message.contains("wx.router")));

        let (_n, normal) = single(
            "unsupported-normal-router",
            "wx.router.navigateTo({ url: 'x' });",
        );
        assert_eq!(messages(&UnsupportedApi, &normal), Vec::<String>::new());
    }

    #[test]
    fn plugin_prefix_does_not_match_similar_names() {
        let (_f, project) = plugin(
            "unsupported-plugin-prefix",
            "wx.routers.navigateTo({});\nmywx.router.x();\nwx.routerX.y();",
        );
        assert_eq!(messages(&UnsupportedApi, &project), Vec::<String>::new());
    }

    /// Skyline 下这三个 context 都要求把组件实例作为第二个参数，否则拿不到 nodeId。
    #[test]
    fn context_apis_in_skyline_are_reported() {
        let (_f, project) = single(
            "unsupported-context-skyline",
            "Page({ a() {
                wx.createVideoContext('v', this);
                wx.createMapContext('m', this);
                wx.createLivePlayerContext('l', this);
            } });",
        );
        let messages = messages(&UnsupportedApi, &project);
        assert_eq!(messages.len(), 3, "{messages:#?}");
        assert!(messages.iter().all(|m| {
            m.contains("必须把组件实例作为第二个参数传入")
                && m.contains("省略 `this` 时拿不到 nodeId")
                && m.contains("可能定位不到节点")
                && m.contains("createSelectorQuery")
        }));
    }

    /// 静态判断不了省略 `this` 的调用，两种写法都报；文案要覆盖省略的情形。
    #[test]
    fn context_apis_report_with_and_without_component_instance() {
        let (_f, project) = single(
            "unsupported-context-arity",
            "Page({ a() {
                wx.createVideoContext('v');
                wx.createVideoContext('v2', this);
            } });",
        );
        let messages = messages(&UnsupportedApi, &project);
        assert_eq!(messages.len(), 2, "{messages:#?}");
        assert_eq!(messages[0], messages[1]);
        assert!(messages[0].contains("省略 `this` 时拿不到 nodeId"));
    }

    /// 当前 SDK 没有 `wx.createEditorContext`，不再报。
    #[test]
    fn editor_context_is_not_reported() {
        let (_f, project) = single(
            "unsupported-context-editor",
            "Page({ a() { wx.createEditorContext('e'); } });",
        );
        assert_eq!(messages(&UnsupportedApi, &project), Vec::<String>::new());
    }

    /// WebView 页面里这些 context 照样能用，不能报。
    #[test]
    fn context_apis_in_webview_page_are_not_reported() {
        let f = Fixture::new("unsupported-context-webview");
        f.write(
            "project.config.json",
            r#"{"miniprogramRoot":"","compileType":"miniprogram"}"#,
        )
        .write(
            "app.json",
            r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
        )
        .write("pages/index/index.json", r#"{"renderer":"webview"}"#)
        .write("pages/index/index.wxml", "<view/>")
        .write(
            "pages/index/index.js",
            "Page({ a() { wx.createVideoContext('v', this); } });",
        );
        assert!(UnsupportedApi.check(&f.project()).is_empty());
    }

    /// 无 selectorQuery 等价物的那几个：不维护了，同样只提“已不再维护”。
    #[test]
    fn deprecated_context_apis_are_reported_in_skyline() {
        let (_f, project) = single(
            "unsupported-context-deprecated",
            "Page({ a() {
                wx.createCameraContext('c', this);
                wx.createLivePusherContext('p', this);
                wx.createAudioContext('a');
            } });",
        );
        let messages = messages(&UnsupportedApi, &project);
        assert_eq!(messages.len(), 3, "{messages:#?}");
        assert!(messages.iter().all(|m| m.contains("已不再维护")));
    }
}
