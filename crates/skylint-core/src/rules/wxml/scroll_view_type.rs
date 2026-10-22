//! `<scroll-view>` 必须显式指定合法 type，否则性能退化。

use crate::diag::{Diagnostic, Severity};
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};

use super::{load_doc, wxml_files};

pub struct ScrollViewType;

impl ScrollViewType {
    pub const ID: &'static str = "skyline/scroll-view-type";
}

/// `scroll-view` 在 Skyline 下的合法 `type`。
/// 依据 `scroll-view.md` 的 type 属性表：list / custom / nested。
const SCROLL_VIEW_TYPES: [&str; 3] = ["list", "custom", "nested"];

impl Rule for ScrollViewType {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "`<scroll-view>` 必须显式指定合法 type，否则性能退化",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        for path in wxml_files(project) {
            let Some(doc) = load_doc(project, path) else {
                continue;
            };
            for element in doc.elements() {
                if element.tag != "scroll-view" {
                    continue;
                }
                match element
                    .attributes
                    .iter()
                    .find(|attr| attr.name.eq_ignore_ascii_case("type"))
                {
                    None => {
                        let (line, column) = doc.position(element.tag_start);
                        let (end_line, end_column) = doc.position(element.tag_end);
                        diagnostics.push(
                            Diagnostic::new(
                                Self::ID,
                                Severity::Warning,
                                "`<scroll-view>` 缺少 type 属性，Skyline 下会退化为非按需渲染，性能下降",
                                path,
                                line,
                                column,
                            )
                            .end(end_line, end_column)
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
                            continue;
                        }
                        let (line, column) = doc.position(attr.start);
                        let (end_line, end_column) = doc.position(attr.end);
                        diagnostics.push(
                            Diagnostic::new(
                                Self::ID,
                                Severity::Error,
                                format!("`<scroll-view type=\"{value}\">` 不是 Skyline 支持的滚动模式"),
                                path,
                                line,
                                column,
                            )
                            .end(end_line, end_column)
                            .help(
                                "合法值：list（普通长列表）、custom（list-view/grid-view/sticky 布局）、nested（嵌套滚动）",
                            ),
                        );
                    }
                }
            }
        }
        diagnostics
    }
}

#[cfg(test)]
mod tests {
    use super::super::testutil::{check_rule, project};
    use super::ScrollViewType;

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
}
