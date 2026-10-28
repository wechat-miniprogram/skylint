//! 横向滚动的 `scroll-view` 若自身是 `display: flex`，必须开启 `enable-flex`。
//!
//! 这是 WebView 一侧的要求（知识库 `scroll/scroll-view.md:329` 的 MUST）：组件只在
//! 宿主 `getComputedStyle(this.$$).display === 'flex'` 时才需要把 flex 布局下传
//! （`wxapplib/src/wx-component/src/wx-scroll-view/`），不是「只要用了 `scroll-x` 就必须写」。

use crate::diag::{Diagnostic, Severity};
use crate::input::wxml::WxmlElement;
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};

use super::structure::find_attr;
use crate::matcher::Tree;

use super::{display_values, is_flex_value, span_diagnostic, visit, walk_skyline};

pub const HORIZONTAL_FLEX_ID: &str = "skyline/horizontal-scroll-enable-flex";

pub struct HorizontalScrollEnableFlex;

impl Rule for HorizontalScrollEnableFlex {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: HORIZONTAL_FLEX_ID,
            severity: Severity::Warning,
            description: "横向滚动的 `<scroll-view>` 需开启 `enable-flex` 以兼容 WebView",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit(project, |doc, out| {
            let tree = Tree::new(doc);
            walk_skyline(doc, |element, _ancestors| {
                if element.tag != "scroll-view"
                    || flag(element, "scroll-x") != Some(true)
                    || flag(element, "enable-flex") != Some(false)
                    || !display_values(project, &tree, element)
                        .iter()
                        .any(|value| is_flex_value(value))
                {
                    return;
                }
                let Some(attr) = find_attr(element, "scroll-x") else {
                    return;
                };
                out.push(
                    span_diagnostic(
                        HORIZONTAL_FLEX_ID,
                        doc,
                        attr.start,
                        attr.end,
                        doc.path(),
                        Severity::Warning,
                        "横向滚动的 `<scroll-view>` 未开启 `enable-flex`，WebView 下横向滚动不会生效",
                    )
                    .help(
                        "添加 `enable-flex`，并给容器写 `style=\"display: flex; flex-direction: row;\"`",
                    ),
                );
            });
        })
    }
}

/// 三态布尔属性：`None` 表示动态绑定；属性缺省为 `Some(false)`。
///
/// WXML 里写 `scroll-x="false"` 是非空字符串，引擎按 [`GetLooseyBool`] 当**真**
/// （`shadow/js_value.h:31-44`），所以只有空串算假。
fn flag(element: &WxmlElement, name: &str) -> Option<bool> {
    match find_attr(element, name) {
        None => Some(false),
        Some(attr) if !attr.is_static => None,
        Some(attr) => Some(
            attr.value
                .as_deref()
                .is_none_or(|value| !value.trim().is_empty()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::HorizontalScrollEnableFlex;
    use crate::Severity;
    use crate::rules::testutil::wxml::{Fixture, check_rule, project};

    fn check_flex(fixture: &Fixture) -> Vec<crate::Diagnostic> {
        check_rule(fixture, &HorizontalScrollEnableFlex)
    }

    // ---- enable-flex ----

    #[test]
    fn flex_display_without_enable_flex_is_reported() {
        let diagnostics = check_flex(&project(
            "flex-missing",
            r#"<scroll-view type="list" scroll-x style="display: flex;"><view/></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, Severity::Warning);
        assert!(diagnostics[0].message.contains("enable-flex"));
    }

    #[test]
    fn flex_display_from_a_class_is_reported() {
        let fixture = project(
            "flex-class",
            r#"<scroll-view type="list" scroll-x class="tabs"><view/></scroll-view>"#,
        );
        fixture.write("pages/index/index.wxss", ".tabs { display: flex; }");
        assert_eq!(check_flex(&fixture).len(), 1);
    }

    #[test]
    fn horizontal_scroll_without_flex_display_is_not_reported() {
        // 组件只在宿主 display:flex 时才需要 enable-flex；skit 的 .tab-list-cnt 就是这种。
        let diagnostics = check_flex(&project(
            "flex-no-display",
            r#"<scroll-view type="list" scroll-x><view/></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn scroll_x_string_false_is_truthy() {
        // WXML 里 `scroll-x="false"` 是非空字符串，引擎按 GetLooseyBool 当开启。
        let diagnostics = check_flex(&project(
            "flex-false",
            r#"<scroll-view type="list" scroll-x="false" style="display: flex;"><view/></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    #[test]
    fn horizontal_scroll_with_enable_flex_is_allowed() {
        let diagnostics = check_flex(&project(
            "flex-ok",
            r#"<scroll-view type="list" scroll-x enable-flex style="display: flex;"><view/></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn dynamic_scroll_x_is_skipped() {
        let diagnostics = check_flex(&project(
            "flex-dynamic",
            r#"<scroll-view type="list" scroll-x="{{x}}" style="display: flex;"><view/></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }
}
