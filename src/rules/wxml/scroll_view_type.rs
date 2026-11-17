//! `<scroll-view>` 应显式指定合法 type。
//!
//! `type` 缺失 / 空值时，glass-easel 的 `<scrollview>` 宿主只有一个直接子节点——
//! 兼容 wrapper `<view wx:if="{{ type === '' }}">`（`wx-scroll-view/index.ts:73-83`），
//! 用户写的子节点都在 wrapper 内部，因此内容**全部照常渲染**，只是没有按需渲染。
//! `skyline_scroll_view.dart:1645-1673` 里的 `children: [firstChild]` 取的正是这个 wrapper。
//!
//! 公开标签的 JS 白名单是 `['', 'list', 'custom', 'nested']`（`index.ts:18`）；
//! `type="default"` 或其它未知静态值会被 observer 回退成 `'list'` 并打印 `console.warn`
//! （`index.ts:119-137`）。C++ 的 `default -> kFlex` 只在直接用内部 `<scrollview>` 宿主时可达。

use crate::diag::{Diagnostic, Severity};
use crate::input::wxml::{WxmlAttribute, WxmlDoc, WxmlElement};
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};

use super::structure::find_attr;
use super::{element_diagnostic, for_each_element, span_diagnostic};

pub struct ScrollViewType;

impl ScrollViewType {
    pub const ID: &'static str = "skyline/scroll-view-type";
}

/// 用户最容易误用的值：`default` 不在公开标签的 JS 白名单里，observer 会回退成 `list` 并告警。
const DEGRADED_TYPE: &str = "default";

impl Rule for ScrollViewType {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Info,
            description: "`<scroll-view>` 建议显式指定 type：省略 type 走传统滚动（没有按需渲染），`default` 与未知值会被 JS 层回退成 `list` 并告警",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        for_each_element(project, self.target(), |site| {
            let (doc, element) = (site.doc, site.element);
            if element.tag != "scroll-view" {
                return;
            }
            let Some(attr) = find_attr(element, "type") else {
                out.push(no_lazy_rendering(doc, element, None));
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
                // `type=""` 与不写 type 等价：内容全部渲染在兼容 wrapper 内。
                "" => out.push(no_lazy_rendering(doc, element, Some(attr))),
                other => out.push(falls_back_to_list(doc, attr, other)),
            }
        });
        out
    }
}

/// 缺省 / `type=""`：走传统滚动，没有按需渲染，但子节点全部渲染在兼容 wrapper 内。
fn no_lazy_rendering(
    doc: &WxmlDoc,
    element: &WxmlElement,
    attr: Option<&WxmlAttribute>,
) -> Diagnostic {
    let message = "`<scroll-view>` 没有指定 type，Skyline 下按传统滚动渲染（没有按需渲染），子节点仍会全部渲染在兼容 wrapper 内";
    let diagnostic = match attr {
        Some(attr) => span_diagnostic(
            ScrollViewType::ID,
            doc,
            attr.start,
            attr.end,
            doc.path(),
            Severity::Info,
            message,
        ),
        None => element_diagnostic(
            ScrollViewType::ID,
            doc,
            element,
            doc.path(),
            Severity::Info,
            message,
        ),
    };
    diagnostic.help(HELP)
}

/// 不在 JS 白名单里的静态值：observer 回退成 `list` 并打印 `console.warn`。
fn falls_back_to_list(doc: &WxmlDoc, attr: &WxmlAttribute, value: &str) -> Diagnostic {
    let message = if value == DEGRADED_TYPE {
        "`<scroll-view type=\"default\">` 不是公开的 type 值（省略 type 并不等于 default），JS 层会回退成 `list` 并在控制台告警".to_string()
    } else {
        format!(
            "`<scroll-view type=\"{value}\">` 不是 Skyline 支持的类型，JS 层会回退成 `list` 并在控制台告警"
        )
    };
    span_diagnostic(
        ScrollViewType::ID,
        doc,
        attr.start,
        attr.end,
        doc.path(),
        Severity::Warning,
        message,
    )
    .help(HELP)
}

const HELP: &str = "按需渲染用 list（普通长列表）、custom（list-view/grid-view/sticky 布局）、nested（嵌套滚动）；兼容传统滚动则省略 type";

#[cfg(test)]
mod tests {
    use super::ScrollViewType;
    use crate::rules::testutil::wxml::{check_rule, project};

    #[test]
    fn scroll_view_without_type_reports_no_lazy_rendering_on_tag() {
        let diagnostics = check_rule(
            &project("scroll-missing", "<scroll-view scroll-y></scroll-view>"),
            &ScrollViewType,
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, crate::Severity::Info);
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (1, 2));
        assert!(diagnostics[0].message.contains("没有按需渲染"));
        assert!(diagnostics[0].message.contains("全部渲染"));
    }

    /// 未知静态值不在 JS 白名单里，observer 会回退成 `list`。
    #[test]
    fn unknown_static_type_warns_about_list_fallback() {
        let diagnostics = check_rule(
            &project(
                "scroll-invalid",
                r#"<scroll-view type="page"></scroll-view>"#,
            ),
            &ScrollViewType,
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, crate::Severity::Warning);
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (1, 14));
        assert!(diagnostics[0].message.contains("type=\"page\""));
        assert!(diagnostics[0].message.contains("回退成 `list`"));
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

    /// `default` 不在白名单里（省略 type 也不等于 default），JS 层同样回退成 `list`。
    #[test]
    fn default_type_warns_about_list_fallback() {
        let diagnostics = check_rule(
            &project(
                "scroll-default",
                r#"<scroll-view type="default"></scroll-view>"#,
            ),
            &ScrollViewType,
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, crate::Severity::Warning);
        assert!(diagnostics[0].message.contains("不是公开的 type 值"));
        assert!(diagnostics[0].message.contains("回退成 `list`"));
    }

    /// 多个子元素也全部渲染在兼容 wrapper 内，不是内容丢失。
    #[test]
    fn missing_type_with_several_children_still_renders_content() {
        let diagnostics = check_rule(
            &project(
                "scroll-two-children",
                "<scroll-view scroll-y><view class=\"a\"/><view class=\"b\"/></scroll-view>",
            ),
            &ScrollViewType,
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, crate::Severity::Info);
        assert!(diagnostics[0].message.contains("全部渲染"));
        assert!(!diagnostics[0].message.contains("只渲染第一个子元素"));
    }

    /// `wx:if` 的两个分支互斥时同样只是缺省 type，仍按 Info 报。
    #[test]
    fn exclusive_branches_are_still_info() {
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

    /// `type=""` 与不写 type 等价。
    #[test]
    fn empty_type_is_treated_as_missing() {
        let diagnostics = check_rule(
            &project("scroll-empty", r#"<scroll-view type=""></scroll-view>"#),
            &ScrollViewType,
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, crate::Severity::Info);
        assert!(diagnostics[0].message.contains("没有指定 type"));
    }
}
