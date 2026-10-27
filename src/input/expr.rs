//! WXML 表达式的静态取值推导。
//!
//! 只回答一个问题：一个动态绑定**可能取哪些字符串**。表达式里出现数据引用、
//! 成员访问、函数调用时取值不可知，一律按「任意」处理——这里不做数据流。
//!
//! 目标不是求值，而是给 class 匹配提供更细的三档：某些词**确定出现**、某些词
//! 只在有限枚举里出现、其余**任意词都可能**。有了前两档，匹配器才能把
//! `class="{{ok ? 'active' : ''}}"` 上 `.modal` 这类选择器判成「确定不命中」，
//! 而不是今天的「不可判定」。
//!
//! 有 [`Env`] 时还能把「确定取值的绑定」代入表达式：`{{renderer === 'skyline'}}`
//! 在 `renderer` 已知时直接折叠成真值，`wx:if` 的不可达分支与 `class` 的另一档
//! 都因此消失——这是同一条推导链的第二个消费者。

use glass_easel_template_compiler::parse::expr::Expression;

use crate::input::env::{Env, Lit};

/// 枚举出的候选值上限。超过就退化成「任意」，避免拼接的笛卡尔积爆炸。
const MAX_CANDIDATES: usize = 16;

/// `class` 的取值推导结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClassTokens {
    /// 任何取值下都出现的词。
    pub definite: Vec<String>,
    /// 只在部分取值下出现的词（表达式能枚举出有限个取值时才有内容）。
    pub possible: Vec<String>,
    /// 除上面两类之外，任意词都可能出现（取值依赖运行时数据）。
    pub any: bool,
}

impl ClassTokens {
    /// 这个词在任何取值下都会出现。
    pub fn is_definite(&self, token: &str, case_sensitive: bool) -> bool {
        self.definite.iter().any(|t| same(t, token, case_sensitive))
    }

    /// 这个词仍可能出现（只对「确定出现」之外的情况有意义）。
    pub fn might_appear(&self, token: &str, case_sensitive: bool) -> bool {
        self.any || self.possible.iter().any(|t| same(t, token, case_sensitive))
    }
}

fn same(a: &str, b: &str, case_sensitive: bool) -> bool {
    if case_sensitive {
        a == b
    } else {
        a.eq_ignore_ascii_case(b)
    }
}

/// 拼接里的一段：静态字面量，或一段动态取值（`None` = 任意值）。
enum Piece {
    Text(String),
    Value(Option<Vec<String>>),
}

/// 表达式可能取到的字符串；`None` 表示取值依赖运行时数据，枚举不出来。
pub fn candidates(expression: &Expression, env: &Env) -> Option<Vec<String>> {
    let mut pieces = Vec::new();
    collect(expression, env, &mut pieces);
    enumerate(&pieces)
}

fn enumerate(pieces: &[Piece]) -> Option<Vec<String>> {
    let lists: Vec<Vec<String>> = pieces
        .iter()
        .map(|piece| match piece {
            Piece::Text(text) => Some(vec![text.clone()]),
            Piece::Value(values) => values.clone(),
        })
        .collect::<Option<_>>()?;
    product(&lists)
}

/// 推导一个 class 绑定的词元信息。
pub fn class_tokens(expression: &Expression, env: &Env) -> ClassTokens {
    let mut pieces = Vec::new();
    collect(expression, env, &mut pieces);
    match enumerate(&pieces) {
        Some(values) => {
            let mut seen: Vec<Vec<&str>> = values
                .iter()
                .map(|value| {
                    let mut words: Vec<&str> = value.split_whitespace().collect();
                    words.sort_unstable();
                    words.dedup();
                    words
                })
                .collect();
            seen.sort();
            seen.dedup();
            let Some((first, rest)) = seen.split_first() else {
                return ClassTokens::default();
            };
            // 每个取值里都出现的词才确定。
            let mut definite = first.clone();
            for words in rest {
                definite.retain(|word| words.contains(word));
            }
            let mut possible: Vec<&str> = seen
                .iter()
                .flatten()
                .copied()
                .filter(|word| !definite.contains(word))
                .collect();
            possible.sort_unstable();
            possible.dedup();
            ClassTokens {
                definite: definite.iter().map(|word| word.to_string()).collect(),
                possible: possible.iter().map(|word| word.to_string()).collect(),
                any: false,
            }
        }
        // 有任一段取值不可知：只能保住静态段里被空格完整包住的词，其余任意。
        None => ClassTokens {
            definite: delimited_words(&pieces),
            possible: Vec::new(),
            any: true,
        },
    }
}

/// 拼接链里「两侧都被空格包住」的词——只有这些词在任何取值下都独立成词。
///
/// 紧贴动态段的词不算：`class="btn-{{type}}"` 的 `btn-` 只是一个词的后半截。
fn delimited_words(pieces: &[Piece]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (index, piece) in pieces.iter().enumerate() {
        let Piece::Text(text) = piece else {
            continue;
        };
        let left_ok = index == 0 || text.starts_with(char::is_whitespace);
        let right_ok = index + 1 == pieces.len() || text.ends_with(char::is_whitespace);
        let mut words: Vec<&str> = text.split_whitespace().collect();
        if !left_ok && !words.is_empty() {
            words.remove(0);
        }
        if !right_ok {
            words.pop();
        }
        for word in words {
            if !out.iter().any(|seen| seen == word) {
                out.push(word.to_string());
            }
        }
    }
    out
}

/// 把表达式拆成「静态文本 + 动态段」的拼接序列。
fn collect(expression: &Expression, env: &Env, out: &mut Vec<Piece>) {
    match expression {
        Expression::ToStringWithoutUndefined { value, .. } => collect(value, env, out),
        Expression::LitStr { value, .. } => out.push(Piece::Text(value.to_string())),
        _ => match concat_operands(expression) {
            Some(operands) => operands
                .into_iter()
                .for_each(|operand| collect(operand, env, out)),
            None => out.push(Piece::Value(eval(expression, env))),
        },
    }
}

/// `+` 只有在链上出现字符串字面量时才是模板拼接；纯算术的 `1 + 2` 不是。
fn concat_operands(expression: &Expression) -> Option<Vec<&Expression>> {
    let mut operands = Vec::new();
    flatten_plus(expression, &mut operands);
    let is_concat = operands
        .iter()
        .any(|operand| matches!(operand, Expression::LitStr { .. }));
    is_concat.then_some(operands)
}

fn flatten_plus<'a>(expression: &'a Expression, out: &mut Vec<&'a Expression>) {
    match expression {
        Expression::Plus { left, right, .. } => {
            flatten_plus(left, out);
            flatten_plus(right, out);
        }
        other => out.push(other),
    }
}

/// 一个子表达式的候选取值；`None` 表示取值不可知。
fn eval(expression: &Expression, env: &Env) -> Option<Vec<String>> {
    // 确定取值的绑定直接代入，比字面量分支更细。
    if let Some(value) = constant(expression, env) {
        return value.text().map(|text| vec![text]);
    }
    match expression {
        Expression::LitStr { value, .. } => Some(vec![value.to_string()]),
        Expression::LitInt { value, .. } => Some(vec![value.to_string()]),
        Expression::LitBool { value, .. } => Some(vec![value.to_string()]),
        // 插值里的 `undefined` 被编译器包成 `ToStringWithoutUndefined`，取值是空串。
        Expression::LitUndefined { .. } => Some(vec![String::new()]),
        Expression::ToStringWithoutUndefined { value, .. } => eval(value, env),
        Expression::Cond {
            true_br, false_br, ..
        } => union(eval(true_br, env)?, eval(false_br, env)?),
        // 逻辑运算与 `??` 返回其中一个操作数。
        Expression::LogicAnd { left, right, .. }
        | Expression::LogicOr { left, right, .. }
        | Expression::NullishCoalescing { left, right, .. } => {
            union(eval(left, env)?, eval(right, env)?)
        }
        Expression::Lt { .. }
        | Expression::Gt { .. }
        | Expression::Lte { .. }
        | Expression::Gte { .. }
        | Expression::Eq { .. }
        | Expression::Ne { .. }
        | Expression::EqFull { .. }
        | Expression::NeFull { .. }
        | Expression::InstanceOf { .. } => Some(vec!["true".to_string(), "false".to_string()]),
        _ => None,
    }
}

fn union(mut left: Vec<String>, right: Vec<String>) -> Option<Vec<String>> {
    left.extend(right);
    (left.len() <= MAX_CANDIDATES).then_some(left)
}

/// 表达式在给定环境下的确定取值；`None` = 依赖运行时数据。
///
/// 只做 JS 里无副作用的那部分折叠：字面量、已知绑定、三元、逻辑运算、
/// 严格相等与数值比较。类型不一致的宽松比较、成员访问、函数调用一律不可知。
pub fn constant(expression: &Expression, env: &Env) -> Option<Lit> {
    match expression {
        Expression::LitStr { value, .. } => Some(Lit::Str(value.to_string())),
        Expression::LitInt { value, .. } => Some(Lit::Num(*value as f64)),
        Expression::LitFloat { value, .. } => Some(Lit::Num(*value)),
        Expression::LitBool { value, .. } => Some(Lit::Bool(*value)),
        Expression::DataField { name, .. } => env.get(name).cloned(),
        Expression::ToStringWithoutUndefined { value, .. } => {
            constant(value, env)?.text().map(Lit::Str)
        }
        Expression::Reverse { value, .. } => Some(Lit::Bool(!truthy(value, env)?)),
        Expression::Cond {
            cond,
            true_br,
            false_br,
            ..
        } => match truthy(cond, env)? {
            true => constant(true_br, env),
            false => constant(false_br, env),
        },
        // 逻辑运算返回其中一个操作数，不是布尔值。
        Expression::LogicAnd { left, right, .. } => {
            let left = constant(left, env)?;
            if left.truthy() {
                constant(right, env)
            } else {
                Some(left)
            }
        }
        Expression::LogicOr { left, right, .. } => {
            let left = constant(left, env)?;
            if left.truthy() {
                Some(left)
            } else {
                constant(right, env)
            }
        }
        Expression::Eq { left, right, .. } | Expression::EqFull { left, right, .. } => Some(
            Lit::Bool(constant(left, env)?.strict_eq(&constant(right, env)?)?),
        ),
        Expression::Ne { left, right, .. } | Expression::NeFull { left, right, .. } => Some(
            Lit::Bool(!constant(left, env)?.strict_eq(&constant(right, env)?)?),
        ),
        Expression::Plus { left, right, .. } => {
            let (left, right) = (constant(left, env)?, constant(right, env)?);
            match (&left, &right) {
                // 有一边是字符串就是拼接。
                (Lit::Str(_), _) | (_, Lit::Str(_)) => {
                    Some(Lit::Str(left.text()? + &right.text()?))
                }
                (Lit::Num(a), Lit::Num(b)) => Some(Lit::Num(a + b)),
                _ => None,
            }
        }
        Expression::Lt { left, right, .. }
        | Expression::Gt { left, right, .. }
        | Expression::Lte { left, right, .. }
        | Expression::Gte { left, right, .. } => {
            let (a, b) = (number(left, env)?, number(right, env)?);
            Some(Lit::Bool(match expression {
                Expression::Lt { .. } => a < b,
                Expression::Gt { .. } => a > b,
                Expression::Lte { .. } => a <= b,
                _ => a >= b,
            }))
        }
        _ => None,
    }
}

/// 表达式在给定环境下是否为真；`None` = 判不了。
pub fn truthy(expression: &Expression, env: &Env) -> Option<bool> {
    constant(expression, env).map(|value| value.truthy())
}

/// 表达式的确定数值。
fn number(expression: &Expression, env: &Env) -> Option<f64> {
    match constant(expression, env)? {
        Lit::Num(value) => Some(value),
        _ => None,
    }
}

fn product(lists: &[Vec<String>]) -> Option<Vec<String>> {
    let mut out = vec![String::new()];
    for list in lists {
        if out.len() * list.len() > MAX_CANDIDATES {
            return None;
        }
        out = out
            .iter()
            .flat_map(|prefix| list.iter().map(move |item| format!("{prefix}{item}")))
            .collect();
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use glass_easel_template_compiler::parse::tag::{ElementKind, Node, Value};

    /// 解析一个 `<view class="...">`，返回 class 的推导结果。
    fn tokens(value: &str) -> super::ClassTokens {
        let source = format!(r#"<view class="{value}"/>"#);
        let (template, _) = glass_easel_template_compiler::parse::parse("t.wxml", &source);
        let Node::Element(element) = &template.content[0] else {
            panic!("expected element");
        };
        let ElementKind::Normal { class, .. } = &element.kind else {
            panic!("expected normal element");
        };
        let glass_easel_template_compiler::parse::tag::ClassAttribute::String(_, value) = class
        else {
            panic!("expected class string");
        };
        let Value::Dynamic { expression, .. } = value else {
            panic!("expected dynamic class");
        };
        super::class_tokens(expression, &crate::input::env::Env::default())
    }

    #[test]
    fn static_text_next_to_a_data_field_stays_definite() {
        let tokens = tokens("novel {{renderer}}");
        assert_eq!(tokens.definite, ["novel"]);
        assert!(tokens.any);
    }

    #[test]
    fn glued_text_is_not_counted_as_a_word() {
        let tokens = tokens("btn-{{type}}");
        assert!(tokens.definite.is_empty(), "{tokens:?}");
        assert!(tokens.any);
    }

    #[test]
    fn ternary_branches_are_enumerable() {
        let tokens = tokens("{{ok ? 'btn-active' : ''}}");
        assert!(tokens.definite.is_empty());
        assert_eq!(tokens.possible, ["btn-active"]);
        assert!(!tokens.any);
    }

    #[test]
    fn ternary_inside_a_static_prefix_keeps_the_words_before_it() {
        let tokens = tokens("modal {{ok ? 'active' : ''}}");
        assert_eq!(tokens.definite, ["modal"]);
        assert_eq!(tokens.possible, ["active"]);
        assert!(!tokens.any);
    }

    #[test]
    fn logical_fallback_with_a_data_field_stays_any() {
        // `name || 'anonymous'` 在 name 为真时是任意字符串，包不住。
        let tokens = tokens("{{name || 'anonymous'}}");
        assert!(tokens.any);
        assert!(tokens.definite.is_empty());
    }

    #[test]
    fn arithmetic_plus_is_not_treated_as_concatenation() {
        // `1 + 2` 是 3 而不是 "12"：两边都是数字就按算术求值。
        let tokens = tokens("{{1 + 2}}");
        assert_eq!(tokens.definite, ["3"]);
        assert!(!tokens.any);
    }

    #[test]
    fn too_many_branches_fall_back_to_any() {
        // 五个三元分支 = 32 种取值，超过上限后不再枚举，只保住静态词。
        let expression = "p {{a ? 'a' : ''}}{{b ? 'b' : ''}}{{c ? 'c' : ''}}\
                          {{d ? 'd' : ''}}{{e ? 'e' : ''}}";
        let tokens = tokens(expression);
        assert_eq!(tokens.definite, ["p"]);
        assert!(tokens.possible.is_empty());
        assert!(tokens.any);
    }
}
