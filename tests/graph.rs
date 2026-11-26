//! `skylint graph` 的端到端语义：单元清单从真磁盘工程来，边只认 `usingComponents`。

use std::fs;
use std::path::PathBuf;

use skylint::Project;
use skylint::graph::DependencyGraph;

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = fixture_root(name);
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn write(&self, rel: &str, content: &str) -> &Self {
        let path = self.root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
        self
    }

    fn graph(&self) -> DependencyGraph {
        let views = Project::load_views(&self.root).unwrap();
        DependencyGraph::of(&views)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn fixture_root(name: &str) -> PathBuf {
    let thread = std::thread::current();
    let test = thread.name().unwrap_or("shared").replace("::", "-");
    std::env::temp_dir().join(format!(
        "skylint-graph-{name}-{test}-{}",
        std::process::id()
    ))
}

/// 页面 → 组件 → 组件，app 级全局组件另挂一个节点，外加一对互相引用的组件。
fn project(name: &str) -> Fixture {
    let f = Fixture::new(name);
    f.write(
        "project.config.json",
        r#"{"miniprogramRoot":"miniprogram/"}"#,
    )
    .write(
        "miniprogram/app.json",
        r#"{"pages":["pages/index/index","pages/detail/index"],"renderer":"skyline","usingComponents":{"nav":"/components/nav/nav"}}"#,
    )
    .write("miniprogram/app.js", "App({})")
    .write(
        "miniprogram/pages/index/index.json",
        r#"{"usingComponents":{"card":"/components/card/index","loop":"/components/loop-a/index"}}"#,
    )
    .write("miniprogram/pages/index/index.wxml", "<card/>")
    .write("miniprogram/pages/index/index.js", "Page({})")
    .write("miniprogram/pages/detail/index.json", r#"{}"#)
    .write("miniprogram/pages/detail/index.wxml", "<nav/>")
    .write("miniprogram/pages/detail/index.js", "Page({})")
    .write("miniprogram/components/nav/nav.json", r#"{"component":true}"#)
    .write("miniprogram/components/nav/nav.wxml", "<view/>")
    .write("miniprogram/components/nav/nav.js", "Component({})")
    .write(
        "miniprogram/components/card/index.json",
        r#"{"component":true,"renderer":"webview","usingComponents":{"icon":"/components/icon/index"}}"#,
    )
    .write("miniprogram/components/card/index.wxml", "<icon/>")
    .write("miniprogram/components/card/index.js", "Component({})")
    .write("miniprogram/components/icon/index.json", r#"{"component":true}"#)
    .write("miniprogram/components/icon/index.wxml", "<view/>")
    .write("miniprogram/components/icon/index.js", "Component({})")
    .write(
        "miniprogram/components/loop-a/index.json",
        r#"{"component":true,"usingComponents":{"b":"/components/loop-b/index"}}"#,
    )
    .write("miniprogram/components/loop-a/index.js", "Component({})")
    .write(
        "miniprogram/components/loop-b/index.json",
        r#"{"component":true,"usingComponents":{"a":"/components/loop-a/index"}}"#,
    )
    .write("miniprogram/components/loop-b/index.js", "Component({})");
    f
}

fn node<'a>(graph: &'a DependencyGraph, id: &str) -> &'a skylint::graph::GraphNode {
    graph
        .nodes
        .iter()
        .find(|node| node.id == id)
        .unwrap_or_else(|| panic!("图里没有 `{id}`"))
}

#[test]
fn pages_components_and_app_are_separate_nodes() {
    let graph = project("nodes").graph();

    let page = node(&graph, "pages/index/index");
    assert_eq!(page.kind, "page");
    assert_eq!(page.renderer, Some("skyline"));
    assert_eq!(
        page.deps,
        vec![
            "components/card/index".to_string(),
            "components/loop-a/index".to_string(),
        ]
    );
    assert_eq!(page.wxml.as_deref(), Some("pages/index/index.wxml"));
    assert!(page.consumers.is_empty(), "页面不能被别的单元引用");

    let card = node(&graph, "components/card/index");
    assert_eq!(card.kind, "component");
    assert_eq!(
        card.renderer,
        Some("webview"),
        "组件 json 里写死的渲染器要带出来"
    );
    assert_eq!(card.consumers, vec!["pages/index/index".to_string()]);

    // app 级 usingComponents 挂在 app 节点，不摊到每个页面。
    let app = node(&graph, "app.json");
    assert_eq!(app.kind, "app");
    assert_eq!(app.deps, vec!["components/nav/nav".to_string()]);
    assert_eq!(
        node(&graph, "pages/detail/index").deps,
        Vec::<String>::new(),
        "detail 没有自己的 usingComponents，全局组件只记在 app 节点上"
    );
    assert_eq!(
        node(&graph, "components/nav/nav").consumers,
        vec!["app.json".to_string()]
    );
}

#[test]
fn text_output_nests_pages_over_components() {
    let text = project("text").graph().to_text(false);
    let lines = [
        "pages/index/index  (page, skyline)",
        "├─ components/card/index  (component, webview)",
        "│  └─ components/icon/index  (component, 继承)",
        "└─ components/loop-a/index  (component, 继承)",
        "   └─ components/loop-b/index  (component, 继承)",
        "      └─ components/loop-a/index  (component, 继承)  ↩ 环",
    ];
    for line in lines {
        assert!(text.contains(line), "缺 `{line}`：\n{text}");
    }
}

#[test]
fn reverse_lists_consumers_from_the_leaf() {
    let text = project("reverse").graph().to_text(true);
    assert!(
        text.contains(
            "components/icon/index  (component, 继承)\n\
             └─ components/card/index  (component, webview)\n   \
             └─ pages/index/index  (page, skyline)\n"
        ),
        "{text}"
    );
}

#[test]
fn plugin_view_nodes_are_prefixed_and_unique() {
    let f = Fixture::new("plugin");
    f.write(
        "project.config.json",
        r#"{"compileType":"plugin","miniprogramRoot":"dist/miniprogram/","pluginRoot":"dist/plugin/"}"#,
    )
    .write(
        "dist/miniprogram/app.json",
        r#"{"pages":["pages/index/index"]}"#,
    )
    .write("dist/miniprogram/pages/index/index.json", r#"{}"#)
    .write("dist/miniprogram/pages/index/index.wxml", "<view/>")
    .write(
        "dist/plugin/plugin.json",
        r#"{"pages":{"novel":"pages/novel/index"}}"#,
    )
    .write("dist/plugin/pages/novel/index.json", r#"{}"#)
    .write("dist/plugin/pages/novel/index.wxml", "<view/>");

    let graph = f.graph();
    let ids: Vec<&str> = graph.nodes.iter().map(|node| node.id.as_str()).collect();
    let unique: std::collections::BTreeSet<&&str> = ids.iter().collect();
    assert_eq!(ids.len(), unique.len(), "两个视图的节点不能撞 id：{ids:?}");
    assert!(ids.contains(&"pages/index/index"), "{ids:?}");
    assert!(
        ids.contains(&"dist/plugin/pages/novel/index"),
        "插件视图的路径要带 pluginRoot 前缀：{ids:?}"
    );
}

#[test]
fn json_carries_schema_version_and_every_node() {
    let graph = project("json").graph();
    let parsed: serde_json::Value = serde_json::from_str(&graph.to_json()).unwrap();
    assert_eq!(parsed["schemaVersion"], 1);
    let ids: Vec<&str> = parsed["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| node["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"pages/index/index"));
    assert!(ids.contains(&"components/icon/index"));
    assert!(ids.contains(&"app.json"));
    // 排序稳定，便于 diff。
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted);
}
