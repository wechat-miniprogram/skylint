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

use std::collections::{HashMap, HashSet, VecDeque};

use crate::project::paths::{posix_dirname, posix_join};

use crate::Project;
use crate::input::wxml::{WxmlAttribute, WxmlDoc, WxmlElement, WxmlNode};
use crate::project::Scope;

/// `<template is>` 调用写在文档顶层：没有所在元素。
const TOP: usize = usize::MAX;

/// `<include>` 互相引用的最大展开深度，超过就按不可知处理（防环）。
const MAX_INCLUDE_DEPTH: usize = 16;

/// 遍历深度上限：项目里不该出现这么深的投影链，超了说明组件互相嵌套成环。
const MAX_NODES: usize = 512;

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

/// 全工程的 WXML 逻辑树。
pub(crate) struct WxmlGraph {
    docs: Vec<DocTree>,
    by_path: HashMap<String, usize>,
    /// 组件文档下标 → 用到它的位置。
    usages: HashMap<usize, Vec<NodeId>>,
    /// 片段文档下标 → 每个 `<include>` 的引入位置（`None` = 文档顶层）。
    includes: HashMap<usize, Vec<(usize, Option<usize>)>>,
    /// 文档下标 → `<import>` 了它的文档。
    importers: HashMap<usize, Vec<usize>>,
}

impl WxmlGraph {
    pub fn new(project: &Project) -> Self {
        let page_paths: HashSet<&str> = project
            .pages
            .iter()
            .filter_map(|page| page.files.wxml.as_deref())
            .collect();
        let options = component_options(project);

        let mut docs = Vec::new();
        let mut by_path = HashMap::new();
        let mut sources = Vec::new();
        let mut push = |path: &str| {
            if by_path.contains_key(path) {
                return;
            }
            let Some(doc) = crate::rules::wxml::load_doc(project, path) else {
                return;
            };
            by_path.insert(path.to_string(), sources.len());
            sources.push(doc);
        };
        for page in &project.pages {
            if let Some(path) = page.files.wxml.as_deref() {
                push(path);
            }
        }
        for component in &project.components {
            if let Some(path) = component.files.wxml.as_deref() {
                push(path);
            }
        }
        // 被 `<import>` / `<include>` 引用的片段：它们也有元素，且 `<import>` 的模板
        // 定义体可能被调用方使用，所以同样进图。
        let fragment_paths: HashSet<&str> =
            project.template_files.iter().map(String::as_str).collect();
        for path in &project.template_files {
            push(path);
        }

        // 组件路径（根相对、无扩展名）→ 组件 WXML 路径。
        let component_wxml: HashMap<&str, &str> = project
            .components
            .iter()
            .filter_map(|unit| Some((unit.path.as_str(), unit.files.wxml.as_deref()?)))
            .collect();

        for doc in sources {
            let path = doc.path().to_string();
            let is_page = page_paths.contains(path.as_str());
            let is_fragment = fragment_paths.contains(path.as_str());
            docs.push(DocTree::build(doc, is_page, is_fragment));
        }
        // 组件标签与模板引用解析在文档建好之后：需要先知道每个文档的下标。
        for tree in &mut docs {
            if let Some(targets) = project.tag_targets.get(&tree.path) {
                for (tag, route) in targets {
                    if let Some(wxml) = component_wxml.get(route.as_str())
                        && let Some(&idx) = by_path.get(*wxml)
                    {
                        tree.components.insert(tag.clone(), idx);
                    }
                }
            }
            tree.options = project
                .components
                .iter()
                .find(|unit| unit.files.wxml.as_deref() == Some(tree.path.as_str()))
                .and_then(|unit| options.get(&unit.path))
                .copied()
                .unwrap_or_default();
        }

        let mut usages: HashMap<usize, Vec<NodeId>> = HashMap::new();
        for (doc, tree) in docs.iter().enumerate() {
            for (idx, element) in tree.elements.iter().enumerate() {
                let tag = element.tag.to_ascii_lowercase();
                let Some(&component) = tree.components.get(&tag) else {
                    continue;
                };
                usages
                    .entry(component)
                    .or_default()
                    .push(NodeId { doc, idx });
            }
        }

        // 片段的引入位置：`<include>` 元素本身不产生元素节点，用「它所在的元素」表示，
        // 写在文档顶层时记 `None`（等价于该文档顶层）。
        let mut includes: HashMap<usize, Vec<(usize, Option<usize>)>> = HashMap::new();
        let mut importers: HashMap<usize, Vec<usize>> = HashMap::new();
        for (doc, tree) in docs.iter().enumerate() {
            let dir = posix_dirname(&tree.path);
            for (src, pos) in &tree.doc_includes {
                let Some(&target) = by_path.get(&resolve_ref(&dir, src)) else {
                    continue;
                };
                let enclosing = tree
                    .elements
                    .iter()
                    .enumerate()
                    .filter(|(_, element)| element.span.0 <= *pos && *pos <= element.span.1)
                    .max_by_key(|(_, element)| element.span.0)
                    .map(|(idx, _)| idx);
                includes.entry(target).or_default().push((doc, enclosing));
            }
            for src in &tree.doc_imports {
                if let Some(&target) = by_path.get(&resolve_ref(&dir, src)) {
                    importers.entry(target).or_default().push(doc);
                }
            }
        }

        Self {
            docs,
            by_path,
            usages,
            includes,
            importers,
        }
    }

    /// 这份文档在不在图里（模板片段、`<import>` 文件不在）。
    pub fn contains(&self, doc_path: &str) -> bool {
        self.by_path.contains_key(doc_path)
    }

    /// 文档路径 + 元素 → 它的全部逻辑祖先（近 → 远）。
    ///
    /// 返回里带文档路径，跨组件边界的祖先来自别的文件，诊断和控制台输出都要能指过去。
    pub fn ancestors<'a>(&'a self, doc_path: &str, element: &WxmlElement) -> AncestorChain<'a> {
        let Some(&doc) = self.by_path.get(doc_path) else {
            return AncestorChain::default();
        };
        let Some(&start) = self.docs[doc]
            .by_start
            .get(&(element.tag_start.line, element.tag_start.utf16_col))
        else {
            return AncestorChain::default();
        };

        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut queue: VecDeque<NodeId> = VecDeque::new();
        match self.parents(NodeId { doc, idx: start }) {
            Parents::Nodes(nodes) => queue.extend(nodes),
            Parents::Top => {}
            Parents::Unknown => {
                return AncestorChain {
                    nodes: Vec::new(),
                    open: true,
                };
            }
        }
        let mut open = false;
        while let Some(node) = queue.pop_front() {
            if !seen.insert(node) {
                continue;
            }
            out.push((
                self.docs[node.doc].path.as_str(),
                &self.docs[node.doc].elements[node.idx],
            ));
            if out.len() >= MAX_NODES {
                break;
            }
            match self.parents(node) {
                Parents::Nodes(nodes) => queue.extend(nodes),
                // 顶层元素如果是「不认识也不是内置」的标签：可能是没解析出实现的
                // 自定义组件，它上面的渲染树不可知（沿用旧口径，宁可漏报）。
                Parents::Top => open |= self.is_unknown_tag(node),
                Parents::Unknown => open = true,
            }
        }
        AncestorChain { nodes: out, open }
    }

    /// 这份文档（含递归使用的组件与引入的模板）能渲染到的全部文档路径，含自身。
    ///
    /// 用于「同一个页面里两个元素会不会互相覆盖」这类需要跨组件配对的判断：
    /// 组件实现里的元素与使用方文件里的元素同处一个渲染树。
    pub fn render_docs(&self, doc_path: &str) -> Vec<&str> {
        let Some(&start) = self.by_path.get(doc_path) else {
            return Vec::new();
        };
        let mut seen = HashSet::new();
        let mut queue = VecDeque::from([start]);
        let mut out = Vec::new();
        while let Some(doc) = queue.pop_front() {
            if !seen.insert(doc) {
                continue;
            }
            out.push(self.docs[doc].path.as_str());
            for &component in self.docs[doc].components.values() {
                queue.push_back(component);
            }
            // `<include>` 的内容是页面的一部分；`<import>` 只带模板定义，不进这个视图。
            for (fragment, sites) in &self.includes {
                if sites.iter().any(|(includer, _)| *includer == doc) {
                    queue.push_back(*fragment);
                }
            }
        }
        out
    }

    /// 运行时父节点。
    fn parents(&self, node: NodeId) -> Parents {
        let tree = &self.docs[node.doc];
        let element = &tree.elements[node.idx];
        let Some(parent) = element.parent else {
            return self.top_parents(node);
        };

        // 父元素是自定义组件 → 本元素被投影进它的某个 `<slot>`。
        let parent_tag = tree.elements[parent].tag.to_ascii_lowercase();
        if let Some(&component) = tree.components.get(&parent_tag) {
            return self.slot_parents(component, element);
        }
        Parents::Nodes(vec![NodeId {
            doc: node.doc,
            idx: parent,
        }])
    }

    /// 文档顶层元素：页面是渲染树的根，组件模板渲染在使用处，模板定义体渲染在调用处。
    fn top_parents(&self, node: NodeId) -> Parents {
        let tree = &self.docs[node.doc];
        let element = &tree.elements[node.idx];

        if let Some(name) = &element.template {
            // 定义体渲染在每个 `<template is="name">` 处；可见范围是本文件加上
            // `<import>` 了它的文件（`import` 不传递，`<include>` 不带定义）。
            let mut candidates = vec![node.doc];
            if let Some(importers) = self.importers.get(&node.doc) {
                candidates.extend(importers.iter().copied());
            }
            let mut out = Vec::new();
            for candidate in candidates {
                let Some(uses) = self.docs[candidate].template_uses.get(name) else {
                    continue;
                };
                for &idx in uses {
                    if idx == TOP {
                        match self.doc_top_parents(candidate) {
                            Parents::Nodes(nodes) => out.extend(nodes),
                            Parents::Top => {}
                            Parents::Unknown => return Parents::Unknown,
                        }
                    } else {
                        out.push(NodeId {
                            doc: candidate,
                            idx,
                        });
                    }
                }
            }
            return if out.is_empty() {
                Parents::Unknown
            } else {
                Parents::Nodes(out)
            };
        }

        self.doc_top_parents(node.doc)
    }

    /// 文档顶层元素渲染在哪：页面是根，组件模板渲染在使用处，片段渲染在每个引入位置。
    fn doc_top_parents(&self, doc: usize) -> Parents {
        self.doc_top_parents_at(doc, 0)
    }

    fn doc_top_parents_at(&self, doc: usize, depth: usize) -> Parents {
        let tree = &self.docs[doc];
        if tree.is_page {
            return Parents::Top;
        }
        // 片段被 `<include>` 整段内联：顶层元素落在每个 `<include>` 所在的位置。
        if tree.is_fragment {
            if depth > MAX_INCLUDE_DEPTH {
                return Parents::Unknown;
            }
            let Some(sites) = self.includes.get(&doc) else {
                // 没人引入：永远不渲染。
                return Parents::Unknown;
            };
            let mut placed = Vec::new();
            for &(includer, enclosing) in sites {
                placed.push(match enclosing {
                    // 内容是内联在 `<include>` 所在的位置，也就是这个元素的子级。
                    Some(idx) => Parents::Nodes(vec![NodeId { doc: includer, idx }]),
                    None => self.doc_top_parents_at(includer, depth + 1),
                });
            }
            return merge(placed);
        }
        let Some(usages) = self.usages.get(&doc) else {
            // 组件模板没有任何使用处：它永远不渲染。
            return Parents::Unknown;
        };
        if !tree.options.virtual_host {
            return Parents::Nodes(usages.clone());
        }
        // `virtualHost`：宿主元素不存在，模板顶层渲染在「宿主所在的位置」——也就是
        // 宿主元素的父级；如果那个父级是组件，仍然要经它的 `<slot>` 投影。
        let placed = usages
            .iter()
            .map(|&usage| self.place_usage(usage))
            .collect();
        merge(placed)
    }

    /// 一个使用位置被 `virtualHost` 抹掉后，它的父级是什么。
    ///
    /// 等价于「宿主元素的父节点」；父节点若是组件，还要经它的 `<slot>` 投影。
    fn place_usage(&self, usage: NodeId) -> Parents {
        let tree = &self.docs[usage.doc];
        let element = &tree.elements[usage.idx];
        let Some(parent) = element.parent else {
            return self.top_parents(usage);
        };
        let parent_tag = tree.elements[parent].tag.to_ascii_lowercase();
        match tree.components.get(&parent_tag) {
            Some(&component) => self.slot_parents(component, element),
            None => Parents::Nodes(vec![NodeId {
                doc: usage.doc,
                idx: parent,
            }]),
        }
    }

    /// 使用方传给组件的子节点，被投影进组件的哪个 `<slot>`。
    ///
    /// 语义见模块注释；没有匹配槽位时子节点不参与合成（不渲染），按不可知处理。
    fn slot_parents(&self, component: usize, child: &FlatElement) -> Parents {
        let tree = &self.docs[component];
        if tree.options.multiple_slots {
            let Some(attr) = child.attr("slot") else {
                return match tree
                    .slots
                    .iter()
                    .find(|(name, _)| name.as_deref() == Some(""))
                {
                    Some(&(_, idx)) => Parents::Nodes(vec![NodeId {
                        doc: component,
                        idx,
                    }]),
                    None => Parents::Unknown,
                };
            };
            if !attr.is_static {
                return Parents::Unknown;
            }
            let name = attr.value.as_deref().unwrap_or("");
            return match tree
                .slots
                .iter()
                .find(|(slot, _)| slot.as_deref().unwrap_or("") == name)
            {
                Some(&(_, idx)) => Parents::Nodes(vec![NodeId {
                    doc: component,
                    idx,
                }]),
                None => Parents::Unknown,
            };
        }
        if tree.options.dynamic_slots {
            return Parents::Unknown;
        }
        // `SlotMode.Single`：所有子节点都进模板里第一个 `<slot>`，`slot` 属性被忽略。
        match tree.slots.first() {
            Some(&(_, idx)) => Parents::Nodes(vec![NodeId {
                doc: component,
                idx,
            }]),
            None => Parents::Unknown,
        }
    }

    /// 使用方传进组件的子节点最终落在哪个元素下面（只看该元素的标签）。
    ///
    /// 用于「文本落在 `<text>` 里还是块容器里」这类需要看投影后容器的判定。
    /// 只处理 `tag` 确实是本文件里注册的组件的情况；其余返回 `None`，调用方按原样处理。
    pub fn projected_parent_tag<'a>(
        &'a self,
        host_doc: &str,
        tag: &str,
        slot: Option<Option<&str>>,
    ) -> Option<SlotHost<'a>> {
        let host = *self.by_path.get(host_doc)?;
        let component = *self.docs[host].components.get(&tag.to_ascii_lowercase())?;
        let tree = &self.docs[component];
        if tree.options.dynamic_slots {
            return Some(SlotHost::Unknown);
        }
        let slot_idx = if tree.options.multiple_slots {
            let name = match slot {
                Some(Some(name)) => name,
                Some(None) => "",
                None => return Some(SlotHost::Unknown),
            };
            tree.slots
                .iter()
                .find(|(slot, _)| slot.as_deref().unwrap_or("") == name)
                .map(|&(_, idx)| idx)
        } else {
            tree.slots.first().map(|&(_, idx)| idx)
        };
        let Some(idx) = slot_idx else {
            // 没有对应槽位：子节点不参与合成，即不渲染。
            return Some(SlotHost::NotRendered);
        };
        Some(match tree.elements[idx].parent {
            Some(parent) => SlotHost::Element(&tree.elements[parent].tag),
            // 槽在模板顶层：内容渲染在宿主位置，容器由使用方的父级决定。
            None => SlotHost::Unknown,
        })
    }

    /// 标签既不是内置组件、也没解析到组件实现。
    fn is_unknown_tag(&self, node: NodeId) -> bool {
        let tree = &self.docs[node.doc];
        let tag = tree.elements[node.idx].tag.to_ascii_lowercase();
        !crate::rules::wxml::is_builtin(&tag) && !tree.components.contains_key(&tag)
    }
}

impl DocTree {
    fn build(doc: WxmlDoc, is_page: bool, doc_is_fragment: bool) -> Self {
        let path = doc.path().to_string();
        let mut elements = Vec::new();
        let mut by_start = HashMap::new();
        flatten(&doc.nodes, None, &doc, &mut elements, &mut by_start);

        let mut slots = Vec::new();
        for (idx, element) in elements.iter().enumerate() {
            if element.tag.eq_ignore_ascii_case("slot") {
                slots.push((element.slot_name.clone(), idx));
            }
        }
        // `<template is="x">` 不产生元素节点（`input/wxml.rs` 只把它记进 `template_calls`），
        // 所以调用点用「它所在的元素」表示；写在文档顶层时记 `TOP`。
        let mut template_uses: HashMap<String, Vec<usize>> = HashMap::new();
        for call in &doc.template_calls {
            let enclosing = elements
                .iter()
                .enumerate()
                .filter(|(_, element)| {
                    let start = (call.start.line, call.start.utf16_col);
                    element.span.0 <= start && start <= element.span.1
                })
                .max_by_key(|(_, element)| element.span.0)
                .map_or(TOP, |(idx, _)| idx);
            template_uses
                .entry(call.name.clone())
                .or_default()
                .push(enclosing);
        }

        Self {
            doc_includes: doc
                .includes()
                .iter()
                .map(|(src, pos)| (src.clone(), (pos.line, pos.utf16_col)))
                .collect(),
            doc_imports: doc.imports().to_vec(),
            path,
            elements,
            by_start,
            slots,
            components: HashMap::new(),
            options: ComponentOptions::default(),
            template_uses,
            is_page,
            is_fragment: doc_is_fragment,
        }
    }
}

fn flatten(
    nodes: &[WxmlNode],
    parent: Option<usize>,
    doc: &WxmlDoc,
    out: &mut Vec<FlatElement>,
    by_start: &mut HashMap<(u32, u32), usize>,
) {
    for node in nodes {
        let WxmlNode::Element(element) = node else {
            continue;
        };
        let idx = out.len();
        let (line, column) = doc.position(element.tag_start);
        let (end_line, end_column) = doc.position(element.tag_end);
        // 原始 Position（编译器的 0 基坐标），与 `TemplateCall` 的坐标同一套。
        let span = (
            (element.start.line, element.start.utf16_col),
            (element.end.line, element.end.utf16_col),
        );

        out.push(FlatElement {
            tag: element.tag.clone(),
            attributes: element.attributes.clone(),
            line,
            column,
            end_line,
            end_column,
            slot_name: element.slot_name.clone(),
            template: element.template.clone(),
            span,
            parent,
        });
        by_start.insert((element.tag_start.line, element.tag_start.utf16_col), idx);
        flatten(&element.children, Some(idx), doc, out, by_start);
    }
}

/// 组件 JS 里的 `options`。多个脚本（分包、`behaviors` 之外的同名 scope）取并集：
/// 任一处写了 `true` 就算开了。
fn component_options(project: &Project) -> HashMap<String, ComponentOptions> {
    let mut out = HashMap::new();
    for unit in &project.components {
        let mut options = ComponentOptions::default();
        for script in project
            .scripts
            .iter()
            .filter(|script| script.scope == Scope::Component(unit.path.clone()))
        {
            let source = script.doc.source();
            options.multiple_slots |= flag(source, "multipleSlots");
            options.dynamic_slots |= flag(source, "dynamicSlots");
            options.virtual_host |= flag(source, "virtualHost");
        }
        out.insert(unit.path.clone(), options);
    }
    out
}

/// 源码里有没有 `name: true`（编译产物里的 `!0` 也算 true）。
///
/// 只认字面量：`multipleSlots: someVar` 之类判不出来，按默认值（未开启）处理——
/// 与 glass-easel 的默认值一致，只是行为设定在别处时这里会看错（记录在案）。
fn flag(source: &str, name: &str) -> bool {
    let mut rest = source;
    while let Some(at) = rest.find(name) {
        let after = &rest[at + name.len()..];
        let after = after.trim_start();
        if let Some(value) = after.strip_prefix(':') {
            let value = value.trim_start();
            if value.starts_with("true") || value.starts_with("!0") {
                return true;
            }
        }
        rest = after;
    }
    false
}

/// 合并多个「可能位置」：任一处不可知 → 不可知；任一处落在文档顶层 → 顶层；
/// 否则是所有父节点的并集。
fn merge(placed: Vec<Parents>) -> Parents {
    let mut out = Vec::new();
    let mut top = false;
    for placed in placed {
        match placed {
            Parents::Nodes(nodes) => out.extend(nodes),
            Parents::Top => top = true,
            Parents::Unknown => return Parents::Unknown,
        }
    }
    if out.is_empty() && !top {
        return Parents::Unknown;
    }
    if top {
        Parents::Top
    } else {
        Parents::Nodes(out)
    }
}

/// `<import src>` / `<include src>` 的路径解析：相对当前文件所在目录，`/` 开头相对根。
///
/// 编译器已剥掉 `.wxml` 后缀，所以这里补回来再查表。
fn resolve_ref(dir: &str, src: &str) -> String {
    let base = if let Some(absolute) = src.strip_prefix('/') {
        absolute.to_string()
    } else {
        posix_join(dir, src)
    };
    if base.ends_with(".wxml") {
        base
    } else {
        format!("{base}.wxml")
    }
}

#[cfg(test)]
mod tests {
    use crate::rules::testutil::Fixture;
    use crate::rules::wxml::load_doc;

    use super::WxmlGraph;

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
}
