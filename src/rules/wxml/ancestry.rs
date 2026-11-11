//! 模板 / 片段元素在**实例化处的祖先**的样式求值。
//!
//! 单文件 `walk` 给的祖先栈只在一个文档里成立：`<template name>` 的定义体渲染在
//! 每个 `<template is>` 调用处，`<include>` 片段渲染在每个引入位置。要问「WebView
//! 会不会截断」这类问题，必须把祖先链换成 [`WxmlGraph::ancestor_ids`] 给的实例化
//! 路径，并回到祖先**自己那份文档**求值——选择器的作用域、级联的样式表都跟着文档走。
//!
//! 两条规则（`webview/text-truncation`、`skyline/component-attribute`）只关心 WebView
//! 语义：Skyline 丢弃属性条件，用它求祖先会给出错的取值，所以这一路的查询统一用
//! [`Dialect::WebView`]。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::input::wxml::{WxmlDoc, WxmlElement};
use crate::matcher::{Dialect, Tree};
use crate::project::Project;
use crate::wxml_graph::{AncestorId, WxmlGraph};

use super::{load_doc, resolved_property_with_dialect};

/// 逐条实例化路径的合成方式。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// 证据型：祖先里任一处成立即成立（「WebView 会不会截断」）。
    ///
    /// 这是可达性问题，只在 [`Ancestors::Closure`] 上成立——按节点去重后不再随
    /// 实例化路径数指数增长。
    Any,
    /// 违规型：所有路径一致才成立（「每个使用点都坏了」）。
    ///
    /// 路径本身就是语义，只能走 [`Ancestors::Paths`]；下一轮的 `implicit-color` /
    /// `fixed-layer` 会用到。
    #[allow(dead_code)]
    All,
}

/// 实例化祖先证据的形态。
pub(crate) enum Ancestors<'a> {
    /// 祖先节点闭包（已去重）：回答「有没有一个祖先成立」。
    Closure {
        nodes: &'a [AncestorId],
        truncated: bool,
    },
    /// 逐条实例化路径（近 → 远）：回答「是不是每条路径都成立」。
    ///
    /// 只有 `Mode::All` 会构造它（下一轮）；本轮两条规则走闭包。
    #[allow(dead_code)]
    Paths {
        paths: &'a [Vec<AncestorId>],
        truncated: bool,
    },
}

/// `(文档实例地址, 原始起点, 属性, 是否 WebView)` → `(取值, 是否判不了)`。
type ResolvedValues = HashMap<(usize, (u32, u32), String, bool), Option<(String, bool)>>;

/// 所有文档的匹配视图。
///
/// 生命周期约束：`Vec<Rc<WxmlDoc>>` 与 `Vec<Tree>` 必须是 [`with_views`] 里的相邻
/// 局部变量——`Tree` 借用文档，把两者塞进同一个自持 struct 会是自引用。
/// 这里只借它们，不拥有。
pub(crate) struct DocViews<'a> {
    project: &'a Project,
    graph: &'a WxmlGraph,
    docs: &'a [Rc<WxmlDoc>],
    trees: &'a [Tree<'a>],
    /// 文档路径 → `docs` / `trees` 下标（与图里的 `NodeId::doc` 无关）。
    by_path: HashMap<&'a str, usize>,
    /// `(文档实例地址, 原始起点, 属性, 是否 WebView)` → `(取值, 是否判不了)`。
    ///
    /// 同一祖先元素会被不同规则、不同属性反复问；级联一次要匹配整张样式表，
    /// 不缓存会把同一份工作重复几十次。键里的文档用地址：同一路径的折叠版与
    /// 全分支版结构不同。
    resolved_cache: RefCell<ResolvedValues>,
}

/// 在全部图文档的匹配视图上执行 `f`。
pub(crate) fn with_views<R>(project: &Project, f: impl FnOnce(&DocViews<'_>) -> R) -> R {
    let graph = project.graph();
    let docs: Vec<Rc<WxmlDoc>> = graph
        .doc_paths()
        .filter_map(|path| load_doc(project, path))
        .collect();
    let trees: Vec<Tree<'_>> = docs.iter().map(|doc| Tree::new(doc.as_ref())).collect();
    let by_path = docs
        .iter()
        .enumerate()
        .map(|(index, doc)| (doc.path(), index))
        .collect();
    f(&DocViews {
        project,
        graph,
        docs: &docs,
        trees: &trees,
        by_path,
        resolved_cache: RefCell::new(HashMap::new()),
    })
}

/// 按实例化证据合成一个结论。
///
/// `claim` 对一组祖先回答 `Some(true/false)`，判不了给 `None`。证据被预算截断、没有
/// 任何证据、或任一（可能成立的）组判不了，都只能返回 `None`——「部分不成立」既不能
/// 推出成立也不能推出不成立。返回 `Option<bool>` 而不是 bool，是为了让调用方没有机会
/// 把「判不了」当成「没这回事」。
///
/// `Mode::Any` 逐节点问闭包（每个节点当作单元素切片），`Mode::All` 逐条路径问——
/// 后者不能去重：不同路径上的同一个节点，上下文与「最近祖先」都不同。
pub(crate) fn aggregate(
    mode: Mode,
    ancestors: Ancestors<'_>,
    claim: impl Fn(&[AncestorId]) -> Option<bool>,
) -> Option<bool> {
    match (mode, ancestors) {
        (Mode::Any, Ancestors::Closure { nodes, truncated }) => {
            if truncated || nodes.is_empty() {
                return None;
            }
            let mut unknown = false;
            for node in nodes {
                match claim(std::slice::from_ref(node)) {
                    Some(true) => return Some(true),
                    Some(false) => {}
                    None => unknown = true,
                }
            }
            (!unknown).then_some(false)
        }
        (Mode::All, Ancestors::Paths { paths, truncated }) => {
            if truncated || paths.is_empty() {
                return None;
            }
            let verdicts: Option<Vec<bool>> = paths.iter().map(|path| claim(path)).collect();
            Some(verdicts?.iter().all(|hit| *hit))
        }
        // 形态与模式不匹配：闭包支撑不了「每条路径一致」，路径枚举不再用于证据型。
        // 这是调用点写错了，按判不了处理。
        _ => None,
    }
}

/// 某个祖先元素上某个属性的有效取值。
///
/// `None`：这个节点不是可求值元素（页面根），或元素不在视图里。
/// `Some((value, undecidable))`：元素存在；`value` 为空串表示这份文档里确定没写，
/// 布尔位表示「有选择器可能命中、但静态判不了」。
pub(crate) fn property_at(
    views: &DocViews<'_>,
    hop: &AncestorId,
    property: &str,
    dialect: Dialect,
) -> Option<(String, bool)> {
    let AncestorId::Node(node) = hop else {
        return None;
    };
    let index = *views.by_path.get(views.graph.doc_path(node.doc))?;
    let doc = views.docs.get(index)?;
    let tree = views.trees.get(index)?;

    let raw = views.graph.flat(*node).raw_start;
    let key = (
        Rc::as_ptr(doc) as usize,
        raw,
        property.to_string(),
        dialect == Dialect::WebView,
    );
    if let Some(hit) = views.resolved_cache.borrow().get(&key) {
        return hit.clone();
    }

    let value = doc.element_at(raw).map(|element| {
        let (resolved, undecidable) =
            resolved_property_with_dialect(views.project, tree, element, property, dialect);
        (
            resolved.map(|resolved| resolved.value).unwrap_or_default(),
            undecidable,
        )
    });
    views.resolved_cache.borrow_mut().insert(key, value.clone());
    value
}

/// WebView 侧会不会截断：自身、单文件祖先，或实例化处的祖先。
///
/// `Some(true)` 是证据，`Some(false)` 是「都确定不截断」，`None` 是判不了（调用方
/// 必须放弃判定）。
pub(crate) fn webview_truncates(
    views: &DocViews<'_>,
    tree: &Tree<'_>,
    element: &WxmlElement,
    ancestors: &[&WxmlElement],
) -> Option<bool> {
    let own = local_truncates(views, tree, element);
    if own == Some(true) {
        return Some(true);
    }
    let mut unknown = own.is_none();
    for candidate in ancestors.iter().rev() {
        match local_truncates(views, tree, candidate) {
            Some(true) => return Some(true),
            Some(false) => {}
            None => unknown = true,
        }
    }
    if !tree.ancestors_unknown(element) {
        return (!unknown).then_some(false);
    }
    // 模板定义体 / 片段：真实祖先在实例化处。问的是「祖先里有没有截断属性」——
    // 可达性，走按节点去重的闭包，不随实例化路径数指数增长。
    let closure = views.graph.ancestor_closure(tree.path(), element);
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
            let (resolved, _) = resolved_property_with_dialect(
                views.project,
                tree,
                candidate,
                "-webkit-line-clamp",
                Dialect::WebView,
            );
            resolved.and_then(|resolved| positive_lines(&resolved.value))
        });
    if local.is_some() {
        return local;
    }
    if !tree.ancestors_unknown(element) {
        return None;
    }
    let paths = views.graph.ancestor_ids(tree.path(), element);
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

fn local_truncates(views: &DocViews<'_>, tree: &Tree<'_>, element: &WxmlElement) -> Option<bool> {
    truncates_with(|property| {
        let (resolved, undecidable) = resolved_property_with_dialect(
            views.project,
            tree,
            element,
            property,
            Dialect::WebView,
        );
        Some((
            resolved.map(|resolved| resolved.value).unwrap_or_default(),
            undecidable,
        ))
    })
}

fn hop_truncates(views: &DocViews<'_>, hop: &AncestorId) -> Option<bool> {
    truncates_with(|property| property_at(views, hop, property, Dialect::WebView))
}

fn any_hop_truncates(views: &DocViews<'_>, path: &[AncestorId]) -> Option<bool> {
    let mut unknown = false;
    for hop in path {
        match hop_truncates(views, hop) {
            Some(true) => return Some(true),
            Some(false) => {}
            None => unknown = true,
        }
    }
    (!unknown).then_some(false)
}

/// 「WebView 会截断吗」的取值级判定：多行 clamp，或单行 nowrap + 裁切 + 省略号。
fn truncates_with(read: impl Fn(&str) -> Option<(String, bool)>) -> Option<bool> {
    let clamp = match read("-webkit-line-clamp") {
        Some((value, false)) => Some(positive_lines(&value).is_some()),
        Some((_, true)) => None,
        None => Some(false),
    };
    let single_line = conj([
        is_at(&read, "white-space", "nowrap"),
        clips_with(&read),
        is_at(&read, "text-overflow", "ellipsis"),
    ]);
    disj([clamp, single_line])
}

fn clips_with(read: &impl Fn(&str) -> Option<(String, bool)>) -> Option<bool> {
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

fn is_at(
    read: &impl Fn(&str) -> Option<(String, bool)>,
    property: &str,
    expected: &str,
) -> Option<bool> {
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
