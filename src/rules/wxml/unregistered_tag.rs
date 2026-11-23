//! WXML 里既不是内置组件、也没有在 `usingComponents` 里声明的标签。
//!
//! 两端都不会把它解析成组件，兜底结果取决于 WebView 的组件框架：
//!
//! - WebView（glass-easel）：退化为原生元素，基础样式里没有它的规则，按 CSS 初始值
//!   `display: inline` 排版——盒子收缩到内容宽度，`height` 等块级属性被忽略（实测 32×22 CSS）；
//! - WebView（旧 exparser）：退化为无模板的默认组件，没有 `<slot>` 可投影，子树不显示；
//! - Skyline：同样退化为引擎的通用元素，默认 `display: flex` + `column` + `stretch`，
//!   即块级 flex 容器——宽度撑满、`height` 生效（实测 355×40 CSS）。
//!
//! 这样写出来的标签在两端都不是你想要的组件，布局不可控，还会在旧框架下整块消失。

use std::collections::BTreeSet;

use crate::diag::{Diagnostic, Severity};
use crate::project::Project;
use crate::rules::{Rule, RuleMeta, VisualCase, VisualExpect};

use super::{element_diagnostic, for_each_element, is_builtin};

pub struct UnregisteredTag;

impl UnregisteredTag {
    pub const ID: &'static str = "skyline/unregistered-tag";

    /// WXML 语法构造不是组件。`template` / `import` / `include` / `wxs` / `block` 由
    /// 解析器摊平，本来就不作为元素出现；`<slot>` 会作为元素出现，但它不是组件。
    const NOT_COMPONENTS: &[&str] = &["template", "import", "include", "wxs", "slot", "block"];

    const HELP: &str = "在页面或组件的 usingComponents 里注册该组件；若本来就不该渲染，改成真实组件或加 wx:if。两端都不会把它当成组件：WebView 退化为未知行内元素，旧组件框架下退化为无模板默认组件、子树不显示；Skyline 退化为块级 flex 容器";
}

/// 视觉用例：同一段 WXML/WXSS 两端各渲一次，整页比对。
///
/// `my-modal` 未在 `usingComponents` 里注册：WebView 把它当行内元素，背景只包住「内容」
/// （32×22 CSS，`height` 被忽略）；Skyline 当普通块级元素，背景铺满 355×40 CSS，后面的
/// 兄弟被推下 18 CSS。差异主体是这块 355×40 的背景，而不是尾部的位移。
static VISUAL_CASES: &[VisualCase] = &[VisualCase {
    wxml: r#"<view class="wrap"><view class="head">标题</view><my-modal class="ghost">内容</my-modal><view class="tail" id="target">尾部</view></view>"#,
    wxss: ".wrap { padding: 10px; }\n.ghost { height: 40px; background: #cde; }\n.tail { height: 20px; background: #edc; }\n",
    js: None,
    expect: VisualExpect::Differ,
    note: "未注册的 <my-modal>：WebView 按行内元素收缩到内容尺寸（背景 32×22，`height` 被忽略）；Skyline 按块级元素铺满 355×40，尾部只差 18px——差异主体是这块 40px 高的背景",
    target: "",
}];

impl Rule for UnregisteredTag {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Error,
            description: "未注册的标签：WebView 只当未知行内元素（块级属性被忽略；旧组件框架下退化为无模板默认组件、子树不显示），Skyline 当块级 flex 容器（宽度撑满）",
        }
    }

    fn visual_cases(&self) -> &'static [VisualCase] {
        VISUAL_CASES
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        // `declared` 按文档缓存：同一份 `usingComponents` 不必每个元素重算。
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
            // Skyline 下不可用的内置组件由 `unsupported-component` 单独报，它们仍是
            // 平台组件，不能算「未注册」。
            if is_builtin(&tag) {
                return;
            }
            if declared.contains(&tag) {
                return;
            }
            out.push(
                element_diagnostic(
                    Self::ID,
                    doc,
                    element,
                    doc.path(),
                    Severity::Error,
                    format!(
                        "未注册标签 `<{}>`：WebView 只当未知行内元素（收缩到内容宽度、`height` 等块级属性被忽略），Skyline 当块级 flex 容器（宽度撑满、`height` 生效），两端布局不同",
                        element.tag
                    ),
                )
                .help(Self::HELP),
            );
        });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::{UnregisteredTag, VISUAL_CASES};
    use crate::rules::testutil::wxml::{Fixture, check_rule, project};

    #[test]
    fn unregistered_tag_is_reported() {
        let diagnostics = check_rule(
            &project("unregistered", r#"<view><modal/></view>"#),
            &UnregisteredTag,
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].severity, crate::Severity::Error);
        assert!(diagnostics[0].message.contains("<modal>"));
        assert!(
            diagnostics[0]
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
        let diagnostics = check_rule(&fixture, &UnregisteredTag);
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
        let diagnostics = check_rule(&fixture, &UnregisteredTag);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn builtin_components_are_not_reported() {
        let diagnostics = check_rule(
            &project(
                "builtin",
                "<view><text/><scroll-view/><page-meta/><root-portal/><sticky-section/></view>",
            ),
            &UnregisteredTag,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// Skyline 下不可用的内置组件仍是平台组件，由 `unsupported-component` 报。
    #[test]
    fn unsupported_builtin_is_not_reported_as_unregistered() {
        let diagnostics = check_rule(
            &project(
                "unsupported-builtin",
                "<view><web-view/><navigation-bar/></view>",
            ),
            &UnregisteredTag,
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
            &UnregisteredTag,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// 独立模板文件里用到的组件，声明在引用它的单元 json 里。
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
        let diagnostics = check_rule(&fixture, &UnregisteredTag);
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
        let diagnostics = check_rule(&fixture, &UnregisteredTag);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].path, "templates/card.wxml");
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
            .write("pages/webview/webview.wxml", "<modal/>");
        let diagnostics = check_rule(&fixture, &UnregisteredTag);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn every_occurrence_is_reported() {
        let diagnostics = check_rule(
            &project("unregistered-twice", "<view><modal/><modal/></view>"),
            &UnregisteredTag,
        );
        assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
    }

    /// 标签名大小写不影响判定。
    #[test]
    fn tag_names_are_matched_case_insensitively() {
        let diagnostics = check_rule(&project("case", "<Page-Meta/>"), &UnregisteredTag);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    /// 视觉用例的同一份 WXML/WXSS 必须真的能触发本规则。
    #[test]
    fn visual_case_triggers_rule() {
        let case = &VISUAL_CASES[0];
        let fixture = Fixture::page("unregistered-visual", case.wxml, case.wxss);
        let diagnostics = check_rule(&fixture, &UnregisteredTag);
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.rule == UnregisteredTag::ID),
            "视觉用例未触发规则"
        );
    }
}
