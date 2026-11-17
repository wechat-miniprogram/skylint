//! WXML 里既不是内置组件、也没有在 `usingComponents` 里声明的标签。
//!
//! 两个渲染器对这类标签的处理完全不同：WebView（exparser）查不到定义时落到默认组件，
//! 默认组件没有模板、没有 `<slot>`，子节点无处投影，整块不渲染；Skyline（glass-easel）
//! 把标签名当 native node 交给渲染层，按普通元素建盒子，子节点照常渲染。迁移后原本
//! 「静默消失」的子树会突然进入布局，这是本规则成立的依据。

use std::collections::BTreeSet;

use crate::diag::{Diagnostic, Severity};
use crate::input::wxml::WxmlDoc;
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};

use super::{element_diagnostic, for_each_element, is_builtin, unsupported_replacement};

pub struct UnregisteredTag;

impl UnregisteredTag {
    pub const ID: &'static str = "skyline/unregistered-tag";

    /// WXML 语法构造不是组件。`template` / `import` / `include` / `wxs` / `block` 由
    /// 解析器摊平，本来就不作为元素出现；`<slot>` 会作为元素出现，但它不是组件。
    const NOT_COMPONENTS: &[&str] = &["template", "import", "include", "wxs", "slot", "block"];
}

impl Rule for UnregisteredTag {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Error,
            description: "未注册的标签：WebView 不渲染其子树，Skyline 按普通元素渲染",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        // `declared` 按文档缓存：同一份 `usingComponents` 不必每个元素重算。
        let mut declared = BTreeSet::new();
        let mut current: Option<String> = None;
        for_each_element(project, self.target(), |site| {
            let (doc, element) = (site.doc, site.element);
            if current.as_deref() != Some(doc.path()) {
                declared = declared_in(project, doc);
                current = Some(doc.path().to_string());
            }
            let tag = element.tag.to_ascii_lowercase();
            if Self::NOT_COMPONENTS.contains(&tag.as_str()) {
                return;
            }
            // Skyline 下不可用的内置组件由 `unsupported-component` 单独报，它们仍是
            // 平台组件，不能算「未注册」。
            if is_builtin(&tag) || unsupported_replacement(&tag).is_some() {
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
                        "未注册标签 `<{}>`：WebView 不渲染其子树，Skyline 会按普通元素渲染",
                        element.tag
                    ),
                )
                .help("在页面或组件的 usingComponents 里注册该组件；若本来就不该渲染，改成真实组件或加 wx:if"),
            );
        });
        out
    }
}

/// 这份文档可见的组件标签名。
///
/// 独立模板与 `<include>` 片段没有自己的 json，声明来自引用它的单元（模板在调用处按
/// 调用方作用域展开），取所有引用单元的并集：只要有一处能解析，就不是「未注册」。
fn declared_in(project: &Project, doc: &WxmlDoc) -> BTreeSet<String> {
    let mut declared = project.declared_components(doc.path());
    if doc.fragment {
        for scope in project
            .template_scopes
            .get(doc.path())
            .into_iter()
            .flatten()
        {
            if let Some(wxml) = project.scope_wxml_any(scope) {
                declared.extend(project.declared_components(wxml));
            }
        }
    }
    declared
}

#[cfg(test)]
mod tests {
    use super::UnregisteredTag;
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
}
