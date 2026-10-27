//! 规则注册表的自检：这些不变量一旦破坏，`--ignore`、CI 过滤与规则文档会一起失真。

use std::collections::HashSet;
use std::path::PathBuf;

use skylint::Registry;
use skylint::rules::Target;

#[test]
fn rule_ids_are_unique_and_namespaced() {
    let entries = Registry::builtin().entries();
    assert!(!entries.is_empty());

    let mut seen = HashSet::new();
    for (meta, target) in &entries {
        let prefix = match target {
            Target::Skyline | Target::Both => "skyline/",
            Target::Webview => "webview/",
        };
        assert!(
            meta.id.starts_with(prefix),
            "{} 的前缀与方向 {} 不符，应为 {prefix}",
            meta.id,
            target.as_str()
        );
        assert!(seen.insert(meta.id), "{} 重复注册", meta.id);
        assert!(!meta.description.trim().is_empty(), "{} 没有说明", meta.id);
    }
}

/// 两个方向都必须有规则：只有 Skyline 规则的注册表说明反向接线没生效。
#[test]
fn both_targets_have_rules() {
    assert!(
        !Registry::builtin()
            .for_target(Target::Webview)
            .entries()
            .is_empty()
    );
    assert!(
        !Registry::builtin()
            .for_target(Target::Skyline)
            .entries()
            .is_empty()
    );
}

/// `docs/RULES.md` 必须与注册表一致。漂了就重生成：
/// `cargo run --example rules_doc`。
#[test]
fn rules_doc_is_current() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/RULES.md");
    let actual = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("读不到 {}: {err}", path.display()));
    assert!(
        actual == Registry::builtin().rules_document(),
        "{} 与注册表不一致。跑 `cargo run --example rules_doc` 重生成",
        path.display()
    );
}
