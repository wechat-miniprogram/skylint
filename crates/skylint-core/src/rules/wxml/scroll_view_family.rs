//! `scroll-view` 家族组件的挂载位置。约束写在 `data/scroll_family.txt` 里，
//! Rust 侧只做「直接父节点 / 最近祖先」查表，再加两条无法用表表达的否定约束
//! （nested-scroll-header 不能在 body 内、draggable-sheet 内部的 scroll-view
//! 必须关联容器）。

use crate::diag::{Diagnostic, Severity};
use crate::input::wxml::{WxmlDoc, WxmlElement, WxmlNode};
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};

use super::{element_diagnostic, is_builtin, load_doc, wxml_files};

pub const ID: &str = "skyline/scroll-view-family";

/// 家族挂载约束表。
const CONSTRAINTS_DATA: &str = include_str!("data/scroll_family.txt");

/// 一个允许的父/祖先节点，`attr` 是该节点必须满足的属性。
struct Target {
    tag: &'static str,
    attr: Option<(&'static str, &'static str)>,
}

struct Constraint {
    child: &'static str,
    /// `true` 表示要求最近的同名祖先满足条件，`false` 表示要求直接父节点。
    ancestor: bool,
    targets: Vec<Target>,
    help: &'static str,
}

fn constraints() -> Vec<Constraint> {
    CONSTRAINTS_DATA
        .lines()
        .filter_map(|line| {
            if line.trim().is_empty() || line.starts_with('#') {
                return None;
            }
            let mut fields = line.split('\t');
            let child = fields.next()?.trim();
            let ancestor = match fields.next()?.trim() {
                "direct" => false,
                "ancestor" => true,
                _ => return None,
            };
            let targets = fields.next()?.split('|').map(parse_target).collect();
            Some(Constraint {
                child,
                ancestor,
                targets,
                help: fields.next()?.trim(),
            })
        })
        .collect()
}

fn parse_target(raw: &'static str) -> Target {
    let raw = raw.trim();
    match raw.split_once('[') {
        Some((tag, rest)) => {
            let inner = rest.strip_suffix(']').unwrap_or(rest);
            let (name, value) = inner.split_once('=').unwrap_or((inner, ""));
            Target {
                tag: tag.trim(),
                attr: Some((name.trim(), value.trim())),
            }
        }
        None => Target {
            tag: raw,
            attr: None,
        },
    }
}

enum AttrCheck {
    Ok,
    Violation,
    /// 动态绑定，静态阶段无法判定。
    Unknown,
}

fn check_attr(element: &WxmlElement, name: &str, expected: &str) -> AttrCheck {
    match find_attr(element, name) {
        None => AttrCheck::Violation,
        Some(attr) if !attr.is_static => AttrCheck::Unknown,
        Some(attr) => match attr.value.as_deref() {
            Some(value) if value.trim().eq_ignore_ascii_case(expected) => AttrCheck::Ok,
            _ => AttrCheck::Violation,
        },
    }
}

fn find_attr<'a>(
    element: &'a WxmlElement,
    name: &str,
) -> Option<&'a crate::input::wxml::WxmlAttribute> {
    element
        .attributes
        .iter()
        .find(|attr| attr.name.eq_ignore_ascii_case(name))
}

/// 把约束目标渲染成文档里的写法，用于诊断文案。
fn render_targets(targets: &[Target]) -> String {
    targets
        .iter()
        .map(|target| match target.attr {
            Some((name, value)) => format!("<{} {}=\"{}\">", target.tag, name, value),
            None => format!("<{}>", target.tag),
        })
        .collect::<Vec<_>>()
        .join(" 或 ")
}

/// 带上实际属性值的节点写法，说明「当前挂在哪」。
fn describe(element: &WxmlElement, name: &str) -> String {
    match find_attr(element, name).and_then(|attr| attr.value.as_deref()) {
        Some(value) => format!("<{} {}=\"{}\">", element.tag, name, value),
        None => format!("<{}>", element.tag),
    }
}

fn placement(
    element: &WxmlElement,
    ancestors: &[&WxmlElement],
    constraint: &Constraint,
) -> Option<String> {
    if constraint.ancestor {
        ancestor_violation(element, ancestors, constraint)
    } else {
        direct_violation(element, ancestors, constraint)
    }
}

fn direct_violation(
    element: &WxmlElement,
    ancestors: &[&WxmlElement],
    constraint: &Constraint,
) -> Option<String> {
    let expected = render_targets(&constraint.targets);
    let Some(parent) = ancestors.last() else {
        return Some(format!(
            "`<{}>` 位于文件顶层，必须直接放在 {} 内",
            element.tag, expected
        ));
    };
    // 自定义组件的渲染树未知，不越过它判定。
    if !is_builtin(&parent.tag) {
        return None;
    }
    let Some(target) = constraint
        .targets
        .iter()
        .find(|target| target.tag == parent.tag)
    else {
        return Some(format!(
            "`<{}>` 必须直接放在 {} 内，当前父节点是 `<{}>`",
            element.tag, expected, parent.tag
        ));
    };
    match target.attr {
        None => None,
        Some((name, value)) => match check_attr(parent, name, value) {
            AttrCheck::Violation => Some(format!(
                "`<{}>` 必须直接放在 {} 内，父节点是 {}",
                element.tag,
                expected,
                describe(parent, name)
            )),
            _ => None,
        },
    }
}

fn ancestor_violation(
    element: &WxmlElement,
    ancestors: &[&WxmlElement],
    constraint: &Constraint,
) -> Option<String> {
    let expected = render_targets(&constraint.targets);
    for ancestor in ancestors.iter().rev() {
        // 自定义组件的渲染树未知，不越过它判定。
        if !is_builtin(&ancestor.tag) {
            return None;
        }
        let Some(target) = constraint
            .targets
            .iter()
            .find(|target| target.tag == ancestor.tag)
        else {
            continue;
        };
        return match target.attr {
            None => None,
            Some((name, value)) => match check_attr(ancestor, name, value) {
                AttrCheck::Violation => Some(format!(
                    "`<{}>` 必须放在 {} 内，最近的 `<{}>` 是 {}",
                    element.tag,
                    expected,
                    ancestor.tag,
                    describe(ancestor, name)
                )),
                _ => None,
            },
        };
    }
    Some(format!(
        "`<{}>` 必须放在 {} 内，当前不在该容器中",
        element.tag, expected
    ))
}

pub struct ScrollViewFamily;

impl Rule for ScrollViewFamily {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Error,
            description: "scroll-view 家族组件（sticky / list-view / grid-view / nested / draggable-sheet）的挂载位置",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let constraints = constraints();
        let mut diagnostics = Vec::new();
        for path in wxml_files(project) {
            let Some(doc) = load_doc(project, path) else {
                continue;
            };
            let mut ancestors = Vec::new();
            collect(
                &doc,
                &doc.nodes,
                &mut ancestors,
                &constraints,
                path,
                &mut diagnostics,
            );
        }
        diagnostics
    }
}

fn collect<'a>(
    doc: &WxmlDoc,
    nodes: &'a [WxmlNode],
    ancestors: &mut Vec<&'a WxmlElement>,
    constraints: &[Constraint],
    path: &str,
    out: &mut Vec<Diagnostic>,
) {
    for node in nodes {
        let WxmlNode::Element(element) = node else {
            continue;
        };
        check_element(doc, element, ancestors, constraints, path, out);
        ancestors.push(element);
        collect(doc, &element.children, ancestors, constraints, path, out);
        ancestors.pop();
    }
}

fn check_element(
    doc: &WxmlDoc,
    element: &WxmlElement,
    ancestors: &[&WxmlElement],
    constraints: &[Constraint],
    path: &str,
    out: &mut Vec<Diagnostic>,
) {
    for constraint in constraints.iter().filter(|c| c.child == element.tag) {
        let Some(message) = placement(element, ancestors, constraint) else {
            continue;
        };
        out.push(
            element_diagnostic(ID, doc, element, path, Severity::Error, message)
                .help(constraint.help),
        );
    }

    // nested-scroll.md NEVER 1「不要在 nested-scroll-body 下面放 nested-scroll-header」。
    if element.tag == "nested-scroll-header"
        && ancestors
            .iter()
            .rev()
            .take_while(|ancestor| is_builtin(&ancestor.tag))
            .any(|ancestor| ancestor.tag == "nested-scroll-body")
    {
        out.push(
            element_diagnostic(
                ID,
                doc,
                element,
                path,
                Severity::Error,
                "`<nested-scroll-header>` 不能放在 `<nested-scroll-body>` 内，头部不会随外层滚动",
            )
            .help("把 `<nested-scroll-header>` 移到 `<nested-scroll-body>` 之前，与它平级"),
        );
    }

    // draggable-sheet.md「内部的 scroll-view 必须设置 associative-container=\"draggable-sheet\"」。
    if element.tag == "scroll-view"
        && ancestors
            .last()
            .is_some_and(|parent| parent.tag == "draggable-sheet")
        && matches!(
            check_attr(element, "associative-container", "draggable-sheet"),
            AttrCheck::Violation
        )
    {
        out.push(
            element_diagnostic(
                ID,
                doc,
                element,
                path,
                Severity::Warning,
                "`<draggable-sheet>` 内的 `<scroll-view>` 必须设置 `associative-container=\"draggable-sheet\"`，否则拖拽联动不生效",
            )
            .help("在内部 `<scroll-view>` 上添加 `associative-container=\"draggable-sheet\"`"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::testutil::{Fixture, check_rule, project};
    use super::ScrollViewFamily;
    use super::constraints;
    use crate::Severity;

    fn check(fixture: &Fixture) -> Vec<crate::Diagnostic> {
        check_rule(fixture, &ScrollViewFamily)
    }

    // ---- 数据表 ----

    #[test]
    fn data_table_is_well_formed() {
        let constraints = constraints();
        assert_eq!(constraints.len(), 5);
        assert!(
            constraints
                .iter()
                .all(|c| !c.child.is_empty() && !c.targets.is_empty() && !c.help.is_empty())
        );
        let list_view = constraints.iter().find(|c| c.child == "list-view").unwrap();
        assert_eq!(list_view.targets.len(), 2);
        assert!(list_view.targets[0].attr.is_some());
        assert!(list_view.targets[1].attr.is_none());
    }

    // ---- sticky-section ----

    #[test]
    fn sticky_section_in_list_scroll_is_reported() {
        let diagnostics = check(&project(
            "sticky-section-list",
            r#"<scroll-view type="list"><sticky-section/></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert!(diagnostics[0].message.contains(r#"type="list""#));
    }

    #[test]
    fn sticky_section_without_scroll_view_is_reported() {
        let diagnostics = check(&project(
            "sticky-section-view",
            "<view><sticky-section/></view>",
        ));
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("不在该容器中"));
    }

    #[test]
    fn sticky_section_in_scroll_view_without_type_is_reported() {
        let diagnostics = check(&project(
            "sticky-section-no-type",
            "<scroll-view><sticky-section/></scroll-view>",
        ));
        assert_eq!(diagnostics.len(), 1);
    }

    // ---- list-view / grid-view ----

    #[test]
    fn list_view_in_plain_view_is_reported() {
        let diagnostics = check(&project("list-view-view", "<view><list-view/></view>"));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert!(diagnostics[0].message.contains("sticky-section"));
    }

    #[test]
    fn list_view_in_list_scroll_is_reported() {
        let diagnostics = check(&project(
            "list-view-list",
            r#"<scroll-view type="list"><list-view/></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains(r#"type="list""#));
    }

    #[test]
    fn list_view_at_root_is_reported() {
        let diagnostics = check(&project("list-view-root", "<list-view/>"));
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn grid_view_in_plain_view_is_reported() {
        let diagnostics = check(&project("grid-view-view", "<view><grid-view/></view>"));
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("grid-view"));
    }

    #[test]
    fn grid_view_in_list_scroll_is_reported() {
        let diagnostics = check(&project(
            "grid-view-list",
            r#"<scroll-view type="list"><grid-view/></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1);
    }

    // ---- nested scroll ----

    #[test]
    fn nested_header_in_list_scroll_is_reported() {
        let diagnostics = check(&project(
            "nested-header-list",
            r#"<scroll-view type="list"><nested-scroll-header/></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains(r#"type="list""#));
    }

    #[test]
    fn nested_header_without_scroll_view_is_reported() {
        let diagnostics = check(&project(
            "nested-header-view",
            "<view><nested-scroll-header/></view>",
        ));
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn nested_body_in_custom_scroll_is_reported() {
        let diagnostics = check(&project(
            "nested-body-custom",
            r#"<scroll-view type="custom"><nested-scroll-body/></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("nested"));
    }

    #[test]
    fn nested_body_without_scroll_view_is_reported() {
        let diagnostics = check(&project(
            "nested-body-view",
            "<view><nested-scroll-body/></view>",
        ));
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn nested_header_inside_body_is_reported() {
        let diagnostics = check(&project(
            "nested-header-body",
            r#"<scroll-view type="nested"><nested-scroll-body><nested-scroll-header/></nested-scroll-body></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("nested-scroll-body"));
    }

    #[test]
    fn nested_header_deep_inside_body_is_reported() {
        let diagnostics = check(&project(
            "nested-header-body-deep",
            r#"<scroll-view type="nested"><nested-scroll-body><view><nested-scroll-header/></view></nested-scroll-body></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1);
    }

    // ---- draggable-sheet ----

    #[test]
    fn draggable_sheet_scroll_without_container_is_reported() {
        let diagnostics = check(&project(
            "draggable-missing",
            r#"<draggable-sheet><scroll-view type="list" scroll-y/></draggable-sheet>"#,
        ));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, Severity::Warning);
        assert!(diagnostics[0].message.contains("associative-container"));
    }

    #[test]
    fn draggable_sheet_scroll_with_wrong_container_is_reported() {
        let diagnostics = check(&project(
            "draggable-wrong",
            r#"<draggable-sheet><scroll-view type="list" associative-container="nested-scroll-view"/></draggable-sheet>"#,
        ));
        assert_eq!(diagnostics.len(), 1);
    }

    // ---- 负例 ----

    /// 文档冲突，故意不报：sticky.md NEVER 2 要求 sticky-header 必须在 sticky-section 内，
    /// 但 scroll-view.md:54 的 custom 模式「支持的子组件」里同时列了 sticky-header，
    /// 生产工程也按后者在 nested-scroll-header 内直接使用 sticky-header。
    #[test]
    fn sticky_header_outside_section_is_not_reported_due_to_conflicting_docs() {
        let diagnostics = check(&project(
            "sticky-conflict",
            concat!(
                r#"<scroll-view type="custom"><sticky-header>标题</sticky-header></scroll-view>"#,
                r#"<scroll-view type="nested"><nested-scroll-header><sticky-header>标题</sticky-header></nested-scroll-header></scroll-view>"#,
                "<view><sticky-header>标题</sticky-header></view>",
            ),
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn sticky_section_under_custom_scroll_is_allowed() {
        let diagnostics = check(&project(
            "sticky-custom",
            r#"<scroll-view type="custom"><sticky-section/></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn dynamic_scroll_view_type_is_skipped() {
        let diagnostics = check(&project(
            "sticky-dynamic",
            r#"<scroll-view type="{{t}}"><sticky-section><sticky-header>标题</sticky-header></sticky-section></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn list_view_under_custom_scroll_is_allowed() {
        let diagnostics = check(&project(
            "list-custom",
            r#"<scroll-view type="custom"><list-view><view/></list-view></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn list_view_under_sticky_section_is_allowed() {
        let diagnostics = check(&project(
            "list-sticky",
            r#"<scroll-view type="custom"><sticky-section><sticky-header>标题</sticky-header><list-view/></sticky-section></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn list_view_under_block_is_allowed() {
        let diagnostics = check(&project(
            "list-block",
            r#"<scroll-view type="custom"><block wx:for="{{list}}"><list-view/></block></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn list_view_under_custom_component_is_skipped() {
        let diagnostics = check(&project(
            "list-custom-comp",
            "<my-box><list-view/></my-box>",
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn grid_view_under_custom_scroll_is_allowed() {
        let diagnostics = check(&project(
            "grid-custom",
            r#"<scroll-view type="custom"><grid-view type="aligned"/></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn grid_view_under_sticky_section_is_allowed() {
        let diagnostics = check(&project(
            "grid-sticky",
            r#"<scroll-view type="custom"><sticky-section><sticky-header>标题</sticky-header><grid-view/></sticky-section></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn nested_parts_under_nested_scroll_are_allowed() {
        let diagnostics = check(&project(
            "nested-ok",
            r#"<scroll-view type="nested"><nested-scroll-header><view/></nested-scroll-header><nested-scroll-body><scroll-view type="list"/></nested-scroll-body></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn nested_parts_under_block_are_allowed() {
        let diagnostics = check(&project(
            "nested-block",
            r#"<scroll-view type="nested"><block wx:if="{{ok}}"><nested-scroll-header/></block><block wx:else><nested-scroll-body/></block></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn nested_parts_with_dynamic_type_are_skipped() {
        let diagnostics = check(&project(
            "nested-dynamic",
            r#"<scroll-view type="{{t}}"><nested-scroll-header/><nested-scroll-body/></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn draggable_sheet_scroll_with_container_is_allowed() {
        let diagnostics = check(&project(
            "draggable-ok",
            r#"<draggable-sheet><scroll-view type="list" associative-container="draggable-sheet"/></draggable-sheet>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn scroll_view_outside_draggable_sheet_is_allowed() {
        let diagnostics = check(&project(
            "draggable-outside",
            r#"<view><scroll-view type="list"/></view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn plain_list_scroll_page_is_allowed() {
        let diagnostics = check(&project(
            "plain",
            r#"<scroll-view type="list"><view wx:for="{{list}}">{{item}}</view></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    // ---- 页面 + 组件 ----

    #[test]
    fn pages_and_components_are_both_checked() {
        let fixture = Fixture::new("page-and-component");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                r#"{"usingComponents":{"my-box":"/components/box/box"}}"#,
            )
            .write(
                "pages/index/index.wxml",
                r#"<scroll-view type="list"><sticky-section/></scroll-view>"#,
            )
            .write("components/box/box.json", r#"{"component":true}"#)
            .write("components/box/box.wxml", "<view><list-view/></view>");

        let diagnostics = check(&fixture);
        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
        let paths: Vec<_> = diagnostics.iter().map(|d| d.path.as_str()).collect();
        assert!(paths.contains(&"pages/index/index.wxml"));
        assert!(paths.contains(&"components/box/box.wxml"));
    }

    // ---- 容错 ----

    #[test]
    fn broken_file_does_not_stop_other_files() {
        let fixture = Fixture::new("scroll-broken");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/bad/bad","pages/good/good"],"renderer":"skyline"}"#,
            )
            .write("pages/bad/bad.json", "{}")
            .write("pages/bad/bad.wxml", "<view>{{ a</view>")
            .write("pages/good/good.json", "{}")
            .write("pages/good/good.wxml", "<view><list-view/></view>");

        let diagnostics = check(&fixture);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].path, "pages/good/good.wxml");
    }
}
