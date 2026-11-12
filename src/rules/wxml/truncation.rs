//! WebView 侧的截断语义：多行 `-webkit-line-clamp`，单行
//! `white-space: nowrap` + 裁切 + `text-overflow: ellipsis`。
//!
//! 迁移相关的两条规则都靠它做准入：「WebView 本来就会截断、Skyline 截不了」才算差异；
//! WebView 也不截断时两端一致，不该报。祖先被 clamp 时内层文本同样被裁掉，所以判定
//! 要连祖先一起看——模板元素还要看**实例化处**的祖先（[`DocViews`]）。
//!
//! 一律按 WebView 语义求值：Skyline 丢弃属性条件，用它判断「WebView 会不会截断」会
//! 给出错的取值。

use crate::input::wxml::WxmlElement;
use crate::matcher::{Dialect, Tree};
use crate::project::Project;
use crate::wxml_graph::AncestorId;

use super::ancestry::{Ancestors, DocViews, Mode, aggregate, property_at};
use super::resolved_property_with_dialect;

/// 一个属性的读取结果：`Some((取值, 是否判不了))`。
///
/// 取值为空串表示「这份文档里确定没写」。
pub(crate) type Reading = Option<(String, bool)>;

/// 把「某个元素上某属性」包成读取器。
fn resolved_reader<'a>(
    project: &'a Project,
    tree: &'a Tree<'_>,
    element: &'a WxmlElement,
) -> impl Fn(&str) -> Reading + 'a {
    move |property| {
        let (resolved, undecidable) =
            resolved_property_with_dialect(project, tree, element, property, Dialect::WebView);
        Some((
            resolved.map(|resolved| resolved.value).unwrap_or_default(),
            undecidable,
        ))
    }
}

/// 元素自身与单文件祖先是否被 WebView 截断。
///
/// `Some(true)` 是证据，`Some(false)` 是「都确定不截断」，`None` 是判不了。
pub(crate) fn webview_truncates_local(
    project: &Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
    ancestors: &[&WxmlElement],
) -> Option<bool> {
    let mut unknown = false;
    for candidate in std::iter::once(element).chain(ancestors.iter().rev().copied()) {
        match truncates(&resolved_reader(project, tree, candidate)) {
            Some(true) => return Some(true),
            Some(false) => {}
            None => unknown = true,
        }
    }
    (!unknown).then_some(false)
}

/// 元素是否在 WebView 里被截断：自身、单文件祖先，或实例化处的祖先。
///
/// 模板定义体 / 片段渲染在实例化位置，真实祖先在别的文档里，要走
/// [`WxmlGraph::ancestor_closure`](crate::wxml_graph::WxmlGraph::ancestor_closure)：
/// 「祖先里有没有截断属性」是可达性问题，按节点去重后不随实例化路径数指数增长。
pub(crate) fn webview_truncates(
    views: &DocViews<'_>,
    tree: &Tree<'_>,
    element: &WxmlElement,
    ancestors: &[&WxmlElement],
) -> Option<bool> {
    let local = webview_truncates_local(views.project(), tree, element, ancestors);
    if local == Some(true) {
        return Some(true);
    }
    let unknown = local.is_none();
    if !tree.ancestors_unknown(element) {
        return (!unknown).then_some(false);
    }
    let closure = views.graph().ancestor_closure(tree.path(), element);
    match aggregate(
        Mode::Any,
        Ancestors::Closure {
            nodes: &closure.0,
            truncated: closure.1,
        },
        |node| any_hop_truncates(views, node),
    ) {
        Some(true) => Some(true),
        None => None,
        Some(false) if unknown => None,
        Some(false) => Some(false),
    }
}

/// 自身或祖先最近的 `-webkit-line-clamp` 行数（多行截断的行数来源）。
///
/// 多条实例化路径给出不同行数时返回 `None`：消息只能写一个数字，写错比不写更糟。
pub(crate) fn clamp_lines(
    views: &DocViews<'_>,
    tree: &Tree<'_>,
    element: &WxmlElement,
    ancestors: &[&WxmlElement],
) -> Option<u32> {
    let local = std::iter::once(element)
        .chain(ancestors.iter().rev().copied())
        .find_map(|candidate| {
            resolved_property_with_dialect(
                views.project(),
                tree,
                candidate,
                "-webkit-line-clamp",
                Dialect::WebView,
            )
            .0
            .and_then(|resolved| positive_lines(&resolved.value))
        });
    if local.is_some() {
        return local;
    }
    if !tree.ancestors_unknown(element) {
        return None;
    }
    let paths = views.graph().ancestor_ids(tree.path(), element);
    let (paths, truncated) = &*paths;
    if *truncated || paths.is_empty() {
        return None;
    }
    let values: Option<Vec<u32>> = paths
        .iter()
        .map(|path| {
            for hop in path {
                match property_at(views, hop, "-webkit-line-clamp", Dialect::WebView) {
                    // 判不了：这条路径给不出确定的最近行数。
                    Some((_, true)) => return None,
                    Some((value, false)) => {
                        if let Some(lines) = positive_lines(&value) {
                            return Some(lines);
                        }
                    }
                    None => {}
                }
            }
            None
        })
        .collect();
    let values = values?;
    let first = *values.first()?;
    values.iter().all(|lines| *lines == first).then_some(first)
}

/// 元素自己是否在 WebView 里裁掉溢出内容（`text-overflow` 生效的前提）。
///
/// `visible` / 缺省时不裁，`hidden` / `clip` / `auto` / `scroll` 都会裁。
pub(crate) fn clips_overflow(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> bool {
    clips_with(&resolved_reader(project, tree, element)) == Some(true)
}

/// `<text max-lines>` 的行数。
///
/// 静态属性与「包着一个数字字面量的插值」（`max-lines="{{2}}"`）等价：引擎拿到的就是 2。
pub(crate) fn max_lines(element: &WxmlElement) -> Option<u32> {
    let attribute = element
        .attributes
        .iter()
        .find(|attr| attr.name == "max-lines")?;
    // 静态写法直接读原文；`max-lines="{{2}}"` 这类绑定没有原文，退到解析期算出的
    // 候选值——只有一个候选且是整数才算得出（`{{a ? 2 : 3}}` 枚举出两个就不猜）。
    let raw = match attribute.value.as_deref() {
        Some(value) => value.trim().to_string(),
        None => match attribute.candidates.as_deref() {
            Some([only]) => only.trim().to_string(),
            _ => return None,
        },
    };
    raw.strip_prefix("{{")
        .and_then(|rest| rest.strip_suffix("}}"))
        .unwrap_or(&raw)
        .trim()
        .parse::<u32>()
        .ok()
}

fn any_hop_truncates(views: &DocViews<'_>, path: &[AncestorId]) -> Option<bool> {
    let mut unknown = false;
    for hop in path {
        match truncates(&|property| property_at(views, hop, property, Dialect::WebView)) {
            Some(true) => return Some(true),
            Some(false) => {}
            None => unknown = true,
        }
    }
    (!unknown).then_some(false)
}

/// 「WebView 会截断吗」的取值级判定：多行 clamp，或单行 nowrap + 裁切 + 省略号。
fn truncates(read: &impl Fn(&str) -> Reading) -> Option<bool> {
    let clamp = match read("-webkit-line-clamp") {
        Some((value, false)) => Some(positive_lines(&value).is_some()),
        Some((_, true)) => None,
        None => Some(false),
    };
    let single_line = conj([
        is_at(read, "white-space", "nowrap"),
        clips_with(read),
        is_at(read, "text-overflow", "ellipsis"),
    ]);
    disj([clamp, single_line])
}

fn clips_with(read: &impl Fn(&str) -> Reading) -> Option<bool> {
    let mut unknown = false;
    for property in ["overflow", "overflow-x", "overflow-y"] {
        match read(property) {
            Some((value, false)) if clips(&value) => return Some(true),
            Some((_, true)) => unknown = true,
            _ => {}
        }
    }
    (!unknown).then_some(false)
}

fn is_at(read: &impl Fn(&str) -> Reading, property: &str, expected: &str) -> Option<bool> {
    match read(property) {
        Some((value, false)) => Some(value.trim().eq_ignore_ascii_case(expected)),
        Some((_, true)) => None,
        None => Some(false),
    }
}

fn conj(values: [Option<bool>; 3]) -> Option<bool> {
    let mut unknown = false;
    for value in values {
        match value {
            Some(false) => return Some(false),
            Some(true) => {}
            None => unknown = true,
        }
    }
    (!unknown).then_some(true)
}

fn disj(values: [Option<bool>; 2]) -> Option<bool> {
    let mut unknown = false;
    for value in values {
        match value {
            Some(true) => return Some(true),
            Some(false) => {}
            None => unknown = true,
        }
    }
    (!unknown).then_some(false)
}

fn positive_lines(value: &str) -> Option<u32> {
    value.trim().parse::<u32>().ok().filter(|lines| *lines > 0)
}

fn clips(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "hidden" | "clip" | "auto" | "scroll"
    )
}
