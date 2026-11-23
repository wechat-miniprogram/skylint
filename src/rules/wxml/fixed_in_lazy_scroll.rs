//! `skyline/fixed-in-lazy-scroll`：`type="list"` 的 scroll-view 惰性挂载，
//! 子树里的 `position: fixed` 会随列表项卸载而消失。
//!
//! 机制：引擎把 fixed 节点从原位换成 replacer、挂到页面根（`shadow_tree.cc`
//! `UpdatePositionFixedNodes`），所以「fixed 套在滚动容器里」本身不失效。但
//! `type="list"` 的 scroll-view 在 `Inflate()` 里 `SetLazyMount()`，视口外的列表项
//! 根本不会挂成 shadow 节点——那一轮 BFS 收不到里面的 fixed 元素，也就不会被提根。
//! 表现是「首屏吸顶正常，一滚动整条消失」，WebView 不会这样。
//!
//! `type="custom"` 走 `HaltLazyMount()`、默认 type 不惰性挂载，都不受影响。
//!
//! 已知边界：`<template>` 定义体里的 fixed 不报——定义体的祖先链在实例化处，
//! 静态阶段拼不出「它被插进哪个 scroll-view」，按宁可漏报处理。

use crate::matcher::Tree;
use crate::project::Project;
use crate::rules::wxml::structure::find_attr;
use crate::rules::wxml::{ElementSite, element_diagnostic};
use crate::rules::wxml::{for_each_element, resolved_property};
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

pub struct FixedInLazyScroll;

impl FixedInLazyScroll {
    pub const ID: &'static str = "skyline/fixed-in-lazy-scroll";

    const MESSAGE: &'static str = "`type=\"list\"` 的 scroll-view 惰性挂载：视口外的列表项会被卸载，里面的 `position: fixed` 元素不会留在页面上（首屏正常，一滚动就消失）";

    const HELP: &'static str = "把吸顶 / 悬浮元素移到 scroll-view 外面（页面用 flex 纵向布局，scroll-view 占剩余高度）；或把该 scroll-view 改成 `type=\"custom\"`";
}

impl Rule for FixedInLazyScroll {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "`type=\"list\"` 的 scroll-view 里用 position: fixed 做吸顶：滚动后会随列表项卸载消失",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        for_each_element(project, self.target(), |site| {
            if !is_fixed(project, site.tree, site.element) {
                return;
            }
            if !inside_lazy_list(site) {
                return;
            }
            out.push(
                element_diagnostic(
                    Self::ID,
                    site.doc,
                    site.element,
                    site.doc.path(),
                    Severity::Warning,
                    Self::MESSAGE,
                )
                .help(site.reach.help(Self::HELP)),
            );
        });
        out
    }
}

fn is_fixed(project: &Project, tree: &Tree<'_>, element: &crate::input::wxml::WxmlElement) -> bool {
    resolved_property(project, tree, element, "position")
        .is_some_and(|resolved| resolved.value.trim().eq_ignore_ascii_case("fixed"))
}

/// 最近的那层 scroll-view 祖先是不是 `type="list"`。
///
/// 只看最近一层：内层是 custom / 默认类型时不会被惰性卸载，外层 list 管不到它。
/// `type` 是动态值时静态判不了，按不报处理。
fn inside_lazy_list(site: &ElementSite<'_, '_>) -> bool {
    let mut current = site.element;
    while let Some(parent) = site.tree.parent_of(current) {
        if parent.tag == "scroll-view" {
            return matches!(
                find_attr(parent, "type").and_then(|attr| attr.value.as_deref()),
                Some(value) if !value.contains("{{") && value.trim() == "list"
            );
        }
        current = parent;
    }
    false
}

#[cfg(test)]
mod tests {
    use crate::rules::testutil::wxml::{Fixture, check_rule, project};

    use super::FixedInLazyScroll;

    fn check(fixture: &Fixture) -> Vec<crate::Diagnostic> {
        check_rule(fixture, &FixedInLazyScroll)
    }

    const FIXED: &str = ".bar { position: fixed; top: 0; }";

    #[test]
    fn fixed_inside_list_scroll_is_reported() {
        let fixture = Fixture::page(
            "filf-list",
            r#"<scroll-view type="list"><view class="bar"/></scroll-view>"#,
            FIXED,
        );
        let diagnostics = check(&fixture);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(
            diagnostics[0].message.contains("惰性挂载"),
            "{diagnostics:#?}"
        );
    }

    /// 中间隔一层普通容器也算在 list 列表里。
    #[test]
    fn deep_descendant_is_reported() {
        let fixture = Fixture::page(
            "filf-deep",
            r#"<scroll-view type="list"><view><view class="bar"/></view></scroll-view>"#,
            FIXED,
        );
        assert_eq!(check(&fixture).len(), 1, "{:#?}", check(&fixture));
    }

    #[test]
    fn custom_and_default_scroll_are_not_reported() {
        for (name, wxml) in [
            (
                "filf-custom",
                r#"<scroll-view type="custom"><view class="bar"/></scroll-view>"#,
            ),
            (
                "filf-plain",
                r#"<scroll-view><view class="bar"/></scroll-view>"#,
            ),
            (
                "filf-nested",
                r#"<scroll-view type="nested"><view class="bar"/></scroll-view>"#,
            ),
        ] {
            let fixture = Fixture::page(name, wxml, FIXED);
            assert!(check(&fixture).is_empty(), "{name}: {:#?}", check(&fixture));
        }
    }

    /// 内层不是 list 时，外层 list 管不到它；内层 list 自己会报。
    #[test]
    fn nearest_scroll_view_wins() {
        let inner_custom = Fixture::page(
            "filf-inner-custom",
            r#"<scroll-view type="list"><scroll-view type="custom"><view class="bar"/></scroll-view></scroll-view>"#,
            FIXED,
        );
        assert!(
            check(&inner_custom).is_empty(),
            "{:#?}",
            check(&inner_custom)
        );

        let inner_list = Fixture::page(
            "filf-inner-list",
            r#"<scroll-view type="custom"><scroll-view type="list"><view class="bar"/></scroll-view></scroll-view>"#,
            FIXED,
        );
        assert_eq!(check(&inner_list).len(), 1, "{:#?}", check(&inner_list));
    }

    /// 动态 `type` 和 scroll-view 之外都不报（后者归 fixed-layer 一族）。
    #[test]
    fn dynamic_type_and_outside_are_not_reported() {
        let dynamic = Fixture::page(
            "filf-dynamic",
            r#"<scroll-view type="{{t}}"><view class="bar"/></scroll-view>"#,
            FIXED,
        );
        assert!(check(&dynamic).is_empty(), "{:#?}", check(&dynamic));

        let outside = Fixture::page("filf-outside", r#"<view class="bar"/>"#, FIXED);
        assert!(check(&outside).is_empty(), "{:#?}", check(&outside));
    }

    /// 非 fixed 的绝对定位不受影响。
    #[test]
    fn absolute_position_is_not_reported() {
        let fixture = Fixture::page(
            "filf-absolute",
            r#"<scroll-view type="list"><view class="bar"/></scroll-view>"#,
            ".bar { position: absolute; top: 0; }",
        );
        assert!(check(&fixture).is_empty(), "{:#?}", check(&fixture));
    }

    /// 模板定义体的祖先链在实例化处，静态判不出来：按宁可漏报不报（见模块注释）。
    #[test]
    fn template_body_is_not_reported() {
        let fixture = project(
            "filf-template",
            concat!(
                r#"<scroll-view type="list"><template is="row"/></scroll-view>"#,
                r#"<template name="row"><view class="bar"/></template>"#,
            ),
        );
        let fixture = fixture.write(
            "pages/index/index.wxss",
            ".bar { position: fixed; top: 0; }",
        );
        assert!(check(fixture).is_empty(), "{:#?}", check(fixture));
    }
}
