//! 单元依赖图：页面 / 组件之间通过 `usingComponents` 形成的引用关系，加上单元
//! 与模板文件（`<import>` / `<include>`）、样式文件（`@import`）之间的文件级引用。
//!
//! 数据全部来自工程加载的结果，不重新解析工程，也不跑规则：
//! [`Project::deps`]（组件闭包，npm 裸包名已回溯到 `miniprogram_npm`）、
//! [`Project::template_scopes`]（模板 → 引用它的单元，已是传递闭包）、
//! [`Project::styles`]（样式文件 → 引入它的单元，`@import` 同样展开）。
//!
//! 五类节点：
//! - 页面：key 是 app.json 里的 route（分包页已带 root 前缀）；
//! - 组件：key 是组件路径；
//! - app 配置（`app.json` / 插件的 `plugin.json`）：**app 级 `usingComponents` 挂在
//!   这个节点上，不摊到每个页面**。「页面能用哪些组件」与「页面声明了哪些组件」
//!   是两件事，图只记后者；要前者就沿 app 节点再展开一层。
//! - 模板 / 样式文件：只有 `consumers`（谁引用了它），没有 `deps`。单元自己的
//!   wxml / wxss 不算引用；`app.wxss` 这类全局样式挂在 app 节点上。
//!
//! 非声明式入口（`custom-tab-bar`、`app-bar` 这类不在任何 `usingComponents` 里的
//! 组件）在图里是根节点：没有任何单元的 `deps` 指向它。

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::project::{Project, Scope, UnitFiles};

/// 产物版本：字段改名 / 语义变化时加一。
pub const SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyGraph {
    pub schema_version: u32,
    pub nodes: Vec<GraphNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphNode {
    /// 页面 route、组件路径、app 配置文件名或文件路径；插件视图带 `pluginRoot/` 前缀。
    pub id: String,
    /// `page` / `component` / `app` / `template` / `style`。
    pub kind: &'static str,
    /// 该单元的 wxml（项目相对）；非单元节点与缺 wxml 的单元为 `None`。
    pub wxml: Option<String>,
    /// 页面是解析后的渲染器；组件与 app 只记自己 json 里写过的，未写为 `None`（继承）。
    pub renderer: Option<&'static str>,
    /// 直接引用的单元（已排序去重）。
    pub deps: Vec<String>,
    /// 被这个单元 `<import>` / `<include>` 的模板文件（含传递引入，已排序去重）。
    pub templates: Vec<String>,
    /// 这个单元引入的样式文件（`@import` 闭包；单元自己的 wxss 不算）。
    pub styles: Vec<String>,
    /// 直接引用本节点的单元 / 文件（已排序去重）。
    pub consumers: Vec<String>,
}

impl DependencyGraph {
    pub fn of(views: &[Project]) -> Self {
        let mut nodes: BTreeMap<String, GraphNode> = BTreeMap::new();
        for view in views {
            let prefix = view.report_prefix.as_str();
            let id_of = |key: &str| format!("{prefix}{key}");
            let wxml_of =
                |files: &UnitFiles| files.wxml.as_ref().map(|path| format!("{prefix}{path}"));
            let deps_of = |key: &str| -> Vec<String> {
                view.deps
                    .get(key)
                    .map(|deps| {
                        deps.iter()
                            .map(|dep| id_of(dep))
                            .collect::<BTreeSet<_>>()
                            .into_iter()
                            .collect()
                    })
                    .unwrap_or_default()
            };

            let app_id = id_of(&view.app_json_path);
            nodes.insert(
                app_id.clone(),
                GraphNode {
                    id: app_id.clone(),
                    kind: "app",
                    wxml: None,
                    renderer: declared_renderer(&view.app_json),
                    deps: deps_of(&view.app_json_path),
                    templates: Vec::new(),
                    styles: Vec::new(),
                    consumers: Vec::new(),
                },
            );
            for page in &view.pages {
                nodes.insert(
                    id_of(&page.route),
                    GraphNode {
                        id: id_of(&page.route),
                        kind: "page",
                        wxml: wxml_of(&page.files),
                        renderer: Some(if page.is_skyline {
                            "skyline"
                        } else {
                            "webview"
                        }),
                        deps: deps_of(&page.route),
                        templates: Vec::new(),
                        styles: Vec::new(),
                        consumers: Vec::new(),
                    },
                );
            }
            for component in &view.components {
                nodes.insert(
                    id_of(&component.path),
                    GraphNode {
                        id: id_of(&component.path),
                        kind: "component",
                        wxml: wxml_of(&component.files),
                        renderer: view
                            .component_json
                            .get(&component.path)
                            .and_then(declared_renderer),
                        deps: deps_of(&component.path),
                        templates: Vec::new(),
                        styles: Vec::new(),
                        consumers: Vec::new(),
                    },
                );
            }

            // 单元自己的 wxss 是单元的一部分，不算「引用了某个样式文件」。
            let own_wxss: BTreeMap<String, &str> = view
                .pages
                .iter()
                .filter_map(|page| {
                    page.files
                        .wxss
                        .as_deref()
                        .map(|path| (id_of(&page.route), path))
                })
                .chain(view.components.iter().filter_map(|component| {
                    component
                        .files
                        .wxss
                        .as_deref()
                        .map(|path| (id_of(&component.path), path))
                }))
                .collect();

            // `template_scopes` 是「模板 → 引用它的单元」，反过来就是单元的模板清单，
            // 且已经是沿 `<import>` / `<include>` 递归出来的传递闭包。
            let mut templates_of: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
            for (path, scopes) in &view.template_scopes {
                for scope in scopes {
                    let Some(unit) = unit_of(scope) else {
                        continue;
                    };
                    templates_of
                        .entry(id_of(unit))
                        .or_default()
                        .insert(format!("{prefix}{path}"));
                }
            }
            let mut styles_of: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
            for sheet in &view.styles {
                let path = sheet.doc.path();
                for scope in &sheet.scopes {
                    let owner = match scope {
                        Scope::Global => Some(app_id.clone()),
                        other => unit_of(other).map(id_of),
                    };
                    let Some(owner) = owner else {
                        continue;
                    };
                    if own_wxss.get(&owner) == Some(&path) {
                        continue;
                    }
                    styles_of
                        .entry(owner)
                        .or_default()
                        .insert(format!("{prefix}{path}"));
                }
            }
            for (unit, files) in templates_of {
                if let Some(node) = nodes.get_mut(&unit) {
                    node.templates = files.into_iter().collect();
                }
            }
            for (unit, files) in styles_of {
                if let Some(node) = nodes.get_mut(&unit) {
                    node.styles = files.into_iter().collect();
                }
            }
        }

        // 被引用但自己没有 json 的单元（缺文件时组件闭包不收录）：仍然放进图里，
        // 否则引用方那条边会指向一个不存在的节点，读图的人看不到「这里少了一份组件」。
        let dangling: Vec<String> = nodes
            .values()
            .flat_map(|node| node.deps.iter().cloned())
            .filter(|dep| !nodes.contains_key(dep))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        for id in dangling {
            nodes.insert(
                id.clone(),
                GraphNode {
                    id,
                    kind: "component",
                    wxml: None,
                    renderer: None,
                    deps: Vec::new(),
                    templates: Vec::new(),
                    styles: Vec::new(),
                    consumers: Vec::new(),
                },
            );
        }

        // 模板 / 样式文件各成一个节点，边从引用它们的单元指过来。
        let files: Vec<(String, &'static str)> = nodes
            .values()
            .flat_map(|node| {
                node.templates
                    .iter()
                    .map(|id| (id.clone(), "template"))
                    .chain(node.styles.iter().map(|id| (id.clone(), "style")))
            })
            .collect();
        for (id, kind) in files {
            nodes.entry(id.clone()).or_insert(GraphNode {
                id,
                kind,
                wxml: None,
                renderer: None,
                deps: Vec::new(),
                templates: Vec::new(),
                styles: Vec::new(),
                consumers: Vec::new(),
            });
        }

        let edges: Vec<(String, String)> = nodes
            .values()
            .flat_map(|node| {
                let owner = node.id.clone();
                node.deps
                    .iter()
                    .chain(node.templates.iter())
                    .chain(node.styles.iter())
                    .map(move |target| (owner.clone(), target.clone()))
            })
            .collect();
        for (consumer, target) in edges {
            if let Some(node) = nodes.get_mut(&target) {
                node.consumers.push(consumer);
            }
        }
        for node in nodes.values_mut() {
            node.consumers.sort();
            node.consumers.dedup();
        }

        Self {
            schema_version: SCHEMA_VERSION,
            nodes: nodes.into_values().collect(),
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("依赖图总是可序列化")
    }

    /// 文本森林。
    ///
    /// 正向（默认）以「没人引用」的单元为根，往下展开它引用的组件；模板与样式文件
    /// 用 `⇢` 前缀挂在引用它们的单元下。`reverse` 反转边：从叶子组件／文件往上
    /// 看「改了它会影响谁」。
    /// 同一个单元被多处引用时每处都完整展开（依赖图看的是路径，不图去重）；
    /// 碰到环标一句 `↩ 环` 就返回；整圈都是环、没有根的节点最后单独打出来。
    pub fn to_text(&self, reverse: bool) -> String {
        let mut tree = Tree {
            nodes: &self.nodes,
            index: self
                .nodes
                .iter()
                .enumerate()
                .map(|(index, node)| (node.id.as_str(), index))
                .collect(),
            reverse,
            visited: BTreeSet::new(),
            stack: Vec::new(),
            out: String::new(),
        };
        tree.render();
        tree.out
    }
}

/// 页面 / 组件作用域对应的单元 key；`Global` 与 `Loose` 不是单元。
fn unit_of(scope: &Scope) -> Option<&str> {
    match scope {
        Scope::Page(route) | Scope::Component(route) => Some(route),
        Scope::Global | Scope::Loose(_) => None,
    }
}

struct Tree<'a> {
    nodes: &'a [GraphNode],
    index: BTreeMap<&'a str, usize>,
    reverse: bool,
    visited: BTreeSet<String>,
    stack: Vec<String>,
    out: String,
}

impl Tree<'_> {
    fn render(&mut self) {
        let mut roots: Vec<String> = Vec::new();
        for node in self.nodes {
            // 一条边都没有的节点（登不进去的单页、没写全局组件的 app 节点、不引用
            // 任何东西的组件）打出来只是一行噪音；JSON 里仍然保留它们。
            if self.edges(node).is_empty() {
                continue;
            }
            let is_root = if self.reverse {
                node.deps.is_empty()
            } else {
                node.consumers.is_empty()
            };
            if is_root {
                roots.push(node.id.clone());
            }
        }
        for id in roots {
            self.walk(&id, "", "");
        }
        // 整圈都是环时没有根，从任意一个节点切进去把它打出来。
        let remaining: Vec<String> = self
            .nodes
            .iter()
            .filter(|node| !self.visited.contains(&node.id) && !self.edges(node).is_empty())
            .map(|node| node.id.clone())
            .collect();
        for id in remaining {
            if self.visited.contains(&id) {
                continue;
            }
            self.walk(&id, "", "");
        }
    }

    /// 当前方向上要展开的边：正向看依赖与引用的文件，`--reverse` 看引用方。
    fn edges(&self, node: &GraphNode) -> Vec<String> {
        if self.reverse {
            return node.consumers.clone();
        }
        node.deps
            .iter()
            .chain(node.templates.iter())
            .chain(node.styles.iter())
            .cloned()
            .collect()
    }

    fn walk(&mut self, id: &str, prefix: &str, connector: &str) {
        let Some(&index) = self.index.get(id) else {
            return;
        };
        let nodes = self.nodes;
        let node = &nodes[index];
        let (kind, renderer, node_id) = (node.kind, node.renderer, node.id.clone());
        let edges = self.edges(node);

        self.out.push_str(prefix);
        self.out.push_str(connector);
        match (kind, connector.is_empty()) {
            // 文件节点在树里用 `⇢` 区分「这是它引用的文件，不是组件」。
            ("template" | "style", false) => {
                self.out.push_str(&format!("⇢ {node_id}"));
            }
            ("template" | "style", true) => {
                self.out.push_str(&format!("{node_id}  ({kind})"));
            }
            _ => {
                self.out.push_str(&node_id);
                self.out
                    .push_str(&format!("  ({kind}, {})", renderer.unwrap_or("继承")));
            }
        }
        if self.stack.contains(&node_id) {
            self.out.push_str("  ↩ 环\n");
            return;
        }
        self.out.push('\n');
        self.visited.insert(node_id.clone());
        self.stack.push(node_id);
        let next_prefix = format!(
            "{prefix}{}",
            match connector {
                "" => "",
                "└─ " => "   ",
                _ => "│  ",
            }
        );
        let last = edges.len().saturating_sub(1);
        for (position, child) in edges.iter().enumerate() {
            self.walk(
                child,
                &next_prefix,
                if position == last {
                    "└─ "
                } else {
                    "├─ "
                },
            );
        }
        self.stack.pop();
    }
}

/// json 里显式写过的 `renderer`；没写或写了别的值都算「继承」。
fn declared_renderer(json: &serde_json::Map<String, serde_json::Value>) -> Option<&'static str> {
    match json.get("renderer").and_then(|value| value.as_str()) {
        Some("skyline") => Some("skyline"),
        Some("webview") => Some("webview"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 直接拼节点校验渲染：节点顺序按 id 排，等价于 `of()` 的产出顺序。
    fn graph(edges: &[(&str, &'static str, &str)]) -> DependencyGraph {
        let mut nodes: BTreeMap<String, GraphNode> = BTreeMap::new();
        for (id, kind, deps) in edges {
            nodes.insert(
                id.to_string(),
                GraphNode {
                    id: id.to_string(),
                    kind,
                    wxml: None,
                    renderer: None,
                    deps: deps.split_whitespace().map(str::to_string).collect(),
                    templates: Vec::new(),
                    styles: Vec::new(),
                    consumers: Vec::new(),
                },
            );
        }
        for node in nodes.values().cloned().collect::<Vec<_>>() {
            for dep in &node.deps {
                if let Some(target) = nodes.get_mut(dep) {
                    target.consumers.push(node.id.clone());
                }
            }
        }
        DependencyGraph {
            schema_version: SCHEMA_VERSION,
            nodes: nodes.into_values().collect(),
        }
    }

    #[test]
    fn text_tree_nests_dependencies() {
        let graph = graph(&[
            ("pages/index/index", "page", "components/card/index"),
            (
                "components/card/index",
                "component",
                "components/icon/index",
            ),
            ("components/icon/index", "component", ""),
        ]);
        let text = graph.to_text(false);
        assert_eq!(
            text,
            "pages/index/index  (page, 继承)\n\
             └─ components/card/index  (component, 继承)\n   \
             └─ components/icon/index  (component, 继承)\n"
        );
    }

    #[test]
    fn reverse_walks_consumers() {
        let graph = graph(&[
            ("pages/a/a", "page", "components/x/index"),
            ("pages/b/b", "page", "components/x/index"),
            ("components/x/index", "component", ""),
        ]);
        let text = graph.to_text(true);
        assert!(text.starts_with("components/x/index  (component, 继承)\n├─ pages/a/a"));
        assert!(text.contains("└─ pages/b/b"));
    }

    #[test]
    fn cycles_do_not_loop_forever() {
        let graph = graph(&[
            ("components/a/index", "component", "components/b/index"),
            ("components/b/index", "component", "components/a/index"),
        ]);
        let text = graph.to_text(false);
        assert_eq!(text.matches("↩ 环").count(), 1, "{text}");
    }
}
