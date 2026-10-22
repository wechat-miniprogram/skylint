//! 值分词工具（多条「值的个数 / 结构」规则共用）。

use crate::input::wxss::{WxssDeclaration, WxssDoc};
use crate::{Diagnostic, Severity};

/// 值里含变量 / 插值时运行时才能确定，一律跳过。
pub(crate) fn is_dynamic(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains("var(")
        || lower.contains("env(")
        || value.contains("{{")
        // 预处理器变量 / 插值：静态求值不了
        || value.contains('$')
        || value.contains("#{")
        || value.trim_start().starts_with('@')
}

/// 按顶层逗号切分，括号 / 方括号 / 引号内的逗号不切。
pub(crate) fn split_top_level(value: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut escaped = false;

    for (i, ch) in value.char_indices() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => quote = Some(ch),
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                push_trimmed(&mut out, &value[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    push_trimmed(&mut out, &value[start..]);
    out
}

/// 按顶层空白与逗号切分，括号 / 引号内不切。
pub(crate) fn split_tokens(value: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut escaped = false;

    for (i, ch) in value.char_indices() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => quote = Some(ch),
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            c if depth == 0 && (c.is_whitespace() || c == ',') => {
                push_trimmed(&mut out, &value[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    push_trimmed(&mut out, &value[start..]);
    out
}

fn push_trimmed<'a>(out: &mut Vec<&'a str>, raw: &'a str) {
    let trimmed = raw.trim();
    if !trimmed.is_empty() {
        out.push(trimmed);
    }
}

/// 一个顶层函数调用：`name(args)`。
pub(crate) struct Call<'a> {
    pub(crate) name: String,
    pub(crate) args: &'a str,
}

pub(crate) fn top_level_calls(value: &str) -> Vec<Call<'_>> {
    let mut out = Vec::new();
    scan_calls(value, false, &mut out);
    out
}

pub(crate) fn all_calls(value: &str) -> Vec<Call<'_>> {
    let mut out = Vec::new();
    scan_calls(value, true, &mut out);
    out
}

fn scan_calls<'a>(value: &'a str, recursive: bool, out: &mut Vec<Call<'a>>) {
    let mut i = 0usize;
    while i < value.len() {
        let ch = value[i..].chars().next().unwrap();
        if ch == '"' || ch == '\'' {
            i = skip_string(value, i);
            continue;
        }
        if ch.is_ascii_alphabetic() || ch == '-' || ch == '_' {
            let start = i;
            let mut j = i;
            while j < value.len() {
                let c = value[j..].chars().next().unwrap();
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    j += c.len_utf8();
                } else {
                    break;
                }
            }
            if value[j..].starts_with('(') {
                if let Some(close) = matching_paren(value, j) {
                    let args = &value[j + 1..close];
                    out.push(Call {
                        name: value[start..j].to_ascii_lowercase(),
                        args,
                    });
                    if recursive {
                        scan_calls(args, true, out);
                    }
                    i = close + 1;
                    continue;
                }
            }
            i = j;
            continue;
        }
        i += ch.len_utf8();
    }
}

fn skip_string(value: &str, start: usize) -> usize {
    let quote = value[start..].chars().next().unwrap();
    let mut escaped = false;
    let mut i = start + quote.len_utf8();
    while i < value.len() {
        let ch = value[i..].chars().next().unwrap();
        i += ch.len_utf8();
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == quote {
            break;
        }
    }
    i
}

/// 从 `open`（指向 `(`）找到配对的 `)`，引号内的括号不计。
fn matching_paren(value: &str, open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut i = open;
    while i < value.len() {
        let ch = value[i..].chars().next().unwrap();
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == q {
                quote = None;
            }
            i += ch.len_utf8();
            continue;
        }
        match ch {
            '"' | '\'' => quote = Some(ch),
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += ch.len_utf8();
    }
    None
}

/// 取 `<number><unit>` 形式的单位（小写）。裸数字（如 `0`）返回 `None`。
pub(crate) fn length_unit(token: &str) -> Option<String> {
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
pub(crate) fn has_angle_unit(expr: &str) -> bool {
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
            if matches!(
                expr[unit_start..j].to_ascii_lowercase().as_str(),
                "deg" | "rad" | "grad" | "turn"
            ) {
                return true;
            }
            i = j.max(i + ch.len_utf8());
        } else {
            i += ch.len_utf8();
        }
    }
    false
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
