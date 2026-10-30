//! WXML 结构约束引擎：关系词汇表、遍历与分派。
//!
//! 三条结构规则（scroll-view 家族、组件子节点、内联文本）共用这里的节点关系
//! 判定与遍历；各规则只保留自己的约束表与规则实现。
//!
//! # 关系语义
//!
//! - `Direct`：直接父节点必须是目标之一（渲染树，block/wx:if/wx:for 已摊平）
//! - `FirstChild`：父节点是目标容器时，子标签必须是它的第一个元素子节点
//! - `Unique`：最近的目标容器内，子标签最多出现一次（多余的不会生效）
//! - `RendersFirst`：子标签只渲染第一个直接子元素，多写的会被静默丢弃
//! - `RequiresAttr { name, unless }`：子标签必须显式声明 `name`；`unless` 为
//!   `(attr, v)` 时，自身 `<attr>=<v>` 不再要求（典型：list-builder 的 child-height
//!   只在定高模式必填）
//! - `AllowedValues`：子标签该属性只能取这些静态值；未声明不报
//! - `Within`：最近的该容器祖先下，子标签必须声明 `name=value`
//! - `ChildrenOnly`：直接元素子节点只能是这些标签。`text` 是特殊值，表示文本节点也允许
//! - `Exactly`：直接元素子节点个数必须正好是 n；文本子节点（含空白）不计
//! - `RequiresChild`：自身条件全部成立时，直接子节点里必须有一个该属性取到目标值的节点
//! - `RequiresChildTag`：自身条件全部成立时，直接子节点里必须有一个该标签的节点
//! - 目标写法：`Target { tag, attr }`，多个目标直接列成数组
//!
//! # 条件分支
//!
//! 同一个 `wx:if` 节点的不同分支互斥（运行时只会渲染其中一个），因此
//!   first-child / unique 比较兄弟时会跳过与当前节点互斥的候选：不同分支不算
//!   「排在前面」、不算重复、不算顺序违规。两个独立的 `wx:if` 之间不互斥，仍按原样判定；
//!   无条件元素与条件分支元素也不互斥（两者可能同时渲染）。
//!
//! `Exactly` 数的是「能同时渲染的子元素」的最大值：同一个 `wx:if` 的不同分支互斥，
//! 取各分支里最多的那一支；不同分支组之间相加。

use crate::diag::{Diagnostic, Severity};
use crate::input::wxml::{WxmlAttribute, WxmlDoc, WxmlElement, WxmlNode};
use crate::wxml_graph::WxmlGraph;

use super::branch::{element_children, exclusive, max_rendered_children};
use super::{element_diagnostic, is_builtin};

/// 一个允许的父/祖先节点，`attr` 是该节点必须满足的属性。
#[derive(Debug, Clone, Copy)]
pub(crate) struct Target {
    pub(crate) tag: &'static str,
    pub(crate) attr: Option<(&'static str, &'static str)>,
}

/// 一个触发条件。
#[derive(Debug, Clone, Copy)]
pub(crate) enum Condition {
    /// `attr="value"`：静态值与 value 相等（忽略大小写）才算成立。
    Equals(&'static str, &'static str),
    /// `attr`：属性存在且未被静态置为假，即 `attr=!(false|0)`——WXML 布尔属性的写法
    /// 太多（`foo`、`foo=""`、`"{{true}}"`），静态阶段无法区分 `"{{true}}"` 与
    /// `"{{value}}"`，所以只排除「确实没开」。
    Present(&'static str),
}

/// 子标签与目标之间的关系，目标容器直接内联在变体里。
#[derive(Debug)]
pub(crate) enum Relation {
    /// 直接父节点必须是目标之一。
    Direct(&'static [Target]),
    /// 最近的同名祖先必须是目标，并满足目标上的属性要求。
    /// 父节点是目标容器时，子节点必须是它的第一个元素子节点。
    FirstChild(&'static [Target]),
    /// 最近的目标容器内，子节点最多出现一次。
    Unique(&'static [Target]),
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
        values: &'static [&'static str],
    },
    /// 最近的该容器祖先下，子节点必须声明 `name="value"`。
    Within {
        container: Target,
        name: &'static str,
        value: &'static str,
    },
    /// 直接元素子节点只能是这些标签。`text` 是特殊值，表示文本节点也算合法。
    ChildrenOnly(&'static [&'static str]),
    /// 直接元素子节点个数必须正好是 n；文本子节点不计。
    Exactly(u32),
    /// 自身条件成立时，直接子节点里必须有 `child_attr=child_value`。
    RequiresChild {
        child_attr: &'static str,
        child_value: &'static str,
        /// 多个条件必须同时成立。
        own: &'static [Condition],
    },
    /// 自身条件成立时，直接子节点里必须有一个 `child_tag` 标签的节点。
    RequiresChildTag {
        child_tag: &'static str,
        /// 多个条件必须同时成立。
        own: &'static [Condition],
    },
}

/// 一条结构约束：命中的标签、关系、严重级别与修复建议。
pub(crate) struct Constraint {
    pub(crate) tag: &'static str,
    pub(crate) relation: Relation,
    pub(crate) severity: Severity,
    pub(crate) help: &'static str,
}

pub(crate) enum AttrCheck {
    Ok,
    Violation,
    /// 动态绑定，静态阶段无法判定。
    Unknown,
}

pub(crate) fn find_attr<'a>(
    element: &'a WxmlElement,
    name: &str,
) -> Option<&'a crate::input::wxml::WxmlAttribute> {
    element
        .attributes
        .iter()
        .find(|attr| attr.name.eq_ignore_ascii_case(name))
}

pub(crate) fn check_attr(element: &WxmlElement, name: &str, expected: &str) -> AttrCheck {
    check_attributes(&element.attributes, name, expected)
}

fn check_attributes(attributes: &[WxmlAttribute], name: &str, expected: &str) -> AttrCheck {
    let found = attributes
        .iter()
        .find(|attr| attr.name.eq_ignore_ascii_case(name));
    match found {
        None => AttrCheck::Violation,
        Some(attr) if !attr.is_static => AttrCheck::Unknown,
        Some(attr) => match attr.value.as_deref() {
            Some(value) if value.trim().eq_ignore_ascii_case(expected) => AttrCheck::Ok,
            _ => AttrCheck::Violation,
        },
    }
}

/// 逻辑祖先：同一文件里的祖先，或经 `<slot>` / 组件边界投影来的祖先。
///
/// 只带标签与属性：跨文件之后没有「兄弟」「子节点」这些本地概念，
/// [`Relation::FirstChild`] / [`Relation::Unique`] 仍只看本文件内的真实父节点。
pub(crate) struct Ancestor<'a> {
    tag: &'a str,
    attributes: &'a [WxmlAttribute],
}

impl<'a> Ancestor<'a> {
    fn local(element: &'a WxmlElement) -> Self {
        Self {
            tag: &element.tag,
            attributes: &element.attributes,
        }
    }

    fn projected(flat: &'a crate::wxml_graph::FlatElement) -> Self {
        Self {
            tag: &flat.tag,
            attributes: &flat.attributes,
        }
    }
}

/// 属性是否被明确关掉：缺失，或静态假值（`false` / `0`）。
/// 动态值（`{{...}}`，包括 `{{true}}`）无法区分，按「可能开启」处理。
fn attr_is_off(element: &WxmlElement, name: &str) -> bool {
    match find_attr(element, name) {
        None => true,
        Some(attr) if attr.is_static => attr
            .value
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| value.eq_ignore_ascii_case("false") || value == "0"),
        Some(_) => false,
    }
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

fn describe_ancestor(ancestor: &Ancestor<'_>, name: &str) -> String {
    let value = ancestor
        .attributes
        .iter()
        .find(|attr| attr.name.eq_ignore_ascii_case(name))
        .and_then(|attr| attr.value.as_deref());
    match value {
        Some(value) => format!("<{} {}=\"{}\">", ancestor.tag, name, value),
        None => format!("<{}>", ancestor.tag),
    }
}

/// 本文件内的最近同名容器（需要它的子节点列表，比如 `FirstChild` / `Unique`）。
fn nearest_container_local<'a>(
    ancestors: &[&'a WxmlElement],
    containers: &[Target],
) -> Option<&'a WxmlElement> {
    for ancestor in ancestors.iter().rev() {
        if !is_builtin(&ancestor.tag) {
            return None;
        }
        if let Some(target) = containers.iter().find(|target| target.tag == ancestor.tag) {
            return match target.attr {
                None => Some(*ancestor),
                Some((name, value)) => match check_attr(ancestor, name, value) {
                    AttrCheck::Ok => Some(*ancestor),
                    AttrCheck::Violation | AttrCheck::Unknown => None,
                },
            };
        }
    }
    None
}

/// 父节点是否属于目标容器（属性不符不算，动态绑定不阻断）。
fn is_container_element(element: &WxmlElement, containers: &[Target]) -> bool {
    containers.iter().any(|target| match target.attr {
        None => target.tag == element.tag,
        Some((name, value)) => {
            target.tag == element.tag
                && !matches!(check_attr(element, name, value), AttrCheck::Violation)
        }
    })
}

/// 祖先链在这一位断掉：自定义组件的渲染树没解析出来（投影没接上），
/// 它之后的祖先静态不可知，不能继续往外判。
fn boundary_unresolved(ancestors: &[Ancestor<'_>], idx: usize, open: bool) -> bool {
    open && idx + 1 == ancestors.len()
}

/// 祖先链是否在这一位断掉（渲染树不可知）。
///
/// 没接上投影（`<import>` 片段、解析不出的自定义组件）时沿用旧口径：遇到自定义
/// 组件就停；投影接上了才允许穿过去。
fn stops_here(ancestors: &[Ancestor<'_>], idx: usize, from_graph: bool, open: bool) -> bool {
    if !is_builtin(ancestors[idx].tag) {
        return !from_graph || boundary_unresolved(ancestors, idx, open);
    }
    false
}

/// 最近的同名容器祖先；遇到自定义组件（渲染树未知）或属性不符就停。
///
/// `ancestors` 是**最近的在前**：本文件内的父节点在前，`<slot>` 投影进来的在后。
fn nearest_container<'a>(
    ancestors: &'a [Ancestor<'a>],
    from_graph: bool,
    open: bool,
    containers: &[Target],
) -> Option<&'a Ancestor<'a>> {
    for (idx, ancestor) in ancestors.iter().enumerate() {
        if stops_here(ancestors, idx, from_graph, open) {
            return None;
        }
        if ancestor.tag == "slot" || !is_builtin(ancestor.tag) {
            continue;
        }
        if let Some(target) = containers.iter().find(|target| target.tag == ancestor.tag) {
            return match target.attr {
                None => Some(ancestor),
                Some((name, value)) => match check_attributes(ancestor.attributes, name, value) {
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
    ancestors: &[Ancestor<'_>],
    from_graph: bool,
    open: bool,
    targets: &[Target],
) -> Option<String> {
    let expected = render_targets(targets);
    // `<slot>` 只是渲染位置，不构成盒子，跳过它取真正的父节点。
    let Some((parent_idx, parent)) = ancestors
        .iter()
        .enumerate()
        .find(|(_, ancestor)| ancestor.tag != "slot")
    else {
        // 模板定义体的根节点在实例化处的父节点未知。
        return (!element.in_template()).then(|| {
            format!(
                "`<{}>` 位于文件顶层，必须直接放在 {} 内",
                element.tag, expected
            )
        });
    };
    // 自定义组件的渲染树没解析出来：父节点静态不可知，不越过它判定。
    if stops_here(ancestors, parent_idx, from_graph, open) {
        return None;
    }
    let Some(target) = targets.iter().find(|target| target.tag == parent.tag) else {
        return (!element.in_template()).then(|| {
            format!(
                "`<{}>` 必须直接放在 {} 内，当前父节点是 `<{}>`",
                element.tag, expected, parent.tag
            )
        });
    };
    match target.attr {
        None => None,
        Some((name, value)) => match check_attributes(parent.attributes, name, value) {
            AttrCheck::Violation => Some(format!(
                "`<{}>` 必须直接放在 {} 内，父节点是 {}",
                element.tag,
                expected,
                describe_ancestor(parent, name)
            )),
            _ => None,
        },
    }
}

fn first_child_violation(
    element: &WxmlElement,
    ancestors: &[&WxmlElement],
    containers: &[Target],
) -> Option<String> {
    let parent = ancestors.last()?;
    // 容器祖先存在但中间隔着内置节点：它仍不是容器的直接子元素。
    // （`<block wx:if>` 已被摊平，不会走到这里。）
    if !is_container_element(parent, containers) {
        if element.in_template() {
            return None;
        }
        let container = nearest_container_local(ancestors, containers)?;
        if !is_builtin(&parent.tag) {
            return None;
        }
        return Some(format!(
            "`<{}>` 必须直接放在 `<{}>` 下，当前位于 `<{}>` 内",
            element.tag, container.tag, parent.tag
        ));
    }
    let mut first = None;
    for sibling in element_children(parent) {
        if std::ptr::eq(sibling, element) {
            break;
        }
        // 互斥分支的兄弟不会与它同时渲染，不算排在它前面。
        if exclusive(sibling, element) {
            continue;
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
    let container = nearest_container_local(ancestors, containers)?;
    // 互斥分支里的同标签节点不会与它同时渲染，不计入重复。只数容器的直接子节点：
    // 引擎的 `children.firstWhere`（skyline_scroll_view.dart:1559-1563）与
    // `MultiSliver` 都只看一层。
    let siblings: Vec<&WxmlElement> = element_children(container)
        .filter(|sibling| sibling.tag == element.tag && !exclusive(sibling, element))
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
        "`{}` 内有 {} 个 `<{}>`，只能有一个",
        describe(container, "type"),
        siblings.len(),
        element.tag
    ))
}

fn renders_first_violation(element: &WxmlElement) -> Option<String> {
    if element.unknown_children {
        return None;
    }
    let count = max_rendered_children(element);
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
    ancestors: &[Ancestor<'_>],
    open: bool,
    from_graph: bool,
    container: &Target,
    name: &str,
    value: &str,
) -> Option<String> {
    // 只在该容器祖先下才要求关联属性。
    nearest_container(ancestors, from_graph, open, std::slice::from_ref(container))?;
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

fn condition_text(condition: &Condition) -> String {
    match condition {
        Condition::Equals(name, value) => format!(" {name}=\"{value}\""),
        Condition::Present(name) => format!(" {name}"),
    }
}

/// 文本级组件：WebView 下默认 inline / inline-block（`wx-text`、`wx-image`、
/// `wx-input`、`wx-icon` 与 `<span>`）。它们进到只允许文本的容器里会变成原子的
/// inline widget span（`shadow/shadow_node.cc:206-209`），布局不变，因此不算违规；
/// 块级节点与自定义组件才会从块变成行内。
const INLINE_TEXT_TAGS: &[&str] = &["text", "span", "image", "input", "icon"];

/// 允许的子节点渲染成文档里的写法：`text` 指 `<text>` 组件与裸文本节点，其余按标签名。
fn render_allowed(allowed: &[&str]) -> String {
    allowed
        .iter()
        .map(|name| match *name {
            "text" => "`<text>` 或文本节点".to_string(),
            tag => format!("`<{tag}>`"),
        })
        .collect::<Vec<_>>()
        .join(" 或 ")
}

/// 子节点类约束（`ChildrenOnly` / `Exactly` / `RequiresChild` / `RequiresChildTag`）的诊断。
fn children_violations(
    id: &'static str,
    doc: &WxmlDoc,
    element: &WxmlElement,
    constraint: &Constraint,
    path: &str,
) -> Vec<Diagnostic> {
    let on_element = |message: String| {
        element_diagnostic(id, doc, element, path, constraint.severity, message)
            .help(constraint.help)
    };
    match &constraint.relation {
        Relation::ChildrenOnly(allowed) => element_children(element)
            // 文本子节点不是元素，天然不在候选集里：`text` 只表示它们合法。
            .filter(|child| {
                // `<slot>` 只是投影位置的占位，不渲染成盒子：那里有什么子节点由使用方决定，
                // 静态不可知，按不可判定放过。
                !child.is_slot
                    && !allowed.contains(&child.tag.as_str())
                    && !INLINE_TEXT_TAGS.contains(&child.tag.as_str())
            })
            .map(|child| {
                element_diagnostic(
                    id,
                    doc,
                    child,
                    path,
                    constraint.severity,
                    format!(
                        "`<{}>` 只能嵌套 {}，当前子节点是 `<{}>`",
                        element.tag,
                        render_allowed(allowed),
                        child.tag
                    ),
                )
                .help(constraint.help)
            })
            .collect(),
        Relation::Exactly(expected) => {
            // 子节点里有 `<slot>`：真实数量取决于使用方投影，静态不可知。
            if element_children(element).any(|child| child.is_slot) {
                return Vec::new();
            }
            let count = max_rendered_children(element);
            // 有 `<template is>` / `<include>` 子节点时展开内容不可知。
            if element.unknown_children && count < *expected as usize {
                return Vec::new();
            }
            // 只有文本子节点也算有响应目标（文本节点能接手势事件），不算漏。
            if count == 0
                && element
                    .children
                    .iter()
                    .any(|child| matches!(child, WxmlNode::Text(text) if text.is_meaningful()))
            {
                return Vec::new();
            }
            if count == *expected as usize {
                return Vec::new();
            }
            let message = if count == 0 {
                format!(
                    "`<{}>` 里没有元素子节点；手势组件不参与布局，事件只能由唯一的直接子节点接收，当前没有响应目标",
                    element.tag
                )
            } else {
                format!(
                    "`<{}>` 有 {count} 个直接子元素；手势组件不参与布局，事件只绑定到唯一的直接子节点",
                    element.tag
                )
            };
            vec![on_element(message)]
        }
        Relation::RequiresChild {
            child_attr,
            child_value,
            own,
        } => {
            // 任一触发条件不成立就不报；动态值无法确认，一律按不成立处理。
            if !conditions_hold(element, own) {
                return Vec::new();
            }
            let mut unknown = false;
            for child in element_children(element) {
                match check_attr(child, child_attr, child_value) {
                    AttrCheck::Ok => return Vec::new(),
                    // 子节点的该属性是动态值：可能是目标节点，静态阶段无法否定。
                    AttrCheck::Unknown => unknown = true,
                    AttrCheck::Violation => {}
                }
            }
            if unknown {
                return Vec::new();
            }
            if element.unknown_children {
                return Vec::new();
            }
            vec![on_element(format!(
                "`<{}{}>` 没有 `{}=\"{}\"` 的直接子节点，自定义刷新节点不会被渲染",
                element.tag,
                own.first().map(condition_text).unwrap_or_default(),
                child_attr,
                child_value
            ))]
        }
        Relation::RequiresChildTag { child_tag, own } => {
            if !conditions_hold(element, own)
                || element.unknown_children
                // 有 `<slot>`：必需的那个子节点可能由使用方投影进来。
                || element_children(element).any(|child| child.is_slot)
                || element_children(element).any(|child| child.tag == *child_tag)
            {
                return Vec::new();
            }
            vec![on_element(format!(
                "`<{}{}>` 没有必需的 `<{child_tag}>` 直接子节点",
                element.tag,
                own.first().map(condition_text).unwrap_or_default()
            ))]
        }
        _ => Vec::new(),
    }
}

/// 触发条件是否全部成立；动态值无法确认，按不成立处理。
fn conditions_hold(element: &WxmlElement, conditions: &[Condition]) -> bool {
    conditions.iter().all(|condition| match condition {
        Condition::Equals(name, value) => matches!(check_attr(element, name, value), AttrCheck::Ok),
        Condition::Present(name) => !attr_is_off(element, name),
    })
}

/// 对单个元素应用约束表，命中的关系直接产出诊断。
///
#[allow(clippy::too_many_arguments)]
///
/// 关系类约束（`Direct` / `Ancestor` / `Within`）看的是**逻辑祖先**：`graph` 把
/// `<slot>` 投影与组件边界接上，元素被传进组件后仍能问「渲染树里它在谁里面」。
/// `FirstChild` / `Unique` 要枚举子节点，跨文件没有这个信息，只看本文件。
pub(crate) fn check_element(
    id: &'static str,
    doc: &WxmlDoc,
    element: &WxmlElement,
    ancestors: &[&WxmlElement],
    graph: &WxmlGraph,
    constraints: &[Constraint],
    path: &str,
    out: &mut Vec<Diagnostic>,
) {
    // 被 import/include 的片段：根节点插到哪由实例化处决定，位置类约束无从判定。
    let fragment_root = doc.fragment && ancestors.is_empty();
    let chain = graph.ancestors(path, element);
    let from_graph = graph.contains(path);
    let (projected, open): (Vec<Ancestor<'_>>, bool) = if from_graph {
        (
            chain
                .nodes
                .iter()
                .map(|(_, flat)| Ancestor::projected(flat))
                .collect(),
            chain.open,
        )
    } else {
        (
            ancestors
                .iter()
                .rev()
                .map(|el| Ancestor::local(el))
                .collect(),
            false,
        )
    };
    for constraint in constraints.iter().filter(|c| c.tag == element.tag) {
        if fragment_root
            && matches!(
                &constraint.relation,
                Relation::Direct(_) | Relation::FirstChild(_) | Relation::Unique(_)
            )
        {
            continue;
        }
        match &constraint.relation {
            Relation::ChildrenOnly(_)
            | Relation::Exactly(_)
            | Relation::RequiresChild { .. }
            | Relation::RequiresChildTag { .. } => {
                out.extend(children_violations(id, doc, element, constraint, path));
                continue;
            }
            _ => {}
        }
        let message = match &constraint.relation {
            Relation::Direct(targets) => {
                direct_violation(element, &projected, from_graph, open, targets)
            }
            Relation::FirstChild(targets) => {
                // 重复节点已由 unique 报在同一个元素上，不再叠一条。
                let duplicate = constraints.iter().any(|other| {
                    if other.tag != element.tag {
                        return false;
                    }
                    let Relation::Unique(targets) = &other.relation else {
                        return false;
                    };
                    unique_violation(element, ancestors, targets).is_some()
                });
                if duplicate {
                    None
                } else {
                    first_child_violation(element, ancestors, targets)
                }
            }
            Relation::Unique(targets) => unique_violation(element, ancestors, targets),
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
            } => within_violation(
                element, &projected, open, from_graph, container, name, value,
            ),
            Relation::ChildrenOnly(_)
            | Relation::Exactly(_)
            | Relation::RequiresChild { .. }
            | Relation::RequiresChildTag { .. } => {
                unreachable!("子节点类约束已在上面处理")
            }
        };
        if let Some(message) = message {
            out.push(
                element_diagnostic(id, doc, element, path, constraint.severity, message)
                    .help(constraint.help),
            );
        }
    }
}
