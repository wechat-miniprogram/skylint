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

/// 选择器里影响「静态能不能判」的形状。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Shape {
    /// 含伪元素：声明的承载者是生成的文本节点，不是宿主元素。
    pub pseudo_element: bool,
    /// 含运行时伪类（`:hover` / `:active` / `:checked` …）。
    pub dynamic_pseudo: bool,
    /// 含组合符（祖先 / 兄弟关系）。
    pub combinator: bool,
    /// 指向运行时根节点（`page` 标签 / `:host`）：它们不在 WXML 文档的元素树里。
    pub runtime_root: bool,
}

/// 解析选择器并取出形状；解析失败返回 `None`。
pub fn shape(selector: &str) -> Option<Shape> {
    let list = imp::parse(selector)?;
    let mut shape = Shape::default();
    for one in list.slice() {
        for component in one.iter_raw_match_order() {
            walk_component(component, &mut shape);
        }
    }
    Some(shape)
}

/// 选择器里出现的全部 class 名（含 `:not()` / `:is()` 内的）；解析失败返回 `None`。
pub fn class_names(selector: &str) -> Option<Vec<String>> {
    let list = imp::parse(selector)?;
    let mut names = Vec::new();
    for one in list.slice() {
        for component in one.iter_raw_match_order() {
            collect_classes(component, &mut names);
        }
    }
    Some(names)
}

fn collect_classes(component: &Component<SelImpl>, out: &mut Vec<String>) {
    match component {
        Component::Class(name) => out.push(name.0.to_string()),
        Component::Negation(list) | Component::Is(list) => {
            for inner in list.slice() {
                for component in inner.iter_raw_match_order() {
                    collect_classes(component, out);
                }
            }
        }
        _ => {}
    }
}

fn walk_component(component: &Component<SelImpl>, shape: &mut Shape) {
    match component {
        Component::Combinator(_) => shape.combinator = true,
        Component::PseudoElement(_) => shape.pseudo_element = true,
        // `page` 是页面根节点，WXML 文档里没有对应元素。
        Component::LocalName(local) if local.name.0 == "page" => shape.runtime_root = true,
        Component::NonTSPseudoClass(pseudo) => {
            if pseudo.name == "host" {
                shape.runtime_root = true;
            } else if !STRUCTURAL_PSEUDO_CLASSES.contains(&pseudo.name.as_str()) {
                shape.dynamic_pseudo = true;
            }
        }
        Component::Negation(list) | Component::Is(list) => {
            for inner in list.slice() {
                for component in inner.iter_raw_match_order() {
                    walk_component(component, shape);
                }
            }
        }
        _ => {}
    }
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
    /// slot / `<template is>` 引出的静态未知占位节点：真实文档里没有对应元素，
    /// 但运行时这里可能出现任意节点。下标接在 `elems` 之后。
    synthetic: Vec<WxmlElement>,
    /// 与 `synthetic` 对齐：该占位来自 `<slot/>`（内容由父级提供，写在别的文件里）。
    synthetic_slot: Vec<bool>,
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
            synthetic: Vec::new(),
            synthetic_slot: Vec::new(),
            parent: Vec::new(),
            pos: Vec::new(),
            by_ptr: HashMap::new(),
            parsed: RefCell::new(HashMap::new()),
            unresolved: Cell::new(false),
        };
        collect(doc, &doc.nodes, None, &mut tree);
        tree.synthesize();
        tree
    }

    /// 给「静态未知」的区域补占位节点：`<slot/>` 与含 `<template is>` / `<include>`
    /// 的元素。
    ///
    /// 每个区域补 [`UNKNOWN_DEPTH`] 层父子链：投影 / 展开的内容自身可以任意嵌套，
    /// 一层只能覆盖「直接子节点」，多层才能让 `.a .b .c` 这类选择器在未知区域里
    /// 走到足够深的位置。
    fn synthesize(&mut self) {
        let mut hosts: Vec<(Option<usize>, bool)> = Vec::new();
        for idx in 0..self.elems.len() {
            let element = self.elems[idx];
            let host = if element.is_slot {
                // slot 会被父级内容替换：投影内容的父节点是 slot 的父节点。
                (self.parent[idx], true)
            } else if element.unknown_children {
                (Some(idx), false)
            } else {
                continue;
            };
            if !hosts.contains(&host) {
                hosts.push(host);
            }
        }
        let Some(donor) = self.elems.first().copied() else {
            return;
        };
        for (host, slot) in hosts {
            let mut parent = host;
            for _ in 0..UNKNOWN_DEPTH {
                let idx = self.elems.len() + self.synthetic.len();
                self.synthetic.push(WxmlElement {
                    tag: String::new(),
                    start: donor.start,
                    end: donor.end,
                    tag_start: donor.tag_start,
                    tag_end: donor.tag_end,
                    attributes: Vec::new(),
                    children: Vec::new(),
                    branches: Vec::new(),
                    in_template: false,
                    unknown_children: true,
                    is_slot: false,
                });
                self.synthetic_slot.push(slot);
                self.parent.push(parent);
                self.pos.push(0);
                parent = Some(idx);
            }
        }
    }

    pub fn matches(&self, element: &WxmlElement, selector: &str, dialect: Dialect) -> Tri {
        let Some(&idx) = self.by_ptr.get(&(element as *const WxmlElement as usize)) else {
            return Tri::No;
        };
        self.match_at(idx, selector, dialect)
    }

    /// 整份文档范围内的求值：真实元素 + slot / `<template is>` 引出的静态未知占位。
    ///
    /// 任一元素 `Yes` 就是 `Yes`；没有 `Yes` 但有 `Unknown` 就是 `Unknown`。
    /// 只有「每个真实元素都确定不命中、且未知区域也不可能命中」才是 `No`。
    pub fn any_match(&self, selector: &str, dialect: Dialect) -> Tri {
        let Some(shape) = shape(selector) else {
            return Tri::Unknown;
        };
        let mut unknown = false;
        for idx in 0..self.elems.len() + self.synthetic.len() {
            if idx >= self.elems.len() {
                let synthetic = idx - self.elems.len();
                // slot 的内容属于父级文档：单 compound 选择器问的是「本文件里有没有
                // 这个节点」，不该把投影内容算进来；只有跨过 slot 的祖先 / 兄弟关系
                // 才静态判不了。
                if self.synthetic_slot[synthetic] && !shape.combinator {
                    continue;
                }
            }
            match self.match_at(idx, selector, dialect) {
                Tri::Yes => return Tri::Yes,
                Tri::Unknown => unknown = true,
                Tri::No => {}
            }
        }
        if unknown {
            return Tri::Unknown;
        }
        // 内联 `<template name>` 的定义体只在实例化处渲染，真实祖先静态不可知。
        if shape.combinator && self.elems.iter().any(|element| element.in_template) {
            return Tri::Unknown;
        }
        Tri::No
    }

    fn match_at(&self, idx: usize, selector: &str, dialect: Dialect) -> Tri {
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
        let element = self.element_at(idx);
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
                // `[foo]` 只看存在性，动态绑定同样确定存在，静态可判。
                Component::AttributeInNoNamespaceExists { .. } => {}
                Component::AttributeInNoNamespace { local_name, .. } => {
                    if dialect == Dialect::WebView
                        && self.attr_condition_unresolved(local_name.0.as_str(), element)
                    {
                        dynamic = true;
                    }
                }
                Component::AttributeOther(other) => {
                    if dialect == Dialect::WebView
                        && self.attr_condition_unresolved(other.local_name.0.as_str(), element)
                    {
                        dynamic = true;
                    }
                }
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
                                Component::AttributeInNoNamespaceExists { .. } => {}
                                Component::AttributeInNoNamespace { local_name, .. } => {
                                    dynamic |= dialect == Dialect::WebView
                                        && self.attr_condition_unresolved(
                                            local_name.0.as_str(),
                                            element,
                                        );
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

    /// 属性取值条件静态判不了：动态绑定且候选集枚举不出来。
    ///
    /// 枚举得出候选时交给真正的匹配去判——全命中 / 全不命中都能定论，
    /// 只有混合候选才是 `Unknown`（由 `element::attr_verdict` 补标记）。
    fn attr_condition_unresolved(&self, name: &str, element: &WxmlElement) -> bool {
        self.attribute(element, name)
            .is_some_and(|attr| !attr.is_static && attr.candidates.is_none())
    }

    /// 动态 class 上这个 token 是否不可判定。
    ///
    /// 确定出现的词 → 判得出（不置标记）；词落在枚举候选里、或取值任意 → 不可判定；
    /// 其余 → 确定不出现。
    fn class_unresolved(&self, token: &str, element: &WxmlElement) -> bool {
        match self.attribute(element, "class") {
            Some(attr) if !attr.is_static => match &attr.class_tokens {
                Some(tokens) => {
                    !tokens.is_definite(token, false) && tokens.might_appear(token, false)
                }
                // 没有词元信息（解析器新增的动态形态）：一律不可判定。
                None => true,
            },
            _ => false,
        }
    }

    /// 兄弟区是否线性：含条件分支 / 循环 / 模板片段时序号不可静态确定。
    fn nonlinear_siblings(&self, idx: usize) -> bool {
        let element = self.element_at(idx);
        if element.in_template || self.doc.fragment {
            return true;
        }
        let siblings: &[WxmlNode] = match self.parent[idx] {
            Some(parent) => &self.element_at(parent).children,
            None => &self.doc.nodes,
        };
        if self.parent[idx].is_some_and(|p| self.element_at(p).unknown_children) {
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

    /// 元素的子节点。占位节点没有真实子节点，返回空切片。
    fn children_at(&self, idx: usize) -> &'a [WxmlNode] {
        match self.elems.get(idx) {
            Some(element) => &element.children,
            None => &[],
        }
    }

    fn sibling(&self, idx: usize, forward: bool) -> Option<&'a WxmlElement> {
        let parent = self.parent[idx]?;
        let children = self.children_at(parent);
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
        self.children_at(idx).iter().find_map(|node| match node {
            WxmlNode::Element(el) => Some(el),
            _ => None,
        })
    }

    fn index_of(&self, element: &WxmlElement) -> Option<usize> {
        self.by_ptr
            .get(&(element as *const WxmlElement as usize))
            .copied()
    }

    pub(crate) fn element_at(&self, idx: usize) -> &WxmlElement {
        match self.elems.get(idx) {
            Some(element) => element,
            None => &self.synthetic[idx - self.elems.len()],
        }
    }

    /// 下标是否指向静态未知占位节点（真实文档里没有这个元素）。
    pub(crate) fn is_synthetic(&self, idx: usize) -> bool {
        idx >= self.elems.len()
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

/// 每个静态未知区域补的占位层数：覆盖投影 / 展开内容里最右几层嵌套。
const UNKNOWN_DEPTH: usize = 3;

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
        wxml::parse("x.wxml", source, &crate::input::env::Env::default()).unwrap()
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
    fn enumerable_class_branches_exclude_other_words() {
        // 三目只能取这两个字面量：不在候选里的词确定不出现，而不是「判不了」。
        let d = doc(r#"<view class="{{ ok ? 'btn-active' : 'btn-idle' }}"/>"#);
        assert_eq!(check(&d, 0, ".btn-active", Dialect::WebView), Tri::Unknown);
        assert_eq!(check(&d, 0, ".btn-idle", Dialect::WebView), Tri::Unknown);
        assert_eq!(check(&d, 0, ".btn-disabled", Dialect::WebView), Tri::No);
        assert_eq!(check(&d, 0, ".other", Dialect::WebView), Tri::No);
    }

    #[test]
    fn enumerable_attribute_values_narrow_equality() {
        let d = doc(r#"<view mode="{{ok ? 'cover' : 'fill'}}"/>"#);
        // 候选里既有 cover 也有 fill：两种取值都可能，判不了。
        assert_eq!(
            check(&d, 0, "[mode='cover']", Dialect::WebView),
            Tri::Unknown
        );
        // 不在候选里的值一定不等。
        assert_eq!(
            check(&d, 0, "[mode='aspectFit']", Dialect::WebView),
            Tri::No
        );
        // 属性存在性与取值无关。
        assert_eq!(check(&d, 0, "[mode]", Dialect::WebView), Tri::Yes);
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
    fn slot_projection_is_unknown_only_across_relations() {
        let d = doc(r#"<view class="list"><slot/></view>"#);
        let tree = Tree::new(&d);
        // 后代关系穿过 slot：投影内容静态看不到。
        assert_eq!(
            tree.any_match(".list .item", Dialect::WebView),
            Tri::Unknown
        );
        // 单 compound 选择器不算投影内容，只看本文档。
        assert_eq!(tree.any_match(".item", Dialect::WebView), Tri::No);
        assert_eq!(tree.any_match(".list", Dialect::WebView), Tri::Yes);
    }

    #[test]
    fn unknown_children_make_every_descendant_unknown() {
        let d = doc(r#"<view class="list"><template is="row"/></view>"#);
        let tree = Tree::new(&d);
        assert_eq!(
            tree.any_match(".list .item", Dialect::WebView),
            Tri::Unknown
        );
        assert_eq!(tree.any_match(".item", Dialect::WebView), Tri::Unknown);
        assert_eq!(tree.any_match(".list", Dialect::WebView), Tri::Yes);
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
