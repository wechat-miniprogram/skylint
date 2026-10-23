//! 渐变参数的额外限制（形状 / 尺寸 / 颜色停止位置）。

use crate::input::wxss::ValueToken;
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

use super::value_util::{
    at_value, is_dynamic, is_ident, length_unit, split_tokens, split_top_level, token_text,
    top_level_calls,
};
use super::visit;

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
                    if value.is_empty() || is_dynamic(&decl.tokens) {
                        continue;
                    }
                    let problems: Vec<String> = top_level_calls(&decl.tokens)
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

fn check_radial(args: &[ValueToken]) -> Vec<String> {
    let parts = split_top_level(args);
    let Some(first) = parts.first() else {
        return Vec::new();
    };
    let mut problems = Vec::new();
    let prelude = radial_prelude(first);
    if let Some(pre) = prelude {
        if split_tokens(pre).iter().any(|group| {
            group
                .first()
                .is_some_and(|token| is_ident(token, "ellipse"))
        }) {
            problems.push("radial-gradient 仅支持 circle 形状，不支持 ellipse".to_string());
        }
        if let Some(size) = radial_size(pre) {
            if let Some(unit) = length_unit(size) {
                if unit != "px" {
                    problems.push(format!(
                        "radial-gradient 尺寸仅支持 px，`{}` 不受支持",
                        token_text(size)
                    ));
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
                "radial-gradient 颜色停止位置仅支持 %，`{}` 不受支持",
                token_text(token)
            ));
        }
    }
    problems
}

/// 第一段是否是形状 / 尺寸 / 位置前言，而不是第一个颜色停止。
///
/// 颜色停止必须以颜色开头，所以「第一个 token 是长度 / 百分比」的段一定是尺寸
/// （`radial-gradient(10em, red, blue)`），归入前言——否则会被当成停止位置误判。
fn radial_prelude(first: &[ValueToken]) -> Option<&[ValueToken]> {
    const KEYWORDS: [&str; 7] = [
        "circle",
        "ellipse",
        "at",
        "closest-side",
        "farthest-side",
        "closest-corner",
        "farthest-corner",
    ];
    let is_prelude = split_tokens(first).iter().any(|group| {
        group.first().is_some_and(|token| {
            KEYWORDS.iter().any(|keyword| is_ident(token, keyword))
                || matches!(token, ValueToken::Dimension { .. } | ValueToken::Percentage(_))
        })
    });
    is_prelude.then_some(first)
}

/// 前言里的尺寸：形状关键字之后、`at` 之前的第一个 token。
fn radial_size(pre: &[ValueToken]) -> Option<&ValueToken> {
    let mut groups = split_tokens(pre);
    if groups
        .first()
        .and_then(|group| group.first())
        .is_some_and(|token| is_ident(token, "circle") || is_ident(token, "ellipse"))
    {
        groups.remove(0);
    }
    if let Some(at) = groups
        .iter()
        .position(|group| group.first().is_some_and(|token| is_ident(token, "at")))
    {
        groups.truncate(at);
    }
    groups.first().and_then(|group| group.first())
}

/// 颜色停止里第一个非 `%` 的位置值。
fn non_percentage_position(stop: &[ValueToken]) -> Option<&ValueToken> {
    split_tokens(stop).into_iter().find_map(|group| {
        let token = group.first()?;
        (length_unit(token).is_some_and(|unit| unit != "%")).then_some(token)
    })
}

fn check_linear(args: &[ValueToken]) -> Vec<String> {
    let parts = split_top_level(args);
    let start = usize::from(parts.first().is_some_and(|first| is_direction(first)));
    let mut problems = Vec::new();
    for stop in &parts[start..] {
        for group in split_tokens(stop) {
            let Some(token) = group.first() else {
                continue;
            };
            if let Some(unit) = length_unit(token) {
                if unit != "%" && !is_fixed_length(&unit) {
                    problems.push(format!(
                        "linear-gradient 颜色停止位置仅支持 % 与固定长度单位，`{}` 不受支持",
                        token_text(token)
                    ));
                }
            }
        }
    }
    problems
}

fn is_direction(first: &[ValueToken]) -> bool {
    let tokens: Vec<&ValueToken> = split_tokens(first)
        .into_iter()
        .filter_map(|group| group.first())
        .collect();
    match tokens.first() {
        Some(token) if is_ident(token, "to") => tokens.len() > 1,
        Some(ValueToken::Dimension { unit, .. }) => matches!(
            unit.to_ascii_lowercase().as_str(),
            "deg" | "rad" | "grad" | "turn"
        ),
        _ => false,
    }
}

/// 固定长度单位（含绝对物理单位）。相对单位（em/rem/vw/vh…）不算。
fn is_fixed_length(unit: &str) -> bool {
    matches!(unit, "px" | "rpx" | "cm" | "mm" | "in" | "pt" | "pc" | "q")
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;

    use super::super::testutil::{messages, single};
    use super::GradientLimit;

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
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Warning));
        assert!(diags.iter().any(|d| d.message.contains("ellipse")));
        assert!(diags.iter().any(|d| d.message.contains("10em")));
        assert!(diags.iter().any(|d| d.message.contains("10px")));
    }

    /// 长度型前言（没有形状关键字）不能被当成颜色停止。
    #[test]
    fn radial_gradient_with_length_only_prelude_is_a_size_not_a_stop() {
        let (_f, project) = single(
            "gradient-length-prelude",
            r#"
            .a { background-image: radial-gradient(10em, red 0%, blue 100%); }
            .b { background-image: radial-gradient(10px 20px, red 0%, blue 100%); }
            "#,
        );
        let diags = GradientLimit.check(&project);
        // `10px 20px` 是合法尺寸；只有 `10em` 被报，且归在「尺寸」而不是「颜色停止」。
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert!(diags[0].message.contains("尺寸仅支持 px"));
        assert!(!diags[0].message.contains("颜色停止位置"));
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
}
