//! 渐变参数的额外限制（形状 / 尺寸 / 颜色停止位置）。
//!
//! 引擎的解析结果决定严重程度：
//! - `radial-gradient` 非 circle 形状 → `image_property_utils.h:107-111` 走 `default`
//!   分支 Diag 后 `return CSSUnsetValue`，整张背景/遮罩图不渲染；
//! - 颜色停止位置不是 `%` 或 `px` → 只丢这一个停止位置（`:32-40`、`:86-97`）。
//!
//! fp 的 `Length` 里 `rpx` 是独立变体（`typing.rs:149-162`），不会归一成 `Px`，
//! 所以 `0rpx` 同样落进「不支持」分支；物理单位 fp 根本不认（`parse_length_unit`）。

use crate::input::wxss::ValueToken;
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

use super::value_util::{
    at_value, is_dynamic, is_ident, length_unit, split_tokens, split_top_level, token_text,
    top_level_calls,
};
use super::visit_declarations;

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
        visit_declarations(project, |doc, _, decl, out| {
            let value = decl.value.trim();
            if value.is_empty() || is_dynamic(&decl.tokens) {
                return;
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
                return;
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
                        "radial-gradient 必须是 circle 形状（默认 ellipse 会导致整张图不渲染）、尺寸用 px，颜色停止用 % 或 px；linear-gradient 颜色停止同样只支持 % 与 px",
                    ));
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
    // fp 没写形状时默认 ellipse（gradient.rs:390、:848），引擎只认 circle，整张图被丢弃。
    let circle = prelude.is_some_and(|pre| {
        split_tokens(pre)
            .iter()
            .any(|group| group.first().is_some_and(|token| is_ident(token, "circle")))
    });
    if !circle {
        problems.push(
            "radial-gradient 只支持 circle 形状，省略形状或写 ellipse 时整张图不会渲染".to_string(),
        );
    }
    if let Some(pre) = prelude
        && let Some(size) = radial_size(pre)
        && let Some(unit) = length_unit(size)
        && unit != "px"
    {
        problems.push(format!(
            "radial-gradient 尺寸仅支持 px，`{}` 不受支持",
            token_text(size)
        ));
    }
    let stops = if prelude.is_some() {
        &parts[1..]
    } else {
        &parts[..]
    };
    for stop in stops {
        if let Some(token) = non_percentage_position(stop) {
            problems.push(format!(
                "radial-gradient 颜色停止位置仅支持 % 与 px，`{}` 不受支持",
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
                || matches!(
                    token,
                    ValueToken::Dimension { .. } | ValueToken::Percentage(_)
                )
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

/// 颜色停止里第一个既不是 `%` 也不是 `px` 的位置值。
fn non_percentage_position(stop: &[ValueToken]) -> Option<&ValueToken> {
    split_tokens(stop).into_iter().find_map(|group| {
        let token = group.first()?;
        (length_unit(token).is_some_and(|unit| unit != "%" && unit != "px")).then_some(token)
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
            if let Some(unit) = length_unit(token)
                && unit != "%"
                && !is_fixed_length(&unit)
            {
                problems.push(format!(
                    "linear-gradient 颜色停止位置仅支持 % 与固定长度单位，`{}` 不受支持",
                    token_text(token)
                ));
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

/// 引擎只把 `Length::Px` 收进停止点（`image_property_utils.h:32-40`）；
/// `rpx` 是 fp 的独立变体，物理单位 fp 不解析，都不算。
fn is_fixed_length(unit: &str) -> bool {
    unit == "px"
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;

    use super::GradientLimit;
    use crate::rules::testutil::wxss::{messages, single};

    #[test]
    fn gradient_limit_reports_shape_size_and_stop_units() {
        let (_f, project) = single(
            "gradient-positive",
            r#"
            .a { background-image: radial-gradient(ellipse, red 0%, blue 100%); }
            .b { background-image: radial-gradient(circle 10em, red 0%, blue 100%); }
            .c { background-image: radial-gradient(circle, red 0rpx, blue 100%); }
            .d { background-image: linear-gradient(red 10em, blue 100%); }
            .e { background-image: radial-gradient(circle, red 0em, blue 100%); }
            "#,
        );
        let diags = GradientLimit.check(&project);
        assert_eq!(diags.len(), 5);
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Warning));
        assert!(diags.iter().any(|d| d.message.contains("ellipse")));
        assert!(diags.iter().any(|d| d.message.contains("10em")));
        assert!(diags.iter().any(|d| d.message.contains("`0rpx`")));
        assert!(diags.iter().any(|d| d.message.contains("`0em`")));
    }

    /// 长度型前言（没有形状关键字）不能被当成颜色停止；省略形状本身也要报。
    #[test]
    fn radial_gradient_with_length_only_prelude_is_a_size_not_a_stop() {
        let (_f, project) = single(
            "gradient-length-prelude",
            r#"
            .a { background-image: radial-gradient(10em, red 0%, blue 100%); }
            .b { background-image: radial-gradient(10px, red 0%, blue 100%); }
            "#,
        );
        let diags = GradientLimit.check(&project);
        assert_eq!(diags.len(), 2, "{diags:#?}");
        // `10em` 归在「尺寸」而不是「颜色停止」。
        assert!(diags[0].message.contains("尺寸仅支持 px"));
        assert!(!diags[0].message.contains("颜色停止位置"));
        assert!(diags[0].message.contains("circle"));
        assert!(diags[1].message.contains("circle"));
    }

    /// `rpx` 停止位置不被引擎接受，`mask-image` 也要查。
    #[test]
    fn rpx_stops_and_mask_gradients_are_reported() {
        let (_f, project) = single(
            "gradient-rpx",
            r#"
            .a { background-image: linear-gradient(red 10rpx, blue 100%); }
            .b { mask-image: linear-gradient(red 10em, blue 100%); }
            .c { -webkit-mask-image: radial-gradient(circle, red 10rpx, blue 100%); }
            "#,
        );
        let diags = GradientLimit.check(&project);
        assert_eq!(diags.len(), 3, "{diags:#?}");
        assert!(diags[0].message.contains("`10rpx`"));
        assert!(diags[1].message.contains("`10em`"));
        assert!(diags[2].message.contains("`10rpx`"));
    }

    #[test]
    fn gradient_limit_accepts_supported_gradients_and_vars() {
        let (_f, project) = single(
            "gradient-negative",
            r#"
            .a { background-image: radial-gradient(circle 50px at 10% 20%, red 0%, blue 100%); }
            .b { background-image: linear-gradient(to right, red 0%, blue 100px); }
            .c { background-image: conic-gradient(from 45deg, red 0%, blue 100%); }
            .d { background-image: radial-gradient(circle, red var(--stop), blue 100%); }
            .f { background-image: linear-gradient(to right, red 0%, rgba(0, 0, 0, .5) 50%); }
            .g { background-image: radial-gradient(circle, red 0px, blue 30px); }
            "#,
        );
        assert_eq!(messages(&GradientLimit, &project), Vec::<String>::new());
    }
}
