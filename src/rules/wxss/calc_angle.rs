//! `calc()` 不支持角度计算。

use crate::input::wxss::ValueToken;
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

use super::value_util::{all_calls, at_value, has_angle_unit, is_dynamic};
use super::visit_declarations;

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
        visit_declarations(project, |doc, _, decl, out| {
            let value = decl.value.trim();
            if value.is_empty() || is_dynamic(&decl.tokens) {
                return;
            }
            if !all_calls(&decl.tokens)
                .iter()
                .any(|c| c.name == "calc" && has_angle_unit(c.args) && !foldable(c.args))
            {
                return;
            }
            out.push(at_value(
                Self::ID,
                doc,
                decl,
                format!("`{}` 的 calc() 包含角度计算，Skyline 不支持", decl.name),
                "角度不要放进 calc()，直接写角度值，例如 `rotate(45deg)`",
            ));
        })
    }
}

/// 纯角度常量表达式：float-pigment 会把它折叠成一个字面角度
/// （`ComputeCalcExpr<Angle>::try_compute`，`calc.rs:187-225`），到不了 Skyline 那一层。
/// 只有混入百分比 / 长度 / 变量时才留下无法求值的表达式。
fn foldable(args: &[ValueToken]) -> bool {
    args.iter().all(|token| match token {
        ValueToken::Comma | ValueToken::Number(_) => true,
        ValueToken::Dimension { unit, .. } => {
            matches!(
                unit.to_ascii_lowercase().as_str(),
                "deg" | "rad" | "grad" | "turn"
            )
        }
        ValueToken::Other(text) => matches!(text.as_str(), "+" | "-" | "*" | "/"),
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;

    use super::CalcAngle;
    use crate::rules::testutil::wxss::{messages, single};

    #[test]
    fn calc_angle_reports_angle_units() {
        let (_f, project) = single(
            "calc-positive",
            r#"
            .a { width: calc(100% - 10deg); }
            .b { margin-left: calc(1rad + 2px); }
            "#,
        );
        let diags = CalcAngle.check(&project);
        assert_eq!(diags.len(), 2);
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Warning));
    }

    #[test]
    fn pure_angle_calc_folds_and_is_accepted() {
        let (_f, project) = single(
            "calc-folded",
            r#"
            .a { transform: rotate(calc(45deg)); }
            .b { transform: rotate(calc(30deg + 15deg)); }
            .c { margin-left: calc(1rad * 2); }
            "#,
        );
        assert_eq!(messages(&CalcAngle, &project), Vec::<String>::new());
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
