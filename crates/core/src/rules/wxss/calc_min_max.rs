//! `min()` / `max()` / `clamp()` 在 Skyline 的数学表达式求值里是空分支。
//!
//! 证据：`css/css_parsing_utils.h:153-159` 的 `Min` / `Max` / `Clamp` 三个 case 都是
//! `// TODO` + `break`，函数末尾 `return MathNumberExpression::Create(0.f)`，所以
//! `width: min(100%, 300px)` 会直接变成 0。
//!
//! 例外：fp 在到达 Skyline 前就会常量折叠，这些写法到不了那一层，不报：
//! - 同 specified unit 的 `+`/`-`，以及带裸数字标量的 `*`/`/`
//!   （`parser/property_value/calc.rs:556-573`）；
//! - 嵌套的常量 `calc()`（`parser/property_value/mod.rs:262-270`）；
//! - number 型属性里的全数字 min/max/clamp（`parser/property_value/mod.rs:514-535`）。

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

/// number 型属性：fp 在这里走数字表达式求值（`parser/property_value/mod.rs:514-535`），
/// 全数字的 `min()` / `max()` / `clamp()` 会被常量折叠。
const NUMBER_PROPERTIES: &[&str] = &[
    "opacity",
    "z-index",
    "flex-grow",
    "flex-shrink",
    "order",
    "font-weight",
    "animation-iteration-count",
    "line-height",
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
        visit_declarations(project, |sheet, _rule, decl, out| {
            let doc = &sheet.doc;
            let name = decl.name.to_ascii_lowercase();
            if decl.value.trim().is_empty()
                || is_dynamic(&decl.tokens)
                || name.starts_with("--")
                || EXCLUDED_PROPERTIES.contains(&name.as_str())
            {
                return;
            }
            let number_property = NUMBER_PROPERTIES.contains(&name.as_str());
            let Some(call) = all_calls(&decl.tokens).into_iter().find(|call| {
                is_math_function(&call.name) && !is_constant(call.args, number_property)
            }) else {
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
/// `number_property` 为真时，全数字的参数也算可折叠。
fn is_constant(args: &[ValueToken], number_property: bool) -> bool {
    let mut parts = args.split(|token| matches!(token, ValueToken::Comma));
    if number_property && !args.is_empty() {
        return parts.all(folds_number);
    }
    // 逐参数折叠：同一 specified unit 的 `+`/`-` 会被 fp 折成字面量
    // （`calc.rs:302-369`）；裸数字在 length 上下文里 `try_compute` 返回 `None`，
    // 所以整条参数取不到值、留下 `Length::Expr` → 引擎按 0 处理。
    let mut unit: Option<String> = None;
    for arg in parts {
        let Some(own) = folds(arg) else {
            return false;
        };
        match &unit {
            None => unit = Some(own),
            Some(existing) if *existing == own => {}
            Some(_) => return false,
        }
    }
    unit.is_some()
}

/// 纯数字表达式（可含 `+ - * /` 和嵌套的常量 `calc()`）。
fn folds_number(arg: &[ValueToken]) -> bool {
    let mut has_number = false;
    for (index, token) in arg.iter().enumerate() {
        match token {
            ValueToken::Number(_) => has_number = true,
            // 除法右侧必须是裸数字且非 0，否则 fp 的 `try_compute` 放弃折叠。
            ValueToken::Other(op) if op == "/" => {
                match arg.get(index + 1).filter(|token| is_bare_number(token)) {
                    Some(divisor) if number_value(divisor) != Some(0.0) => {}
                    _ => return false,
                }
            }
            ValueToken::Other(op) if matches!(op.as_str(), "+" | "-" | "*") => {}
            ValueToken::Function { name, args } if name.eq_ignore_ascii_case("calc") => {
                if !folds_number(args) {
                    return false;
                }
                has_number = true;
            }
            _ => return false,
        }
    }
    has_number
}

fn is_bare_number(token: &ValueToken) -> bool {
    matches!(token, ValueToken::Number(_))
}

fn number_value(token: &ValueToken) -> Option<f64> {
    match token {
        ValueToken::Number(raw) => raw.parse().ok(),
        _ => None,
    }
}

fn folds(arg: &[ValueToken]) -> Option<String> {
    let mut unit: Option<String> = None;
    for (index, token) in arg.iter().enumerate() {
        let key = match token {
            ValueToken::Number(_) => continue,
            ValueToken::Percentage(_) => "%".to_string(),
            ValueToken::Dimension { unit, .. } => unit.to_ascii_lowercase(),
            ValueToken::Other(op) if op == "+" || op == "-" => continue,
            // 乘法只要一侧是裸数字（`10px * 2` / `2 * 10px`），单位不变。
            ValueToken::Other(op) if op == "*" => {
                let left = index > 0 && is_bare_number(&arg[index - 1]);
                let right = arg.get(index + 1).is_some_and(is_bare_number);
                if !left && !right {
                    return None;
                }
                continue;
            }
            // 除法右侧必须是裸数字且非 0（fp 的 `mul_div`）。
            ValueToken::Other(op) if op == "/" => {
                let divisor = arg.get(index + 1).filter(|token| is_bare_number(token))?;
                if number_value(divisor) == Some(0.0) {
                    return None;
                }
                continue;
            }
            // 嵌套的 `calc()` 递归判定（`parser/property_value/mod.rs:262-270`）。
            ValueToken::Function { name, args } if name.eq_ignore_ascii_case("calc") => {
                folds(args)?
            }
            _ => return None,
        };
        match &unit {
            None => unit = Some(key),
            Some(existing) if *existing == key => {}
            Some(_) => return None,
        }
    }
    // 整个参数都是裸数字：fp 折不成长度，引擎会求值 0。
    unit
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;

    use super::CalcMinMax;
    use crate::rules::testutil::wxss::{messages, page};

    #[test]
    fn dynamic_arguments_are_reported() {
        let (_f, project) = page(
            "calc-min-max-positive",
            r#"<view class="a"/><view class="b"/><view class="c"/><view class="d"/><view class="e"/>"#,
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

    /// 乘除标量、嵌套常量 `calc()` 都会被 fp 常量折叠，到不了 Skyline。
    #[test]
    fn scalar_mul_div_and_nested_calc_are_accepted() {
        let (_f, project) = page(
            "calc-min-max-folded",
            r#"<view class="a"/><view class="b"/><view class="c"/><view class="d"/><view class="e"/><view class="f"/>"#,
            r#"
            .a { width: min(10px * 2, 30px); }
            .b { width: max(2 * 10px, 30px); }
            .c { width: min(calc(10px + 10px), 30px); }
            .d { width: clamp(1px * 2, 20px, 3px * 2); }
            .e { width: min(10px / 2, 30px); }
            .f { margin: max(100% / 2, 10%); }
            "#,
        );
        assert_eq!(messages(&CalcMinMax, &project), Vec::<String>::new());
    }

    /// number 型属性上的全数字 min/max/clamp 会被 fp 折叠。
    #[test]
    fn number_properties_with_all_number_math_are_accepted() {
        let (_f, project) = page(
            "calc-min-max-number-properties",
            r#"<view class="a"/><view class="b"/><view class="c"/><view class="d"/><view class="e"/><view class="f"/>"#,
            r#"
            .a { opacity: min(0.5, 0.8); }
            .b { z-index: max(1, 2); }
            .c { flex-grow: max(0.5, 1); }
            .d { line-height: clamp(1, 1.5, 2); }
            .e { font-weight: min(400, 700); }
            .f { animation-iteration-count: min(2, 3); }
            "#,
        );
        assert_eq!(messages(&CalcMinMax, &project), Vec::<String>::new());
    }

    /// 单位不统一、乘除的不是裸数字、除以 0：fp 折不掉，仍然报。
    #[test]
    fn mixed_units_and_non_scalar_math_are_reported() {
        let (_f, project) = page(
            "calc-min-max-unfolded",
            r#"<view class="a"/><view class="b"/><view class="c"/><view class="d"/>"#,
            r#"
            .a { width: min(1rem, 10px); }
            .b { width: min(10px * 2px, 30px); }
            .c { width: min(10px / 0, 30px); }
            .d { width: min(10px / 2px, 30px); }
            "#,
        );
        let diags = CalcMinMax.check(&project);
        assert_eq!(diags.len(), 4);
    }

    #[test]
    fn constant_arguments_and_dynamic_values_are_accepted() {
        let (_f, project) = page(
            "calc-min-max-negative",
            r#"<view class="a"/><view class="b"/><view class="c"/><view class="d"/><view class="e"/><view class="f"/><view class="g"/><view class="h"/>"#,
            r#"
            .a { width: min(10px, 20px); }
            .b { width: clamp(1px, 2px, 3px); }
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
