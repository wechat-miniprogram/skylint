//! WXML 逻辑树：把自定义组件的 `<slot>` 投影接上。
//!
//! [`crate::rules::wxml::walk`] 给的祖先栈只在单个文件里成立。组件模板里的
//! `<slot>` 在静态树上是个死胡同——运行时的子节点写在使用方文件里；反过来，
//! 使用方传进去的子节点渲染在组件模板里，它的祖先包含组件自己的节点。规则里凡
//! 是问「这个元素的祖先链上有没有 X」的（手势冲突、`display` 继承、滚动容器嵌
//! 套……），只看单文件就会漏掉跨边界的那一半。
//!
//! 这里建一张图：节点是 `(文档, 元素)`，边是运行时的父边。
//!
//! - 同一份文档内：普通父子；
//! - 使用方的子节点 → 组件模板里对应 `<slot>`（`slot="name"` 配 `<slot name>`，
//!   无名配默认 `<slot>`）；
//! - 组件模板的根节点 → 每一处用到这个组件的位置（可能有多处，图因此是 DAG）。
//!
//! 祖先查询是对 DAG 的存在性遍历：`ancestors` 返回所有可能祖先，去重、限深。

use std::collections::{HashMap, HashSet, VecDeque};

use crate::Project;
use crate::input::wxml::{WxmlAttribute, WxmlDoc, WxmlElement, WxmlNode};

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
/// `open` 表示链尾停在了一个解析不出来的自定义组件上：再往上是什么静态不可知，
/// 依赖祖先的判定必须放弃（宁可漏报）。
#[derive(Default)]
pub(crate) struct AncestorChain<'a> {
    pub nodes: Vec<(&'a str, &'a FlatElement)>,
    pub open: bool,
}

struct DocTree {
    path: String,
    elements: Vec<FlatElement>,
    /// 前序下标 → 源码位置，用于把元素查回下标。
    by_start: HashMap<(u32, u32), usize>,
    /// `<slot>`：名字（`None` = 默认）与下标。
    slots: Vec<(Option<String>, usize)>,
    /// 本文件里声明的组件标签 → 组件文档下标。
    components: HashMap<String, usize>,
}

/// 全工程的 WXML 逻辑树。
pub(crate) struct WxmlGraph {
    docs: Vec<DocTree>,
    by_path: HashMap<String, usize>,
    /// 组件文档下标 → 用到它的位置。
    usages: HashMap<usize, Vec<NodeId>>,
}

impl WxmlGraph {
    pub fn new(project: &Project) -> Self {
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

        // 组件路径（根相对、无扩展名）→ 组件 WXML 路径。
        let component_wxml: HashMap<&str, &str> = project
            .components
            .iter()
            .filter_map(|unit| Some((unit.path.as_str(), unit.files.wxml.as_deref()?)))
            .collect();

        for doc in sources {
            docs.push(DocTree::build(doc));
        }
        // 组件标签解析在文档建好之后：需要先知道每个组件文档的下标。
        for tree in &mut docs {
            let Some(targets) = project.tag_targets.get(&tree.path) else {
                continue;
            };
            for (tag, route) in targets {
                if let Some(wxml) = component_wxml.get(route.as_str())
                    && let Some(&idx) = by_path.get(*wxml)
                {
                    tree.components.insert(tag.clone(), idx);
                }
            }
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

        Self {
            docs,
            by_path,
            usages,
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
        for parent in self.parents(NodeId { doc, idx: start }) {
            queue.push_back(parent);
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
            let parents = self.parents(node);
            if parents.is_empty() {
                open = self.is_unresolved_custom(node);
            }
            for parent in parents {
                queue.push_back(parent);
            }
        }
        AncestorChain { nodes: out, open }
    }

    /// 这个节点是「解析不出来的自定义组件」：不在内置表里，它的文档也没声明
    /// 它能渲染成什么——它后面的祖先静态不可知。
    fn is_unresolved_custom(&self, node: NodeId) -> bool {
        let tree = &self.docs[node.doc];
        let tag = &tree.elements[node.idx].tag;
        tag != "slot"
            && !crate::rules::wxml::is_builtin(tag)
            && !tree.components.contains_key(&tag.to_ascii_lowercase())
    }

    /// 运行时父节点：可能有多个（组件被多处使用、同名 slot 有多个）。
    fn parents(&self, node: NodeId) -> Vec<NodeId> {
        let tree = &self.docs[node.doc];
        let Some(parent) = tree.elements[node.idx].parent else {
            // 文档根节点：渲染在被使用的位置。
            return self.usages.get(&node.doc).cloned().unwrap_or_default();
        };

        // 父元素是自定义组件 → 本元素被投影进组件的 `<slot>`。
        let parent_tag = tree.elements[parent].tag.to_ascii_lowercase();
        if let Some(&component) = tree.components.get(&parent_tag) {
            let name = tree.elements[node.idx]
                .attr("slot")
                .and_then(|attr| attr.value.clone());
            let slots = self.slot_nodes(component, name.as_deref());
            if !slots.is_empty() {
                return slots;
            }
        }

        vec![NodeId {
            doc: node.doc,
            idx: parent,
        }]
    }

    /// 组件文档里与 `name` 匹配的 `<slot>`。`None` 表示取默认 slot。
    fn slot_nodes(&self, doc: usize, name: Option<&str>) -> Vec<NodeId> {
        self.docs[doc]
            .slots
            .iter()
            .filter(|(slot, _)| slot.as_deref() == name)
            .map(|&(_, idx)| NodeId { doc, idx })
            .collect()
    }
}

impl DocTree {
    fn build(doc: WxmlDoc) -> Self {
        let path = doc.path().to_string();
        let mut elements = Vec::new();
        let mut by_start = HashMap::new();
        let mut slots = Vec::new();
        flatten(&doc.nodes, None, &doc, &mut elements, &mut by_start);
        for (idx, element) in elements.iter().enumerate() {
            if element.tag.eq_ignore_ascii_case("slot") {
                slots.push((element.slot_name.clone(), idx));
            }
        }
        Self {
            path,
            elements,
            by_start,
            slots,
            components: HashMap::new(),
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

        out.push(FlatElement {
            tag: element.tag.clone(),
            attributes: element.attributes.clone(),
            line,
            column,
            end_line,
            end_column,
            slot_name: element.slot_name.clone(),
            parent,
        });
        by_start.insert((element.tag_start.line, element.tag_start.utf16_col), idx);
        flatten(&element.children, Some(idx), doc, out, by_start);
    }
}

#[cfg(test)]
mod tests {
    use crate::rules::testutil::Fixture;
    use crate::rules::wxml::load_doc;

    use super::WxmlGraph;

    fn fixture(name: &str, page: &str, component: &str) -> Fixture {
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
            .write("components/panel/panel.js", "Component({});")
            .write("components/panel/panel.wxml", component);
        fixture
    }

    /// 收集 `tag` + `class` 命中的元素的逻辑祖先标签。
    fn ancestor_tags(fixture: &Fixture, path: &str, class: &str) -> Vec<String> {
        let project = fixture.project();
        let doc = load_doc(&project, path).expect("doc");
        let mut element = None;
        crate::rules::wxml::walk(&doc, |candidate, _| {
            if candidate
                .attributes
                .iter()
                .any(|attr| attr.name == "class" && attr.value.as_deref() == Some(class))
            {
                element = Some((candidate.tag_start.line, candidate.tag_start.utf16_col));
            }
        });
        let (line, col) = element.expect("element with class");
        // ancestors 用的是 tag_start 定位，这里按位置重新取一遍元素。
        let mut found = None;
        crate::rules::wxml::walk(&doc, |candidate, _| {
            if (candidate.tag_start.line, candidate.tag_start.utf16_col) == (line, col) {
                found = Some(candidate.clone());
            }
        });
        let element = found.expect("element");
        WxmlGraph::new(&project)
            .ancestors(path, &element)
            .nodes
            .iter()
            .map(|(_, flat)| flat.tag.clone())
            .collect()
    }

    #[test]
    fn slot_children_see_component_ancestors() {
        let fixture = fixture(
            "graph-slot",
            r#"<panel><view class="target"/></panel>"#,
            r#"<view class="host"><slot/></view>"#,
        );
        let tags = ancestor_tags(&fixture, "pages/index/index.wxml", "target");
        assert_eq!(
            tags,
            vec!["slot".to_string(), "view".to_string(), "panel".to_string()]
        );
    }

    #[test]
    fn named_slot_picks_its_own_parent_chain() {
        let fixture = fixture(
            "graph-named-slot",
            r#"<panel><view slot="a" class="target"/></panel>"#,
            r#"<view class="other"><slot name="a"/></view>"#,
        );
        let tags = ancestor_tags(&fixture, "pages/index/index.wxml", "target");
        assert_eq!(tags.first().map(String::as_str), Some("slot"));
        assert_eq!(tags.get(1).map(String::as_str), Some("view"));
    }

    #[test]
    fn component_template_root_ends_at_its_usage() {
        let fixture = fixture(
            "graph-root",
            r#"<panel><view class="target"/></panel>"#,
            r#"<view class="host"><slot/></view>"#,
        );
        let tags = ancestor_tags(&fixture, "components/panel/panel.wxml", "host");
        assert_eq!(tags, vec!["panel".to_string()]);
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
