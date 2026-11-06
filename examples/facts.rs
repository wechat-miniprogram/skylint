//! 打印一份 WXML 的元素事实：单元配置、每个元素的级联取值、逻辑祖先链。
//!
//! ```bash
//! cargo run --example facts -- <项目根> <wxml 路径> [属性…]
//! ```
//!
//! 这是 `skylint_core::inspect` 的最小用法示例：迁移工具拿它生成作业单，不需要自己
//! 重算级联或祖先链。

use std::collections::HashSet;

use skylint_core::{LoadedDoc, Project, Registry};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let root = args.next().ok_or("缺少项目根目录参数")?;
    let doc_path = args.next().ok_or("缺少 WXML 路径参数")?;
    let properties: Vec<String> = {
        let rest: Vec<String> = args.collect();
        if rest.is_empty() {
            vec![
                "display".into(),
                "position".into(),
                "width".into(),
                "height".into(),
            ]
        } else {
            rest
        }
    };

    let projects = Project::load_views(std::path::Path::new(&root))?;
    for project in &projects {
        // 视图内路径相对自己的根（插件视图带 `dist/plugin/` 前缀），所以按后缀匹配。
        let resolved = project
            .pages
            .iter()
            .filter_map(|page| page.files.wxml.as_deref())
            .chain(
                project
                    .components
                    .iter()
                    .filter_map(|component| component.files.wxml.as_deref()),
            )
            .chain(project.template_files.iter().map(String::as_str))
            .find(|path| *path == doc_path || path.ends_with(&doc_path));
        let Some(resolved) = resolved else {
            continue;
        };
        let doc_path = resolved.to_string();
        let loaded = LoadedDoc::load(project, &doc_path).ok_or("WXML 解析失败")?;
        let facts = loaded.facts(project);
        let unit = facts.unit();
        println!(
            "unit {:?} renderer={:?} reach={:?} display_block={} multiple_slots={} virtual_host={}",
            unit.kind,
            unit.renderer,
            unit.reach,
            unit.default_display_block,
            unit.multiple_slots,
            unit.virtual_host
        );

        for usage in facts.usages_recursive(4) {
            println!(
                "usage depth={} {}:{} <{}>",
                usage.depth, usage.doc, usage.line, usage.tag
            );
        }
        for element in facts.elements() {
            let mut line = format!(
                "{}:{}  <{}>",
                element.tag_start.line, element.tag_start.utf16_col, element.tag
            );
            for property in &properties {
                let Some(value) = facts.resolved(element, property) else {
                    continue;
                };
                let origin = value
                    .selector
                    .clone()
                    .or_else(|| Some("inline".to_string()))
                    .unwrap_or_default();
                line.push_str(&format!(
                    "  {property}={} [{origin} spec={}{}]",
                    value.value,
                    value.specificity,
                    if value.important { " !important" } else { "" }
                ));
            }
            let ancestors = facts.ancestors(element);
            if !ancestors.nodes.is_empty() {
                let chain: Vec<&str> = ancestors
                    .nodes
                    .iter()
                    .map(|node| node.tag.as_str())
                    .collect();
                line.push_str(&format!(
                    "  ancestors={}{}",
                    chain.join("<"),
                    if ancestors.open { " (open)" } else { "" }
                ));
            }
            println!("{line}");
            for handler in facts.handlers(element) {
                match &handler.function {
                    Some(function) => println!(
                        "    {} -> {}{} @{}:{}",
                        handler.attribute,
                        handler.name.as_deref().unwrap_or("?"),
                        if function.worklet { " (worklet)" } else { "" },
                        function.script,
                        function.line
                    ),
                    None => println!(
                        "    {} -> {} (no function)",
                        handler.attribute,
                        handler.name.as_deref().unwrap_or("dynamic")
                    ),
                }
            }
        }

        let ignored = HashSet::new();
        let views: Vec<&Project> = projects.iter().collect();
        let diagnostics = Registry::builtin().run_views(&views, &ignored);
        let count = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.path == doc_path)
            .count();
        println!("diagnostics in this doc: {count}");
        return Ok(());
    }
    Err(format!("工程里找不到 {doc_path}").into())
}
