//! 模板 / 片段元素在**实例化处的祖先**的样式求值。
//!
//! 单文件 `walk` 给的祖先栈只在一个文档里成立：`<template name>` 的定义体渲染在
//! 每个 `<template is>` 调用处，`<include>` 片段渲染在每个引入位置。要问「WebView
//! 会不会截断」这类问题，必须把祖先链换成 [`WxmlGraph::ancestor_ids`] 给的实例化
//! 路径，并回到祖先**自己那份文档**求值——选择器的作用域、级联的样式表都跟着文档走。
//!
//! 本模块只负责「祖先是谁、它的某个属性值是多少」；截断语义在
//! [`super::truncation`]。求值一律显式传方言：调用方要 WebView 语义就传
//! [`Dialect::WebView`]，否则 Skyline 会丢弃属性条件、给出错的取值。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::input::wxml::WxmlDoc;
use crate::matcher::{Dialect, Tree};
use crate::project::Project;
use crate::wxml_graph::{AncestorId, NodeId, WxmlGraph};

use super::load_doc;
use super::resolved_property_with_dialect;

/// 实例化证据的合成方式。
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

/// 祖先属性求值的缓存键。
#[derive(Clone, PartialEq, Eq, Hash)]
struct ResolvedKey {
    /// `Rc<WxmlDoc>` 的地址：同一路径的折叠版与全分支版结构不同，不能按路径区分。
    doc: usize,
    /// 元素的原始起点（编译器 0 基坐标）。
    start: (u32, u32),
    property: String,
    dialect: Dialect,
}

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
    /// 祖先属性的级联结果。
    ///
    /// 同一祖先元素会被不同规则、不同属性反复问；级联一次要匹配整张样式表，
    /// 不缓存会把同一份工作重复几十次。
    resolved_cache: RefCell<HashMap<ResolvedKey, Option<(String, bool)>>>,
}

/// 在**图里每个文档**各自的匹配视图上执行 `f`。
///
/// `views` 不是把多份 WXML 拼成的一棵节点图：它持有每个文档自己的 `Tree`（选择器
/// 只在单文件内求值）外加一份共享的级联缓存。模板定义体 / 片段的祖先落在别的文档里，
/// 求值必须先顺着 [`AncestorId`] 找到那个文档，再用**它自己**的 `Tree` 与文档级作用域
/// 算属性值——这正是 [`property_at`] 负责的事。
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

impl DocViews<'_> {
    pub(crate) fn project(&self) -> &Project {
        self.project
    }

    pub(crate) fn graph(&self) -> &WxmlGraph {
        self.graph
    }

    /// 节点 → 它自己那份文档上的某个属性值。
    ///
    /// `None`：节点在这份视图里找不到（文档不参与渲染）。
    /// `Some((value, undecidable))`：元素存在；空串表示这份文档里确定没写，布尔位
    /// 表示「有选择器可能命中、但静态判不了」。
    fn resolve(&self, node: NodeId, property: &str, dialect: Dialect) -> Option<(String, bool)> {
        let index = *self.by_path.get(self.graph.doc_path(node.doc))?;
        let doc = self.docs.get(index)?;
        let tree = self.trees.get(index)?;
        let start = self.graph.flat(node).raw_start;
        let key = ResolvedKey {
            doc: Rc::as_ptr(doc) as usize,
            start,
            property: property.to_string(),
            dialect,
        };
        if let Some(hit) = self.resolved_cache.borrow().get(&key) {
            return hit.clone();
        }
        let value = doc.element_at(start).map(|element| {
            let (resolved, undecidable) =
                resolved_property_with_dialect(self.project, tree, element, property, dialect);
            (
                resolved.map(|resolved| resolved.value).unwrap_or_default(),
                undecidable,
            )
        });
        self.resolved_cache.borrow_mut().insert(key, value.clone());
        value
    }
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

/// 某个祖先元素上某个属性的有效取值（回它**自己那份文档**求级联）。
///
/// 例：模板 `tpls/book.wxml` 里是 `<template name="t"><text class="name">`，
/// 页面 `pages/list/index.wxml` 用 `<view class="clamp"><template is="t"/></view>`
/// 实例化它，`pages/list/index.wxss` 写 `.clamp { -webkit-line-clamp: 2 }`。
/// 对模板里的 `<text>` 调用 [`WxmlGraph::ancestor_closure`]，闭包里会有页面那个
/// `<view>` 节点；`property_at(views, 该节点, "-webkit-line-clamp", Dialect::WebView)`
/// 就返回 `Some(("2", false))`——取值来自**页面**那份文档的样式表，而不是模板自己的。
///
/// `None`：这个节点不是可求值元素（页面根 [`AncestorId::PageRoot`]），或元素不在视图里。
/// `Some((value, undecidable))`：元素存在；空串表示这份文档里确定没写，布尔位表示
/// 「有选择器可能命中、但静态判不了」。
pub(crate) fn property_at(
    views: &DocViews<'_>,
    hop: &AncestorId,
    property: &str,
    dialect: Dialect,
) -> Option<(String, bool)> {
    match hop {
        AncestorId::Node(node) => views.resolve(*node, property, dialect),
        AncestorId::PageRoot => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::matcher::Dialect;
    use crate::rules::testutil::wxml::Fixture;
    use crate::rules::wxml::load_doc;

    use super::{AncestorId, property_at, with_views};

    fn fixture(name: &str, template: &str, style: &str) -> Fixture {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write(
                "pages/index/index.wxml",
                r#"<import src="/tpls/t.wxml"/><view class="clamp"><template is="t"/></view>"#,
            )
            .write("tpls/t.wxml", template)
            .write("pages/index/index.wxss", style);
        fixture
    }

    /// 祖先的属性值来自**它自己那份文档**的样式表，而不是模板的。
    #[test]
    fn ancestor_property_comes_from_its_own_document() {
        let fixture = fixture(
            "ancestry-own-doc",
            r#"<template name="t"><text class="name">a</text></template>"#,
            ".clamp { -webkit-line-clamp: 2; }",
        );
        let project = fixture.project();
        let doc = load_doc(&project, "tpls/t.wxml").expect("模板文档");
        let text = doc.elements().find(|e| e.tag == "text").expect("text");
        let closure = project.graph().ancestor_closure("tpls/t.wxml", text);

        let values = with_views(&project, |views| {
            closure
                .0
                .iter()
                .filter_map(|hop| property_at(views, hop, "-webkit-line-clamp", Dialect::WebView))
                .collect::<Vec<_>>()
        });
        assert!(
            values
                .iter()
                .any(|(value, undecidable)| !*undecidable && value.trim() == "2"),
            "实例化处的 clamp 必须能读到：{values:?}"
        );
    }

    /// 属性没写时返回空串而不是 `None`：调用方要能区分「确定没写」与「求值不了」。
    #[test]
    fn missing_property_reads_as_empty() {
        let fixture = fixture(
            "ancestry-missing",
            r#"<template name="t"><text class="name">a</text></template>"#,
            ".clamp { color: red; }",
        );
        let project = fixture.project();
        let doc = load_doc(&project, "tpls/t.wxml").expect("模板文档");
        let text = doc.elements().find(|e| e.tag == "text").expect("text");
        let closure = project.graph().ancestor_closure("tpls/t.wxml", text);

        let written = with_views(&project, |views| {
            closure
                .0
                .iter()
                .filter_map(|hop| property_at(views, hop, "display", Dialect::WebView))
                .collect::<Vec<_>>()
        });
        assert!(
            written
                .iter()
                .all(|(value, undecidable)| value.is_empty() && !*undecidable),
            "{written:?}"
        );
    }

    /// 方言决定属性选择器能不能命中：Skyline 丢弃属性条件，WebView 按真值匹配。
    /// 缓存键必须带方言，否则先问的那一方会把结果污染给另一方。
    #[test]
    fn dialect_selects_which_selectors_match() {
        let fixture = fixture(
            "ancestry-dialect",
            r#"<template name="t"><text class="name">a</text></template>"#,
            ".name[data-role='title'] { -webkit-line-clamp: 3; }",
        );
        let project = fixture.project();
        let doc = load_doc(&project, "tpls/t.wxml").expect("模板文档");
        let text = doc.elements().find(|e| e.tag == "text").expect("text");
        let node = AncestorId::Node(
            project
                .graph()
                .element_node("tpls/t.wxml", text)
                .expect("在图上"),
        );

        let (webview, skyline) = with_views(&project, |views| {
            (
                property_at(views, &node, "-webkit-line-clamp", Dialect::WebView),
                property_at(views, &node, "-webkit-line-clamp", Dialect::Skyline),
            )
        });
        assert!(
            matches!(&webview, Some((value, _)) if value.is_empty()),
            "WebView 下这个属性不该取到值：{webview:?}"
        );
        assert_eq!(
            skyline,
            Some(("3".to_string(), false)),
            "Skyline 忽略属性条件"
        );
    }

    /// 页面根不是可求值元素。
    #[test]
    fn page_root_has_no_property() {
        let fixture = fixture(
            "ancestry-page-root",
            r#"<template name="t"><text class="name">a</text></template>"#,
            ".clamp { color: red; }",
        );
        let project = fixture.project();
        let value = with_views(&project, |views| {
            property_at(views, &AncestorId::PageRoot, "color", Dialect::WebView)
        });
        assert_eq!(value, None);
    }
}
