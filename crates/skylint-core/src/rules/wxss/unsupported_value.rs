//! 属性可用，但该**整值**在 Skyline 下不支持，声明会被静默忽略。

use crate::project::Project;
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

use super::dict_table::{normalize_value, table};
use super::value_util::at_declaration;
use super::visit_declarations;

pub struct UnsupportedValue;

impl UnsupportedValue {
    pub const ID: &'static str = "skyline/unsupported-value";
}

impl Rule for UnsupportedValue {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Error,
            description: "属性在 Skyline 下可用，但该值不支持，声明会被静默忽略",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let table = table();
        visit_declarations(project, |doc, decl, out| {
            let name = decl.name.to_ascii_lowercase();
            let Some(value) = normalize_value(&decl.value) else {
                return;
            };
            let Some(advice) = table.value(&name, &value) else {
                return;
            };
            out.push(at_declaration(
                Self::ID,
                doc,
                decl,
                Severity::Error,
                format!(
                    "`{name}: {}` 在 Skyline 下不受支持，该值会被静默忽略",
                    decl.value.trim()
                ),
                advice,
            ));
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;

    use super::UnsupportedValue;
    use crate::rules::testutil::wxss::{messages, single};

    #[test]
    fn value_reports_unsupported_values() {
        let (_f, project) = single(
            "value-positive",
            r#"
            .a { display: grid; }
            .b { position: sticky; }
            .c { overflow: auto; }
            .d { visibility: collapse; }
            .e { white-space: pre-wrap; }
            .f { font-weight: bolder; }
            .g { border-top-style: double; }
            .h { DISPLAY: GRID !important; }
            "#,
        );
        let diags = UnsupportedValue.check(&project);
        assert_eq!(diags.len(), 8);
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Error));
        assert!(diags.iter().any(|d| d.message.contains("display: grid")));
        assert!(diags.iter().any(|d| d.message.contains("position: sticky")));
        assert!(diags.iter().any(|d| d.message.contains("overflow: auto")));
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("border-top-style: double"))
        );
        assert_eq!(diags[0].help.as_deref(), Some("改用 flex 布局替代 grid"));
    }

    #[test]
    fn value_ignores_supported_values_vars_and_partial_matches() {
        let (_f, project) = single(
            "value-negative",
            r#"
            .a { display: flex; }
            .b { position: relative; }
            .c { overflow: hidden; }
            .d { display: var(--d); }
            .e { margin: 0 auto; }
            .f { background: url(grid.png); }
            .g { overflow: auto hidden; }
            .h { content: "pre-wrap"; }
            .i { display: env(display); }
            .j { width: calc(100% - 1px); }
            .k { text-decoration-line: underline line-through; }
            .l { --grid: 1; }
            .m { border-style: hidden; }
            .n { border-top-style: hidden; }
            "#,
        );
        assert_eq!(messages(&UnsupportedValue, &project), Vec::<String>::new());
    }
}
