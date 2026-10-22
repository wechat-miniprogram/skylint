//! `text-decoration-line` 只支持单值。

use crate::project::Project;
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

use super::value_util::{at_value, is_dynamic, split_tokens};
use super::visit;

pub struct TextDecorationLine;

impl TextDecorationLine {
    pub const ID: &'static str = "skyline/text-decoration-line";
}

impl Rule for TextDecorationLine {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "text-decoration-line 只支持单值，多值组合不生效",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit(project, |doc, out| {
            for rule in &doc.rules {
                for decl in &rule.declarations {
                    if !decl.name.eq_ignore_ascii_case("text-decoration-line") {
                        continue;
                    }
                    let value = decl.value.trim();
                    if value.is_empty() || is_dynamic(&decl.tokens) {
                        continue;
                    }
                    if split_tokens(&decl.tokens).len() <= 1 {
                        continue;
                    }
                    out.push(at_value(
                        Self::ID,
                        doc,
                        decl,
                        format!("`text-decoration-line: {value}` 是多值组合，Skyline 只支持单值"),
                        "只保留一个值，例如 `text-decoration-line: underline`",
                    ));
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;

    use super::super::testutil::{messages, single};
    use super::TextDecorationLine;

    #[test]
    fn text_decoration_line_reports_combinations() {
        let (_f, project) = single(
            "decoration-positive",
            r#"
            .a { text-decoration-line: underline line-through; }
            .b { text-decoration-line: overline underline; }
            .c { text-decoration-line: none underline; }
            "#,
        );
        let diags = TextDecorationLine.check(&project);
        assert_eq!(diags.len(), 3);
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Warning));
    }

    #[test]
    fn text_decoration_line_accepts_single_values_and_vars() {
        let (_f, project) = single(
            "decoration-negative",
            r#"
            .a { text-decoration-line: underline; }
            .b { text-decoration-line: none; }
            .c { text-decoration-line: line-through; }
            .d { text-decoration-line: var(--line); }
            "#,
        );
        assert_eq!(
            messages(&TextDecorationLine, &project),
            Vec::<String>::new()
        );
    }
}
