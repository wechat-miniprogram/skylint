//! Skyline 下没有可用实现的组件：未注册的标签，与已注册但进了黑名单的平台组件。
//!
//! 两者共用一个判定入口——「这个名字在 Skyline 下解析不到可用实现」——但后果分档：
//!
//! - 未注册标签，三种渲染器各说各的：
//!   - WebView（glass-easel）：退化为原生元素，基础样式里没有它的规则，按 CSS 初始值
//!     `display: inline` 排版——盒子收缩到内容宽度，`height` 等块级属性被忽略（实测 32×22 CSS）；
//!   - WebView（旧 exparser）：退化为无模板的默认组件，没有 `<slot>` 可投影，子树不显示；
//!   - Skyline：退化为引擎的通用元素，默认 `display: flex` + `column` + `stretch`，
//!     即块级 flex 容器——宽度撑满、`height` 生效（实测 355×40 CSS）。
//! - 已注册但 Skyline 不支持的组件：按 [`tags`] 里的组件表分三档——完全没有实现、
//!   盒子还在但原生绘制丢失、只能整页保持 WebView。
//!
//! 声明集合统一用 [`Project::visible_declared_components`]（含模板作用域并集）：只在
//! `usingComponents` 里写了同名标签就说明用的是自定义组件，两档后果都不该报。

use std::collections::BTreeSet;

use crate::diag::{Diagnostic, Severity};
use crate::project::Project;
use crate::rules::{Rule, RuleMeta, VisualCase, VisualExpect};

use super::tags::{Blocked, Loss, blocked};
use super::{element_diagnostic, for_each_element, is_builtin};

pub struct ComponentSupport;

/// 一个标签在 Skyline 下解析不到可用实现。
enum Resolution {
    /// 不是内置组件，`usingComponents` 里也没有声明。
    Unregistered,
    /// 是平台内置组件，但 Skyline 黑名单里有它。
    Blocked(&'static Blocked),
}

impl ComponentSupport {
    pub const ID: &'static str = "skyline/component-support";

    /// WXML 语法构造不是组件。`template` / `import` / `include` / `wxs` / `block` 由
    /// 解析器摊平，本来就不作为元素出现；`<slot>` 会作为元素出现，但它不是组件。
    const NOT_COMPONENTS: &[&str] = &["template", "import", "include", "wxs", "slot", "block"];

    const UNREGISTERED_HELP: &str = "在页面或组件的 usingComponents 里注册该组件；若本来就不该渲染，改成真实组件或加 wx:if。两端都解析不到这个标签：WebView 退化为行内元素（旧组件框架下子树消失），Skyline 退化为块级 flex 容器";

    /// 唯一的判定入口：声明优先（声明了同名标签就是自定义组件），再看内置表。
    fn resolve(tag: &str, declared: &BTreeSet<String>) -> Option<Resolution> {
        if declared.contains(tag) {
            return None;
        }
        if let Some(row) = blocked(tag) {
            return Some(Resolution::Blocked(row));
        }
        if is_builtin(tag) {
            return None;
        }
        Some(Resolution::Unregistered)
    }

    /// 黑名单组件的后果档位；未注册标签不用它（后果由三种渲染器各自决定）。
    fn consequence(loss: Loss) -> &'static str {
        match loss {
            Loss::Unavailable => "该组件在 Skyline 下没有实现，盒子与内容都不出现",
            Loss::PaintingLost => "盒子与 CSS 仍在，但原生绘制丢失",
            Loss::PageOptOut => "它只能在 WebView 渲染器下工作，这一页要整页保持 WebView",
        }
    }
}

/// 视觉用例：同一段 WXML/WXSS 两端各渲一次。
///
/// 第一条是未注册标签：`my-modal` 未在 `usingComponents` 里注册，WebView 把它当行内元素，
/// 背景只包住「内容」（32×22 CSS，`height` 被忽略）；Skyline 当普通块级元素，背景铺满
/// 355×40 CSS，后面的兄弟被推下 18 CSS。
///
/// 第二条是黑名单组件：`<progress>` 在 Skyline 下盒子仍在（`height` / `background` 生效），
/// 占位高度与 WebView 一致，只是不画那层原生轨道（绿色填充 + 灰轨）。
static VISUAL_CASES: &[VisualCase] = &[
    VisualCase {
        wxml: r#"<view class="wrap"><view class="head">标题</view><my-modal class="ghost">内容</my-modal><view class="tail" id="target">尾部</view></view>"#,
        wxss: ".wrap { padding: 10px; }\n.ghost { height: 40px; background: #cde; }\n.tail { height: 20px; background: #edc; }\n",
        js: None,
        expect: VisualExpect::Differ,
        note: "未注册的 <my-modal>：WebView 按行内元素收缩到内容尺寸（背景 32×22，`height` 被忽略）；Skyline 按块级元素铺满 355×40，尾部只差 18px——差异主体是这块 40px 高的背景（差异包络 355×60 CSS，背景约 68%、尾部 18px 位移约 32%）。整页模式，证据强度弱于元素模式（无 bbox 兜底）",
        target: "",
    },
    VisualCase {
        wxml: r#"<view class="wrap"><view class="head">标题</view><progress class="bar" id="target" percent="60"/><view class="tail">尾部</view></view>"#,
        wxss: ".wrap { padding: 10px; }\n.bar { height: 20px; background: #cde; }\n.tail { height: 20px; background: #edc; }\n",
        js: None,
        expect: VisualExpect::Differ,
        note: "两端的 <progress> 都占 20px 高度、尾部位置一致；差异是整条 355×6 CSS 的原生轨道：WebView 在 CSS 底色上叠了一层原生绘制——前 213 CSS（60% 宽）是绿色填充（#09bb07，仅 6 CSS 高），其余 142 CSS 是 #ebebeb 灰轨道；Skyline 只保留 CSS 底色，这层原生绘制整体缺失",
        target: "#target",
    },
];

impl Rule for ComponentSupport {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Error,
            description: "Skyline 下没有可用实现的组件：未注册标签按行内/无模板/块级 flex 三种退化方式渲染，黑名单组件按不可用/绘制丢失/整页 opt-out 分档",
        }
    }

    fn visual_cases(&self) -> &'static [VisualCase] {
        VISUAL_CASES
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        // 声明按文档缓存：同一份 `usingComponents` 不必每个元素重算。
        let mut declared = BTreeSet::new();
        let mut current: Option<String> = None;
        for_each_element(project, self.target(), |site| {
            let (doc, element) = (site.doc, site.element);
            if current.as_deref() != Some(doc.path()) {
                declared = project.visible_declared_components(doc.path());
                current = Some(doc.path().to_string());
            }
            let tag = element.tag.to_ascii_lowercase();
            if Self::NOT_COMPONENTS.contains(&tag.as_str()) {
                return;
            }
            let Some(resolution) = Self::resolve(&tag, &declared) else {
                return;
            };
            let (message, help) = match resolution {
                Resolution::Unregistered => (
                    format!(
                        "未注册标签 `<{}>`：glass-easel 当未知行内元素（收缩到内容宽度、`height` 等块级属性被忽略），旧 exparser 当无模板默认组件（子树整块不显示），Skyline 当块级 flex 容器（宽度撑满、`height` 生效）——两端布局不同",
                        element.tag
                    ),
                    Self::UNREGISTERED_HELP,
                ),
                Resolution::Blocked(row) => (
                    format!(
                        "Skyline 不支持组件 `<{}>`：{}",
                        element.tag,
                        Self::consequence(row.loss)
                    ),
                    row.advice,
                ),
            };
            out.push(
                element_diagnostic(Self::ID, doc, element, doc.path(), Severity::Error, message)
                    .help(help),
            );
        });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::{ComponentSupport, VISUAL_CASES};
    use crate::rules::testutil::wxml::{Fixture, check_rule, project};

    #[test]
    fn unregistered_tag_is_reported_with_all_three_renderer_branches() {
        let diagnostics = check_rule(
            &project("unregistered", r#"<view><modal/></view>"#),
            &ComponentSupport,
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        let diagnostic = &diagnostics[0];
        assert_eq!(diagnostic.rule, ComponentSupport::ID);
        assert_eq!(diagnostic.severity, crate::Severity::Error);
        assert!(diagnostic.message.contains("<modal>"));
        // 三种渲染器的后果都要在文案里，不能只讲一端。
        for branch in ["glass-easel", "exparser", "Skyline"] {
            assert!(
                diagnostic.message.contains(branch),
                "缺少 {branch} 分支：{}",
                diagnostic.message
            );
        }
        assert!(
            diagnostic
                .help
                .as_deref()
                .unwrap()
                .contains("usingComponents")
        );
    }

    #[test]
    fn declared_component_is_not_reported() {
        let fixture = Fixture::new("declared");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                r#"{"usingComponents":{"my-card":"/components/card/card"}}"#,
            )
            .write("pages/index/index.wxml", r#"<my-card/>"#)
            .write("components/card/card.json", r#"{"component":true}"#)
            .write("components/card/card.wxml", "<view/>");
        let diagnostics = check_rule(&fixture, &ComponentSupport);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    /// app 级声明同样覆盖页面。
    #[test]
    fn app_level_declaration_is_not_reported() {
        let fixture = Fixture::new("app-declared");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline","usingComponents":{"my-card":"/components/card/card"}}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", r#"<my-card/>"#)
            .write("components/card/card.json", r#"{"component":true}"#)
            .write("components/card/card.wxml", "<view/>");
        let diagnostics = check_rule(&fixture, &ComponentSupport);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    /// 声明了同名标签时用的是自定义组件，黑名单里的同名平台组件不参与判定。
    #[test]
    fn declared_custom_component_shadows_the_builtin() {
        let fixture = Fixture::new("declared-shadow");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                r#"{"usingComponents":{"progress":"/components/bar/bar"}}"#,
            )
            .write("pages/index/index.wxml", r#"<progress/>"#)
            .write("components/bar/bar.json", "{}")
            .write("components/bar/bar.wxml", r#"<view/>"#);
        let diagnostics = check_rule(&fixture, &ComponentSupport);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    /// 回归：`<import>` 的模板里用到黑名单同名组件，声明写在引用它的页面 json 里。
    /// 旧实现只在模板文件自己的 `declared_components` 里找（模板没有 json，恒为空），
    /// 于是把「引用方声明过的自定义组件」误报成 Skyline 不支持。
    #[test]
    fn imported_template_uses_the_referencing_unit_declarations() {
        let fixture = Fixture::new("imported-declared");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                r#"{"usingComponents":{"progress":"/components/bar/bar"}}"#,
            )
            .write(
                "pages/index/index.wxml",
                r#"<import src="../../templates/bar"/><template is="bar"/>"#,
            )
            .write(
                "templates/bar.wxml",
                r#"<template name="bar"><progress percent="60"/></template>"#,
            )
            .write("components/bar/bar.json", r#"{"component":true}"#)
            .write("components/bar/bar.wxml", r#"<view/>"#);
        let diagnostics = check_rule(&fixture, &ComponentSupport);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    /// 同一份夹具去掉声明后必须报：证明上一条的空结果不是「模板压根没被扫到」。
    #[test]
    fn imported_template_without_declaration_is_reported() {
        let fixture = Fixture::new("imported-undeclared");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write(
                "pages/index/index.wxml",
                r#"<import src="../../templates/bar"/><template is="bar"/>"#,
            )
            .write(
                "templates/bar.wxml",
                r#"<template name="bar"><progress percent="60"/></template>"#,
            );
        let diagnostics = check_rule(&fixture, &ComponentSupport);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].path, "templates/bar.wxml");
    }

    /// 同一机制的另一面：模板里用未注册标签时，声明也取引用方单元的并集。
    #[test]
    fn template_fragment_uses_the_referencing_unit_declarations() {
        let fixture = Fixture::new("template-declared");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                r#"{"usingComponents":{"my-card":"/components/card/card"}}"#,
            )
            .write(
                "pages/index/index.wxml",
                r#"<import src="../../templates/card"/><template is="card"/>"#,
            )
            .write(
                "templates/card.wxml",
                r#"<template name="card"><my-card/></template>"#,
            )
            .write("components/card/card.json", r#"{"component":true}"#)
            .write("components/card/card.wxml", "<view/>");
        let diagnostics = check_rule(&fixture, &ComponentSupport);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    /// 模板里真没声明的标签照样报。
    #[test]
    fn unregistered_tag_inside_template_is_reported() {
        let fixture = Fixture::new("template-unregistered");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write(
                "pages/index/index.wxml",
                r#"<import src="../../templates/card"/><template is="card"/>"#,
            )
            .write(
                "templates/card.wxml",
                r#"<template name="card"><modal/></template>"#,
            );
        let diagnostics = check_rule(&fixture, &ComponentSupport);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].path, "templates/card.wxml");
    }

    /// 黑名单组件的三档后果：文案要说清这一档，advice 也要对得上。
    #[test]
    fn blacklisted_components_report_their_consequence() {
        let diagnostics = check_rule(
            &project(
                "blacklist-tiers",
                r#"<view><editor/><progress/><web-view src="https://x"/></view>"#,
            ),
            &ComponentSupport,
        );
        assert_eq!(diagnostics.len(), 3, "{diagnostics:?}");

        let tier = |tag: &str| {
            diagnostics
                .iter()
                .find(|diagnostic| diagnostic.message.contains(tag))
                .unwrap_or_else(|| panic!("没有报 {tag}：{diagnostics:#?}"))
        };

        let editor = tier("<editor>");
        assert!(
            editor.message.contains("盒子与内容都不出现"),
            "{}",
            editor.message
        );

        let progress = tier("<progress>");
        assert!(
            progress.message.contains("原生绘制丢失"),
            "{}",
            progress.message
        );
        assert!(
            progress.help.as_deref().unwrap().contains("自绘进度条"),
            "{:?}",
            progress.help
        );

        let web_view = tier("<web-view>");
        assert!(
            web_view.message.contains("整页保持 WebView"),
            "{}",
            web_view.message
        );
        assert!(
            web_view
                .help
                .as_deref()
                .unwrap()
                .contains(r#""renderer": "webview""#)
        );
    }

    /// 黑名单组件同样按标签名大小写无关判定。
    #[test]
    fn blacklisted_tag_is_matched_case_insensitively() {
        let diagnostics = check_rule(&project("blacklist-case", "<Web-View/>"), &ComponentSupport);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    }

    /// 知识库把组件名拼成 `keyboard-accessary`；官方名是 keyboard-accessory，
    /// 按错拼收录等于这条永远命中不了。
    #[test]
    fn keyboard_accessory_is_reported_under_its_official_name() {
        let diagnostics = check_rule(
            &project("keyboard", r#"<keyboard-accessory/>"#),
            &ComponentSupport,
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(diagnostics[0].message.contains("keyboard-accessory"));
    }

    /// 内置但受支持的组件一个都不报。
    #[test]
    fn supported_components_are_allowed() {
        let diagnostics = check_rule(
            &project(
                "supported",
                "<view><form/><scroll-view/><navigator/><rich-text/><page-meta/><span/><sticky-header/><list-view/><root-portal/><sticky-section/></view>",
            ),
            &ComponentSupport,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// 语法构造不是组件：解析器摊平 `block` / `template` / `wxs` / `import` / `include`，
    /// `<slot>` 虽作为元素出现，也不是组件。
    #[test]
    fn wxml_constructs_are_not_reported() {
        let diagnostics = check_rule(
            &project(
                "constructs",
                r#"<wxs module="m">var a = 1</wxs>
<import src="./tpl"/>
<include src="./frag"/>
<template name="card"><view/></template>
<template is="card"/>
<block wx:for="{{list}}"><view>{{item}}</view></block>
<slot/>
<slot name="header"/>"#,
            ),
            &ComponentSupport,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// 页面 json 显式 `"renderer": "webview"`：这页不迁 Skyline，整页跳过。
    #[test]
    fn explicit_webview_page_is_skipped() {
        let fixture = Fixture::new("opt-out");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index","pages/webview/webview"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", "<view>skyline</view>")
            .write("pages/webview/webview.json", r#"{"renderer":"webview"}"#)
            .write(
                "pages/webview/webview.wxml",
                r#"<modal/><web-view src="https://x"/>"#,
            );
        let diagnostics = check_rule(&fixture, &ComponentSupport);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    /// app 级 webview、页面自己没表态：只是还没迁，仍然检查。
    #[test]
    fn inherited_webview_page_is_still_checked() {
        let fixture = Fixture::new("not-migrated");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"webview"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", r#"<web-view src="https://x"/>"#);
        let diagnostics = check_rule(&fixture, &ComponentSupport);
        assert_eq!(diagnostics.len(), 1);
    }

    /// 跳过是按页的：同一工程里没表态的页面照样检查。
    #[test]
    fn opt_out_does_not_leak_to_sibling_pages() {
        let fixture = Fixture::new("opt-out-sibling");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/a/a","pages/b/b"],"renderer":"skyline"}"#,
            )
            .write("pages/a/a.json", r#"{"renderer":"webview"}"#)
            .write("pages/a/a.wxml", r#"<web-view src="https://a"/>"#)
            .write("pages/b/b.json", "{}")
            .write("pages/b/b.wxml", r#"<web-view src="https://b"/>"#);
        let diagnostics = check_rule(&fixture, &ComponentSupport);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].path, "pages/b/b.wxml");
    }

    /// 内联 `<template name>` 的定义体同样要查：它是文件里真实存在的标签。
    #[test]
    fn component_inside_inline_template_is_reported() {
        let fixture = Fixture::new("inline-template");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write(
                "pages/index/index.wxml",
                r#"<template name="card"><web-view src="https://x"/></template><template is="card"/>"#,
            );
        let diagnostics = check_rule(&fixture, &ComponentSupport);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(diagnostics[0].message.contains("web-view"));
    }

    /// `<import>` 引入的模板不属于任何单元，但结构规则仍要检查它。
    #[test]
    fn imported_template_is_scanned() {
        let fixture = Fixture::new("imported-template");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write(
                "pages/index/index.wxml",
                r#"<import src="../../templates/pagination" /><view/>"#,
            )
            .write(
                "templates/pagination.wxml",
                r#"<web-view src="https://x"/>"#,
            );
        let diagnostics = check_rule(&fixture, &ComponentSupport);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].path, "templates/pagination.wxml");
    }

    /// `<include>` 同样覆盖；模板 import 模板（A→B→C）要递归到最里层。
    #[test]
    fn included_templates_are_scanned_recursively() {
        let fixture = Fixture::new("included-template");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write(
                "pages/index/index.wxml",
                r#"<include src="../../templates/a"/>"#,
            )
            .write("templates/a.wxml", r#"<include src="b"/>"#)
            .write("templates/b.wxml", r#"<import src="./c.wxml"/>"#)
            .write("templates/c.wxml", r#"<web-view src="https://x"/>"#);
        let diagnostics = check_rule(&fixture, &ComponentSupport);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].path, "templates/c.wxml");
    }

    /// 同一模板被多个页面引用时只扫一次。
    #[test]
    fn template_shared_by_two_pages_is_scanned_once() {
        let fixture = Fixture::new("shared-template");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/a/a","pages/b/b"],"renderer":"skyline"}"#,
            )
            .write("pages/a/a.json", "{}")
            .write("pages/a/a.wxml", r#"<import src="../../tpl/x"/>"#)
            .write("pages/b/b.json", "{}")
            .write("pages/b/b.wxml", r#"<import src="../../tpl/x"/>"#)
            .write("tpl/x.wxml", r#"<web-view src="https://x"/>"#);
        let diagnostics = check_rule(&fixture, &ComponentSupport);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    }

    #[test]
    fn every_occurrence_is_reported() {
        let diagnostics = check_rule(
            &project(
                "twice",
                "<view><modal/><modal/><web-view/><web-view/></view>",
            ),
            &ComponentSupport,
        );
        assert_eq!(diagnostics.len(), 4, "{diagnostics:?}");
    }

    /// 标签名大小写不影响判定。
    #[test]
    fn tag_names_are_matched_case_insensitively() {
        let diagnostics = check_rule(&project("case", "<Page-Meta/>"), &ComponentSupport);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    /// 视觉用例的同一份 WXML/WXSS 必须真的能触发本规则（两条都要）。
    #[test]
    fn visual_cases_trigger_rule() {
        for case in VISUAL_CASES {
            let fixture = Fixture::page("component-support-visual", case.wxml, case.wxss);
            let diagnostics = check_rule(&fixture, &ComponentSupport);
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.rule == ComponentSupport::ID),
                "视觉用例未触发规则：{}",
                case.note
            );
        }
    }
}
