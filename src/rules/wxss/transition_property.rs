//! `transition` / `transition-property` 里的属性名必须落在 `<animatable-property>` 白名单内。
//!
//! 数据来自 `references/animation.md:38-58` 的 BNF，颜色与字号等带 `kInterpolable` 的属性
//! 也已在引擎中实现（`css_properties/transition_property.cc:223-256`）。`visibility`、
//! `pointer-events` 等不在表内。`animation` 简写不判：其中的
//! `animation-name` 是自定义标识符，无法与属性名区分。
//!
//! 引擎的 case 列表只有长写（`MarginLeft`、`BorderTopWidth`、`BackgroundColor` 等），
//! 简写 `margin`/`padding`/`flex`/`border`/`background` 会落到 `default:` 分支被丢弃
//! （`transition_property.cc:308-310`），所以白名单里也只保留长写。

use crate::input::wxss::ValueToken;
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

use super::value_util::{at_value, is_dynamic};
use super::visit_declarations;

/// 来源：skyline-skills/skills/skyline-wxss/references/animation.md:38-58 的 `<animatable-property>` BNF
///
/// 文本/字体类、`visibility`、`pointer-events` 不在表内（同文档 :60-64）。
/// 一行一个属性名（小写），按白名单匹配。
const ANIMATABLE_PROPERTIES: &[&str] = &[
    "all",
    "none",
    "transform",
    "transform-origin",
    "opacity",
    "width",
    "height",
    "min-width",
    "max-width",
    "min-height",
    "max-height",
    "margin-top",
    "margin-right",
    "margin-bottom",
    "margin-left",
    "padding-top",
    "padding-right",
    "padding-bottom",
    "padding-left",
    "top",
    "right",
    "bottom",
    "left",
    "flex-grow",
    "flex-shrink",
    "flex-basis",
    "border-top-width",
    "border-right-width",
    "border-bottom-width",
    "border-left-width",
    "border-top-color",
    "border-right-color",
    "border-bottom-color",
    "border-left-color",
    "border-top-left-radius",
    "border-top-right-radius",
    "border-bottom-left-radius",
    "border-bottom-right-radius",
    "background-color",
    "background-position",
    "background-position-x",
    "background-position-y",
    "filter",
    "backdrop-filter",
    "box-shadow",
    "z-index",
    "text-decoration-color",
    "text-decoration-thickness",
    "color",
    "font-size",
    "font-weight",
    "letter-spacing",
    "word-spacing",
    "mask",
    "mask-size",
    "mask-position",
    "mask-position-x",
    "mask-position-y",
];

const HELP: &str = "改用受支持的动画属性（transform / opacity 等），可动画属性表见 animation.md";

/// CSS 全局关键字，不是属性名（两种属性上都合法）。
const GLOBAL_KEYWORDS: [&str; 5] = ["inherit", "initial", "unset", "revert", "revert-layer"];

/// `transition` 简写里的时间函数关键字，不是属性名。
const TIMING_KEYWORDS: [&str; 8] = [
    "ease",
    "linear",
    "ease-in",
    "ease-out",
    "ease-in-out",
    "step-start",
    "step-end",
    // `transition` 里不会出现，但按 animation 简写误用时也一并放过。
    "infinite",
];

/// CSS Transitions Level 2 的行为关键字（`transition-behavior`），也不是属性名。
const BEHAVIOR_KEYWORDS: [&str; 2] = ["allow-discrete", "normal"];

/// 文档明确列出「不支持 transition/animation」的文本与字体相关属性。
const TEXT_FONT: [&str; 9] = [
    "text-align",
    "text-shadow",
    "direction",
    "white-space",
    "word-break",
    "font-style",
    "font-family",
    "font-feature-settings",
    "line-height",
];

const TEXT_FONT_TAIL: &str = "Skyline 下文本与字体相关属性不会产生过渡动画";
const OTHER_TAIL: &str = "该属性不在 Skyline 的可动画属性白名单内";

pub struct TransitionProperty;

impl TransitionProperty {
    pub const ID: &'static str = "skyline/transition-property";
}

impl Rule for TransitionProperty {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "transition 里的属性名不在 Skyline 的可动画属性白名单内，该段过渡不生效",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit_declarations(project, |doc, decl, out| {
            let name = decl.name.to_ascii_lowercase();
            let longhand = name == "transition-property";
            if !longhand && name != "transition" {
                return;
            }
            if decl.value.trim().is_empty() || is_dynamic(&decl.tokens) {
                return;
            }
            let mut offenders: Vec<&str> = Vec::new();
            for token in &decl.tokens {
                let ValueToken::Ident(ident) = token else {
                    continue;
                };
                // 厂商前缀（-webkit-transform…）在 Skyline 下的处理没有证据，不报。
                if ident.starts_with('-') {
                    continue;
                }
                if ANIMATABLE_PROPERTIES.contains(&ident.to_ascii_lowercase().as_str()) {
                    continue;
                }
                // 全局关键字、时间函数、transition-behavior 关键字都不是属性名。
                if GLOBAL_KEYWORDS
                    .iter()
                    .any(|keyword| keyword.eq_ignore_ascii_case(ident))
                    || BEHAVIOR_KEYWORDS
                        .iter()
                        .any(|keyword| keyword.eq_ignore_ascii_case(ident))
                    || (!longhand
                        && TIMING_KEYWORDS
                            .iter()
                            .any(|keyword| keyword.eq_ignore_ascii_case(ident)))
                {
                    continue;
                }
                if !offenders
                    .iter()
                    .any(|seen| seen.eq_ignore_ascii_case(ident))
                {
                    offenders.push(ident);
                }
            }
            if offenders.is_empty() {
                return;
            }
            let list = offenders
                .iter()
                .map(|ident| format!("`{ident}`"))
                .collect::<Vec<_>>()
                .join("、");
            let tail = if offenders.iter().any(|ident| {
                TEXT_FONT
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(ident))
            }) {
                TEXT_FONT_TAIL
            } else {
                OTHER_TAIL
            };
            out.push(at_value(
                Self::ID,
                doc,
                decl,
                format!(
                    "`{name}: {}` 里的 {list} 不支持过渡（{tail}）",
                    decl.value.trim()
                ),
                HELP,
            ));
        })
    }
}

#[cfg(test)]
mod tests {

    use crate::rules::Rule;

    use super::TransitionProperty;
    use crate::rules::testutil::wxss::{messages, single};

    #[test]
    fn properties_outside_the_whitelist_are_reported() {
        let (_f, project) = single(
            "transition-positive",
            r#"
            .a { transition: text-shadow .2s; }
            .b { transition-property: color, text-shadow; }
            .c { transition: visibility .2s; }
            .d { transition: text-align .2s, font-size .2s; }
            "#,
        );
        let diags = TransitionProperty.check(&project);
        assert_eq!(diags.len(), 4);
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Warning));
        assert!(diags[0].message.contains("`text-shadow`"));
        assert!(diags[0].message.contains("文本与字体相关属性"));
        assert!(diags[1].message.contains("`text-shadow`"));
        assert!(!diags[1].message.contains("`color`"));
        assert!(diags[2].message.contains("`visibility`"));
        assert!(diags[2].message.contains(super::OTHER_TAIL));
        // 同一声明里多个违规 ident 合成一条。
        assert!(diags[3].message.contains("`text-align`"));
        assert!(!diags[3].message.contains("`font-size`"));
    }

    #[test]
    fn whitelisted_properties_and_timing_keywords_are_accepted() {
        let (_f, project) = single(
            "transition-negative",
            r#"
            .a { transition: width .3s ease; }
            .b { transition: transform .2s ease-in-out; }
            .c { transition: opacity .2s cubic-bezier(.4, 0, .2, 1); }
            .d { transition-property: all, background-position-x, mask-position-y; }
            .e { transition: -webkit-transform .2s; }
            .f { transition: color var(--d) ease; }
            .g { transition: border-top-width 1s; }
            .h { transition: color .2s; }
            .i { transition-property: font-size, font-weight, letter-spacing, word-spacing; }
            "#,
        );
        assert_eq!(
            messages(&TransitionProperty, &project),
            Vec::<String>::new()
        );
    }

    /// 引擎只实现了长写，`margin` / `border` / `background` 等简写落到 default 分支被丢弃。
    #[test]
    fn shorthand_properties_are_rejected() {
        let (_f, project) = single(
            "transition-shorthands",
            r#"
            .a { transition: margin .3s; }
            .b { transition: border 1s; }
            .c { transition: background .2s steps(4, end); }
            .d { transition-property: padding, border-radius, background-size; }
            "#,
        );
        let diags = TransitionProperty.check(&project);
        assert_eq!(diags.len(), 4);
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Warning));
        assert!(diags[0].message.contains("`margin`"));
        assert!(diags[1].message.contains("`border`"));
        assert!(diags[2].message.contains("`background`"));
        assert!(diags[3].message.contains("`padding`"));
        assert!(diags[3].message.contains("`border-radius`"));
        assert!(diags[3].message.contains("`background-size`"));
    }

    #[test]
    fn global_keywords_are_not_property_names() {
        let (_f, project) = single(
            "transition-global-keywords",
            r#"
            .a { transition: inherit; }
            .b { transition: initial; }
            .c { transition: unset; }
            .d { transition-property: revert; }
            .e { transition: REVERT-LAYER; }
            .f { transition: all .3s; }
            "#,
        );
        assert_eq!(
            messages(&TransitionProperty, &project),
            Vec::<String>::new()
        );
    }

    /// `transition-behavior` 的关键字不是属性名（CSS Transitions Level 2）。
    #[test]
    fn behavior_keywords_are_not_property_names() {
        let (_f, project) = single(
            "transition-behavior-keywords",
            r#"
            .a { transition: transform 1s allow-discrete; }
            .b { transition: opacity .2s normal; }
            .c { transition-property: transform, allow-discrete; }
            "#,
        );
        assert_eq!(
            messages(&TransitionProperty, &project),
            Vec::<String>::new()
        );
    }

    #[test]
    fn animation_shorthand_is_not_checked() {
        let (_f, project) = single(
            "transition-animation",
            r#"
            .a { animation: color 1s linear infinite; }
            .b { animation-name: color; }
            "#,
        );
        assert_eq!(
            messages(&TransitionProperty, &project),
            Vec::<String>::new()
        );
    }
}
