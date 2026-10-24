//! `<scroll-view>` 必须显式指定合法 type，否则性能退化。

use crate::diag::{Diagnostic, Severity};
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};

use super::{element_diagnostic, span_diagnostic, visit, walk};

pub struct ScrollViewType;

impl ScrollViewType {
    pub const ID: &'static str = "skyline/scroll-view-type";
}

/// `scroll-view` 在 Skyline 下的合法 `type`。
/// 依据 `scroll-view.md` 的 type 属性表：list / custom / nested。
const SCROLL_VIEW_TYPES: [&str; 3] = ["list", "custom", "nested"];

/// 引擎接受但退化为传统滚动：`scroll_view_shadow_node.cc:20-23` 把 `default` 映射成
/// `ScrollView::Type::kFlex`，走普通 Inflate、不触发按需渲染。知识库缺失这个值。
const DEGRADED_TYPE: &str = "default";

impl Rule for ScrollViewType {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "`<scroll-view>` 必须显式指定合法 type，否则性能退化",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit(project, |doc, out| {
            walk(doc, |element, _ancestors| {
                if element.tag != "scroll-view" {
                    return;
                }
                match element
                    .attributes
                    .iter()
                    .find(|attr| attr.name.eq_ignore_ascii_case("type"))
                {
                    None => {
                        out.push(
                            element_diagnostic(
                                Self::ID,
                                doc,
                                element,
                                doc.path(),
                                Severity::Warning,
                                "`<scroll-view>` 缺少 type 属性，Skyline 下会退化为非按需渲染，性能下降",
                            )
                            .help(
                                "显式指定 type：list（普通长列表）、custom（list-view/grid-view/sticky 布局）、nested（嵌套滚动）",
                            ),
                        );
                    }
                    // 动态值无法在静态阶段确定，跳过。
                    Some(attr) if !attr.is_static => {}
                    Some(attr) => {
                        let value = attr.value.as_deref().unwrap_or("").trim();
                        if SCROLL_VIEW_TYPES.contains(&value) {
                            return;
                        }
                        let (severity, message, help) = if value == DEGRADED_TYPE {
                            (
                                Severity::Warning,
                                "`<scroll-view type=\"default\">` 在 Skyline 下按传统滚动渲染，没有按需渲染".to_string(),
                                "type=default 是合法的（引擎映射到 kFlex），但长列表下性能明显低于 list/custom；建议改用 list（普通长列表）、custom（list-view/grid-view/sticky 布局）、nested（嵌套滚动）",
                            )
                        } else {
                            (
                                Severity::Error,
                                format!(
                                    "`<scroll-view type=\"{value}\">` 不是 Skyline 支持的滚动模式"
                                ),
                                "合法值：list（普通长列表）、custom（list-view/grid-view/sticky 布局）、nested（嵌套滚动）",
                            )
                        };
                        out.push(
                            span_diagnostic(
                                Self::ID,
                                doc,
                                attr.start,
                                attr.end,
                                doc.path(),
                                severity,
                                message,
                            )
                            .help(help),
                        );
                    }
                }
            });
        })
    }
}

#[cfg(test)]
mod tests {
    use super::ScrollViewType;
    use crate::rules::testutil::wxml::{check_rule, project};

    #[test]
    fn scroll_view_without_type_warns_on_tag() {
        let diagnostics = check_rule(
            &project("scroll-missing", "<scroll-view scroll-y></scroll-view>"),
            &ScrollViewType,
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, crate::Severity::Warning);
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (1, 2));
    }

    #[test]
    fn scroll_view_with_invalid_static_type_errors_on_attribute() {
        let diagnostics = check_rule(
            &project(
                "scroll-invalid",
                r#"<scroll-view type="page"></scroll-view>"#,
            ),
            &ScrollViewType,
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, crate::Severity::Error);
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (1, 14));
        assert!(diagnostics[0].help.as_deref().unwrap().contains("nested"));
    }

    #[test]
    fn valid_and_dynamic_scroll_view_types_are_allowed() {
        let diagnostics = check_rule(
            &project(
                "scroll-ok",
                "<view><scroll-view type=\"list\"/><scroll-view type=\"custom\"/><scroll-view type=\"nested\"/><scroll-view type=\"{{t}}\"/></view>",
            ),
            &ScrollViewType,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// `default` 是引擎接受的合法值（映射到 kFlex），只是没有按需渲染，不能报 error。
    #[test]
    fn default_type_is_valid_but_warns_about_missing_lazy_rendering() {
        let diagnostics = check_rule(
            &project(
                "scroll-default",
                r#"<scroll-view type="default"></scroll-view>"#,
            ),
            &ScrollViewType,
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, crate::Severity::Warning);
        assert!(diagnostics[0].message.contains("按需渲染"));
    }
}
