//! 值的个数 / 结构类限制。
//!
//! 只判定「值本身合法，但值的个数或函数结构超出 Skyline 限制」的情形：
//! 属性/单个值是否支持由 `dict` 负责，这里不重复。

use crate::input::wxss::{WxssDeclaration, WxssDoc};
use crate::project::Project;
use crate::{Diagnostic, Severity};

use super::visit;
use crate::rules::{Rule, RuleMeta};

/// 值里含变量 / 插值时运行时才能确定，一律跳过。
fn is_dynamic(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains("var(") || lower.contains("env(") || value.contains("{{")
}

/// 按顶层逗号切分，括号 / 方括号 / 引号内的逗号不切。
fn split_top_level(value: &str) -> Vec<&str> {
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
fn split_tokens(value: &str) -> Vec<&str> {
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
struct Call<'a> {
    name: String,
    args: &'a str,
}

fn top_level_calls(value: &str) -> Vec<Call<'_>> {
    let mut out = Vec::new();
    scan_calls(value, false, &mut out);
    out
}

fn all_calls(value: &str) -> Vec<Call<'_>> {
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
fn length_unit(token: &str) -> Option<String> {
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
fn has_angle_unit(expr: &str) -> bool {
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

fn at_value(
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

/// 渐变参数的额外限制。
pub struct GradientLimit;

impl GradientLimit {
    pub const ID: &'static str = "skyline/gradient-limit";
}

impl Rule for GradientLimit {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "渐变的形状 / 尺寸 / 颜色停止位置超出 Skyline 支持范围",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit(project, |doc, out| {
            for rule in &doc.rules {
                for decl in &rule.declarations {
                    // mask-image 的渐变由 MaskImageGradient 统一处理，避免重复报。
                    if decl.name.eq_ignore_ascii_case("mask-image") {
                        continue;
                    }
                    let value = decl.value.trim();
                    if value.is_empty() || is_dynamic(value) {
                        continue;
                    }
                    let problems: Vec<String> = top_level_calls(value)
                        .iter()
                        .flat_map(|call| match call.name.as_str() {
                            "radial-gradient" => check_radial(call.args),
                            "linear-gradient" => check_linear(call.args),
                            _ => Vec::new(),
                        })
                        .collect();
                    if problems.is_empty() {
                        continue;
                    }
                    out.push(at_value(
                        Self::ID,
                        doc,
                        decl,
                        format!(
                            "`{}` 的渐变参数在 Skyline 下不受支持：{}",
                            decl.name,
                            problems.join("；")
                        ),
                        "radial-gradient 仅支持 circle 形状、px 尺寸与 % 颜色停止；linear-gradient 颜色停止仅支持 % 与固定长度单位（px、rpx 等）",
                    ));
                }
            }
        })
    }
}

fn check_radial(args: &str) -> Vec<String> {
    let parts = split_top_level(args);
    let Some(first) = parts.first() else {
        return Vec::new();
    };
    let mut problems = Vec::new();
    let prelude = radial_prelude(first);
    if let Some(pre) = prelude {
        if split_tokens(pre)
            .iter()
            .any(|t| t.eq_ignore_ascii_case("ellipse"))
        {
            problems.push("radial-gradient 仅支持 circle 形状，不支持 ellipse".to_string());
        }
        if let Some(size) = radial_size(pre) {
            if let Some(unit) = length_unit(size) {
                if unit != "px" {
                    problems.push(format!("radial-gradient 尺寸仅支持 px，`{size}` 不受支持"));
                }
            }
        }
    }
    let stops = if prelude.is_some() {
        &parts[1..]
    } else {
        &parts[..]
    };
    for stop in stops {
        if let Some(token) = non_percentage_position(stop) {
            problems.push(format!(
                "radial-gradient 颜色停止位置仅支持 %，`{token}` 不受支持"
            ));
        }
    }
    problems
}

/// 第一段是否是形状 / 尺寸 / 位置前言，而不是第一个颜色停止。
fn radial_prelude(first: &str) -> Option<&str> {
    let tokens = split_tokens(first);
    let is_prelude = tokens.iter().any(|t| {
        let lower = t.to_ascii_lowercase();
        matches!(
            lower.as_str(),
            "circle"
                | "ellipse"
                | "at"
                | "closest-side"
                | "farthest-side"
                | "closest-corner"
                | "farthest-corner"
        )
    });
    is_prelude.then_some(first)
}

/// 前言里的尺寸：形状关键字之后、`at` 之前的第一个 token。
fn radial_size(pre: &str) -> Option<&str> {
    let mut tokens = split_tokens(pre);
    if matches!(
        tokens.first().map(|t| t.to_ascii_lowercase()).as_deref(),
        Some("circle") | Some("ellipse")
    ) {
        tokens.remove(0);
    }
    if let Some(at) = tokens.iter().position(|t| t.eq_ignore_ascii_case("at")) {
        tokens.truncate(at);
    }
    tokens.first().copied()
}

/// 颜色停止里第一个非 `%` 的位置值。
fn non_percentage_position(stop: &str) -> Option<&str> {
    split_tokens(stop)
        .into_iter()
        .find(|token| length_unit(token).is_some_and(|unit| unit != "%"))
}

fn check_linear(args: &str) -> Vec<String> {
    let parts = split_top_level(args);
    let start = usize::from(parts.first().is_some_and(|first| is_direction(first)));
    let mut problems = Vec::new();
    for stop in &parts[start..] {
        for token in split_tokens(stop) {
            if let Some(unit) = length_unit(token) {
                if unit != "%" && !is_fixed_length(&unit) {
                    problems.push(format!(
                        "linear-gradient 颜色停止位置仅支持 % 与固定长度单位，`{token}` 不受支持"
                    ));
                }
            }
        }
    }
    problems
}

fn is_direction(first: &str) -> bool {
    let lower = first.trim().to_ascii_lowercase();
    lower.starts_with("to ")
        || matches!(
            length_unit(&lower).as_deref(),
            Some("deg") | Some("rad") | Some("grad") | Some("turn")
        )
}

/// 固定长度单位（含绝对物理单位）。相对单位（em/rem/vw/vh…）不算。
fn is_fixed_length(unit: &str) -> bool {
    matches!(unit, "px" | "rpx" | "cm" | "mm" | "in" | "pt" | "pc" | "q")
}

/// 背景 / 遮罩的多值数量限制。
pub struct MultiValueLimit;

impl MultiValueLimit {
    pub const ID: &'static str = "skyline/multi-value-limit";
}

impl Rule for MultiValueLimit {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "background / mask 的多值数量超出 Skyline 支持范围",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit(project, |doc, out| {
            for rule in &doc.rules {
                for decl in &rule.declarations {
                    let Some(max) = value_limit(&decl.name) else {
                        continue;
                    };
                    let value = decl.value.trim();
                    if value.is_empty() || is_dynamic(value) {
                        continue;
                    }
                    let count = split_top_level(value).len();
                    if count <= max {
                        continue;
                    }
                    let message = if max == 1 {
                        format!(
                            "`{}` 在 Skyline 下不支持多组值，当前有 {count} 组",
                            decl.name
                        )
                    } else {
                        format!(
                            "`{}` 在 Skyline 下最多支持 {max} 个值，当前有 {count} 个",
                            decl.name
                        )
                    };
                    out.push(at_value(
                        Self::ID,
                        doc,
                        decl,
                        message,
                        "将多值拆成多条声明，或只保留 Skyline 支持的组数",
                    ));
                }
            }
        })
    }
}

fn value_limit(name: &str) -> Option<usize> {
    match name.to_ascii_lowercase().as_str() {
        "background-image" | "mask-image" | "background-position" => Some(2),
        "background-repeat" | "background-size" => Some(1),
        _ => None,
    }
}

/// filter / backdrop-filter 的函数限制。
pub struct FilterLimit;

impl FilterLimit {
    pub const ID: &'static str = "skyline/filter-limit";
}

impl Rule for FilterLimit {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "filter / backdrop-filter 使用了不支持的函数或组合",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit(project, |doc, out| {
            for rule in &doc.rules {
                for decl in &rule.declarations {
                    if !decl.name.eq_ignore_ascii_case("filter")
                        && !decl.name.eq_ignore_ascii_case("backdrop-filter")
                    {
                        continue;
                    }
                    let value = decl.value.trim();
                    if value.is_empty() || is_dynamic(value) {
                        continue;
                    }
                    let calls = top_level_calls(value);
                    let mut reasons = Vec::new();
                    if calls.iter().any(|c| c.name == "url") {
                        reasons.push("不支持 url()");
                    }
                    if calls.iter().any(|c| c.name == "drop-shadow") {
                        reasons.push("不支持 drop-shadow()，可用 box-shadow 替代");
                    }
                    if calls.len() > 1 {
                        reasons.push("不支持多个函数组合");
                    }
                    if reasons.is_empty() {
                        continue;
                    }
                    out.push(at_value(
                        Self::ID,
                        doc,
                        decl,
                        format!(
                            "`{}` 在 Skyline 下{}",
                            decl.name,
                            reasons.join("；")
                        ),
                        "filter / backdrop-filter 只支持单个 blur/brightness/contrast/grayscale/hue-rotate/invert/opacity/saturate/sepia 函数",
                    ));
                }
            }
        })
    }
}

/// `text-decoration-line` 只支持单值。
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
                    if value.is_empty() || is_dynamic(value) {
                        continue;
                    }
                    if split_tokens(value).len() <= 1 {
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

/// `calc()` 不支持角度计算。
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
                    if value.is_empty() || is_dynamic(value) {
                        continue;
                    }
                    if !all_calls(value)
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

/// `mask-image` 不支持渐变，只支持 `url()`。
pub struct MaskImageGradient;

impl MaskImageGradient {
    pub const ID: &'static str = "skyline/mask-image-gradient";
}

impl Rule for MaskImageGradient {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "mask-image 不支持 linear-gradient / radial-gradient，仅 url() 可用",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit(project, |doc, out| {
            for rule in &doc.rules {
                for decl in &rule.declarations {
                    if !decl.name.eq_ignore_ascii_case("mask-image") {
                        continue;
                    }
                    let value = decl.value.trim();
                    if value.is_empty() || is_dynamic(value) {
                        continue;
                    }
                    let calls = all_calls(value);
                    let Some(gradient) = calls
                        .iter()
                        .find(|c| c.name == "linear-gradient" || c.name == "radial-gradient")
                    else {
                        continue;
                    };
                    out.push(at_value(
                        Self::ID,
                        doc,
                        decl,
                        format!(
                            "`mask-image` 不支持 `{}()`，Skyline 仅支持 url() 形式",
                            gradient.name
                        ),
                        "mask-image 仅支持 url()，请改用 url() 或 PNG 遮罩图",
                    ));
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::fs;
    use std::path::PathBuf;

    use super::*;

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("skylint-wxss-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            Self { root }
        }

        fn write(&self, rel: &str, content: &str) -> &Self {
            self.write_bytes(rel, content.as_bytes())
        }

        fn write_bytes(&self, rel: &str, content: &[u8]) -> &Self {
            let path = self.root.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
            self
        }

        fn project(&self) -> Project {
            Project::load(&self.root).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn single(name: &str, wxss: &str) -> (Fixture, Project) {
        let f = Fixture::new(name);
        f.write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", "<view/>")
            .write("pages/index/index.wxss", wxss);
        let project = f.project();
        (f, project)
    }

    fn messages(rule: &dyn Rule, project: &Project) -> Vec<String> {
        rule.check(project).into_iter().map(|d| d.message).collect()
    }

    #[test]
    fn gradient_limit_reports_shape_size_and_stop_units() {
        let (_f, project) = single(
            "gradient-positive",
            r#"
            .a { background-image: radial-gradient(ellipse, red 0%, blue 100%); }
            .b { background-image: radial-gradient(circle 10em, red 0%, blue 100%); }
            .c { background-image: radial-gradient(circle, red 10px, blue 100%); }
            .d { background-image: linear-gradient(red 10em, blue 100%); }
            "#,
        );
        let diags = GradientLimit.check(&project);
        assert_eq!(diags.len(), 4);
        assert!(diags.iter().all(|d| d.severity == Severity::Warning));
        assert!(diags.iter().any(|d| d.message.contains("ellipse")));
        assert!(diags.iter().any(|d| d.message.contains("10em")));
        assert!(diags.iter().any(|d| d.message.contains("10px")));
    }

    #[test]
    fn gradient_limit_accepts_supported_gradients_and_vars() {
        let (_f, project) = single(
            "gradient-negative",
            r#"
            .a { background-image: radial-gradient(circle 50px at 10% 20%, red 0%, blue 100%); }
            .b { background-image: linear-gradient(to right, red 0%, blue 100px, green 10rpx); }
            .c { background-image: conic-gradient(from 45deg, red 0%, blue 100%); }
            .d { background-image: radial-gradient(circle, red var(--stop), blue 100%); }
            .e { mask-image: linear-gradient(red 10em, blue); }
            .f { background-image: linear-gradient(to right, red 0%, rgba(0, 0, 0, .5) 50%); }
            "#,
        );
        assert_eq!(messages(&GradientLimit, &project), Vec::<String>::new());
    }

    #[test]
    fn multi_value_reports_excess_values() {
        let (_f, project) = single(
            "multi-positive",
            r#"
            .a { background-image: url(a.png), url(b.png), url(c.png); }
            .b { background-position: 0 0, 10px 10px, 20px 20px; }
            .c { background-repeat: no-repeat, repeat; }
            .d { background-size: 10px 10px, 20px 20px; }
            "#,
        );
        let diags = MultiValueLimit.check(&project);
        assert_eq!(diags.len(), 4);
        assert!(diags.iter().all(|d| d.severity == Severity::Warning));
        assert!(diags.iter().any(|d| d.message.contains("3 个")));
    }

    #[test]
    fn multi_value_accepts_two_values_and_commas_inside_parens() {
        let (_f, project) = single(
            "multi-negative",
            r#"
            .a { background-image: url(a.png), url(b.png); }
            .b { background-image: url("a,b.png"), url(c.png); }
            .c { background-position: 0 0, 10px 10px; }
            .d { background-repeat: no-repeat; }
            .e { background-size: 10px 10px; }
            .f { background-image: var(--imgs); }
            .g { background-image: linear-gradient(red, blue), url(a.png); }
            "#,
        );
        assert_eq!(messages(&MultiValueLimit, &project), Vec::<String>::new());
    }

    #[test]
    fn filter_limit_reports_url_drop_shadow_and_combinations() {
        let (_f, project) = single(
            "filter-positive",
            r#"
            .a { filter: url(#f); }
            .b { filter: drop-shadow(0 0 2px #000); }
            .c { backdrop-filter: blur(2px) brightness(1.2); }
            "#,
        );
        let diags = FilterLimit.check(&project);
        assert_eq!(diags.len(), 3);
        assert!(diags.iter().all(|d| d.severity == Severity::Warning));
        assert!(diags.iter().any(|d| d.message.contains("url()")));
        assert!(diags.iter().any(|d| d.message.contains("drop-shadow()")));
        assert!(diags.iter().any(|d| d.message.contains("多个函数")));
    }

    #[test]
    fn filter_limit_accepts_single_functions_and_vars() {
        let (_f, project) = single(
            "filter-negative",
            r#"
            .a { filter: blur(2px); }
            .b { backdrop-filter: grayscale(1); }
            .c { filter: none; }
            .d { filter: var(--f); }
            .e { filter: sepia(0.5); }
            "#,
        );
        assert_eq!(messages(&FilterLimit, &project), Vec::<String>::new());
    }

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
        assert!(diags.iter().all(|d| d.severity == Severity::Warning));
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
        assert!(diags.iter().all(|d| d.severity == Severity::Warning));
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

    #[test]
    fn mask_image_gradient_reports_gradients() {
        let (_f, project) = single(
            "mask-positive",
            r#"
            .a { mask-image: linear-gradient(red, blue); }
            .b { mask-image: radial-gradient(circle, red 0%, blue 100%); }
            .c { mask-image: url(a.png), linear-gradient(red, blue); }
            "#,
        );
        let diags = MaskImageGradient.check(&project);
        assert_eq!(diags.len(), 3);
        assert!(diags.iter().all(|d| d.severity == Severity::Warning));
        assert!(diags.iter().any(|d| d.message.contains("linear-gradient")));
        assert!(diags.iter().any(|d| d.message.contains("radial-gradient")));
    }

    #[test]
    fn mask_image_gradient_accepts_url_none_and_vars() {
        let (_f, project) = single(
            "mask-negative",
            r#"
            .a { mask-image: url(a.png); }
            .b { mask-image: none; }
            .c { mask-image: var(--mask); }
            .d { mask-image: conic-gradient(red, blue); }
            "#,
        );
        assert_eq!(messages(&MaskImageGradient, &project), Vec::<String>::new());
    }

    #[test]
    fn broken_sources_do_not_stop_the_scan() {
        let f = Fixture::new("shape-isolation");
        f.write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/a/a","pages/b/b"],"renderer":"skyline"}"#,
            )
            .write("pages/a/a.json", "{}")
            .write("pages/a/a.wxml", "<view/>")
            .write("pages/a/a.wxss", ".a { filter: url(#f); }")
            // 非法 UTF-8：读失败，跳过。
            .write("pages/b/b.json", "{}")
            .write("pages/b/b.wxml", "<view/>")
            .write_bytes("pages/b/b.wxss", &[0xff, 0xfe, 0x00]);

        let project = f.project();
        let diags = FilterLimit.check(&project);
        assert_eq!(diags.len(), 1);
        let paths: HashSet<_> = diags.iter().map(|d| d.path.as_str()).collect();
        assert_eq!(paths, HashSet::from(["pages/a/a.wxss"]));
    }
}
