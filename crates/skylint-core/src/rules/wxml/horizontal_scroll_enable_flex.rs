//! 横向滚动的 `scroll-view` 必须开启 `enable-flex`。

use crate::diag::{Diagnostic, Severity};
use crate::input::wxml::{WxmlAttribute, WxmlElement};
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};

use super::{load_doc, wxml_files};

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
        let mut diagnostics = Vec::new();
        for path in wxml_files(project) {
            let Some(doc) = load_doc(project, path) else {
                continue;
            };
            for element in doc.elements() {
                if element.tag != "scroll-view"
                    || flag(element, "scroll-x") != Some(true)
                    || flag(element, "enable-flex") != Some(false)
                {
                    continue;
                }
                let Some(attr) = find_attr(element, "scroll-x") else {
                    continue;
                };
                let (line, column) = doc.position(attr.start);
                let (end_line, end_column) = doc.position(attr.end);
                diagnostics.push(
                    Diagnostic::new(
                        HORIZONTAL_FLEX_ID,
                        Severity::Warning,
                        "横向滚动的 `<scroll-view>` 未开启 `enable-flex`，WebView 下横向滚动不会生效",
                        path,
                        line,
                        column,
                    )
                    .end(end_line, end_column)
                    .help(
                        "添加 `enable-flex`，并给容器写 `style=\"display: flex; flex-direction: row;\"`",
                    ),
                );
            }
        }
        diagnostics
    }
}

/// 三态布尔属性：`None` 表示动态绑定；属性缺省为 `Some(false)`。
fn flag(element: &WxmlElement, name: &str) -> Option<bool> {
    match find_attr(element, name) {
        None => Some(false),
        Some(attr) if !attr.is_static => None,
        Some(attr) => Some(
            attr.value
                .as_deref()
                .is_none_or(|value| !matches!(value.trim(), "" | "false" | "0")),
        ),
    }
}

fn find_attr<'a>(element: &'a WxmlElement, name: &str) -> Option<&'a WxmlAttribute> {
    element
        .attributes
        .iter()
        .find(|attr| attr.name.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::super::testutil::{Fixture, check_rule, project};
    use super::HorizontalScrollEnableFlex;
    use crate::Severity;

    fn check_flex(fixture: &Fixture) -> Vec<crate::Diagnostic> {
        check_rule(fixture, &HorizontalScrollEnableFlex)
    }

    // ---- enable-flex ----

    #[test]
    fn horizontal_scroll_without_enable_flex_is_reported() {
        let diagnostics = check_flex(&project(
            "flex-missing",
            r#"<scroll-view type="list" scroll-x><view/></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, Severity::Warning);
        assert!(diagnostics[0].message.contains("enable-flex"));
    }

    #[test]
    fn horizontal_scroll_with_true_value_without_enable_flex_is_reported() {
        let diagnostics = check_flex(&project(
            "flex-true",
            r#"<scroll-view type="list" scroll-x="true"><view/></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn horizontal_scroll_with_enable_flex_is_allowed() {
        let diagnostics = check_flex(&project(
            "flex-ok",
            r#"<scroll-view type="list" scroll-x enable-flex><view/></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn dynamic_scroll_x_is_skipped() {
        let diagnostics = check_flex(&project(
            "flex-dynamic",
            r#"<scroll-view type="list" scroll-x="{{x}}"><view/></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn scroll_x_false_without_enable_flex_is_allowed() {
        let diagnostics = check_flex(&project(
            "flex-false",
            r#"<scroll-view type="list" scroll-x="false"><view/></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }
}
