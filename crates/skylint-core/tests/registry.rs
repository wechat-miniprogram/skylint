//! 规则注册表的自检：这些不变量一旦破坏，规则文档、`--ignore`、CI 过滤会一起失真。

use std::collections::HashSet;

use skylint_core::Registry;

#[test]
fn rule_ids_are_unique_and_namespaced() {
    let metas = Registry::builtin().metas();
    assert!(!metas.is_empty());

    let mut seen = HashSet::new();
    for meta in &metas {
        assert!(
            meta.id.starts_with("skyline/"),
            "{} 缺少 skyline/ 前缀",
            meta.id
        );
        assert!(seen.insert(meta.id), "{} 重复注册", meta.id);
        assert!(!meta.description.trim().is_empty(), "{} 没有说明", meta.id);
    }
}

#[test]
fn markdown_lists_every_rule() {
    let registry = Registry::builtin();
    let markdown = registry.to_markdown();
    for meta in registry.metas() {
        assert!(
            markdown.contains(&format!("`{}`", meta.id)),
            "规则文档漏了 {}",
            meta.id
        );
    }
}
