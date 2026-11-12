//! WXML 逻辑树：把自定义组件的 `<slot>` 投影、`<template is>` 实例化接上。
//!
//! [`crate::rules::wxml::walk`] 给的祖先栈只在单个文件里成立。组件模板里的
//! `<slot>` 在静态树上是个死胡同——运行时的子节点写在使用方文件里；反过来，
//! 使用方传进去的子节点渲染在组件模板里，它的祖先包含组件自己的节点。规则里凡
//! 是问「这个元素的祖先链上有没有 X」的（手势冲突、`display` 继承、滚动容器嵌
//! 套……），只看单文件就会漏掉跨边界的那一半。
//!
//! 这里建一张图：节点是 `(文档, 元素)`，边是运行时的父边。
//!
//! ## 组件边界（glass-easel 语义）
//!
//! - 使用方传给组件的子节点，运行时被投影进组件模板里的某个 `<slot>`：
//!   `shadow_root.ts` 的 `getContainingSlot()` 按 **slot 模式**决定投到哪：
//!   - `SlotMode.Single`（默认，`multipleSlots` 未开）：所有子节点都进模板里
//!     **第一个** `<slot>`，`slot="x"` 属性被完全忽略（`_$singleSlot = subtreeSlotStart.value`，
//!     且 `_$insertChildReassignSlot(shadowRoot, null, …)` 的 `name` 为 `null` 时不做名字过滤）；
//!   - `SlotMode.Multiple`（`multipleSlots: true`）：按子节点的 `slot` 属性（缺省为 `''`）
//!     匹配同名的 `<slot name>`；**没有对应 slot 的子节点不参与合成**，即不渲染；
//!   - `SlotMode.Dynamic`（`dynamicSlots: true`）：槽位由 `slotValues` 运行时决定，静态不可知。
//! - 组件模板的顶层元素渲染在使用这个组件的元素（宿主）内部；`virtualHost: true` 时
//!   宿主元素不存在（`component.ts` 不创建 backend element），模板顶层元素直接渲染在
//!   使用位置的父节点里。
//!
//! ## 模板与片段边界
//!
//! - `<template name="x">` 的定义体不在原位置渲染，只渲染在每个 `<template is="x">`
//!   处（`parse/tag.rs` 把定义体放进 `globals.sub_templates`）。可见范围是本文件加上
//!   **`<import>` 了它的文件**——`imports` 不通传递，`<include>` 也不带模板定义。
//! - `<include src>` 是**整段内联**：`proc_gen/tag.rs:1293-1316` 在引入位置生成
//!   `G[path]('')(R,C,D,U)` 并把结果 `J(child)` 挂到当前位置，所以片段文件的顶层
//!   元素就渲染在 `<include>` 所在的位置（不是它的父级之外）。
//!
//! 祖先查询是对 DAG 的存在性遍历：`ancestors` 返回所有可能祖先，去重、限深；遇到静态
//! 不可知的边（动态槽名、`dynamicSlots`、动态 `<template is>`、未渲染）就把 `open`
//! 置位，调用方据此放弃判定（宁可漏报）。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::input::wxml::{WxmlAttribute, WxmlElement};
use crate::matcher::{FlatNode, node_of};

/// `<template is>` 调用写在文档顶层：没有所在元素。
const TOP: usize = usize::MAX;

/// `<include>` 互相引用的最大展开深度，超过就按不可知处理（防环）。
const MAX_INCLUDE_DEPTH: usize = 16;

/// 遍历深度上限：项目里不该出现这么深的投影链，超了说明组件互相嵌套成环。
const MAX_NODES: usize = 512;

/// 单条祖先路径的分叉上限：多使用点会乘出指数条路径，超过就截断（宁可漏报）。
const MAX_PATHS: usize = 32;

/// 单次路径枚举的节点预算。防的正是「分叉多到指数级、但条条都走不到叶子」，
/// 那种情况下 `MAX_PATHS` 永远数不满，递归先爆了。
const MAX_VISITS: usize = 4096;

/// 单次祖先闭包遍历的节点预算。
///
/// 闭包按节点去重，每个祖先只访问一次，所以这里的预算就是「一个元素最多能看见
/// 多少个不同的祖先节点」——与实例化路径数无关。实测最大闭包：novel 69 个节点
/// （740 个元素，p99 = 66）、businessview 42 个（1296 个元素），4096 对最大的
/// novel 留了约 59 倍余量。祖先节点数受组件层级约束，不是设计上会涨的量。
const MAX_CLOSURE_VISITS: usize = 4096;

/// 单条祖先路径上的一节。
#[derive(Clone)]
pub(crate) enum AncestorRef<'a> {
    /// 渲染树里真实存在的元素（跨组件边界时来自别的文档）。
    Element {
        node: NodeId,
        element: &'a FlatElement,
    },
    /// 页面根：WXML 里没有对应标签，但 WebView 的 DOM 根是 `<page>`。
    PageRoot,
}

impl AncestorRef<'_> {
    /// 匹配用的扁平节点：跨边界的选择器匹配只看标签 / id / class。
    pub(crate) fn node(&self) -> FlatNode {
        match self {
            Self::Element { element, .. } => node_of(&element.tag, &element.attributes),
            // WebView 的 DOM 根就是 `<page>`。
            Self::PageRoot => FlatNode {
                tag: "page".to_string(),
                ..FlatNode::default()
            },
        }
    }
}

/// 一次组件使用点（反链）：哪个文档用哪个标签引用了这份组件。
#[derive(Debug, Clone)]
pub(crate) struct UsageSite {
    pub doc: String,
    pub tag: String,
    pub line: u32,
    pub column: u32,
}

/// 递归反链上的一跳。
#[derive(Debug, Clone)]
pub(crate) struct UsageHop {
    pub site: UsageSite,
    pub depth: u32,
}

/// 逻辑树里的一个元素。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct NodeId {
    pub doc: usize,
    pub idx: usize,
}

/// 扁平化后的元素：只留规则要用的字段，不重复整棵子树。
pub(crate) struct FlatElement {
    pub tag: String,
    pub attributes: Vec<WxmlAttribute>,
    /// 标签名在源码里的位置（诊断落点）。
    pub line: u32,
    pub column: u32,
    pub end_line: u32,
    pub end_column: u32,
    /// 标签名的原始起点（编译器 0 基坐标），身份可逆：用它能在 `WxmlDoc` 里查回
    /// 同一个元素。`line/column` 是 1-based 显示坐标，两者不能混用。
    pub raw_start: (u32, u32),
    /// `<slot name="x">` 的槽名，其余为 `None`。
    slot_name: Option<String>,
    /// 来自内联 `<template name="x">` 的定义体时的模板名。
    template: Option<String>,
    /// 整个元素（含子节点）的范围，用于把 `<template is>` 调用点归到所在的元素。
    span: ((u32, u32), (u32, u32)),
    parent: Option<usize>,
}

impl FlatElement {
    pub fn attr(&self, name: &str) -> Option<&WxmlAttribute> {
        self.attributes
            .iter()
            .find(|attr| attr.name.eq_ignore_ascii_case(name))
    }

    /// `<slot name="x">` 的槽名；默认槽与普通元素为 `None`。
    pub(crate) fn slot_name(&self) -> Option<&str> {
        self.slot_name.as_deref()
    }
}

/// 逻辑祖先链。
///
/// `open` 表示链尾停在了一处静态不可知的边上：再往上是什么不可知，依赖祖先的判定
/// 必须放弃（宁可漏报）。
#[derive(Default)]
pub(crate) struct AncestorChain<'a> {
    pub nodes: Vec<(&'a str, &'a FlatElement)>,
    pub open: bool,
}

/// 组件模板的 `options`（来自组件 JS），决定 slot 语义与宿主是否存在。
#[derive(Clone, Copy, Default)]
struct ComponentOptions {
    multiple_slots: bool,
    dynamic_slots: bool,
    virtual_host: bool,
}

struct DocTree {
    path: String,
    elements: Vec<FlatElement>,
    /// 前序下标 → 源码位置，用于把元素查回下标。
    by_start: HashMap<(u32, u32), usize>,
    /// `<slot>`：名字（`None` = 默认）与下标，按文档顺序。
    slots: Vec<(Option<String>, usize)>,
    /// 本文件里声明的组件标签 → 组件文档下标。
    components: HashMap<String, usize>,
    /// 组件模板的 options；页面与片段是默认值。
    options: ComponentOptions,
    /// 本文件里 `<template is="x">` 的调用点。
    template_uses: HashMap<String, Vec<usize>>,
    /// 这份文档是一个页面（渲染树的根），而不是组件模板。
    is_page: bool,
    /// 这份文档是被 `<include>` 引入的片段：它的顶层元素渲染在每个引入位置。
    is_fragment: bool,
    /// `<include src>`：原始路径与标签起点（编译器 0 基坐标）。
    doc_includes: Vec<(String, (u32, u32))>,
    /// `<import src>`：原始路径。
    doc_imports: Vec<String>,
}

/// 投影后的容器。
pub enum SlotHost<'a> {
    /// 落在该标签的元素里（`<text>` 之类）。
    Element(&'a str),
    /// 没有匹配的槽位，子节点不渲染。
    NotRendered,
    /// 动态槽名 / `dynamicSlots` / 槽在模板顶层：静态不可知。
    Unknown,
}

/// 一个节点的父边。
enum Parents {
    Nodes(Vec<NodeId>),
    /// 文档顶层：页面到此为止，链结束。
    Top,
    /// 静态不可知：动态槽名、`dynamicSlots`、动态 `<template is>`、未渲染、片段无处引用。
    Unknown,
}

/// 元素的实例化祖先路径（已转成匹配用节点）与是否被路径上限截断。
pub(crate) type AncestorNodePaths = (Vec<Vec<FlatNode>>, bool);

/// 祖先路径上的一个节点身份：仍是某个元素，或页面根。
///
/// 与 [`AncestorRef`] 的区别只是不含借用：可以安全地放进 `RefCell` 缓存。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum AncestorId {
    Node(NodeId),
    PageRoot,
}

/// 元素的实例化祖先路径（节点身份）与是否被路径上限截断。
pub(crate) type AncestorIdPaths = (Vec<Vec<AncestorId>>, bool);

/// 元素的实例化祖先**闭包**（按节点去重，BFS 顺序）与是否被预算截断。
pub(crate) type AncestorClosure = (Vec<AncestorId>, bool);

/// (文档实例, 元素起点, 是否 WebView 语义) → (选择器, 判定)：模板文档的判定缓存。
///
/// 选择器的判定与方言有关（Skyline 丢弃属性条件），键里必须带方言，否则同一元素
/// 上的 WebView 与 Skyline 查询会互相污染。
pub(crate) type VerdictCache = HashMap<(usize, (u32, u32), bool), HashMap<String, Option<bool>>>;

/// 祖先相关的按元素缓存。
///
/// 三个视图共享同一个键 `(doc, idx)` 与同一套预算，只是回答不同的问题：匹配用节点、
/// 样式求值要节点身份、证据型判定要闭包。元素级规则会按属性反复问同一个元素，
/// 枚举一次要回溯整张图，三者都必须缓存。
#[derive(Default)]
struct AncestorCaches {
    /// 实例化祖先路径，已转成匹配用节点（[`FlatNode`]）。
    nodes: RefCell<HashMap<(usize, usize), Rc<AncestorNodePaths>>>,
    /// 实例化祖先路径，保留节点身份（[`AncestorId`]）。
    ids: RefCell<HashMap<(usize, usize), Rc<AncestorIdPaths>>>,
    /// 实例化祖先闭包，按节点去重。
    closure: RefCell<HashMap<(usize, usize), Rc<AncestorClosure>>>,
}

/// 全工程的 WXML 逻辑树。
///
/// 节点 = 元素，边 = 运行时父边（真实父节点、`<slot>` 投影、组件根的使用点、
/// `<template is>` 调用点、`<include>` 引入位置）。上层规则只读这里的关系，不再自己
/// 解析 WXML。
pub(crate) struct WxmlGraph {
    /// 全部参与渲染判定的文档（页面 / 组件 / 被引用的模板与片段）。
    docs: Vec<DocTree>,
    /// 路径 → `docs` 下标（也是 [`NodeId::doc`]）。
    by_path: HashMap<String, usize>,
    // —— 图边 ——
    /// 组件文档下标 → 用到它的位置。
    usages: HashMap<usize, Vec<NodeId>>,
    /// 片段文档下标 → 每个 `<include>` 的引入位置（`None` = 文档顶层）。
    includes: HashMap<usize, Vec<(usize, Option<usize>)>>,
    /// 文档下标 → `<import>` 了它的文档。
    importers: HashMap<usize, Vec<usize>>,
    // —— 缓存 ——
    ancestor_caches: AncestorCaches,
    /// (文档实例, 元素起点, 方言) → (选择器, 是否命中)：模板文档的选择器判定。
    ///
    /// 判定只取决于 (元素, 选择器)，但级联是「按属性」走的：`implicit-color` 一个元素
    /// 要问 9 个属性，同一条选择器就会被判 9 次。一轮 lint 里选择器集合是稳定的。
    /// 键里的文档是**解析实例**（`WxmlDoc` 地址）：同一路径的折叠版与全分支版结构不同。
    verdict_cache: RefCell<VerdictCache>,
}

mod ancestors;
mod build;
mod components;

impl WxmlGraph {
    /// 文档路径 + 元素 → 图里的节点身份。
    ///
    /// `None`：文档不在图里，或这个元素来自另一份解析实例（折叠版 vs 全分支版）。
    pub(crate) fn element_node(&self, doc_path: &str, element: &WxmlElement) -> Option<NodeId> {
        let &doc = self.by_path.get(doc_path)?;
        let &idx = self.docs[doc]
            .by_start
            .get(&(element.tag_start.line, element.tag_start.utf16_col))?;
        Some(NodeId { doc, idx })
    }

    /// 这份文档在不在图里（模板片段、`<import>` 文件不在）。
    pub fn contains(&self, doc_path: &str) -> bool {
        self.by_path.contains_key(doc_path)
    }

    /// 文档下标 → 路径。
    pub(crate) fn doc_path(&self, doc: usize) -> &str {
        self.docs.get(doc).map_or("", |tree| tree.path.as_str())
    }

    /// 图里的全部文档路径（顺序即 [`NodeId::doc`] 的下标）。
    pub(crate) fn doc_paths(&self) -> impl Iterator<Item = &str> {
        self.docs.iter().map(|tree| tree.path.as_str())
    }

    /// 节点 → 扁平元素。
    pub(crate) fn flat(&self, node: NodeId) -> &FlatElement {
        &self.docs[node.doc].elements[node.idx]
    }

    /// 这份文档的根在渲染树里有位置（页面根 / 被使用的组件 / 被引入的片段）。
    ///
    /// 没有任何实例化处的文档永远不会渲染，对它求值得到的祖先没有意义。
    /// 留给下一轮的结构规则；本轮祖先身份由实例化路径保证。
    #[allow(dead_code)]
    pub(crate) fn root_placed(&self, doc: usize) -> bool {
        self.docs
            .get(doc)
            .is_some_and(|_| !matches!(self.doc_top_parents(doc), Parents::Unknown))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use crate::rules::testutil::Fixture;
    use crate::rules::wxml::{load_doc, walk};

    use super::{AncestorId, WxmlGraph};

    fn fixture(name: &str, page: &str, component: &str) -> Fixture {
        fixture_with(name, page, component, "Component({});")
    }

    fn fixture_with(name: &str, page: &str, component: &str, component_js: &str) -> Fixture {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                r#"{"usingComponents":{"panel":"/components/panel/panel"}}"#,
            )
            .write("pages/index/index.wxml", page)
            .write("pages/index/index.js", "Page({});")
            .write("components/panel/panel.json", r#"{"component":true}"#)
            .write("components/panel/panel.js", component_js)
            .write("components/panel/panel.wxml", component);
        fixture
    }

    /// `path` 里 `class` 命中的元素的逻辑祖先标签，以及链尾是否开口。
    fn chain(fixture: &Fixture, path: &str, class: &str) -> (Vec<String>, bool) {
        let project = fixture.project();
        let doc = load_doc(&project, path).expect("doc");
        let mut found = None;
        crate::rules::wxml::walk(&doc, |candidate, _| {
            if candidate
                .attributes
                .iter()
                .any(|attr| attr.name == "class" && attr.value.as_deref() == Some(class))
            {
                found = Some(candidate.clone());
            }
        });
        let element = found.expect("element");
        let graph = WxmlGraph::new(&project);
        let chain = graph.ancestors(path, &element);
        (
            chain
                .nodes
                .iter()
                .map(|(_, flat)| flat.tag.clone())
                .collect(),
            chain.open,
        )
    }

    #[test]
    fn slot_children_see_component_ancestors() {
        let fixture = fixture(
            "graph-slot",
            r#"<panel><view class="target"/></panel>"#,
            r#"<view class="host"><slot/></view>"#,
        );
        assert_eq!(
            chain(&fixture, "pages/index/index.wxml", "target").0,
            vec!["slot".to_string(), "view".to_string(), "panel".to_string()]
        );
    }

    #[test]
    fn named_slot_picks_its_own_parent_chain() {
        let fixture = fixture_with(
            "graph-named-slot",
            r#"<panel><view slot="a" class="target"/></panel>"#,
            r#"<view class="other"><slot name="a"/></view>"#,
            "Component({ options: { multipleSlots: true } });",
        );
        let (tags, open) = chain(&fixture, "pages/index/index.wxml", "target");
        assert_eq!(tags.first().map(String::as_str), Some("slot"));
        assert_eq!(tags.get(1).map(String::as_str), Some("view"));
        assert!(!open);
    }

    /// 默认（未开 `multipleSlots`）是单槽模式：`slot="a"` 被忽略，子节点进第一个 `<slot>`。
    #[test]
    fn single_slot_mode_ignores_slot_names() {
        let fixture = fixture(
            "graph-single-slot",
            r#"<panel><view slot="a" class="target"/></panel>"#,
            r#"<view class="host"><slot/></view>"#,
        );
        let (tags, open) = chain(&fixture, "pages/index/index.wxml", "target");
        assert_eq!(
            tags,
            vec!["slot".to_string(), "view".to_string(), "panel".to_string()]
        );
        assert!(!open);
    }

    /// 模板里没有对应名字的槽：子节点不参与合成，祖先不可知。
    #[test]
    fn missing_named_slot_is_unknown() {
        let fixture = fixture_with(
            "graph-missing-slot",
            r#"<panel><view slot="b" class="target"/></panel>"#,
            r#"<view class="host"><slot name="a"/></view>"#,
            "Component({ options: { multipleSlots: true } });",
        );
        let (tags, open) = chain(&fixture, "pages/index/index.wxml", "target");
        assert!(tags.is_empty(), "{tags:?}");
        assert!(open);
    }

    /// `multipleSlots` 下未写 `slot` 的子节点进默认槽（`<slot/>` 在槽表里是 `None`）。
    #[test]
    fn multiple_slots_default_slot_takes_unlabelled_children() {
        let fixture = fixture_with(
            "graph-multiple-default-slot",
            r#"<panel><view class="target"/></panel>"#,
            r#"<view class="host"><slot name="title"/><slot/></view>"#,
            "Component({ options: { multipleSlots: true } });",
        );
        let (tags, open) = chain(&fixture, "pages/index/index.wxml", "target");
        assert_eq!(
            tags,
            vec!["slot".to_string(), "view".to_string(), "panel".to_string()]
        );
        assert!(!open);
    }

    /// `multipleSlots` 下没有默认槽：未写 `slot` 的子节点不渲染，祖先不可知。
    #[test]
    fn multiple_slots_without_default_slot_is_unknown() {
        let fixture = fixture_with(
            "graph-multiple-no-default",
            r#"<panel><view class="target"/></panel>"#,
            r#"<view class="host"><slot name="title"/></view>"#,
            "Component({ options: { multipleSlots: true } });",
        );
        let (tags, open) = chain(&fixture, "pages/index/index.wxml", "target");
        assert!(tags.is_empty(), "{tags:?}");
        assert!(open);
    }

    /// 动态槽名：静态不可知。
    #[test]
    fn dynamic_slot_name_is_unknown() {
        let fixture = fixture_with(
            "graph-dynamic-slot",
            r#"<panel><view slot="{{where}}" class="target"/></panel>"#,
            r#"<view class="host"><slot name="a"/></view>"#,
            "Component({ options: { multipleSlots: true } });",
        );
        let (tags, open) = chain(&fixture, "pages/index/index.wxml", "target");
        assert!(tags.is_empty(), "{tags:?}");
        assert!(open);
    }

    /// `virtualHost: true`：宿主元素不存在，模板顶层直接挂在使位置的父节点里。
    #[test]
    fn virtual_host_hides_the_host_element() {
        let fixture = fixture_with(
            "graph-virtual-host",
            r#"<view class="outer"><panel><view class="target"/></panel></view>"#,
            r#"<view class="host"><slot/></view>"#,
            "Component({ options: { virtualHost: true } });",
        );
        let (tags, open) = chain(&fixture, "pages/index/index.wxml", "target");
        assert_eq!(
            tags,
            vec!["slot".to_string(), "view".to_string(), "view".to_string()],
            "宿主 panel 不应出现在链里"
        );
        assert!(!open);
    }

    /// 内联模板定义体渲染在 `<template is>` 处，而不是文档顶层。
    #[test]
    fn template_definition_body_is_parented_by_its_usage() {
        let fixture = fixture(
            "graph-template",
            r#"<template name="row"><view class="target"/></template><view class="outer"><template is="row"/></view>"#,
            r#"<view class="host"><slot/></view>"#,
        );
        let (tags, open) = chain(&fixture, "pages/index/index.wxml", "target");
        // `<template is>` 不产生元素节点，链上直接是它所在的元素。
        assert_eq!(tags, vec!["view".to_string()]);
        assert!(!open);
    }

    /// 被 `<include>` 引入的片段：顶层元素渲染在 `<include>` 所在的位置。
    #[test]
    fn include_places_fragment_roots_at_the_include_site() {
        let fixture = Fixture::new("graph-include");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write(
                "pages/index/index.wxml",
                r#"<view class="wrap"><include src="./part"/></view>"#,
            )
            .write("pages/index/index.js", "Page({});")
            .write("pages/index/part.wxml", r#"<view class="target"/>"#);
        let (tags, open) = chain(&fixture, "pages/index/part.wxml", "target");
        assert_eq!(tags, vec!["view".to_string()]);
        assert!(!open);
    }

    /// `<import>` 只带模板定义，定义体渲染在引入方的 `<template is>` 处。
    #[test]
    fn import_makes_template_definitions_visible_to_the_importer() {
        let fixture = Fixture::new("graph-import");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write(
                "pages/index/index.wxml",
                r#"<import src="./tpl"/><view class="outer"><template is="row"/></view>"#,
            )
            .write("pages/index/index.js", "Page({});")
            .write(
                "pages/index/tpl.wxml",
                r#"<template name="row"><view class="target"/></template>"#,
            );
        let (tags, open) = chain(&fixture, "pages/index/tpl.wxml", "target");
        assert_eq!(tags, vec!["view".to_string()]);
        assert!(!open);
    }

    /// `virtualHost` 只抹掉宿主元素，父级的 `<slot>` 投影照旧。
    #[test]
    fn virtual_host_still_projects_into_the_parent_slot() {
        let fixture = Fixture::new("graph-virtual-nested");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                r#"{"usingComponents":{"panel":"/components/panel/panel","virtual":"/components/virtual/virtual"}}"#,
            )
            .write(
                "pages/index/index.wxml",
                r#"<panel><virtual><view class="target"/></virtual></panel>"#,
            )
            .write("pages/index/index.js", "Page({});")
            .write("components/panel/panel.json", r#"{"component":true}"#)
            .write("components/panel/panel.js", "Component({});")
            .write(
                "components/panel/panel.wxml",
                r#"<view class="host"><slot/></view>"#,
            )
            .write("components/virtual/virtual.json", r#"{"component":true}"#)
            .write(
                "components/virtual/virtual.js",
                "Component({ options: { virtualHost: true } });",
            )
            .write(
                "components/virtual/virtual.wxml",
                r#"<view class="vhost"><slot/></view>"#,
            );
        let (tags, open) = chain(&fixture, "pages/index/index.wxml", "target");
        // virtual 的宿主不存在 → 不出现；但它的父级 panel 的 slot 仍要接上。
        assert_eq!(
            tags,
            vec![
                "slot".to_string(),
                "view".to_string(),
                "slot".to_string(),
                "view".to_string(),
                "panel".to_string()
            ]
        );
        assert!(!open);
    }

    /// 三层模板、每层两个使用点：8 条实例化路径，闭包只有十几个节点。
    fn template_chain(name: &str) -> Fixture {
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
                r#"<import src="/tpls/l1"/><view><view class="a"><template is="l1"/></view><view class="b"><template is="l1"/></view></view>"#,
            )
            .write("pages/index/index.js", "Page({});")
            .write(
                "tpls/l1.wxml",
                r#"<import src="./l2"/><template name="l1"><view class="l1"><view class="a"><template is="l2"/></view><view class="b"><template is="l2"/></view></view></template>"#,
            )
            .write(
                "tpls/l2.wxml",
                r#"<import src="./leaf"/><template name="l2"><view class="l2"><view class="a"><template is="leaf"/></view><view class="b"><template is="leaf"/></view></view></template>"#,
            )
            .write(
                "tpls/leaf.wxml",
                r#"<template name="leaf"><text class="target" max-lines="2">x</text></template>"#,
            );
        fixture
    }

    /// 闭包 BFS 与路径枚举的并集必须一致：前者只是把去重范围从「单条路径」
    /// 扩大成「整次遍历」，父边的定义完全相同。
    #[test]
    fn closure_equals_the_union_of_all_paths() {
        let fixture = template_chain("graph-closure-equivalence");
        let project = fixture.project();
        let doc = load_doc(&project, "tpls/leaf.wxml").expect("doc");
        let mut target = None;
        walk(&doc, |element, _| {
            if element
                .attributes
                .iter()
                .any(|attr| attr.name == "class" && attr.value.as_deref() == Some("target"))
            {
                target = Some(element);
            }
        });
        let target = target.expect("target element");
        let graph = WxmlGraph::new(&project);

        let (paths, paths_truncated) = &*graph.ancestor_ids("tpls/leaf.wxml", target);
        let (closure, closure_truncated) = &*graph.ancestor_closure("tpls/leaf.wxml", target);
        assert!(!paths_truncated);
        assert!(!closure_truncated);

        let union: HashSet<AncestorId> = paths.iter().flatten().copied().collect();
        let set: HashSet<AncestorId> = closure.iter().copied().collect();
        assert_eq!(set, union);
        assert_eq!(paths.len(), 8, "每层两个使用点，三层就是 8 条路径");
        assert!(
            union.len() <= 20,
            "闭包应只有十几个节点，实际 {}",
            union.len()
        );
    }

    #[test]
    fn component_template_root_ends_at_its_usage() {
        let fixture = fixture(
            "graph-root",
            r#"<panel><view class="target"/></panel>"#,
            r#"<view class="host"><slot/></view>"#,
        );
        assert_eq!(
            chain(&fixture, "components/panel/panel.wxml", "host").0,
            vec!["panel".to_string()]
        );
    }

    #[test]
    fn self_referencing_component_terminates() {
        let fixture = fixture(
            "graph-cycle",
            r#"<panel><view class="target"/></panel>"#,
            r#"<view class="host"><slot/></view>"#,
        );
        let project = fixture.project();
        let graph = WxmlGraph::new(&project);
        let doc = load_doc(&project, "components/panel/panel.wxml").expect("doc");
        let element = match &doc.nodes[0] {
            crate::input::wxml::WxmlNode::Element(element) => element,
            _ => panic!("root element"),
        };
        assert!(
            graph
                .ancestors("components/panel/panel.wxml", element)
                .nodes
                .len()
                < super::MAX_NODES
        );
    }

    /// 未被任何使用点实例化的组件根：路径枚举停在静态未知的边上，必须报 truncated。
    ///
    /// 修复前「丢弃了一条可能路径」不置 truncated，调用方会把「剩下的路径都不命中」
    /// 当成确定不命中；这个用例钉住 dropped 并入 truncated。
    #[test]
    fn unplaced_component_root_is_truncated() {
        let fixture = fixture("graph-unplaced", "<view/>", r#"<view class="target"/>"#);
        let project = fixture.project();
        let graph = WxmlGraph::new(&project);
        let doc = load_doc(&project, "components/panel/panel.wxml").expect("doc");
        let element = match &doc.nodes[0] {
            crate::input::wxml::WxmlNode::Element(element) => element,
            _ => panic!("root element"),
        };
        let paths = graph.ancestor_ids("components/panel/panel.wxml", element);
        assert!(paths.0.is_empty());
        assert!(paths.1, "丢弃的路径必须让 truncated 为真");
    }

    /// `raw_start` 必须能把图里的节点查回源文档里的同一个元素。
    ///
    /// `line/column` 是 1-based 显示坐标，`raw_start` 是编译器 0 基坐标，两者混用
    /// 会静默指向文档里另一个元素——这条测试守住这个错位。
    #[test]
    fn flat_element_round_trips_to_its_source_element() {
        let fixture = fixture(
            "graph-roundtrip",
            r#"<view class="a"><text class="b">x</text><view class="c"/></view>"#,
            r#"<view class="host"><slot/></view>"#,
        );
        let project = fixture.project();
        let graph = WxmlGraph::new(&project);
        let mut seen = 0;
        for doc in 0..graph.doc_paths().count() {
            let path = graph.doc_path(doc).to_string();
            let source = load_doc(&project, &path).expect("doc");
            for idx in 0..graph.docs[doc].elements.len() {
                let flat = graph.flat(super::NodeId { doc, idx });
                let element = source
                    .element_at(flat.raw_start)
                    .unwrap_or_else(|| panic!("{path}: {}", flat.raw_start.0));
                assert_eq!(element.tag, flat.tag, "{path} 标签不一致");
                assert_eq!(
                    (element.tag_start.line, element.tag_start.utf16_col),
                    flat.raw_start,
                    "{path}"
                );
                assert_eq!(source.position(element.tag_start), (flat.line, flat.column));
                seen += 1;
            }
        }
        assert!(seen > 0);
    }
}
