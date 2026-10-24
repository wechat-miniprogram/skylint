//! 规则注册表的自检：这些不变量一旦破坏，`--ignore`、CI 过滤与规则文档会一起失真。

use std::collections::HashSet;
use std::path::PathBuf;

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

/// `docs/RULES.md` 必须与注册表一致。改了规则元信息后重跑：
/// `UPDATE_DOCS=1 cargo test -p skylint-core rules_doc`。
#[test]
fn rules_doc_is_current() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/RULES.md");
    let expected = Registry::builtin().rules_document();

    if std::env::var_os("UPDATE_DOCS").is_some() {
        std::fs::write(&path, &expected).expect("写入 docs/RULES.md 失败");
        return;
    }

    let actual = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("读不到 {}: {err}", path.display()));
    assert!(
        actual == expected,
        "{} 与注册表不一致。跑 `UPDATE_DOCS=1 cargo test -p skylint-core rules_doc` 重新生成",
        path.display()
    );
}
