//! 生成 `docs/RULES.md`：`cargo run -p skylint-core --example rules_doc`。
//!
//! 文档由 `Registry::rules_document()` 产出，`tests/registry.rs` 只负责校验它没漂移。

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/RULES.md");
    std::fs::write(&path, skylint_core::Registry::builtin().rules_document())?;
    println!("wrote {}", path.display());
    Ok(())
}
