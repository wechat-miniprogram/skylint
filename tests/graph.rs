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
    .write("miniprogram/app.wxss", ".global { color: #333; }\n")
    .write(
        "miniprogram/pages/index/index.json",
        r#"{"usingComponents":{"card":"/components/card/index","loop":"/components/loop-a/index","tpl-comp":"/components/tpl-comp/index"}}"#,
    )
    .write(
        "miniprogram/pages/index/index.wxml",
        "<import src=\"../../templates/header.wxml\"/>\n<include src=\"../../partials/tip.wxml\"/>\n<card/>",
    )
    .write(
        "miniprogram/pages/index/index.wxss",
        "@import \"../../styles/theme.wxss\";\n.a { color: red; }\n",
    )
    .write("miniprogram/pages/index/index.js", "Page({})")
    .write(
        "miniprogram/templates/header.wxml",
        "<import src=\"./inner.wxml\"/>\n<template name=\"header\"><view>h</view></template>\n",
    )
    .write(
        "miniprogram/templates/inner.wxml",
        "<template name=\"inner\"><view>i</view></template>\n",
    )
    .write("miniprogram/partials/tip.wxml", "<tpl-comp/>\n")
    .write(
        "miniprogram/components/tpl-comp/index.json",
        r#"{"component":true}"#,
    )
    .write("miniprogram/components/tpl-comp/index.wxml", "<view/>")
    .write("miniprogram/components/tpl-comp/index.js", "Component({})")
    .write("miniprogram/styles/theme.wxss", ".t { color: blue; }\n")
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
            "components/tpl-comp/index".to_string(),
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
        "├─ components/loop-a/index  (component, 继承)",
        "│  └─ components/loop-b/index  (component, 继承)",
        "│     └─ components/loop-a/index  (component, 继承)  ↩ 环",
        "├─ ⇢ partials/tip.wxml",
        "├─ ⇢ templates/header.wxml",
        "├─ ⇢ templates/inner.wxml",
        "└─ ⇢ styles/theme.wxss",
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
fn templates_and_styles_become_file_nodes() {
    let graph = project("files").graph();

    let page = node(&graph, "pages/index/index");
    assert_eq!(
        page.templates,
        vec![
            "partials/tip.wxml".to_string(),
            "templates/header.wxml".to_string(),
            // `header.wxml` 自己又 `<import>` 了它，闭包要展开到第二代。
            "templates/inner.wxml".to_string(),
        ]
    );
    assert_eq!(
        page.styles,
        vec!["styles/theme.wxss".to_string()],
        "单元自己的 wxss 不算引用"
    );

    let tip = node(&graph, "partials/tip.wxml");
    assert_eq!(tip.kind, "template");
    assert_eq!(tip.consumers, vec!["pages/index/index".to_string()]);
    assert!(tip.deps.is_empty(), "模板文件不引用组件");

    let theme = node(&graph, "styles/theme.wxss");
    assert_eq!(theme.kind, "style");
    assert_eq!(theme.consumers, vec!["pages/index/index".to_string()]);

    // 全局样式挂在 app 节点：它的 consumers 就是 app.json。
    let app = node(&graph, "app.json");
    assert_eq!(app.styles, vec!["app.wxss".to_string()]);
    assert_eq!(
        node(&graph, "app.wxss").consumers,
        vec!["app.json".to_string()]
    );
}

#[test]
fn file_nodes_are_part_of_the_tree() {
    let graph = project("files-tree").graph();
    let text = graph.to_text(false);
    assert!(text.contains("⇢ partials/tip.wxml"), "{text}");
    assert!(text.contains("⇢ styles/theme.wxss"), "{text}");

    // 反向落在模板上：直接回答「改这个文件影响哪些页面」。
    let reverse = graph.to_text(true);
    assert!(
        reverse.contains("partials/tip.wxml  (template)\n└─ pages/index/index"),
        "{reverse}"
    );
}

#[test]
fn used_lists_only_components_written_in_wxml() {
    let graph = project("used").graph();

    // `loop` 只在 json 里声明（wxml 没写），不该进 used；`tpl-comp` 写在被 include
    // 的片段里，算页面用到。
    assert_eq!(
        node(&graph, "pages/index/index").used,
        vec![
            "components/card/index".to_string(),
            "components/tpl-comp/index".to_string(),
        ]
    );
    assert_eq!(
        node(&graph, "components/card/index").used,
        vec!["components/icon/index".to_string()]
    );
    // app 级声明的组件按「页面实际写了这个标签」归到页面，不攒到 app 节点上。
    assert_eq!(
        node(&graph, "pages/detail/index").used,
        vec!["components/nav/nav".to_string()]
    );
    assert!(
        node(&graph, "app.json").used.is_empty(),
        "app 没有 wxml，不该有任何使用点"
    );
}

#[test]
fn tags_written_in_included_templates_count_as_used() {
    let graph = project("used-template").graph();
    // `tpl-comp` 只在被 `<include>` 的片段里出现，仍然算页面用到了它。
    assert!(
        node(&graph, "pages/index/index")
            .used
            .contains(&"components/tpl-comp/index".to_string()),
        "{:#?}",
        node(&graph, "pages/index/index").used
    );
}

#[test]
fn generic_values_count_as_used() {
    let f = Fixture::new("generic");
    f.write("project.config.json", r#"{"miniprogramRoot":"miniprogram/"}"#)
        .write(
            "miniprogram/app.json",
            r#"{"pages":["pages/index/index"]}"#,
        )
        .write("miniprogram/app.js", "App({})")
        .write(
            "miniprogram/pages/index/index.json",
            r#"{"usingComponents":{"r-list":"/components/r-list/index","nearby-cell":"/components/nearby-cell/index"}}"#,
        )
        .write(
            "miniprogram/pages/index/index.wxml",
            "<r-list generic:listitem=\"nearby-cell\"/>\n",
        )
        .write("miniprogram/pages/index/index.js", "Page({})")
        .write(
            "miniprogram/components/r-list/index.json",
            r#"{"component":true,"componentGenerics":{"listitem":true}}"#,
        )
        .write("miniprogram/components/r-list/index.wxml", "<view><listitem/></view>")
        .write("miniprogram/components/r-list/index.js", "Component({})")
        .write(
            "miniprogram/components/nearby-cell/index.json",
            r#"{"component":true}"#,
        )
        .write("miniprogram/components/nearby-cell/index.wxml", "<view/>")
        .write("miniprogram/components/nearby-cell/index.js", "Component({})");

    let graph = f.graph();
    let page = node(&graph, "pages/index/index");
    // `nearby-cell` 从不作为标签出现，只以泛型实现的值出现。
    assert!(
        page.used
            .contains(&"components/nearby-cell/index".to_string()),
        "{:#?}",
        page.used
    );
    assert!(
        page.used.contains(&"components/r-list/index".to_string()),
        "{:#?}",
        page.used
    );
    // 占位标签 `<listitem>` 是泛型名，不是组件，不该被归到 r-list 的 used 里。
    assert!(
        node(&graph, "components/r-list/index").used.is_empty(),
        "{:#?}",
        node(&graph, "components/r-list/index").used
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
    assert_eq!(parsed["schemaVersion"], 3);
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
