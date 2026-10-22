//! `calc()` 不支持角度计算。

use crate::project::Project;
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

use super::value_util::{all_calls, at_value, has_angle_unit, is_dynamic};
use super::visit;

pub struct CalcAngle;

impl CalcAngle {
    pub const ID: &'static str = "skyline/calc-angle";
}

impl Rule for CalcAngle {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "calc() 不支持角度计算，表达式中的角度单位不生效",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit(project, |doc, out| {
            for rule in &doc.rules {
                for decl in &rule.declarations {
                    let value = decl.value.trim();
                    if value.is_empty() || is_dynamic(&decl.tokens) {
                        continue;
                    }
                    if !all_calls(&decl.tokens)
                        .iter()
                        .any(|c| c.name == "calc" && has_angle_unit(c.args))
                    {
                        continue;
                    }
                    out.push(at_value(
                        Self::ID,
                        doc,
                        decl,
                        format!("`{}` 的 calc() 包含角度计算，Skyline 不支持", decl.name),
                        "角度不要放进 calc()，直接写角度值，例如 `rotate(45deg)`",
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
    use super::CalcAngle;

    #[test]
    fn calc_angle_reports_angle_units() {
        let (_f, project) = single(
            "calc-positive",
            r#"
            .a { width: calc(100% - 10deg); }
            .b { transform: rotate(calc(45deg)); }
            .c { margin-left: calc(1rad * 2); }
            "#,
        );
        let diags = CalcAngle.check(&project);
        assert_eq!(diags.len(), 3);
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Warning));
    }

    #[test]
    fn calc_angle_accepts_lengths_plain_angles_and_vars() {
        let (_f, project) = single(
            "calc-negative",
            r#"
            .a { width: calc(100% - 10px); }
            .b { height: calc(10px + 5rpx); }
            .c { transform: rotate(45deg); }
            .d { width: calc(var(--x) + 1deg); }
            .e { padding-top: env(safe-area-inset-top); }
            "#,
        );
        assert_eq!(messages(&CalcAngle, &project), Vec::<String>::new());
    }
}
