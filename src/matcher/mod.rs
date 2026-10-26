//! 选择器 ↔ WXML 元素匹配。
//!
//! 两层：`selectors`（servo/stylo）负责 CSS 文法与匹配算法，本模块负责
//! 「WXML 树」这层适配 + 两种方言的差异 + 静态未知（[`Tri::Unknown`]）。
//!
//! 方言是必需的：同一段选择器在两个渲染器下的可匹配范围不同。
//!
//! - **WebView**：标准 CSS。
//! - **Skyline**：属性选择器的条件被丢弃（`[foo]` 等价 `*`，`css/selector.h` 的
//!   `CompoundSelector` 只有 id/tag/classes）；`:nth-of-type` 与 `::selection`
//!   之类没有对应 case，降级成「没有这个伪类」；而未知伪类（`:active` 等）会让
//!   整条规则在解析阶段被丢弃（`float-pigment-css/parser/mod.rs:989-991`）。
//!
//! 三态里只有 `Unknown` 是我们自己的：引擎的 `matches_selector` 只返回 bool
//! （`KleeneValue::Unknown` 仅在 invalidation 比较模式下产生）。所以不确定项在
//! 调用引擎**之前**就判定完，见 [`Tree::classify`]。

mod element;
mod imp;

pub use imp::{Ident, SelImpl};

use std::cell::Cell;
use std::cell::RefCell;
use std::collections::HashMap;

use selectors::context::{
    MatchingContext, MatchingForInvalidation, MatchingMode, NeedsSelectorFlags, QuirksMode,
    SelectorCaches,
};
use selectors::matching::matches_selector;
use selectors::parser::{Combinator, Component, Selector, SelectorList};

use crate::input::wxml::{WxmlDoc, WxmlElement, WxmlNode};

/// 选择器在哪个渲染器下求值。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    WebView,
    Skyline,
}

/// 主体 compound（最右一段）的构成。
///
/// Skyline 的 `CompoundSelector` 只保留 id / 标签 / class（`css/selector.h:78`），
/// 属性条件在转换时被丢弃（`css/css_parser_impl.h:191-220`），所以「主体有没有
/// id/tag/class」决定这条规则是「整条失效」还是「条件被放宽」。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Subject {
    pub has_tag: bool,
    pub has_id: bool,
    pub has_class: bool,
    /// 主体里有 `*`。
    pub universal: bool,
    /// 主体里的属性选择器名（小写，保持书写顺序）。
    pub attributes: Vec<String>,
    /// 主体之前（祖先 / 兄弟段）的属性选择器名：这些条件的丢弃同样会放宽匹配。
    pub other_attributes: Vec<String>,
    /// 主体带 `:host`：`rule_set.cc:150-158` 会给它单独的宿主桶，不受 id/tag/class 影响。
    pub has_host: bool,
}

impl Subject {
    /// 主体是否有 Skyline 能认的定位条件。
    pub fn has_locator(&self) -> bool {
        self.has_tag || self.has_id || self.has_class
    }
}

/// 这条选择器在 Skyline 下会不会被收进某个规则桶。
///
/// 主体 compound 要有 id / 标签 / class；`:host` 走宿主桶（`rule_set.cc:150-158`），
/// 也收得进去。两者都没有时规则根本不入集。
fn skyline_bucketed(selector: &Selector<SelImpl>) -> bool {
    for component in selector.iter_raw_match_order() {
        match component {
            Component::Combinator(_) => return false,
            Component::LocalName(_) | Component::ID(_) | Component::Class(_) => return true,
            Component::NonTSPseudoClass(pseudo) if pseudo.name.eq_ignore_ascii_case("host") => {
                return true;
            }
            _ => {}
        }
    }
    false
}

/// 解析一条选择器，取出主体 compound 的构成。
///
/// 选择器列表（`a, b`）返回 `None`：调用方按单个选择器处理，列表形态不猜。
pub fn subject(selector: &str) -> Option<Subject> {
    let list = imp::parse(selector)?;
    let [one] = list.slice() else {
        return None;
    };
    let mut subject = Subject::default();
    let mut in_subject = true;
    for component in one.iter_raw_match_order() {
        // 匹配顺序是从右往左：碰到组合符，主体就结束了。
        if matches!(component, Component::Combinator(_)) {
            in_subject = false;
            continue;
        }
        let attributes = if in_subject {
            &mut subject.attributes
        } else {
            &mut subject.other_attributes
        };
        match component {
            Component::LocalName(_) => subject.has_tag = true,
            Component::ID(_) => subject.has_id = true,
            Component::Class(_) => subject.has_class = true,
            Component::ExplicitUniversalType => subject.universal = true,
            Component::AttributeInNoNamespaceExists { local_name, .. }
            | Component::AttributeInNoNamespace { local_name, .. } => {
                attributes.push(local_name.0.to_string())
            }
            Component::AttributeOther(other) => attributes.push(other.local_name.0.to_string()),
            Component::NonTSPseudoClass(pseudo) if pseudo.name.eq_ignore_ascii_case("host") => {
                subject.has_host = true;
            }
            _ => {}
        }
    }
    Some(subject)
}

/// 动态 class 原文里「确定存在」的 token。
///
/// 只看插值之外的直属文本：`class="novel {{renderer}}"` → `novel`；
/// `class="a-{{b}}"` 整体丢弃（插值可能补出 `a-1`）；
/// `class="{{ r === 'skyline' ? 'skyline' : 'webview' }}"` 里的 `skyline`/`webview`
/// 属于表达式内部，不是确定的类名。
pub(crate) fn certain_tokens(raw: &str) -> Vec<&str> {
    let mut words = Vec::new();
    let mut cursor = 0;
    while cursor < raw.len() {
        let seg_end = raw[cursor..]
            .find("{{")
            .map_or(raw.len(), |offset| cursor + offset);
        let segment = &raw[cursor..seg_end];
        let from_value_start = cursor == 0;
        let to_value_end = seg_end == raw.len();
        let mut items: Vec<&str> = segment.split_whitespace().collect();
        // 紧贴插值的词只能算前缀/后缀，不成词。
        if !(from_value_start || segment.starts_with(char::is_whitespace)) && !items.is_empty() {
            items.remove(0);
        }
        if !(to_value_end || segment.ends_with(char::is_whitespace)) {
            items.pop();
        }
        words.extend(items);
        match raw[seg_end..].find("}}") {
            Some(close) => cursor = seg_end + close + 2,
            None => break,
        }
    }
    words
}

/// 三值判定：`Unknown` 表示静态阶段无法确定（动态属性 / 运行时状态 / 非线性兄弟区）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tri {
    No,
    Yes,
    Unknown,
}

/// 一份 WXML 文档的匹配视图：元素索引 + 父子 + 兄弟位置。
///
/// 元素地址来自 `WxmlDoc` 自己持有的 `Vec<WxmlNode>`，文档不可变期内稳定，
/// 因此用地址做索引是安全的（只用于查表，不做指针解引用之外的还原）。
pub struct Tree<'a> {
    doc: &'a WxmlDoc,
    elems: Vec<&'a WxmlElement>,
    parent: Vec<Option<usize>>,
    /// 在父节点里的「元素序号」（跳过文本 / 注释）。
    pos: Vec<usize>,
    by_ptr: HashMap<usize, usize>,
    parsed: RefCell<HashMap<String, Option<SelectorList<SelImpl>>>>,
    /// 本次匹配过程中是否问到了「静态判不了」的属性（含祖先节点）。
    /// `has_class` 等只能返回 `bool`，不可判定的情况由这个标记带回来。
    unresolved: Cell<bool>,
}

impl<'a> Tree<'a> {
    pub fn new(doc: &'a WxmlDoc) -> Self {
        let mut tree = Self {
            doc,
            elems: Vec::new(),
            parent: Vec::new(),
            pos: Vec::new(),
            by_ptr: HashMap::new(),
            parsed: RefCell::new(HashMap::new()),
            unresolved: Cell::new(false),
        };
        collect(doc, &doc.nodes, None, &mut tree);
        tree
    }

    pub fn matches(&self, element: &WxmlElement, selector: &str, dialect: Dialect) -> Tri {
        let Some(&idx) = self.by_ptr.get(&(element as *const WxmlElement as usize)) else {
            return Tri::No;
        };
        let Some(list) = self.parse(selector) else {
            // 解析失败：这条选择器我们看不见，比「不命中」更接近事实。
            return Tri::Unknown;
        };
        let mut unknown = false;
        for one in list.slice() {
            match self.match_one(one, idx, dialect) {
                Tri::Yes => return Tri::Yes,
                Tri::Unknown => unknown = true,
                Tri::No => {}
            }
        }
        if unknown { Tri::Unknown } else { Tri::No }
    }

    pub fn doc(&self) -> &'a WxmlDoc {
        self.doc
    }

    /// 这份文档的路径。
    pub fn path(&self) -> &'a str {
        self.doc.path()
    }

    /// 选择器文本是否能被解析（用于诊断「我们看不见这条选择器」）。
    pub fn parseable(&self, selector: &str) -> bool {
        self.parse(selector).is_some()
    }

    fn parse(&self, selector: &str) -> Option<SelectorList<SelImpl>> {
        if let Some(hit) = self.parsed.borrow().get(selector) {
            return hit.clone();
        }
        let parsed = imp::parse(selector);
        self.parsed
            .borrow_mut()
            .insert(selector.to_string(), parsed.clone());
        parsed
    }

    fn match_one(&self, selector: &Selector<SelImpl>, idx: usize, dialect: Dialect) -> Tri {
        if let Some(verdict) = self.classify(selector, idx, dialect) {
            return verdict;
        }
        self.unresolved.set(false);
        let node = element::Node {
            tree: self,
            idx,
            dialect,
        };
        let mut caches = SelectorCaches::default();
        let mut context = MatchingContext::new(
            MatchingMode::Normal,
            None,
            &mut caches,
            QuirksMode::NoQuirks,
            NeedsSelectorFlags::No,
            MatchingForInvalidation::No,
        );
        let matched = matches_selector(selector, 0, None, &node, &mut context);
        if matched {
            Tri::Yes
        } else if self.unresolved.replace(false) {
            // 有环节问到了不可判定的属性：不命中可能是「本轮不可判」而非「确实不命中」。
            Tri::Unknown
        } else {
            Tri::No
        }
    }

    /// 静态不可判定的情况，先于引擎短路。
    fn classify(&self, selector: &Selector<SelImpl>, idx: usize, dialect: Dialect) -> Option<Tri> {
        if dialect == Dialect::Skyline && !skyline_bucketed(selector) {
            // Skyline 按 id / tag / class 三类桶匹配（`selector_matcher.cc:456-500`），
            // 主体三者都没有的规则连规则集都进不去（`rule_set.cc:106-160`）——
            // `[foo]` 不是等价 `*`，而是整条声明从不生效。
            return Some(Tri::No);
        }
        let element = self.elems[idx];
        let mut positional = false;
        let mut dynamic = false;
        for component in selector.iter_raw_match_order() {
            match component {
                Component::Combinator(Combinator::NextSibling | Combinator::LaterSibling) => {
                    positional = true;
                }
                Component::Nth(_) | Component::NthOf(_) => positional = true,
                Component::NonTSPseudoClass(pseudo) => match dialect {
                    Dialect::Skyline => {
                        if !SKYLINE_PSEUDO_CLASSES.contains(&pseudo.name.as_str()) {
                            // 引擎在解析阶段就丢掉整条规则。
                            return Some(Tri::No);
                        }
                    }
                    Dialect::WebView => {
                        if !STRUCTURAL_PSEUDO_CLASSES.contains(&pseudo.name.as_str()) {
                            // `:hover` / `:active` / `:checked`：结构上可能命中，取决于运行时。
                            return Some(Tri::Unknown);
                        }
                    }
                },
                Component::PseudoElement(pseudo) => {
                    if dialect == Dialect::Skyline
                        && !matches!(pseudo.name.as_str(), "before" | "after")
                    {
                        // Skyline 把认不出的伪元素降级成「没有伪元素」，声明落在宿主上；
                        // 这个方向是过匹配，静态判不出来。
                        return Some(Tri::Unknown);
                    }
                    // 伪元素声明的承载者是生成的文本节点，不是宿主元素。
                    return Some(Tri::No);
                }
                Component::Class(name) => {
                    if self.class_unresolved(&name.0, element) {
                        dynamic = true;
                    }
                }
                Component::ID(_) => {
                    if self.has_dynamic(Some("id"), element) {
                        dynamic = true;
                    }
                }
                Component::AttributeInNoNamespaceExists { local_name, .. } => match dialect {
                    Dialect::Skyline => {}
                    Dialect::WebView => {
                        if self.has_dynamic(Some(local_name.0.as_str()), element) {
                            dynamic = true;
                        }
                    }
                },
                Component::AttributeInNoNamespace { local_name, .. } => match dialect {
                    Dialect::Skyline => {}
                    Dialect::WebView => {
                        if self.has_dynamic(Some(local_name.0.as_str()), element) {
                            dynamic = true;
                        }
                    }
                },
                Component::AttributeOther(other) => match dialect {
                    Dialect::Skyline => {}
                    Dialect::WebView => {
                        if self.has_dynamic(Some(other.local_name.0.as_str()), element) {
                            dynamic = true;
                        }
                    }
                },
                Component::Empty => {
                    if element.unknown_children {
                        dynamic = true;
                    }
                }
                Component::Negation(list) | Component::Is(list) => {
                    for inner in list.slice() {
                        for component in inner.iter_raw_match_order() {
                            match component {
                                Component::Class(name) => {
                                    if self.class_unresolved(&name.0, element) {
                                        dynamic = true;
                                    }
                                }
                                Component::ID(_) => {
                                    if self.has_dynamic(Some("id"), element) {
                                        dynamic = true;
                                    }
                                }
                                Component::AttributeInNoNamespaceExists { local_name, .. }
                                | Component::AttributeInNoNamespace { local_name, .. } => {
                                    dynamic |= dialect == Dialect::WebView
                                        && self.has_dynamic(Some(local_name.0.as_str()), element);
                                }
                                _ => {}
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if dynamic {
            return Some(Tri::Unknown);
        }
        if positional && self.nonlinear_siblings(idx) {
            // 按运行时 shadow tree 计数（`selector_matcher.cc:126`）：`wx:if` 的假分支
            // 不占位、`wx:for` 每份都是真兄弟，所以静态模板算不出序号。
            return Some(Tri::Unknown);
        }
        None
    }

    fn attribute<'e>(
        &self,
        element: &'e WxmlElement,
        name: &str,
    ) -> Option<&'e crate::input::wxml::WxmlAttribute> {
        element
            .attributes
            .iter()
            .find(|attr| attr.name.eq_ignore_ascii_case(name))
    }

    /// 选择器依赖的属性在元素上是动态绑定（`{{}}`）。
    fn has_dynamic(&self, name: Option<&str>, element: &WxmlElement) -> bool {
        name.and_then(|name| self.attribute(element, name))
            .is_some_and(|attr| !attr.is_static)
    }

    /// 动态 class 上这个 token 是否不可判定（原文里没写死）。
    fn class_unresolved(&self, token: &str, element: &WxmlElement) -> bool {
        match self.attribute(element, "class") {
            Some(attr) if !attr.is_static => !attr
                .raw_value
                .as_deref()
                .is_some_and(|raw| certain_tokens(raw).into_iter().any(|t| t.eq_ignore_ascii_case(token))),
            _ => false,
        }
    }

    /// 兄弟区是否线性：含条件分支 / 循环 / 模板片段时序号不可静态确定。
    fn nonlinear_siblings(&self, idx: usize) -> bool {
        let element = self.elems[idx];
        if element.in_template || self.doc.fragment {
            return true;
        }
        let siblings: &[WxmlNode] = match self.parent[idx] {
            Some(parent) => &self.elems[parent].children,
            None => &self.doc.nodes,
        };
        if self.parent[idx].is_some_and(|p| self.elems[p].unknown_children) {
            return true;
        }
        siblings.iter().any(|node| match node {
            WxmlNode::Element(el) => {
                !el.branches.is_empty()
                    || el.in_template
                    || self.attribute(el, "wx:for").is_some()
                    || self.attribute(el, "wx:if").is_some()
            }
            _ => false,
        })
    }

    fn sibling(&self, idx: usize, forward: bool) -> Option<&'a WxmlElement> {
        let parent = self.parent[idx]?;
        let children = &self.elems[parent].children;
        let here = self.pos[idx];
        let range: Box<dyn Iterator<Item = usize>> = if forward {
            Box::new(here + 1..children.len())
        } else {
            Box::new((0..here).rev())
        };
        for position in range {
            if let Some(WxmlNode::Element(el)) = children.get(position) {
                return Some(el);
            }
        }
        None
    }

    fn first_child(&self, idx: usize) -> Option<&'a WxmlElement> {
        self.elems[idx].children.iter().find_map(|node| match node {
            WxmlNode::Element(el) => Some(el),
            _ => None,
        })
    }

    fn index_of(&self, element: &WxmlElement) -> Option<usize> {
        self.by_ptr
            .get(&(element as *const WxmlElement as usize))
            .copied()
    }

    pub(crate) fn element_at(&self, idx: usize) -> &'a WxmlElement {
        self.elems[idx]
    }

    pub(crate) fn parent_idx(&self, idx: usize) -> Option<usize> {
        self.parent[idx]
    }

    pub(crate) fn sibling_idx(&self, idx: usize, forward: bool) -> Option<usize> {
        self.sibling(idx, forward).and_then(|el| self.index_of(el))
    }

    pub(crate) fn first_child_idx(&self, idx: usize) -> Option<usize> {
        self.first_child(idx).and_then(|el| self.index_of(el))
    }
}

/// Skyline 认识的结构伪类（`css/selector.h:67-76` 的 7 个 + `:is`/`:where` 走结构化组件）。
const SKYLINE_PSEUDO_CLASSES: &[&str] = &[
    "host",
    "empty",
    "only-child",
    "first-child",
    "last-child",
    "not",
    "nth-child",
    "nth-last-child",
    "is",
    "where",
];

/// WebView 下不需要运行时状态就能判定的伪类。
const STRUCTURAL_PSEUDO_CLASSES: &[&str] = &[
    "first-child",
    "last-child",
    "only-child",
    "empty",
    "root",
    "not",
    "is",
    "where",
    "nth-child",
    "nth-last-child",
    "nth-of-type",
    "nth-last-of-type",
    "first-of-type",
    "last-of-type",
    "only-of-type",
    "host",
];

fn collect<'a>(
    doc: &'a WxmlDoc,
    nodes: &'a [WxmlNode],
    parent: Option<usize>,
    tree: &mut Tree<'a>,
) {
    let mut position = 0;
    for node in nodes {
        let WxmlNode::Element(element) = node else {
            continue;
        };
        let idx = tree.elems.len();
        tree.elems.push(element);
        tree.parent.push(parent);
        tree.pos.push(position);
        tree.by_ptr
            .insert(element as *const WxmlElement as usize, idx);
        position += 1;
        collect(doc, &element.children, Some(idx), tree);
    }
    let _ = doc;
}

#[cfg(test)]
mod tests {
    use super::{Dialect, Tree, Tri};
    use crate::input::wxml::{self, WxmlDoc};

    /// 按文档顺序取第 n 个元素（0 起）。
    fn at(doc: &WxmlDoc, n: usize) -> &crate::input::wxml::WxmlElement {
        doc.elements().nth(n).expect("元素不存在")
    }

    fn check(doc: &WxmlDoc, n: usize, selector: &str, dialect: Dialect) -> Tri {
        Tree::new(doc).matches(at(doc, n), selector, dialect)
    }

    fn doc(source: &str) -> WxmlDoc {
        wxml::parse("x.wxml", source).unwrap()
    }

    // 元素序：0=根 .a.b#root，1=.c，2=.c.d，3=image.c
    const T1: &str = r#"<view class="a b" id="root"><view class="c"/><view class="c d"/><image class="c"/></view>"#;

    #[test]
    fn basic_compounds() {
        let d = doc(T1);
        assert_eq!(check(&d, 0, ".a", Dialect::WebView), Tri::Yes);
        assert_eq!(check(&d, 0, ".a.b", Dialect::WebView), Tri::Yes);
        assert_eq!(check(&d, 0, "#root", Dialect::WebView), Tri::Yes);
        assert_eq!(check(&d, 0, "view.a.b", Dialect::WebView), Tri::Yes);
        assert_eq!(check(&d, 0, ".c", Dialect::WebView), Tri::No);
        assert_eq!(check(&d, 1, ".a .missing", Dialect::WebView), Tri::No);
    }

    #[test]
    fn combinators() {
        let d = doc(T1);
        assert_eq!(check(&d, 1, ".b .c", Dialect::WebView), Tri::Yes);
        assert_eq!(check(&d, 1, ".b > .c", Dialect::WebView), Tri::Yes);
        // 中间隔了一层：`.a > .c` 对 `.c` 不成立（父是 `.a`，直接子正是它，成立）
        assert_eq!(check(&d, 1, ".a > .c", Dialect::WebView), Tri::Yes);
        // 相邻兄弟：第二个 `.c` 前一个是 `.c`；第一个不是。
        assert_eq!(check(&d, 2, ".c + .c", Dialect::WebView), Tri::Yes);
        assert_eq!(check(&d, 1, ".c + .c", Dialect::WebView), Tri::No);
        // 通用兄弟：`image` 前面有 `.c`。
        assert_eq!(check(&d, 3, ".c ~ image", Dialect::WebView), Tri::Yes);
    }

    #[test]
    fn structural_pseudo_classes() {
        // 0=.list，1=.item.x，2=.item，3=.item.y
        let d = doc(
            r#"<view class="list"><view class="item x"/><view class="item"/><view class="item y"/></view>"#,
        );
        assert_eq!(
            check(&d, 1, ".item:nth-child(1)", Dialect::WebView),
            Tri::Yes
        );
        assert_eq!(
            check(&d, 2, ".item:nth-child(2)", Dialect::WebView),
            Tri::Yes
        );
        assert_eq!(
            check(&d, 1, ".item:nth-child(2n+1)", Dialect::WebView),
            Tri::Yes
        );
        assert_eq!(
            check(&d, 2, ".item:nth-child(2n+1)", Dialect::WebView),
            Tri::No
        );
        assert_eq!(check(&d, 1, ".item:not(.x)", Dialect::WebView), Tri::No);
        assert_eq!(check(&d, 2, ".item:not(.x)", Dialect::WebView), Tri::Yes);
        assert_eq!(
            check(&d, 1, ".item:nth-last-child(n+2)", Dialect::WebView),
            Tri::Yes
        );
        assert_eq!(
            check(&d, 3, ".item:nth-last-child(n+2)", Dialect::WebView),
            Tri::No
        );
        assert_eq!(
            check(&d, 1, ".item:first-child", Dialect::WebView),
            Tri::Yes
        );
        assert_eq!(check(&d, 3, ".item:last-child", Dialect::WebView), Tri::Yes);
    }

    #[test]
    fn attribute_selectors_differ_by_dialect() {
        let d = doc(r#"<view class="x" data-theme="light" aria-checked="true"/>"#);
        assert_eq!(check(&d, 0, "[data-theme]", Dialect::WebView), Tri::Yes);
        assert_eq!(
            check(&d, 0, "[data-theme='light']", Dialect::WebView),
            Tri::Yes
        );
        assert_eq!(
            check(&d, 0, "[data-theme='dark']", Dialect::WebView),
            Tri::No
        );
        assert_eq!(check(&d, 0, "[class^='x']", Dialect::WebView), Tri::Yes);
        assert_eq!(
            check(&d, 0, "[aria-checked='TRUE' i]", Dialect::WebView),
            Tri::Yes
        );
        // Skyline 丢弃属性条件；但主体只剩属性时规则进不了规则桶，整条不生效。
        assert_eq!(
            check(&d, 0, "[data-theme='dark']", Dialect::Skyline),
            Tri::No
        );
        assert_eq!(
            check(&d, 0, ".x[data-theme='dark']", Dialect::Skyline),
            Tri::Yes
        );
    }

    #[test]
    fn dynamic_attributes_are_unknown() {
        let d = doc(r#"<view class="{{cls}} static"><text/></view>"#);
        // 原文里写死的 token 一定出现在运行时 class 列表里。
        assert_eq!(check(&d, 0, ".static", Dialect::WebView), Tri::Yes);
        assert_eq!(check(&d, 0, ".cls", Dialect::WebView), Tri::Unknown);
        assert_eq!(check(&d, 0, "[class='x']", Dialect::WebView), Tri::Unknown);
        // 不依赖动态属性的选择器照常判定。
        assert_eq!(check(&d, 0, "view", Dialect::WebView), Tri::Yes);
    }

    #[test]
    fn interpolated_token_boundaries_stay_unknown() {
        // `a-{{b}}` 可能拼成 `a-1`，`a-` 本身不是确定的类名。
        let d = doc(r#"<view class="a-{{b}}"/><view class="c{{d}} e"/>"#);
        assert_eq!(check(&d, 0, ".a-", Dialect::WebView), Tri::Unknown);
        assert_eq!(check(&d, 1, ".e", Dialect::WebView), Tri::Yes);
        assert_eq!(check(&d, 1, ".c", Dialect::WebView), Tri::Unknown);
    }

    #[test]
    fn tokens_inside_an_expression_are_not_certain() {
        // 三目表达式里的 `webview` 只是分支字面量，Skyline 下不会出现在 class 列表里。
        let d = doc(
            r#"<view class="c {{ r === 'skyline' ? 'skyline' : 'webview' }}"/><view class="dark"/>"#,
        );
        assert_eq!(check(&d, 0, ".c", Dialect::WebView), Tri::Yes);
        assert_eq!(check(&d, 0, ".c.webview", Dialect::WebView), Tri::Unknown);
        assert_eq!(check(&d, 0, ".c.skyline", Dialect::WebView), Tri::Unknown);
    }

    #[test]
    fn dynamic_ancestor_class_does_not_kill_the_selector() {
        // 迁移范式：根节点 `class="novel {{renderer}}"`，后代选择器靠 `novel` 这一节。
        let d = doc(r#"<view class="novel {{renderer}}"><view class="label"/></view>"#);
        assert_eq!(check(&d, 1, ".novel .label", Dialect::WebView), Tri::Yes);
        assert_eq!(check(&d, 1, ".novel .label", Dialect::Skyline), Tri::Yes);
        // 祖先的动态部分仍判不了。
        assert_eq!(
            check(&d, 1, ".skyline .label", Dialect::WebView),
            Tri::Unknown
        );
    }

    #[test]
    fn conditional_branches_make_positions_unknown() {
        let d = doc(
            r#"<view class="card"><view wx:if="{{p}}" class="t"/><view wx:else class="t"/></view>"#,
        );
        assert_eq!(check(&d, 1, ".t + .t", Dialect::WebView), Tri::Unknown);
        assert_eq!(
            check(&d, 1, ".t:nth-child(1)", Dialect::WebView),
            Tri::Unknown
        );
        // 纯后代关系不受影响。
        assert_eq!(check(&d, 1, ".card .t", Dialect::WebView), Tri::Yes);
    }

    #[test]
    fn unknown_pseudo_classes_are_dialect_dependent() {
        let d = doc(r#"<view class="x"/>"#);
        // Skyline：解析阶段整条规则被丢弃。
        assert_eq!(check(&d, 0, ".x:active", Dialect::Skyline), Tri::No);
        // WebView：运行时状态，静态不可知。
        assert_eq!(check(&d, 0, ".x:active", Dialect::WebView), Tri::Unknown);
        // 支持的伪类照常判定。
        assert_eq!(check(&d, 0, ".x:first-child", Dialect::Skyline), Tri::Yes);
    }
}
