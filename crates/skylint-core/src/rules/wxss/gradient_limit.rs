//! 渐变参数的额外限制（形状 / 尺寸 / 颜色停止位置）。

use crate::project::Project;
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

use super::value_util::{
    at_value, is_dynamic, length_unit, split_tokens, split_top_level, top_level_calls,
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
