//! `display: flex` 在 Skyline 下默认 `flex-direction: column`，WebView 为 `row`。
//!
//! 同一条规则内没有 `flex-direction` 时，迁移后布局方向会翻转，报 warning。

use crate::project::Project;
use crate::{Diagnostic, Severity};

use super::visit;
use crate::rules::{Rule, RuleMeta};

pub struct FlexDirection;

impl FlexDirection {
    pub const ID: &'static str = "skyline/flex-direction";
}

impl Rule for FlexDirection {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "display: flex 未声明 flex-direction，迁移后默认方向会从 row 变为 column",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit(project, |doc, out| {
            for rule in &doc.rules {
                // 无选择器的块是预处理器的 mixin 函数体：它只是片段，`display: flex`
                // 和 `flex-direction` 可能分散在定义处与调用处，判不了也不该判。
                if rule.selectors.is_empty() {
                    continue;
                }
                if rule.declaration("flex-direction").is_some() {
                    continue;
                }
                let Some(display) = rule.declarations.iter().find(|decl| {
                    decl.name.eq_ignore_ascii_case("display") && is_flex_value(&decl.value)
                }) else {
                    continue;
                };
                let (line, column) = doc.position(display.offset);
                let (end_line, end_column) = doc.position(display.value_end());
                out.push(
                    Diagnostic::new(
                        Self::ID,
                        Severity::Warning,
                        "display: flex 未显式声明 flex-direction；Skyline 默认 column，WebView 默认 row，迁移后布局方向会翻转",
                        doc.path(),
                        line,
                        column,
                    )
                    .end(end_line, end_column)
                    .help("为这条规则补上 `flex-direction: row`，或按 column 重新设计布局"),
                );
            }
        })
    }
}

fn is_flex_value(value: &str) -> bool {
    let lower = value.trim().to_ascii_lowercase();
    if lower.contains("var(") || lower.contains("env(") || lower.contains("{{") {
        return false;
    }
    matches!(lower.as_str(), "flex" | "inline-flex")
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;

    use super::super::testutil::{messages, single};
    use super::FlexDirection;

    #[test]
    fn flex_direction_reports_flex_without_direction() {
        let (_f, project) = single(
            "flex-positive",
            r#"
            .a { display: flex; }
            .b { display: inline-flex; color: red; }
            @media (min-width: 10px) { .c { display: flex; } }
            .d { display: FLEX; }
            "#,
        );
        let diags = FlexDirection.check(&project);
        assert_eq!(diags.len(), 4);
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Warning));
        // 嵌套在 @media 里的规则同样要报。
        assert!(diags.iter().any(|d| d.message.contains("flex-direction")));
    }

    #[test]
    fn flex_direction_ignores_declared_direction_block_and_vars() {
        let (_f, project) = single(
            "flex-negative",
            r#"
            .a { display: flex; flex-direction: column; }
            .b { display: block; }
            .c { flex-direction: row; }
            .d { display: var(--display); }
            .f { display: flex; flex-direction: var(--dir); }
            "#,
        );
        assert_eq!(messages(&FlexDirection, &project), Vec::<String>::new());
    }
}
