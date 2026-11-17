//! 祖先与 slot 投影查询：运行时父边、实例化路径、闭包、投影容器。

use std::collections::{HashSet, VecDeque};
use std::rc::Rc;

use crate::input::wxml::WxmlElement;
use crate::matcher::{FlatNode, node_of};

use super::{
    AncestorChain, AncestorClosure, AncestorId, AncestorIdPaths, AncestorNodePaths, AncestorRef,
    FlatElement, MAX_CLOSURE_VISITS, MAX_INCLUDE_DEPTH, MAX_NODES, MAX_PATHS, MAX_VISITS, NodeId,
    Parents, SlotHost, TOP, WxmlGraph,
};

impl WxmlGraph {
    /// 文档路径 + 元素 → 它的全部逻辑祖先（近 → 远）。
    ///
    /// 返回里带文档路径，跨组件边界的祖先来自别的文件，诊断和控制台输出都要能指过去。
    pub fn ancestors<'a>(&'a self, doc_path: &str, element: &WxmlElement) -> AncestorChain<'a> {
        let Some(start) = self.element_node(doc_path, element) else {
            return AncestorChain::default();
        };

        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut queue: VecDeque<NodeId> = VecDeque::new();
        match self.parents(start) {
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

    /// 渲染树里某个元素的**逐条**祖先路径（近 → 远）。
    ///
    /// [`Self::ancestors`] 是多使用点的并集，只能回答「有没有一条成立」；选择器匹配要
    /// 按顺序逐级回溯（后代 / 子代组合符），必须拿到真实的单条路径，所以单独提供。
    /// 静态不可知的边不产出路径（宁可漏报），路径数超过 [`MAX_PATHS`] 时截断。
    pub(crate) fn ancestor_paths<'a>(
        &'a self,
        doc_path: &str,
        element: &WxmlElement,
    ) -> Vec<Vec<AncestorRef<'a>>> {
        self.ancestor_paths_capped(doc_path, element).0
    }

    /// 同上，另外告诉你路径是否被 [`MAX_PATHS`] / [`MAX_VISITS`] 截断。
    ///
    /// 截断时「没有一条路径命中」不等于「哪条路径都不命中」，调用方要把后者当
    /// 判不了处理。
    pub(crate) fn ancestor_paths_capped<'a>(
        &'a self,
        doc_path: &str,
        element: &WxmlElement,
    ) -> (Vec<Vec<AncestorRef<'a>>>, bool) {
        let Some(start) = self.element_node(doc_path, element) else {
            return (Vec::new(), false);
        };
        let mut out = Vec::new();
        let mut seen = HashSet::from([start]);
        let mut budget = MAX_VISITS;
        let mut dropped = false;
        self.collect_paths(
            start,
            &mut Vec::new(),
            &mut out,
            &mut seen,
            &mut budget,
            &mut dropped,
        );
        let truncated = budget == 0 || out.len() >= MAX_PATHS || dropped;
        (out, truncated)
    }

    /// 模板文档里「这条选择器命中这个元素吗」的判定，按 (元素, 选择器) 缓存。
    ///
    /// 纯加速：结论只由 `compute` 决定，缓存键是文档实例 + 元素起点 + 选择器原文。
    pub(crate) fn cached_verdict(
        &self,
        doc_id: usize,
        element: &WxmlElement,
        webview: bool,
        selector: &str,
        compute: impl FnOnce() -> Option<bool>,
    ) -> Option<bool> {
        let key = (
            doc_id,
            (element.tag_start.line, element.tag_start.utf16_col),
            webview,
        );
        if let Some(hit) = self
            .verdict_cache
            .borrow()
            .get(&key)
            .and_then(|per_element| per_element.get(selector))
        {
            return *hit;
        }
        let value = compute();
        self.verdict_cache
            .borrow_mut()
            .entry(key)
            .or_default()
            .insert(selector.to_string(), value);
        value
    }

    /// 元素的实例化祖先路径（已转成匹配用节点），按元素缓存。
    ///
    /// 返回 `(paths, truncated)`；`paths` 为空 = 这个元素一条实例化路径都走不通，
    /// 依赖祖先的选择器无法判定。
    pub(crate) fn ancestor_nodes(
        &self,
        doc_path: &str,
        element: &WxmlElement,
    ) -> Rc<AncestorNodePaths> {
        let Some(node) = self.element_node(doc_path, element) else {
            return Rc::new((Vec::new(), false));
        };
        if let Some(hit) = self
            .ancestor_caches
            .nodes
            .borrow()
            .get(&(node.doc, node.idx))
        {
            return hit.clone();
        }
        let ids = self.ancestor_ids(doc_path, element);
        let nodes = ids
            .0
            .iter()
            .map(|path| {
                path.iter()
                    .map(|hop| match hop {
                        AncestorId::Node(node) => {
                            let flat = self.flat(*node);
                            node_of(&flat.tag, &flat.attributes)
                        }
                        AncestorId::PageRoot => FlatNode {
                            tag: "page".to_string(),
                            ..FlatNode::default()
                        },
                    })
                    .collect()
            })
            .collect();
        let value = Rc::new((nodes, ids.1));
        self.ancestor_caches
            .nodes
            .borrow_mut()
            .insert((node.doc, node.idx), value.clone());
        value
    }

    /// 元素的实例化祖先路径（节点身份），按元素缓存。
    ///
    /// 与 [`Self::ancestor_nodes`] 同源，区别是保住每个祖先的 `(文档, 元素)` 身份：
    /// 样式求值要回到祖先元素本身，才能用**它自己那份文档**的 `Tree` 与文档级作用域。
    pub(crate) fn ancestor_ids(
        &self,
        doc_path: &str,
        element: &WxmlElement,
    ) -> Rc<AncestorIdPaths> {
        let Some(node) = self.element_node(doc_path, element) else {
            return Rc::new((Vec::new(), false));
        };
        if let Some(hit) = self.ancestor_caches.ids.borrow().get(&(node.doc, node.idx)) {
            return hit.clone();
        }
        let (paths, truncated) = self.ancestor_paths_capped(doc_path, element);
        let ids = paths
            .iter()
            .map(|path| {
                path.iter()
                    .map(|hop| match hop {
                        AncestorRef::Element { node, .. } => AncestorId::Node(*node),
                        AncestorRef::PageRoot => AncestorId::PageRoot,
                    })
                    .collect()
            })
            .collect();
        let value = Rc::new((ids, truncated));
        self.ancestor_caches
            .ids
            .borrow_mut()
            .insert((node.doc, node.idx), value.clone());
        value
    }

    /// 元素的实例化祖先**闭包**：沿父边 BFS，按节点去重。
    ///
    /// 回答的是可达性（「祖先里有没有某个节点」）：路径数随使用点组合爆炸，节点数
    /// 不会，逐条路径枚举会把同一个祖先在不同路径上访问几十上百遍。闭包每个节点
    /// 只访问一次，代价 O(节点数 × 平均出边)。
    ///
    /// 与 [`Self::ancestor_ids`] 的路径并集等价（未截断时）：父边的定义完全相同，只是
    /// 去重范围从「每条路径内部」改成「整次遍历」。需要「每条路径都成立」的
    /// `Mode::All` 仍然只能逐路径。
    pub(crate) fn ancestor_closure(
        &self,
        doc_path: &str,
        element: &WxmlElement,
    ) -> Rc<AncestorClosure> {
        let Some(start) = self.element_node(doc_path, element) else {
            return Rc::new((Vec::new(), false));
        };
        if let Some(hit) = self
            .ancestor_caches
            .closure
            .borrow()
            .get(&(start.doc, start.idx))
        {
            return hit.clone();
        }

        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut queue: VecDeque<NodeId> = VecDeque::new();
        let mut page_root = false;
        let mut truncated = false;
        match self.parents(start) {
            Parents::Nodes(nodes) => queue.extend(nodes),
            // 页面顶层：DOM 根是 `<page>`（与 collect_paths 的判定同源）。
            Parents::Top => page_root = self.docs[start.doc].is_page,
            Parents::Unknown => truncated = true,
        }
        while let Some(node) = queue.pop_front() {
            if !seen.insert(node) {
                continue;
            }
            if out.len() >= MAX_CLOSURE_VISITS {
                truncated = true;
                break;
            }
            out.push(AncestorId::Node(node));
            match self.parents(node) {
                Parents::Nodes(nodes) => queue.extend(nodes),
                Parents::Top => page_root |= self.docs[node.doc].is_page,
                // 静态不可知的边：闭包在此断开，但其它分支仍然可达。
                Parents::Unknown => truncated = true,
            }
        }
        if page_root {
            out.push(AncestorId::PageRoot);
        }

        let value = Rc::new((out, truncated));
        self.ancestor_caches
            .closure
            .borrow_mut()
            .insert((start.doc, start.idx), value.clone());
        value
    }

    fn collect_paths<'a>(
        &'a self,
        node: NodeId,
        prefix: &mut Vec<AncestorRef<'a>>,
        out: &mut Vec<Vec<AncestorRef<'a>>>,
        seen: &mut HashSet<NodeId>,
        budget: &mut usize,
        dropped: &mut bool,
    ) {
        if out.len() >= MAX_PATHS || *budget == 0 {
            // 预算用尽 / 路径数封顶：还有一些可能路径没被枚举。
            *dropped = true;
            return;
        }
        *budget -= 1;
        match self.parents(node) {
            Parents::Nodes(nodes) if nodes.is_empty() => out.push(prefix.clone()),
            Parents::Nodes(nodes) => {
                for parent in nodes {
                    if out.len() >= MAX_PATHS || *budget == 0 {
                        *dropped = true;
                        return;
                    }
                    // 组件可以引用自身（`<slot>` 回环），按路径去重防环。
                    if !seen.insert(parent) {
                        *dropped = true;
                        continue;
                    }
                    if prefix.len() >= MAX_NODES {
                        *dropped = true;
                        seen.remove(&parent);
                        return;
                    }
                    prefix.push(AncestorRef::Element {
                        node: parent,
                        element: &self.docs[parent.doc].elements[parent.idx],
                    });
                    self.collect_paths(parent, prefix, out, seen, budget, dropped);
                    prefix.pop();
                    seen.remove(&parent);
                }
            }
            // 页面根在 WXML 里没有对应标签，但 WebView 的 DOM 根是 `<page>`，
            // `page view {}` 这类选择器靠它才成立。
            Parents::Top => {
                if self.docs[node.doc].is_page {
                    prefix.push(AncestorRef::PageRoot);
                    out.push(prefix.clone());
                    prefix.pop();
                } else {
                    out.push(prefix.clone());
                }
            }
            // 静态不可知的边：这条路径走不下去，不等于其它路径都断在这里。
            Parents::Unknown => *dropped = true,
        }
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
    pub(super) fn doc_top_parents(&self, doc: usize) -> Parents {
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
                // 默认槽的两种存储形态都认：`input/wxml.rs` 把 `<slot/>` 与静态空
                // `name=""` 归一成 `None`，其余构建路径可能留下 `Some("")`。
                return match tree
                    .slots
                    .iter()
                    .find(|(name, _)| name.is_none() || name.as_deref() == Some(""))
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
        element: &WxmlElement,
    ) -> Option<SlotHost<'a>> {
        let slot = slot_assignment(element);
        let host = *self.by_path.get(host_doc)?;
        let component = *self.docs[host]
            .components
            .get(&element.tag.to_ascii_lowercase())?;
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

/// 子节点被投进哪个槽：`None` = 没写（默认槽）；`Some(None)` = 动态槽名，判不了。
fn slot_assignment(element: &WxmlElement) -> Option<Option<&str>> {
    match element
        .attributes
        .iter()
        .find(|attr| attr.name.eq_ignore_ascii_case("slot"))
    {
        None => Some(None),
        Some(attr) if attr.is_static => Some(Some(attr.value.as_deref().unwrap_or(""))),
        Some(_) => None,
    }
}

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
