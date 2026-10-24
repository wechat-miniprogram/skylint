//! `border-style: none` 在 Skyline 下不会把 `border-width` 归零。
//!
//! 证据：`css_properties/border_top_style.cc:27-29` 只写 style、不动 width；
//! `css_properties/border_top_width.cc:11-12` 的 initial 是 3px；
//! `style/style_builder.cc:819-838` 无条件把四个 width 交给 yoga，没有 `style == none → 0`
//! 的分支；`style/computed_style.cc:1059-1060` 任一 border 长写变更都会把 kBorder 标脏。
//! WebView 下 style 为 none 时 used border-width 为 0，所以元素在 Skyline 下会多占宽度。

use crate::input::wxss::{ValueToken, WxssDeclaration, WxssDoc, WxssRule};
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

use super::value_util::{at_value, is_dynamic};
use super::visit_rules;

const HELP: &str = "把该侧 border-width 显式设为 0，或改用 background / 伪元素绘制边框，避免 Skyline 按 width 占位";

pub struct BorderNoneWidth;

impl BorderNoneWidth {
    pub const ID: &'static str = "skyline/border-none-width";
}

impl Rule for BorderNoneWidth {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "border-style 为 none 时 Skyline 仍按 border-width 参与布局，元素比 WebView 多占宽度",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit_rules(project, |sheet, rule, out| {
            check_rule(&sheet.doc, rule, out);
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

impl Side {
    const ALL: [Side; 4] = [Side::Top, Side::Right, Side::Bottom, Side::Left];

    fn index(self) -> usize {
        match self {
            Side::Top => 0,
            Side::Right => 1,
            Side::Bottom => 2,
            Side::Left => 3,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Side::Top => "上",
            Side::Right => "右",
            Side::Bottom => "下",
            Side::Left => "左",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Width {
    Zero,
    NonZero,
    /// 值无法静态判定（例如 `var()` 之外的自定义形态），按不报处理。
    Unknown,
}

#[derive(Clone, Copy, Default)]
struct SideState {
    /// `Some(true)` 表示 style 最终为 none；`Some(false)` 表示其他受支持值。
    style: Option<bool>,
    width: Option<Width>,
    /// 把该侧 style 置为 none 的声明下标（诊断落点）。
    style_decl: Option<usize>,
}

fn check_rule(doc: &WxssDoc, rule: &WxssRule, out: &mut Vec<Diagnostic>) {
    let mut states = [SideState::default(); 4];
    for (index, decl) in rule.declarations.iter().enumerate() {
        apply_declaration(decl, index, &mut states);
    }

    let affected: Vec<Side> = Side::ALL
        .into_iter()
        .filter(|side| {
            let state = &states[side.index()];
            state.style == Some(true) && !matches!(state.width, Some(Width::Zero | Width::Unknown))
        })
        .collect();
    if affected.is_empty() {
        return;
    }

    let Some(index) = affected
        .iter()
        .filter_map(|side| states[side.index()].style_decl)
        .min()
    else {
        return;
    };
    let decl = &rule.declarations[index];
    let sides = affected
        .iter()
        .map(|side| side.label())
        .collect::<Vec<_>>()
        .join("、");
    out.push(at_value(
        BorderNoneWidth::ID,
        doc,
        decl,
        format!(
            "`border-style: none` 不改变 border-width，Skyline 下仍按 3px（或声明的非 0 宽度）参与布局，元素比 WebView 多占宽度（受影响的侧：{sides}）"
        ),
        HELP,
    ));
}

fn apply_declaration(decl: &WxssDeclaration, index: usize, states: &mut [SideState; 4]) {
    let name = decl.name.to_ascii_lowercase();
    if is_dynamic(&decl.tokens) {
        return;
    }
    match name.as_str() {
        "border" => apply_border_shorthand(&decl.tokens, &Side::ALL, index, states),
        "border-style" => apply_style_shorthand(&decl.tokens, &Side::ALL, index, states),
        "border-width" => apply_width_shorthand(&decl.tokens, &Side::ALL, states),
        "border-top" => apply_border_shorthand(&decl.tokens, &[Side::Top], index, states),
        "border-right" => apply_border_shorthand(&decl.tokens, &[Side::Right], index, states),
        "border-bottom" => apply_border_shorthand(&decl.tokens, &[Side::Bottom], index, states),
        "border-left" => apply_border_shorthand(&decl.tokens, &[Side::Left], index, states),
        "border-top-style" => apply_style_shorthand(&decl.tokens, &[Side::Top], index, states),
        "border-right-style" => apply_style_shorthand(&decl.tokens, &[Side::Right], index, states),
        "border-bottom-style" => {
            apply_style_shorthand(&decl.tokens, &[Side::Bottom], index, states)
        }
        "border-left-style" => apply_style_shorthand(&decl.tokens, &[Side::Left], index, states),
        "border-top-width" => apply_width_shorthand(&decl.tokens, &[Side::Top], states),
        "border-right-width" => apply_width_shorthand(&decl.tokens, &[Side::Right], states),
        "border-bottom-width" => apply_width_shorthand(&decl.tokens, &[Side::Bottom], states),
        "border-left-width" => apply_width_shorthand(&decl.tokens, &[Side::Left], states),
        _ => {}
    }
}

/// `border` / `border-top` 简写：只关心 style 与 width 两段，颜色忽略。
fn apply_border_shorthand(
    tokens: &[ValueToken],
    sides: &[Side],
    index: usize,
    states: &mut [SideState; 4],
) {
    let mut style = None;
    let mut width = None;
    for token in tokens {
        match token {
            ValueToken::Ident(ident) => {
                if let Some(none) = style_value(ident) {
                    style = Some(none);
                } else if let Some(value) = width_keyword(ident) {
                    width = Some(value);
                }
            }
            ValueToken::Dimension { .. } | ValueToken::Number(_) | ValueToken::Percentage(_) => {
                width = Some(width_value(token));
            }
            _ => {}
        }
    }
    if let Some(style) = style {
        set_style(sides, style, index, states);
    }
    if let Some(width) = width {
        set_width(sides, width, states);
    }
}

/// `border-style` / `border-*-style`：1–4 个 ident 按 CSS 顺序映射到四侧。
fn apply_style_shorthand(
    tokens: &[ValueToken],
    sides: &[Side],
    index: usize,
    states: &mut [SideState; 4],
) {
    let mut values = Vec::new();
    for token in tokens {
        match token {
            ValueToken::Ident(ident) => match style_value(ident) {
                Some(none) => values.push(none),
                None => return,
            },
            ValueToken::Comma => {}
            _ => return,
        }
    }
    let Some(pairs) = expand(&values, sides) else {
        return;
    };
    for (side, none) in pairs {
        set_style(&[side], none, index, states);
    }
}

/// `border-width` / `border-*-width`：1–4 个长度按 CSS 顺序映射到四侧。
fn apply_width_shorthand(tokens: &[ValueToken], sides: &[Side], states: &mut [SideState; 4]) {
    let mut values = Vec::new();
    for token in tokens {
        match token {
            ValueToken::Dimension { .. } | ValueToken::Number(_) | ValueToken::Percentage(_) => {
                values.push(width_value(token))
            }
            ValueToken::Ident(ident) => match width_keyword(ident) {
                Some(width) => values.push(width),
                None => return,
            },
            ValueToken::Comma => {}
            _ => return,
        }
    }
    let Some(pairs) = expand(&values, sides) else {
        return;
    };
    for (side, width) in pairs {
        set_width(&[side], width, states);
    }
}

/// 1–4 个值按 CSS 简写规则展开；值个数不合法时整条声明跳过。
fn expand<T: Copy>(values: &[T], sides: &[Side]) -> Option<Vec<(Side, T)>> {
    // 长写（`border-top-style` 等）只允许单值。
    if sides.len() == 1 {
        return match values {
            [value] => Some(vec![(sides[0], *value)]),
            _ => None,
        };
    }
    let [top, right, bottom, left] = match values {
        [all] => [all, all, all, all],
        [vertical, horizontal] => [vertical, horizontal, vertical, horizontal],
        [top, horizontal, bottom] => [top, horizontal, bottom, horizontal],
        [top, right, bottom, left] => [top, right, bottom, left],
        _ => return None,
    };
    Some(vec![
        (Side::Top, *top),
        (Side::Right, *right),
        (Side::Bottom, *bottom),
        (Side::Left, *left),
    ])
}

fn style_value(ident: &str) -> Option<bool> {
    if ident.eq_ignore_ascii_case("none") {
        return Some(true);
    }
    matches!(
        ident.to_ascii_lowercase().as_str(),
        "solid"
            | "dashed"
            | "dotted"
            | "hidden"
            | "double"
            | "groove"
            | "ridge"
            | "inset"
            | "outset"
    )
    .then_some(false)
}

fn width_keyword(ident: &str) -> Option<Width> {
    matches!(
        ident.to_ascii_lowercase().as_str(),
        "thin" | "medium" | "thick"
    )
    .then_some(Width::NonZero)
}

fn width_value(token: &ValueToken) -> Width {
    match token {
        ValueToken::Dimension { value, .. } | ValueToken::Number(value) => {
            match value.parse::<f64>() {
                Ok(0.0) => Width::Zero,
                Ok(_) => Width::NonZero,
                Err(_) => Width::Unknown,
            }
        }
        ValueToken::Percentage(_) => Width::NonZero,
        _ => Width::Unknown,
    }
}

fn set_style(sides: &[Side], none: bool, index: usize, states: &mut [SideState; 4]) {
    for side in sides {
        let state = &mut states[side.index()];
        state.style = Some(none);
        if none {
            state.style_decl = Some(index);
        }
    }
}

fn set_width(sides: &[Side], width: Width, states: &mut [SideState; 4]) {
    for side in sides {
        states[side.index()].width = Some(width);
    }
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;

    use super::BorderNoneWidth;
    use crate::rules::testutil::wxss::{messages, single};

    #[test]
    fn none_style_with_implicit_or_nonzero_width_is_reported() {
        let (_f, project) = single(
            "border-none-width-positive",
            r#"
            .a { border: none; }
            .b { border: 2px none; }
            .c { border-style: none; }
            .d { border-top-style: none; border-top-width: 2px; }
            .e { border-top-style: none; }
            "#,
        );
        let diags = BorderNoneWidth.check(&project);
        assert_eq!(diags.len(), 5);
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Warning));
        assert!(diags[0].message.contains("上、右、下、左"));
        assert!(diags[1].message.contains("上、右、下、左"));
        assert!(diags[2].message.contains("上、右、下、左"));
        // 只报了上侧。
        assert!(diags[3].message.contains("受影响的侧：上"));
        assert!(!diags[3].message.contains('右'));
        assert!(diags[4].message.contains("受影响的侧：上"));
    }

    #[test]
    fn zero_width_solid_style_and_dynamic_values_are_accepted() {
        let (_f, project) = single(
            "border-none-width-negative",
            r#"
            .a { border: none; border-width: 0; }
            .b { border: 1px solid; }
            .c { border: var(--x) none; }
            .d { border-top-width: 0; }
            .e { border-style: none; border-width: 0 0 0 0; }
            .f { border: 0; }
            "#,
        );
        assert_eq!(messages(&BorderNoneWidth, &project), Vec::<String>::new());
    }
}
