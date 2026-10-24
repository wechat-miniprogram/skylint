//! `min()` / `max()` / `clamp()` 在 Skyline 的数学表达式求值里是空分支。
//!
//! 证据：`css/css_parsing_utils.h:153-159` 的 `Min` / `Max` / `Clamp` 三个 case 都是
//! `// TODO` + `break`，函数末尾 `return MathNumberExpression::Create(0.f)`，所以
//! `width: min(100%, 300px)` 会直接变成 0。
//!
//! 例外：参数全是纯 px / 数字常量时，float-pigment 会先做常量折叠
//! （`float-pigment-css/src/typing.rs:149-162`、`parser/property_value/mod.rs:271-288`），
//! 这种写法到不了 Skyline 那一层，不报。

use crate::input::wxss::ValueToken;
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

use super::value_util::{all_calls, at_value, is_dynamic};
use super::visit_declarations;

const HELP: &str = "改用 JS 计算后传入固定值；参数全为 px / 数字常量时引擎会提前折叠，不受此限制";

/// 不可能含长度表达式的属性：值里出现 `min(` 等只可能是自定义标识符，报出来是噪音。
const EXCLUDED_PROPERTIES: &[&str] = &[
    "animation",
    "animation-name",
    "transition",
    "transition-property",
    "font-family",
];

pub struct CalcMinMax;

impl CalcMinMax {
    pub const ID: &'static str = "skyline/calc-min-max";
}

impl Rule for CalcMinMax {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "min() / max() / clamp() 在 Skyline 下求值为 0，整个表达式失效",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit_declarations(project, |doc, decl, out| {
            let name = decl.name.to_ascii_lowercase();
            if decl.value.trim().is_empty()
                || is_dynamic(&decl.tokens)
                || name.starts_with("--")
                || EXCLUDED_PROPERTIES.contains(&name.as_str())
            {
                return;
            }
            let Some(call) = all_calls(&decl.tokens)
                .into_iter()
                .find(|call| is_math_function(&call.name) && !is_constant(call.args))
            else {
                return;
            };
            out.push(at_value(
                Self::ID,
                doc,
                decl,
                format!(
                    "`{name}: {}` 里的 `{}()` 在 Skyline 下求值为 0，整个表达式失效",
                    decl.value.trim(),
                    call.name
                ),
                HELP,
            ));
        })
    }
}

fn is_math_function(name: &str) -> bool {
    matches!(name, "min" | "max" | "clamp")
}

/// 参数是否可常量折叠：数字、同一单位的长度、或全是百分比。
/// fp 的 `fold_length_extreme` / `fold_length_clamp` 要求「同一个 specified unit」
/// （`calc.rs:352-386`，百分比在 fp 里是 `Length::Ratio`，也参与折叠）。
fn is_constant(args: &[ValueToken]) -> bool {
    let mut unit: Option<String> = None;
    for token in args {
        let key = match token {
            ValueToken::Comma | ValueToken::Number(_) => continue,
            ValueToken::Percentage(_) => "%".to_string(),
            ValueToken::Dimension { unit, .. } => unit.to_ascii_lowercase(),
            _ => return false,
        };
        match &unit {
            None => unit = Some(key),
            Some(existing) if *existing == key => {}
            Some(_) => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;

    use super::CalcMinMax;
    use crate::rules::testutil::wxss::{messages, single};

    #[test]
    fn dynamic_arguments_are_reported() {
        let (_f, project) = single(
            "calc-min-max-positive",
            r#"
            .a { width: min(100%, 300px); }
            .b { height: clamp(100px, 20vh, 300px); }
            .c { padding: calc(min(8px, 2vw) + 4px); }
            .d { margin: max(10px, 5%); }
            .e { border-radius: min(1rem, 10px); }
            "#,
        );
        let diags = CalcMinMax.check(&project);
        assert_eq!(diags.len(), 5);
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Warning));
        assert!(diags[0].message.contains("`min()`"));
        assert!(diags[1].message.contains("`clamp()`"));
        assert!(diags[2].message.contains("`min()`"));
        assert!(diags[3].message.contains("`max()`"));
    }

    #[test]
    fn constant_arguments_and_dynamic_values_are_accepted() {
        let (_f, project) = single(
            "calc-min-max-negative",
            r#"
            .a { width: min(10px, 20px); }
            .b { width: clamp(1px, 2px, 3px); }
            .c { width: max(0, 10px); }
            .d { width: var(--w); }
            .e { --w: min(100%, 10px); }
            .f { transition: min .3s; }
            .g { width: calc(100% - 10px); }
            .h { width: min(1rem, 2rem); }
            .i { width: min(50%, 60%); }
            "#,
        );
        assert_eq!(messages(&CalcMinMax, &project), Vec::<String>::new());
    }
}
