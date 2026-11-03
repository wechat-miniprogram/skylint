//! `<scroll-view>` 必须显式指定合法 type。
//!
//! `type` 缺失 / 空值 / `default` 在引擎里分别映射成 `kNone` / `kNone` / `kFlex`，
//! 而 Dart 侧 `skyline_scroll_view.dart:1648-1675` 把这两类走同一条分支：
//! `children: [firstChild]` —— **只渲染第一个子元素**，其余直接丢弃。所以有多个子
//! 元素时这是内容丢失，不是性能问题；只有一个子元素时才是性能退化。

use crate::diag::{Diagnostic, Severity};
use crate::input::wxml::{WxmlDoc, WxmlElement};
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};

use super::branch::max_rendered_children;
use super::structure::find_attr;
use super::{element_diagnostic, span_diagnostic, visit, walk_skyline};

pub struct ScrollViewType;

impl ScrollViewType {
    pub const ID: &'static str = "skyline/scroll-view-type";
}

/// 引擎接受但退化为传统滚动：`scroll_view_shadow_node.cc:20-23` 把 `default` 映射成
/// `ScrollView::Type::kFlex`，走普通 Inflate、不触发按需渲染。知识库缺失这个值。
const DEGRADED_TYPE: &str = "default";

impl Rule for ScrollViewType {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Info,
            description: "`<scroll-view>` 必须显式指定合法 type，否则 Skyline 只渲染第一个子元素",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit(project, |doc, out| {
            walk_skyline(doc, |element, _ancestors| {
                if element.tag != "scroll-view" {
                    return;
                }
                let Some(attr) = find_attr(element, "type") else {
                    out.push(renders_first_only(doc, element, None));
                    return;
                };
                // 动态值无法在静态阶段确定，跳过。
                if !attr.is_static {
                    return;
                }
                let value = attr.value.as_deref().unwrap_or("").trim();
                match value {
                    // 只有这三种走「按需渲染」的分支。
                    "list" | "custom" | "nested" => {}
                    // `type=""` 与不写 type 是同一条 kNone 路径（`scroll_view_shadow_node.cc:20-29`）。
                    "" => out.push(renders_first_only(doc, element, Some(attr))),
                    // `default` 映射成 kFlex，`skyline_scroll_view.dart:1648-1675` 与 kNone 同分支。
                    _ if value == DEGRADED_TYPE => {
                        out.push(renders_first_only(doc, element, Some(attr)))
                    }
                    other => out.push(
                        span_diagnostic(
                            Self::ID,
                            doc,
                            attr.start,
                            attr.end,
                            doc.path(),
                            Severity::Error,
                            format!("`<scroll-view type=\"{other}\">` 不是 Skyline 支持的滚动模式"),
                        )
                        .help("合法值：list（普通长列表）、custom（list-view/grid-view/sticky 布局）、nested（嵌套滚动）"),
                    ),
                }
            });
        })
    }
}

/// 缺省 / 空值 / `default`：引擎只渲染第一个子元素。
///
/// 多个可渲染子元素时其余内容直接消失，按 Error 报；只有一个子元素时只是少了按需渲染，
/// 按 Info 报（`max_rendered_children` 已经处理 `wx:if` 各分支互斥）。
fn renders_first_only(
    doc: &WxmlDoc,
    element: &WxmlElement,
    attr: Option<&crate::input::wxml::WxmlAttribute>,
) -> Diagnostic {
    let dropped = max_rendered_children(element) >= 2;
    let (severity, message, help) = if dropped {
        (
            Severity::Error,
            "`<scroll-view>` 没有可用的 type，Skyline 下只渲染第一个子元素，其余子元素不会显示",
            "显式指定 type：list（普通长列表）、custom（list-view/grid-view/sticky 布局）、nested（嵌套滚动）",
        )
    } else {
        (
            Severity::Info,
            "`<scroll-view>` 没有可用的 type，Skyline 下按传统滚动渲染，没有按需渲染",
            "显式指定 type：list（普通长列表）、custom（list-view/grid-view/sticky 布局）、nested（嵌套滚动）",
        )
    };
    match attr {
        Some(attr) => span_diagnostic(
            ScrollViewType::ID,
            doc,
            attr.start,
            attr.end,
            doc.path(),
            severity,
            message,
        ),
        None => element_diagnostic(
            ScrollViewType::ID,
            doc,
            element,
            doc.path(),
            severity,
            message,
        ),
    }
    .help(help)
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
        assert_eq!(diagnostics[0].severity, crate::Severity::Info);
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
        assert_eq!(diagnostics[0].severity, crate::Severity::Info);
        assert!(diagnostics[0].message.contains("按传统滚动渲染"));
    }

    /// 多个子元素时不是性能问题：引擎只渲染第一个，其余内容消失（Error）。
    #[test]
    fn missing_type_with_several_children_drops_content() {
        let diagnostics = check_rule(
            &project(
                "scroll-two-children",
                "<scroll-view scroll-y><view class=\"a\"/><view class=\"b\"/></scroll-view>",
            ),
            &ScrollViewType,
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, crate::Severity::Error);
        assert!(diagnostics[0].message.contains("只渲染第一个子元素"));
    }

    /// `wx:if` 的两个分支互斥，实际只渲染一个，不该按「丢内容」报。
    #[test]
    fn exclusive_branches_count_as_one_child() {
        let diagnostics = check_rule(
            &project(
                "scroll-branches",
                "<scroll-view scroll-y><view wx:if=\"{{a}}\"/><view wx:else/></scroll-view>",
            ),
            &ScrollViewType,
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, crate::Severity::Info);
    }

    /// `type=""` 与不写 type 在引擎里是同一条 kNone 路径，不能报 error。
    #[test]
    fn empty_type_is_treated_as_missing() {
        let diagnostics = check_rule(
            &project("scroll-empty", r#"<scroll-view type=""></scroll-view>"#),
            &ScrollViewType,
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, crate::Severity::Info);
        assert!(diagnostics[0].message.contains("没有可用的 type"));
    }
}
