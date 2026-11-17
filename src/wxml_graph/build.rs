//! 建图：把工程里的 WXML 文档摊平成节点、连上运行时父边。
//!
//! 只做构建；查询在 `mod.rs`（小访问器）、`ancestors.rs`（祖先与 slot 投影）、
//! `components.rs`（组件使用关系）。

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::Project;
use crate::input::wxml::{WxmlDoc, WxmlNode};
use crate::project::Scope;
use crate::project::paths::{posix_dirname, posix_join};

use super::{AncestorCaches, ComponentOptions, DocTree, FlatElement, NodeId, TOP, WxmlGraph};

/// 进图的文档集合。
///
/// 三类来源：页面的 WXML、组件的 WXML、被 `<import>` / `<include>` 引用的模板与片段。
/// 后者也要进图：`<import>` 的模板定义体可能被调用方实例化，`<include>` 的片段渲染在
/// 每个引入位置——它们的元素同样参与祖先与样式判定。
struct Sources {
    docs: Vec<Rc<WxmlDoc>>,
    /// 路径 → `docs` 下标（也是 [`NodeId::doc`]）。
    index: HashMap<String, usize>,
    /// 页面 WXML：渲染树的根，向上走到它就是 `<page>`。
    pages: HashSet<String>,
    /// 被引用的模板与片段：它们自己不是页面。
    templates: HashSet<String>,
}

impl Sources {
    fn collect(project: &Project) -> Self {
        let pages: HashSet<String> = project
            .pages
            .iter()
            .filter_map(|page| page.files.wxml.clone())
            .collect();
        let templates: HashSet<String> = project.template_files.iter().cloned().collect();

        let mut docs = Vec::new();
        let mut index = HashMap::new();
        let paths = project
            .pages
            .iter()
            .filter_map(|page| page.files.wxml.as_deref())
            .chain(
                project
                    .components
                    .iter()
                    .filter_map(|unit| unit.files.wxml.as_deref()),
            )
            .chain(project.template_files.iter().map(String::as_str));
        for path in paths {
            // 同一路径只进图一次：页面、组件、模板三处可能引用同一份 WXML。
            if index.contains_key(path) {
                continue;
            }
            let Some(doc) = crate::rules::wxml::load_doc(project, path) else {
                continue;
            };
            index.insert(path.to_string(), docs.len());
            docs.push(doc);
        }
        Self {
            docs,
            index,
            pages,
            templates,
        }
    }

    fn build(&self) -> Vec<DocTree> {
        self.docs
            .iter()
            .map(|doc| {
                DocTree::build(
                    doc,
                    self.pages.contains(doc.path()),
                    self.templates.contains(doc.path()),
                )
            })
            .collect()
    }
}

/// 解析组件标签与组件模板 `options`，并统计「组件 → 使用点」。
///
/// 必须在文档建好之后：两边都要用到文档下标。
fn link_components(
    project: &Project,
    docs: &mut [DocTree],
    index: &HashMap<String, usize>,
) -> HashMap<usize, Vec<NodeId>> {
    let options = component_options(project);
    // 组件路径（根相对、无扩展名）→ 组件 WXML 路径。
    let component_wxml: HashMap<&str, &str> = project
        .components
        .iter()
        .filter_map(|unit| Some((unit.path.as_str(), unit.files.wxml.as_deref()?)))
        .collect();

    for tree in docs.iter_mut() {
        if let Some(targets) = project.tag_targets.get(&tree.path) {
            for (tag, route) in targets {
                if let Some(wxml) = component_wxml.get(route.as_str())
                    && let Some(&idx) = index.get(*wxml)
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
    usages
}

/// 解析 `<include>` / `<import>` 的引入位置。
///
/// `<include>` 元素本身不产生元素节点，用「它所在的元素」表示，写在文档顶层时记
/// `None`（等价于该文档顶层）。
#[allow(clippy::type_complexity)]
fn link_includes(
    docs: &[DocTree],
    index: &HashMap<String, usize>,
) -> (
    HashMap<usize, Vec<(usize, Option<usize>)>>,
    HashMap<usize, Vec<usize>>,
) {
    let mut includes: HashMap<usize, Vec<(usize, Option<usize>)>> = HashMap::new();
    let mut importers: HashMap<usize, Vec<usize>> = HashMap::new();
    for (doc, tree) in docs.iter().enumerate() {
        let dir = posix_dirname(&tree.path);
        for (src, pos) in &tree.doc_includes {
            let Some(&target) = index.get(&resolve_ref(&dir, src)) else {
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
            if let Some(&target) = index.get(&resolve_ref(&dir, src)) {
                importers.entry(target).or_default().push(doc);
            }
        }
    }
    (includes, importers)
}

impl WxmlGraph {
    pub fn new(project: &Project) -> Self {
        let sources = Sources::collect(project);
        let mut docs = sources.build();
        let usages = link_components(project, &mut docs, &sources.index);
        let (includes, importers) = link_includes(&docs, &sources.index);
        Self {
            docs,
            by_path: sources.index,
            usages,
            includes,
            importers,
            ancestor_caches: AncestorCaches::default(),
            verdict_cache: RefCell::new(HashMap::new()),
        }
    }
}

impl DocTree {
    fn build(doc: &WxmlDoc, is_page: bool, doc_is_fragment: bool) -> Self {
        let path = doc.path().to_string();
        let mut elements = Vec::new();
        let mut by_start = HashMap::new();
        flatten(&doc.nodes, None, doc, &mut elements, &mut by_start);

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
            raw_start: (element.tag_start.line, element.tag_start.utf16_col),
            slot_name: element.slot_name.clone(),
            template: element.template.clone(),
            span,
            parent,
        });
        by_start.insert((element.tag_start.line, element.tag_start.utf16_col), idx);
        flatten(&element.children, Some(idx), doc, out, by_start);
    }
}

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
