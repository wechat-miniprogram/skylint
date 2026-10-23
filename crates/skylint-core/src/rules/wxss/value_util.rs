//! 值词元工具（多条「值的个数 / 结构」规则共用）。

use crate::input::wxss::{ValueToken, WxssDeclaration, WxssDoc};
use crate::{Diagnostic, Severity};

/// 值里含变量 / 插值时运行时才能确定，一律跳过。
///
/// 口径：`var(` / `env(` / `{{` / `$` / `#{` 在任意位置（包括函数参数和字符串里）
/// 都算，`@` 只算值开头（Less 变量）。没有词元可用的地方（数据表归一化）用
/// [`text_is_dynamic`]，避免再维护第二套口径。
pub(crate) fn is_dynamic(tokens: &[ValueToken]) -> bool {
    tokens.iter().any(token_is_dynamic) || starts_with_at(tokens)
}

/// 纯文本层面的同一判定。
pub(crate) fn text_is_dynamic(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("var(")
        || lower.contains("env(")
        || text.contains("{{")
        || text.contains('$')
        || text.contains("#{")
        || text.trim_start().starts_with('@')
}

fn token_is_dynamic(token: &ValueToken) -> bool {
    match token {
        ValueToken::Dynamic => true,
        ValueToken::Function { name, args } => {
            let call = format!("{}(", name.to_ascii_lowercase());
            call.contains("var(") || call.contains("env(") || args.iter().any(token_is_dynamic)
        }
        ValueToken::Dimension { value, unit } => text_is_dynamic(value) || text_is_dynamic(unit),
        ValueToken::Number(text)
        | ValueToken::Percentage(text)
        | ValueToken::Color(text)
        | ValueToken::Url(text)
        | ValueToken::Ident(text)
        | ValueToken::Str(text)
        | ValueToken::Other(text) => text_is_dynamic(text),
        ValueToken::Comma => false,
    }
}

fn starts_with_at(tokens: &[ValueToken]) -> bool {
    match tokens.first() {
        Some(ValueToken::Other(text)) => text.trim_start().starts_with('@'),
        _ => false,
    }
}

/// 按顶层逗号切分。
pub(crate) fn split_top_level(tokens: &[ValueToken]) -> Vec<&[ValueToken]> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for (i, token) in tokens.iter().enumerate() {
        if matches!(token, ValueToken::Comma) {
            push_group(&mut out, &tokens[start..i]);
            start = i + 1;
        }
    }
    push_group(&mut out, &tokens[start..]);
    out
}

fn push_group<'a>(out: &mut Vec<&'a [ValueToken]>, group: &'a [ValueToken]) {
    if !group.is_empty() {
        out.push(group);
    }
}

/// 按顶层空白与逗号切分。AST 已经把空白拆开，这里只去掉逗号。
pub(crate) fn split_tokens(tokens: &[ValueToken]) -> Vec<&[ValueToken]> {
    tokens
        .iter()
        .filter(|token| !matches!(token, &&ValueToken::Comma))
        .map(std::slice::from_ref)
        .collect()
}

/// 一个顶层函数调用：`name(args)`。
pub(crate) struct Call<'a> {
    pub(crate) name: String,
    pub(crate) args: &'a [ValueToken],
}

pub(crate) fn top_level_calls(tokens: &[ValueToken]) -> Vec<Call<'_>> {
    tokens
        .iter()
        .filter_map(|token| match token {
            ValueToken::Function { name, args } => Some(Call {
                name: name.to_ascii_lowercase(),
                args,
            }),
            // `url(...)` 在 AST 里是独立词元；规则层按旧口径当成 `url()` 调用。
            ValueToken::Url(_) => Some(Call {
                name: "url".to_string(),
                args: &[],
            }),
            _ => None,
        })
        .collect()
}

pub(crate) fn all_calls(tokens: &[ValueToken]) -> Vec<Call<'_>> {
    let mut out = Vec::new();
    collect_calls(tokens, &mut out);
    out
}

fn collect_calls<'a>(tokens: &'a [ValueToken], out: &mut Vec<Call<'a>>) {
    for token in tokens {
        match token {
            ValueToken::Function { name, args } => {
                out.push(Call {
                    name: name.to_ascii_lowercase(),
                    args,
                });
                collect_calls(args, out);
            }
            ValueToken::Url(_) => out.push(Call {
                name: "url".to_string(),
                args: &[],
            }),
            _ => {}
        }
    }
}

/// 取 `<number><unit>` 形式的单位（小写）。裸数字返回 `None`。
pub(crate) fn length_unit(token: &ValueToken) -> Option<String> {
    match token {
        ValueToken::Dimension { unit, .. } => Some(unit.to_ascii_lowercase()),
        ValueToken::Percentage(_) => Some("%".to_string()),
        // `Other` 保留原文（例如预处理器二元运算），按旧口径扫描。
        ValueToken::Other(text) => text_length_unit(text),
        _ => None,
    }
}

fn text_length_unit(token: &str) -> Option<String> {
    let bytes = token.as_bytes();
    let mut i = 0usize;
    if matches!(bytes.first(), Some(b'+') | Some(b'-')) {
        i = 1;
    }
    let mut seen_digit = false;
    while i < bytes.len() {
        match bytes[i] {
            b'0'..=b'9' => {
                seen_digit = true;
                i += 1;
            }
            b'.' => i += 1,
            _ => break,
        }
    }
    if !seen_digit || i >= bytes.len() {
        return None;
    }
    let unit = &token[i..];
    if unit.chars().all(|c| c.is_ascii_alphabetic() || c == '%') {
        Some(unit.to_ascii_lowercase())
    } else {
        None
    }
}

/// 表达式里是否出现角度单位。
pub(crate) fn has_angle_unit(tokens: &[ValueToken]) -> bool {
    tokens.iter().any(|token| match token {
        ValueToken::Dimension { unit, .. } => is_angle_unit(unit),
        ValueToken::Function { args, .. } => has_angle_unit(args),
        ValueToken::Other(text) => text_has_angle_unit(text),
        _ => false,
    })
}

fn is_angle_unit(unit: &str) -> bool {
    matches!(
        unit.to_ascii_lowercase().as_str(),
        "deg" | "rad" | "grad" | "turn"
    )
}

fn text_has_angle_unit(expr: &str) -> bool {
    let mut i = 0usize;
    while i < expr.len() {
        let ch = expr[i..].chars().next().unwrap();
        if ch.is_ascii_digit() || ch == '.' {
            let mut j = i;
            while j < expr.len() {
                let c = expr[j..].chars().next().unwrap();
                if c.is_ascii_digit() || c == '.' {
                    j += c.len_utf8();
                } else {
                    break;
                }
            }
            let unit_start = j;
            while j < expr.len() {
                let c = expr[j..].chars().next().unwrap();
                if c.is_ascii_alphabetic() {
                    j += c.len_utf8();
                } else {
                    break;
                }
            }
            if is_angle_unit(&expr[unit_start..j]) {
                return true;
            }
            i = j.max(i + ch.len_utf8());
        } else {
            i += ch.len_utf8();
        }
    }
    false
}

/// 标识符词元是否等于 `name`（ASCII 大小写不敏感）。
pub(crate) fn is_ident(token: &ValueToken, name: &str) -> bool {
    matches!(token, ValueToken::Ident(ident) if ident.eq_ignore_ascii_case(name))
}

/// 词元的原文（用于诊断消息里引用具体值）。
pub(crate) fn token_text(token: &ValueToken) -> String {
    match token {
        ValueToken::Dimension { value, unit } => format!("{value}{unit}"),
        ValueToken::Number(text)
        | ValueToken::Percentage(text)
        | ValueToken::Color(text)
        | ValueToken::Url(text)
        | ValueToken::Ident(text)
        | ValueToken::Str(text)
        | ValueToken::Other(text) => text.clone(),
        ValueToken::Comma => ",".to_string(),
        ValueToken::Dynamic => String::new(),
        ValueToken::Function { name, .. } => format!("{name}(...)"),
    }
}

pub(crate) fn at_value(
    id: &'static str,
    doc: &WxssDoc,
    decl: &WxssDeclaration,
    message: String,
    help: impl Into<String>,
) -> Diagnostic {
    let (line, column) = doc.position(decl.value_offset);
    let (end_line, end_column) = doc.position(decl.value_end());
    Diagnostic::new(id, Severity::Warning, message, doc.path(), line, column)
        .end(end_line, end_column)
        .help(help)
}

/// 定位在整条声明（属性名起点到值结尾）上的诊断。
pub(crate) fn at_declaration(
    id: &'static str,
    doc: &WxssDoc,
    decl: &WxssDeclaration,
    severity: Severity,
    message: impl Into<String>,
    help: impl Into<String>,
) -> Diagnostic {
    let (line, column) = doc.position(decl.offset);
    let (end_line, end_column) = doc.position(decl.value_end());
    Diagnostic::new(id, severity, message, doc.path(), line, column)
        .end(end_line, end_column)
        .help(help)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::wxss;

    fn value_tokens(value: &str) -> Vec<ValueToken> {
        let source = format!(".a {{ x: {value}; }}");
        let doc = wxss::parse("a.wxss", &source).unwrap();
        doc.rules[0].declarations[0].tokens.clone()
    }

    #[test]
    fn split_top_level_ignores_commas_in_quotes_and_parens() {
        let tokens = value_tokens(r#"url("a,b.png"), url(c.png)"#);
        let parts = split_top_level(&tokens);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0], [ValueToken::Url(r#"url("a,b.png")"#.to_string())]);

        let tokens = value_tokens("linear-gradient(red, blue), url(a.png)");
        let parts = split_top_level(&tokens);
        assert_eq!(parts.len(), 2);
        assert!(matches!(parts[0][0], ValueToken::Function { .. }));
    }

    #[test]
    fn split_top_level_drops_empty_groups() {
        let tokens = value_tokens("red,, blue");
        assert_eq!(split_top_level(&tokens).len(), 2);
    }

    #[test]
    fn split_tokens_splits_whitespace_and_commas_only() {
        assert_eq!(split_tokens(&value_tokens("red url(x) no-repeat")).len(), 3);
        // 函数内部的逗号与空白不切。
        assert_eq!(
            split_tokens(&value_tokens("rgba(0, 0, 0, .2) none")).len(),
            2
        );
        // 顶层逗号等同于分隔符，且不产生空 token。
        assert_eq!(split_tokens(&value_tokens("no-repeat, repeat")).len(), 2);
    }

    #[test]
    fn split_tokens_keeps_nested_parens_whole() {
        // 函数内部的逗号 / 空白不切；函数后的两个位置值各成一个 token。
        assert_eq!(
            split_tokens(&value_tokens("linear-gradient(to right, red, blue) 0 0")).len(),
            3
        );
        assert_eq!(split_tokens(&value_tokens("calc(1px + var(--x))")).len(), 1);
    }

    #[test]
    fn all_calls_walks_nested_functions() {
        let names: Vec<_> = all_calls(&value_tokens("calc(100% - var(--x))"))
            .into_iter()
            .map(|call| call.name)
            .collect();
        assert_eq!(names, vec!["calc", "var"]);

        let names: Vec<_> = all_calls(&value_tokens("linear-gradient(red, blue), url(a.png)"))
            .into_iter()
            .map(|call| call.name)
            .collect();
        assert_eq!(names, vec!["linear-gradient", "url"]);

        let names: Vec<_> = all_calls(&value_tokens("translate(10px, 20px) rotate(45deg)"))
            .into_iter()
            .map(|call| call.name)
            .collect();
        assert_eq!(names, vec!["translate", "rotate"]);
    }

    fn tokens_of(path: &str, value: &str) -> Vec<ValueToken> {
        let source = format!(".a {{ x: {value}; }}");
        let doc = wxss::parse(path, &source).unwrap();
        doc.rules[0].declarations[0].tokens.clone()
    }

    #[test]
    fn is_dynamic_matches_the_old_text_rules() {
        assert!(is_dynamic(&value_tokens("var(--x)")));
        assert!(is_dynamic(&value_tokens("translate(10px, var(--x))")));
        assert!(!is_dynamic(&value_tokens("red")));
        assert!(!is_dynamic(&value_tokens("translate(10px, 20px)")));
    }

    #[test]
    fn is_dynamic_sees_preprocessor_variables_and_interpolation() {
        assert!(is_dynamic(&tokens_of("a.scss", "$x")));
        assert!(is_dynamic(&tokens_of("a.scss", "#{$x}")));
        assert!(is_dynamic(&tokens_of("a.less", "@w")));
        // `@` 只算值开头；出现在中间时按旧口径不算（除非含 `$` / `#{`）。
        assert!(!is_dynamic(&tokens_of("a.less", "10px + 5px")));
    }
}
