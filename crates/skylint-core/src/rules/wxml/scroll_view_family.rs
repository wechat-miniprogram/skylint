//! `scroll-view` 家族组件之间的结构约束：挂载位置、子节点顺序与唯一性、必需属性、
//! 关联容器。约束写在 `data/scroll_family.txt` 里，Rust 侧只按关系类型解释表格。
//!
//! 表格表达不了的否定约束（nested-scroll-header 不能在 body 内）留在代码里。

use crate::diag::{Diagnostic, Severity};
use crate::input::wxml::{WxmlDoc, WxmlElement, WxmlNode};
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};

use super::{element_diagnostic, is_builtin, load_doc, wxml_files};

pub const ID: &str = "skyline/scroll-view-family";

/// 家族结构约束表。
const CONSTRAINTS_DATA: &str = include_str!("data/scroll_family.txt");

/// 一个允许的父/祖先节点，`attr` 是该节点必须满足的属性。
struct Target {
    tag: &'static str,
    attr: Option<(&'static str, &'static str)>,
}

/// 子标签与目标之间的关系。表格里「关系」列的值决定「目标」列怎么读。
enum Relation {
    /// 直接父节点必须是目标之一。
    Direct,
    /// 最近的同名祖先必须是目标，并满足目标上的属性要求。
    Ancestor,
    /// 父节点是目标容器时，子节点必须是它的第一个元素子节点。
    FirstChild,
    /// 最近的目标容器内，子节点最多出现一次。
    Unique,
    /// 目标容器内，子节点必须排在同级 `<tag>` 之前。
    Before(&'static str),
    /// 子节点只渲染第一个直接子元素。
    RendersFirst,
    /// 子节点自身必须声明该属性；`unless` 里的属性取该值时不再要求。
    RequiresAttr {
        name: &'static str,
        unless: Option<(&'static str, &'static str)>,
    },
    /// 子节点自身的该属性只能取这些静态值。
    AllowedValues {
        name: &'static str,
        values: Vec<&'static str>,
    },
    /// 最近的该容器祖先下，子节点必须声明 `name="value"`。
    Within {
        container: Target,
        name: &'static str,
        value: &'static str,
    },
}

struct Constraint {
    child: &'static str,
    relation: Relation,
    /// `Direct` / `Ancestor` / `FirstChild` / `Unique` / `Before` 用的容器列表。
    containers: Vec<Target>,
    severity: Severity,
    help: &'static str,
}

fn constraints() -> Vec<Constraint> {
    CONSTRAINTS_DATA
        .lines()
        .filter_map(parse_constraint)
        .collect()
}

fn parse_constraint(line: &'static str) -> Option<Constraint> {
    if line.trim().is_empty() || line.starts_with('#') {
        return None;
    }
    let mut fields = line.split('\t').map(str::trim);
    let child = fields.next()?;
    let raw_relation = fields.next()?;
    let target = fields.next()?;
    let severity = match fields.next()? {
        "error" => Severity::Error,
        "warning" => Severity::Warning,
        "info" => Severity::Info,
        _ => return None,
    };
    let help = fields.next()?;
    let (relation, containers) = parse_relation(raw_relation, target)?;
    Some(Constraint {
        child,
        relation,
        containers,
        severity,
        help,
    })
}

fn parse_relation(raw: &'static str, target: &'static str) -> Option<(Relation, Vec<Target>)> {
    let (relation, containers) = match raw.split_once(':') {
        Some(("before", sibling)) => (Relation::Before(sibling), parse_targets(target)),
        Some(("within", container)) => {
            let (name, value) = split_attr(target);
            (
                Relation::Within {
                    container: parse_target(container),
                    name,
                    value: value?,
                },
                Vec::new(),
            )
        }
        Some(("requires-attr-unless", condition)) => {
            let (attribute, value) = condition.split_once('=')?;
            (
                Relation::RequiresAttr {
                    name: split_attr(target).0,
                    unless: Some((attribute.trim(), value.trim())),
                },
                Vec::new(),
            )
        }
        Some(_) => return None,
        None => match raw {
            "direct" => (Relation::Direct, parse_targets(target)),
            "ancestor" => (Relation::Ancestor, parse_targets(target)),
            "first-child" => (Relation::FirstChild, parse_targets(target)),
            "unique" => (Relation::Unique, parse_targets(target)),
            "renders-first" => (Relation::RendersFirst, Vec::new()),
            "requires-attr" => (
                Relation::RequiresAttr {
                    name: split_attr(target).0,
                    unless: None,
                },
                Vec::new(),
            ),
            "allowed-values" => {
                let (name, values) = target.split_once('=')?;
                (
                    Relation::AllowedValues {
                        name,
                        values: values.split('|').collect(),
                    },
                    Vec::new(),
                )
            }
            _ => return None,
        },
    };
    Some((relation, containers))
}

fn parse_targets(raw: &'static str) -> Vec<Target> {
    raw.split('|').map(parse_target).collect()
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

/// `name=value` → (`name`, Some(`value`))；`name` → (`name`, None)。
fn split_attr(raw: &'static str) -> (&'static str, Option<&'static str>) {
    match raw.split_once('=') {
        Some((name, value)) => (name.trim(), Some(value.trim())),
        None => (raw.trim(), None),
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

fn element_children(element: &WxmlElement) -> impl Iterator<Item = &WxmlElement> {
    element.children.iter().filter_map(|node| match node {
        WxmlNode::Element(element) => Some(element),
        _ => None,
    })
}

/// 父节点是否属于目标容器（属性不符不算，动态绑定不阻断）。
fn is_container(element: &WxmlElement, containers: &[Target]) -> bool {
    containers.iter().any(|target| match target.attr {
        None => target.tag == element.tag,
        Some((name, value)) => {
            target.tag == element.tag
                && !matches!(check_attr(element, name, value), AttrCheck::Violation)
        }
    })
}

/// 最近的同名容器祖先；遇到自定义组件（渲染树未知）或属性不符就停。
fn nearest_container<'a>(
    ancestors: &[&'a WxmlElement],
    containers: &[Target],
) -> Option<&'a WxmlElement> {
    for ancestor in ancestors.iter().rev() {
        if !is_builtin(&ancestor.tag) {
            return None;
        }
        if let Some(target) = containers.iter().find(|target| target.tag == ancestor.tag) {
            return match target.attr {
                None => Some(ancestor),
                Some((name, value)) => match check_attr(ancestor, name, value) {
                    AttrCheck::Ok => Some(ancestor),
                    AttrCheck::Violation | AttrCheck::Unknown => None,
                },
            };
        }
    }
    None
}

fn direct_violation(
    element: &WxmlElement,
    ancestors: &[&WxmlElement],
    targets: &[Target],
) -> Option<String> {
    let expected = render_targets(targets);
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
    let Some(target) = targets.iter().find(|target| target.tag == parent.tag) else {
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
    targets: &[Target],
) -> Option<String> {
    let expected = render_targets(targets);
    for ancestor in ancestors.iter().rev() {
        // 自定义组件的渲染树未知，不越过它判定。
        if !is_builtin(&ancestor.tag) {
            return None;
        }
        let Some(target) = targets.iter().find(|target| target.tag == ancestor.tag) else {
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

fn first_child_violation(
    element: &WxmlElement,
    ancestors: &[&WxmlElement],
    containers: &[Target],
) -> Option<String> {
    let parent = ancestors.last()?;
    if !is_builtin(&parent.tag) || !is_container(parent, containers) {
        return None;
    }
    let mut first = None;
    for sibling in element_children(parent) {
        if std::ptr::eq(sibling, element) {
            break;
        }
        // 同标签的重复节点交给 unique，避免同一位置两条诊断。
        if sibling.tag == element.tag {
            return None;
        }
        first.get_or_insert(sibling);
    }
    let first = first?;
    Some(format!(
        "`<{}>` 必须是 `<{}>` 的第一个元素子节点，当前前面还有 `<{}>`",
        element.tag, parent.tag, first.tag
    ))
}

fn unique_violation(
    element: &WxmlElement,
    ancestors: &[&WxmlElement],
    containers: &[Target],
) -> Option<String> {
    let container = nearest_container(ancestors, containers)?;
    let siblings: Vec<&WxmlElement> = element_children(container)
        .filter(|sibling| sibling.tag == element.tag)
        .collect();
    if siblings.len() < 2 {
        return None;
    }
    let index = siblings
        .iter()
        .position(|sibling| std::ptr::eq(*sibling, element))?;
    if index == 0 {
        return None;
    }
    Some(format!(
        "`{}` 内有 {} 个 `<{}>`，只有第一个会生效",
        describe(container, "type"),
        siblings.len(),
        element.tag
    ))
}

fn before_violation(
    element: &WxmlElement,
    ancestors: &[&WxmlElement],
    containers: &[Target],
    sibling_tag: &str,
) -> Option<String> {
    let container = nearest_container(ancestors, containers)?;
    let siblings: Vec<&WxmlElement> = element_children(container).collect();
    let this = siblings
        .iter()
        .position(|sibling| std::ptr::eq(*sibling, element))?;
    let other = siblings
        .iter()
        .position(|sibling| sibling.tag == sibling_tag)?;
    if other > this {
        return None;
    }
    Some(format!(
        "`<{}>` 排在 `<{}>` 之后，必须放在同级 `<{}>` 之前",
        element.tag, sibling_tag, sibling_tag
    ))
}

fn renders_first_violation(element: &WxmlElement) -> Option<String> {
    let count = element_children(element).count();
    if count < 2 {
        return None;
    }
    Some(format!(
        "`<{}>` 有 {count} 个直接子元素，只渲染第一个，其余会被静默丢弃",
        element.tag
    ))
}

fn requires_attr_violation(
    element: &WxmlElement,
    name: &str,
    unless: Option<(&str, &str)>,
) -> Option<String> {
    if let Some((attribute, value)) = unless
        && !matches!(check_attr(element, attribute, value), AttrCheck::Violation)
    {
        return None;
    }
    if find_attr(element, name).is_some() {
        return None;
    }
    Some(format!(
        "`<{}>` 未声明 `{name}` 属性，该属性是必须的",
        element.tag
    ))
}

fn allowed_values_violation(element: &WxmlElement, name: &str, values: &[&str]) -> Option<String> {
    let attr = find_attr(element, name)?;
    if !attr.is_static {
        return None;
    }
    let actual = attr.value.as_deref().unwrap_or("").trim();
    if values
        .iter()
        .any(|value| value.eq_ignore_ascii_case(actual))
    {
        return None;
    }
    let allowed = values
        .iter()
        .map(|value| format!("`{value}`"))
        .collect::<Vec<_>>()
        .join(" 或 ");
    Some(format!(
        "`<{} {}=\"{}\">` 的 `{}` 只能是 {allowed}，当前值不生效",
        element.tag, name, actual, name
    ))
}

fn within_violation(
    element: &WxmlElement,
    ancestors: &[&WxmlElement],
    container: &Target,
    name: &str,
    value: &str,
) -> Option<String> {
    // 只在该容器祖先下才要求关联属性。
    nearest_container(ancestors, std::slice::from_ref(container))?;
    let requirement = format!("{name}=\"{value}\"");
    let inside = describe(element, "type");
    match find_attr(element, name) {
        None => Some(format!(
            "`{inside}` 位于 `<{}>` 内，必须设置 `{requirement}`",
            container.tag
        )),
        Some(attr) if !attr.is_static => None,
        Some(attr) => {
            let actual = attr.value.as_deref().unwrap_or("").trim();
            if actual.eq_ignore_ascii_case(value) {
                return None;
            }
            Some(format!(
                "`{inside}` 位于 `<{}>` 内，当前是 `{name}=\"{actual}\"`，必须设置为 `{requirement}`",
                container.tag
            ))
        }
    }
}

pub struct ScrollViewFamily;

impl Rule for ScrollViewFamily {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Error,
            description: "scroll-view 家族组件的结构约束（挂载位置 / 顺序 / 唯一性 / 必需属性 / 关联容器）",
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
        let message = match &constraint.relation {
            Relation::Direct => direct_violation(element, ancestors, &constraint.containers),
            Relation::Ancestor => ancestor_violation(element, ancestors, &constraint.containers),
            Relation::FirstChild => {
                first_child_violation(element, ancestors, &constraint.containers)
            }
            Relation::Unique => unique_violation(element, ancestors, &constraint.containers),
            Relation::Before(sibling) => {
                before_violation(element, ancestors, &constraint.containers, sibling)
            }
            Relation::RendersFirst => renders_first_violation(element),
            Relation::RequiresAttr { name, unless } => {
                requires_attr_violation(element, name, *unless)
            }
            Relation::AllowedValues { name, values } => {
                allowed_values_violation(element, name, values)
            }
            Relation::Within {
                container,
                name,
                value,
            } => within_violation(element, ancestors, container, name, value),
        };
        if let Some(message) = message {
            out.push(
                element_diagnostic(ID, doc, element, path, constraint.severity, message)
                    .help(constraint.help),
            );
        }
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
        assert_eq!(constraints.len(), 17);
        assert!(
            constraints
                .iter()
                .all(|c| !c.child.is_empty() && !c.help.is_empty())
        );
        let list_view = constraints.iter().find(|c| c.child == "list-view").unwrap();
        assert_eq!(list_view.containers.len(), 2);
        assert!(list_view.containers[0].attr.is_some());
        assert!(list_view.containers[1].attr.is_none());
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
        let diagnostics = check(&project(
            "grid-view-view",
            r#"<view><grid-view cross-axis-count="2"/></view>"#,
        ));
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("grid-view"));
    }

    #[test]
    fn grid_view_in_list_scroll_is_reported() {
        let diagnostics = check(&project(
            "grid-view-list",
            r#"<scroll-view type="list"><grid-view cross-axis-count="2"/></scroll-view>"#,
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
            r#"<scroll-view type="custom"><grid-view type="aligned" cross-axis-count="2"/></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn grid_view_under_sticky_section_is_allowed() {
        let diagnostics = check(&project(
            "grid-sticky",
            r#"<scroll-view type="custom"><sticky-section><sticky-header>标题</sticky-header><grid-view cross-axis-count="2"/></sticky-section></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn nested_parts_under_nested_scroll_are_allowed() {
        let diagnostics = check(&project(
            "nested-ok",
            r#"<scroll-view type="nested"><nested-scroll-header><view/></nested-scroll-header><nested-scroll-body><scroll-view type="list" associative-container="nested-scroll-view"/></nested-scroll-body></scroll-view>"#,
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

    // ---- 顺序：sticky-header 必须是 sticky-section 的第一个元素子节点 ----

    #[test]
    fn sticky_header_not_first_in_section_is_reported() {
        let diagnostics = check(&project(
            "sticky-first-view",
            r#"<scroll-view type="custom"><sticky-section><view/><sticky-header>标题</sticky-header></sticky-section></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert!(diagnostics[0].message.contains("第一个元素子节点"));
    }

    #[test]
    fn sticky_header_after_list_view_in_section_is_reported() {
        let diagnostics = check(&project(
            "sticky-first-list",
            r#"<scroll-view type="custom"><sticky-section><list-view><view/></list-view><sticky-header>标题</sticky-header></sticky-section></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("<list-view>"));
    }

    #[test]
    fn sticky_header_first_in_section_is_allowed() {
        let diagnostics = check(&project(
            "sticky-first-ok",
            r#"<scroll-view type="custom"><sticky-section><sticky-header>标题</sticky-header><list-view><view/></list-view></sticky-section></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn sticky_header_first_after_if_block_is_allowed() {
        // block 已摊平：header 仍是 sticky-section 的第一个元素子节点。
        let diagnostics = check(&project(
            "sticky-first-if",
            r#"<scroll-view type="custom"><sticky-section><block wx:if="{{ok}}"><sticky-header>标题</sticky-header></block><list-view><view/></list-view></sticky-section></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn sticky_header_first_after_for_block_is_allowed() {
        let diagnostics = check(&project(
            "sticky-first-for",
            r#"<scroll-view type="custom"><sticky-section><block wx:for="{{list}}"><sticky-header>标题</sticky-header></block><view/></sticky-section></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// 这正是当初撤销「必须在 sticky-section 内」的场景：两种直接子节点都要放过。
    #[test]
    fn sticky_header_direct_child_of_custom_scroll_is_allowed() {
        let diagnostics = check(&project(
            "sticky-first-scroll",
            r#"<scroll-view type="custom"><sticky-header>标题</sticky-header><sticky-section><sticky-header>标题</sticky-header></sticky-section></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    // ---- 唯一性：每个 sticky-section 最多一个 sticky-header ----

    #[test]
    fn two_sticky_headers_in_section_is_reported() {
        let diagnostics = check(&project(
            "sticky-unique",
            r#"<scroll-view type="custom"><sticky-section><sticky-header>一</sticky-header><sticky-header>二</sticky-header></sticky-section></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert!(diagnostics[0].message.contains("2 个 `<sticky-header>`"));
        assert!(diagnostics[0].message.contains("只有第一个会生效"));
    }

    #[test]
    fn three_sticky_headers_in_section_report_each_extra() {
        let diagnostics = check(&project(
            "sticky-unique-three",
            r#"<scroll-view type="custom"><sticky-section><sticky-header>一</sticky-header><sticky-header>二</sticky-header><sticky-header>三</sticky-header></sticky-section></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
        assert!(
            diagnostics
                .iter()
                .all(|d| d.message.contains("3 个 `<sticky-header>`"))
        );
    }

    #[test]
    fn single_sticky_header_is_allowed() {
        let diagnostics = check(&project(
            "sticky-unique-one",
            r#"<scroll-view type="custom"><sticky-section><sticky-header>标题</sticky-header></sticky-section></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn sticky_headers_in_separate_sections_are_allowed() {
        let diagnostics = check(&project(
            "sticky-unique-separate",
            r#"<scroll-view type="custom"><sticky-section><sticky-header>一</sticky-header></sticky-section><sticky-section><sticky-header>二</sticky-header></sticky-section></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    // ---- 唯一性：外层最多一个 nested-scroll-body ----

    #[test]
    fn two_nested_bodies_is_reported() {
        let diagnostics = check(&project(
            "nested-body-unique",
            r#"<scroll-view type="nested"><nested-scroll-body><view/></nested-scroll-body><nested-scroll-body><view/></nested-scroll-body></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert!(
            diagnostics[0]
                .message
                .contains("2 个 `<nested-scroll-body>`")
        );
    }

    #[test]
    fn three_nested_bodies_report_each_extra() {
        let diagnostics = check(&project(
            "nested-body-unique-three",
            concat!(
                r#"<scroll-view type="nested"><nested-scroll-body><view/></nested-scroll-body>"#,
                r#"<nested-scroll-body><view/></nested-scroll-body>"#,
                r#"<nested-scroll-body><view/></nested-scroll-body></scroll-view>"#,
            ),
        ));
        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
    }

    #[test]
    fn nested_bodies_in_separate_scrolls_are_allowed() {
        let diagnostics = check(&project(
            "nested-body-unique-separate",
            concat!(
                r#"<scroll-view type="nested"><nested-scroll-body><view/></nested-scroll-body></scroll-view>"#,
                r#"<scroll-view type="nested"><nested-scroll-body><view/></nested-scroll-body></scroll-view>"#,
            ),
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn nested_bodies_with_dynamic_type_are_skipped() {
        let diagnostics = check(&project(
            "nested-body-unique-dynamic",
            r#"<scroll-view type="{{t}}"><nested-scroll-body><view/></nested-scroll-body><nested-scroll-body><view/></nested-scroll-body></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    // ---- 顺序：nested-scroll-header 必须排在 nested-scroll-body 之前 ----

    #[test]
    fn nested_header_after_body_is_reported() {
        let diagnostics = check(&project(
            "nested-before",
            r#"<scroll-view type="nested"><nested-scroll-body><view/></nested-scroll-body><nested-scroll-header><view/></nested-scroll-header></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert!(diagnostics[0].message.contains("必须放在同级"));
    }

    #[test]
    fn nested_header_after_body_with_sibling_between_is_reported() {
        let diagnostics = check(&project(
            "nested-before-between",
            r#"<scroll-view type="nested"><nested-scroll-body><view/></nested-scroll-body><view/><nested-scroll-header><view/></nested-scroll-header></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    #[test]
    fn nested_header_before_body_is_allowed() {
        let diagnostics = check(&project(
            "nested-before-ok",
            r#"<scroll-view type="nested"><nested-scroll-header><view/></nested-scroll-header><nested-scroll-body><view/></nested-scroll-body></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn nested_body_without_header_is_allowed() {
        let diagnostics = check(&project(
            "nested-before-no-header",
            r#"<scroll-view type="nested"><nested-scroll-body><view/></nested-scroll-body></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn nested_header_without_body_is_allowed() {
        let diagnostics = check(&project(
            "nested-before-no-body",
            r#"<scroll-view type="nested"><nested-scroll-header><view/></nested-scroll-header></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    // ---- 只渲染第一个直接子节点 ----

    #[test]
    fn nested_header_with_multiple_children_is_reported() {
        let diagnostics = check(&project(
            "nested-header-multi",
            r#"<scroll-view type="nested"><nested-scroll-header><view/><view/></nested-scroll-header><nested-scroll-body><view/></nested-scroll-body></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].severity, Severity::Warning);
        assert!(diagnostics[0].message.contains("2 个直接子元素"));
    }

    #[test]
    fn nested_body_with_multiple_children_is_reported() {
        let diagnostics = check(&project(
            "nested-body-multi",
            r#"<scroll-view type="nested"><nested-scroll-header><view/></nested-scroll-header><nested-scroll-body><view/><view/></nested-scroll-body></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].severity, Severity::Warning);
        assert!(diagnostics[0].message.contains("nested-scroll-body"));
    }

    #[test]
    fn nested_parts_with_single_child_are_allowed() {
        let diagnostics = check(&project(
            "nested-parts-single",
            r#"<scroll-view type="nested"><nested-scroll-header><view/></nested-scroll-header><nested-scroll-body><view/></nested-scroll-body></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn nested_body_with_nested_grandchildren_is_allowed() {
        // 只数直接元素子节点，孙节点不算。
        let diagnostics = check(&project(
            "nested-body-grandchildren",
            r#"<scroll-view type="nested"><nested-scroll-header><view><view/><view/></view></nested-scroll-header><nested-scroll-body><view><view/><view/></view></nested-scroll-body></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    // ---- 必需属性 ----

    #[test]
    fn list_builder_without_child_height_is_reported() {
        let diagnostics = check(&project(
            "list-builder-attr",
            "<view><list-builder/></view>",
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert!(diagnostics[0].message.contains("child-height"));
    }

    #[test]
    fn list_builder_with_type_but_without_child_height_is_reported() {
        let diagnostics = check(&project(
            "list-builder-attr-static",
            r#"<view><list-builder type="static"/></view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    #[test]
    fn list_builder_with_child_height_is_allowed() {
        let diagnostics = check(&project(
            "list-builder-attr-ok",
            r#"<view><list-builder child-height="80"/></view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn list_builder_with_dynamic_child_height_is_skipped() {
        let diagnostics = check(&project(
            "list-builder-attr-dynamic",
            r#"<view><list-builder child-height="{{h}}"/></view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn grid_view_without_cross_axis_count_is_reported() {
        let diagnostics = check(&project(
            "grid-view-attr",
            r#"<scroll-view type="custom"><grid-view/></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert!(diagnostics[0].message.contains("cross-axis-count"));
    }

    #[test]
    fn grid_view_in_sticky_section_without_cross_axis_count_is_reported() {
        let diagnostics = check(&project(
            "grid-view-attr-sticky",
            r#"<scroll-view type="custom"><sticky-section><grid-view/></sticky-section></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    #[test]
    fn grid_view_with_cross_axis_count_is_allowed() {
        let diagnostics = check(&project(
            "grid-view-attr-ok",
            r#"<scroll-view type="custom"><grid-view cross-axis-count="3"/></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn grid_view_with_dynamic_cross_axis_count_is_skipped() {
        let diagnostics = check(&project(
            "grid-view-attr-dynamic",
            r#"<scroll-view type="custom"><grid-view cross-axis-count="{{n}}"/></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    // ---- 属性取值 ----

    #[test]
    fn list_builder_with_unknown_type_is_reported() {
        let diagnostics = check(&project(
            "list-builder-type",
            r#"<view><list-builder child-height="80" type="flex"/></view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert!(diagnostics[0].message.contains("`static`"));
        assert!(diagnostics[0].message.contains("`dynamic`"));
    }

    #[test]
    fn list_builder_with_unknown_type_variant_is_reported() {
        let diagnostics = check(&project(
            "list-builder-type-b",
            r#"<view><list-builder child-height="80" type="virtual"/></view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    #[test]
    fn list_builder_with_allowed_types_is_allowed() {
        for (name, wxml) in [
            (
                "list-builder-type-static",
                r#"<view><list-builder child-height="80" type="static"/></view>"#,
            ),
            (
                "list-builder-type-dynamic",
                r#"<view><list-builder type="dynamic"/></view>"#,
            ),
            (
                "list-builder-type-default",
                r#"<view><list-builder child-height="80"/></view>"#,
            ),
        ] {
            let diagnostics = check(&project(name, wxml));
            assert!(diagnostics.is_empty(), "{name}: {diagnostics:#?}");
        }
    }

    #[test]
    fn list_builder_with_dynamic_type_is_skipped() {
        let diagnostics = check(&project(
            "list-builder-type-dynamic-binding",
            r#"<view><list-builder child-height="80" type="{{t}}"/></view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn grid_view_with_unknown_type_is_reported() {
        let diagnostics = check(&project(
            "grid-view-type",
            r#"<scroll-view type="custom"><grid-view cross-axis-count="2" type="grid"/></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("`masonry`"));
    }

    #[test]
    fn grid_view_with_unknown_type_variant_is_reported() {
        let diagnostics = check(&project(
            "grid-view-type-b",
            r#"<scroll-view type="custom"><grid-view cross-axis-count="2" type="waterfall"/></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    #[test]
    fn grid_view_with_allowed_types_is_allowed() {
        for (name, wxml) in [
            (
                "grid-view-type-aligned",
                r#"<scroll-view type="custom"><grid-view cross-axis-count="2" type="aligned"/></scroll-view>"#,
            ),
            (
                "grid-view-type-masonry",
                r#"<scroll-view type="custom"><grid-view cross-axis-count="2" type="masonry"/></scroll-view>"#,
            ),
        ] {
            let diagnostics = check(&project(name, wxml));
            assert!(diagnostics.is_empty(), "{name}: {diagnostics:#?}");
        }
    }

    #[test]
    fn grid_view_with_dynamic_type_is_skipped() {
        let diagnostics = check(&project(
            "grid-view-type-dynamic",
            r#"<scroll-view type="custom"><grid-view cross-axis-count="2" type="{{t}}"/></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    // ---- 关联容器：nested-scroll-body 内层 scroll-view ----

    #[test]
    fn inner_scroll_without_associative_container_is_reported() {
        let diagnostics = check(&project(
            "nested-assoc",
            r#"<scroll-view type="nested"><nested-scroll-body><scroll-view type="list"/></nested-scroll-body></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert!(diagnostics[0].message.contains("associative-container"));
        assert!(diagnostics[0].message.contains("nested-scroll-view"));
    }

    #[test]
    fn deeply_nested_inner_scroll_without_associative_container_is_reported() {
        let diagnostics = check(&project(
            "nested-assoc-deep",
            r#"<scroll-view type="nested"><nested-scroll-body><swiper><swiper-item><scroll-view type="list"/></swiper-item></swiper></nested-scroll-body></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    #[test]
    fn inner_scroll_with_wrong_associative_container_is_reported() {
        let diagnostics = check(&project(
            "nested-assoc-wrong",
            r#"<scroll-view type="nested"><nested-scroll-body><scroll-view type="list" associative-container="draggable-sheet"/></nested-scroll-body></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("当前是"));
    }

    #[test]
    fn inner_scroll_with_associative_container_is_allowed() {
        let diagnostics = check(&project(
            "nested-assoc-ok",
            r#"<scroll-view type="nested"><nested-scroll-body><scroll-view type="list" associative-container="nested-scroll-view"/></nested-scroll-body></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn inner_scroll_with_dynamic_associative_container_is_skipped() {
        let diagnostics = check(&project(
            "nested-assoc-dynamic",
            r#"<scroll-view type="nested"><nested-scroll-body><scroll-view type="list" associative-container="{{c}}"/></nested-scroll-body></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn inner_scroll_under_if_block_is_allowed() {
        let diagnostics = check(&project(
            "nested-assoc-block",
            r#"<scroll-view type="nested"><nested-scroll-body><block wx:if="{{ok}}"><scroll-view type="list" associative-container="nested-scroll-view"/></block></nested-scroll-body></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn scroll_view_in_nested_header_is_allowed() {
        // header 内的 scroll-view 不参与嵌套滚动（sticky 的横向 tab 列表就是这种写法）。
        let diagnostics = check(&project(
            "nested-assoc-header",
            r#"<scroll-view type="nested"><nested-scroll-header><sticky-header><scroll-view type="list" scroll-x/></sticky-header></nested-scroll-header><nested-scroll-body><view/></nested-scroll-body></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn inner_scroll_in_body_under_dynamic_type_is_still_reported() {
        // 约束条件是有没有 nested-scroll-body 祖先，与外壳 type 无关：外壳动态绑定也可能是 nested。
        let diagnostics = check(&project(
            "nested-assoc-type-dynamic",
            r#"<scroll-view type="{{t}}"><nested-scroll-body><scroll-view type="list"/></nested-scroll-body></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    // ---- 关联容器：draggable-sheet 内的 scroll-view ----

    #[test]
    fn draggable_sheet_scroll_with_dynamic_container_is_skipped() {
        let diagnostics = check(&project(
            "draggable-dynamic",
            r#"<draggable-sheet><scroll-view type="list" associative-container="{{c}}"/></draggable-sheet>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }
}
