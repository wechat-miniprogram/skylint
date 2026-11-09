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
//!
//! 同规则内标准兜底豁免：解析层失败的声明会被整条丢弃，并不影响同一 `WxssRule` 里
//! 的其它声明。因此若同名属性还有另一条不含解析失败属性名的声明（如 `transition:
//! transform .35s`），最终生效的过渡与 WebView 一致，不再报。兜底声明的 duration /
//! delay 等参数若与此条不同，最终以兜底声明为准；本规则不比较参数，接受这点边界差异。

use crate::input::wxss::{ValueToken, WxssDeclaration, WxssRule};
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
const DROP_HELP: &str = "把 `visibility` 从这条 transition 里删掉（留着它整条声明都会失效）；显隐改用 class 切换，如 `.show { visibility: visible }`";

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

/// float-pigment 的 `TransitionPropertyItem` 是闭集
/// （`float-pigment-css/src/property.rs:1028-1101`，需跟 fp 版本同步）。闭集之外的
/// 属性名（含所有 `-` 前缀名）会让整条 `transition` 在解析层就失败。
const TRANSITION_PROPERTY_ITEMS: &[&str] = &[
    "none",
    "transform",
    "transform-origin",
    "line-height",
    "opacity",
    "all",
    "height",
    "width",
    "min-height",
    "max-height",
    "min-width",
    "max-width",
    "margin-top",
    "margin-right",
    "margin-bottom",
    "margin-left",
    "margin",
    "padding-top",
    "padding-right",
    "padding-bottom",
    "padding-left",
    "padding",
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
    "border",
    "border-width",
    "border-radius",
    "border-color",
    "border-left",
    "border-top",
    "border-right",
    "border-bottom",
    "z-index",
    "filter",
    "backdrop-filter",
    "box-shadow",
    "color",
    "text-decoration-color",
    "text-decoration-thickness",
    "text-underline-offset",
    "font-size",
    "font-weight",
    "letter-spacing",
    "word-spacing",
    "background-color",
    "background-position",
    "background-size",
    "background",
    "flex",
    "background-position-x",
    "background-position-y",
    "mask-size",
    "mask-position-x",
    "mask-position-y",
    "mask-position",
    "mask",
];

/// 闭集内、但 `transition_property.cc:22-311` 的 switch 没有对应 case（`margin` /
/// `padding` / `border*` / `background*` / `flex` / `text-underline-offset` /
/// `line-height` 等）的属性：落到 `default:` 只丢这一项，同一声明里其它属性照样过渡。
const ITEM_ONLY_TAIL: &str = "该属性不会产生过渡动画，同一声明里的其它属性仍然生效";
const ROLLBACK_TAIL: &str =
    "解析器不认识这个属性名，整条声明会被丢弃，同一声明里的其它属性也不会产生过渡";

/// 解析层就失败的属性名：不在 fp 的属性名闭集里，含所有厂商前缀名（闭集里没有
/// 任何 `-webkit-*` 别名）。后果是**整条** `transition` 回滚——
/// `transition: -webkit-transform .35s, transform .35s` 的后半段也会一起消失。
fn is_parse_fatal(ident: &str) -> bool {
    ident.starts_with('-')
        || !TRANSITION_PROPERTY_ITEMS
            .iter()
            .any(|name| name.eq_ignore_ascii_case(ident))
}

/// 不是属性名的词元：全局关键字、时间函数、`transition-behavior` 关键字。
fn is_keyword(ident: &str, shorthand: bool) -> bool {
    GLOBAL_KEYWORDS
        .iter()
        .any(|keyword| keyword.eq_ignore_ascii_case(ident))
        || BEHAVIOR_KEYWORDS
            .iter()
            .any(|keyword| keyword.eq_ignore_ascii_case(ident))
        || (shorthand
            && TIMING_KEYWORDS
                .iter()
                .any(|keyword| keyword.eq_ignore_ascii_case(ident)))
}

/// 值里所有不属于可动画白名单、也不是关键字的属性名（去重，保留原文大小写）。
fn offending_idents(decl: &WxssDeclaration, shorthand: bool) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    for token in &decl.tokens {
        let ValueToken::Ident(ident) = token else {
            continue;
        };
        if ANIMATABLE_PROPERTIES.contains(&ident.to_ascii_lowercase().as_str())
            || is_keyword(ident, shorthand)
        {
            continue;
        }
        if !out.iter().any(|seen| seen.eq_ignore_ascii_case(ident)) {
            out.push(ident);
        }
    }
    out
}

/// 同一规则块里是否还有另一条同名声明，且它的取值没有任何解析层失败的属性名。
/// 这样的声明就是标准兜底写法，最终过渡由它决定。
fn has_standard_sibling(rule: &WxssRule, decl: &WxssDeclaration, name: &str) -> bool {
    rule.declarations.iter().any(|other| {
        other.offset != decl.offset
            && other.name.eq_ignore_ascii_case(name)
            && !other.value.trim().is_empty()
            && !is_dynamic(&other.tokens)
            && offending_idents(other, name == "transition")
                .iter()
                .all(|ident| !is_parse_fatal(ident))
    })
}

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
        visit_declarations(project, |sheet, rule, decl, out| {
            let doc = &sheet.doc;
            let name = decl.name.to_ascii_lowercase();
            let shorthand = name == "transition";
            if !shorthand && name != "transition-property" {
                return;
            }
            if decl.value.trim().is_empty() || is_dynamic(&decl.tokens) {
                return;
            }
            let offenders = offending_idents(decl, shorthand);
            if offenders.is_empty() {
                return;
            }
            let drops_whole = offenders.iter().any(|ident| is_parse_fatal(ident));
            // 同一规则里还有标准写法时，解析失败的那条被丢弃后最终过渡仍与 WebView 一致。
            if drops_whole && has_standard_sibling(rule, decl, &name) {
                return;
            }
            let list = offenders
                .iter()
                .map(|ident| format!("`{ident}`"))
                .collect::<Vec<_>>()
                .join("、");
            let tail = if drops_whole {
                ROLLBACK_TAIL
            } else {
                ITEM_ONLY_TAIL
            };
            out.push(at_value(
                Self::ID,
                doc,
                decl,
                format!(
                    "`{name}: {}` 里的 {list} 不受支持（{tail}）",
                    decl.value.trim()
                ),
                if drops_whole { DROP_HELP } else { HELP },
            ));
        })
    }
}

#[cfg(test)]
mod tests {

    use crate::rules::Rule;

    use super::TransitionProperty;
    use crate::rules::testutil::wxss::{messages, page};

    #[test]
    fn properties_outside_the_whitelist_are_reported() {
        let (_f, project) = page(
            "transition-positive",
            r#"<view class="a"/><view class="b"/><view class="c"/><view class="d"/>"#,
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
        // 整条回滚：不在 fp 的属性名闭集里。
        assert!(
            diags[0].message.contains("整条声明会被丢弃"),
            "{}",
            diags[0].message
        );
        assert!(diags[1].message.contains("`text-shadow`"));
        assert!(!diags[1].message.contains("`color`"));
        assert!(diags[2].message.contains("`visibility`"));
        // visibility 不是「这一项不生效」，而是整条 transition 解析失败。
        assert!(
            diags[2].message.contains("整条声明会被丢弃"),
            "{}",
            diags[2].message
        );
        // 同一声明里多个违规 ident 合成一条。
        assert!(diags[3].message.contains("`text-align`"));
        assert!(!diags[3].message.contains("`font-size`"));
    }

    #[test]
    fn whitelisted_properties_and_timing_keywords_are_accepted() {
        let (_f, project) = page(
            "transition-negative",
            r#"<view class="a"/><view class="b"/><view class="c"/><view class="d"/><view class="f"/><view class="g"/><view class="h"/><view class="i"/>"#,
            r#"
            .a { transition: width .3s ease; }
            .b { transition: transform .2s ease-in-out; }
            .c { transition: opacity .2s cubic-bezier(.4, 0, .2, 1); }
            .d { transition-property: all, background-position-x, mask-position-y; }
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

    /// 厂商前缀名在 fp 的属性名闭集里不存在：整条 `transition` 回滚，同行的标准属性也一起失效。
    #[test]
    fn vendor_prefixed_property_drops_the_whole_declaration() {
        let (_f, project) = page(
            "transition-vendor",
            r#"<view class="a"/>"#,
            r#".a { transition: -webkit-transform .35s ease, transform .35s ease; }"#,
        );
        let diags = TransitionProperty.check(&project);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("`-webkit-transform`"));
        assert!(
            diags[0].message.contains("整条声明会被丢弃"),
            "{}",
            diags[0].message
        );
    }

    /// `line-height` 在闭集里、只是没有 Apply case：只丢这一项，其余属性照常过渡。
    #[test]
    fn line_height_only_drops_its_own_item() {
        let (_f, project) = page(
            "transition-line-height",
            r#"<view class="a"/>"#,
            r#".a { transition: line-height .2s, color .2s; }"#,
        );
        let diags = TransitionProperty.check(&project);
        assert_eq!(diags.len(), 1);
        assert!(
            diags[0].message.contains("其他属性仍然生效")
                || diags[0].message.contains("其它属性仍然生效"),
            "{}",
            diags[0].message
        );
    }

    /// 引擎只实现了长写，`margin` / `border` / `background` 等简写落到 default 分支被丢弃。
    #[test]
    fn shorthand_properties_are_rejected() {
        let (_f, project) = page(
            "transition-shorthands",
            r#"<view class="a"/><view class="b"/><view class="c"/><view class="d"/>"#,
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
        let (_f, project) = page(
            "transition-global-keywords",
            r#"<view class="a"/><view class="b"/><view class="c"/><view class="d"/><view class="e"/><view class="f"/>"#,
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
        let (_f, project) = page(
            "transition-behavior-keywords",
            r#"<view class="a"/><view class="b"/><view class="c"/>"#,
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

    /// weui 的四行叠写：同规则里还有标准 `transition: transform`，解析失败的
    /// `-webkit-transform`（无论单独出现还是混在标准属性后面）都不再报。
    #[test]
    fn standard_sibling_in_same_rule_exempts_rollback() {
        let (_f, project) = page(
            "transition-standard-sibling",
            r#"<view class="a"/><view class="b"/><view class="c"/>"#,
            r#"
            .a {
              transition: -webkit-transform .35s ease;
              transition: transform .35s ease;
              transition: transform .35s ease;
              transition: transform .35s ease, -webkit-transform .35s ease;
            }
            .b { transition-property: -webkit-transform; transition-property: transform; }
            .c { transition: -webkit-transform .35s, opacity .35s; transition: opacity .35s; }
            "#,
        );
        assert_eq!(
            messages(&TransitionProperty, &project),
            Vec::<String>::new()
        );
    }

    /// 标准兜底只在同一个规则块内生效：别的选择器里的标准写法救不了这条。
    #[test]
    fn standard_sibling_in_another_rule_does_not_exempt() {
        let (_f, project) = page(
            "transition-standard-sibling-isolation",
            r#"<view class="a"/><view class="b"/>"#,
            r#"
            .a { transition: -webkit-transform .35s ease; }
            .b { transition: transform .35s ease; }
            "#,
        );
        let diags = TransitionProperty.check(&project);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("`-webkit-transform`"));
    }

    /// 判据是 fp 的属性名闭集：`position` 在闭集外，整条回滚；
    /// `border-radius` 在闭集内但 Apply 层没有 case，只丢这一项。
    #[test]
    fn closed_set_membership_decides_rollback() {
        let (_f, project) = page(
            "transition-closed-set",
            r#"<view class="a"/><view class="b"/>"#,
            r#"
            .a { transition: position .2s; }
            .b { transition: border-radius .2s, color .2s; }
            "#,
        );
        let diags = TransitionProperty.check(&project);
        assert_eq!(diags.len(), 2);
        assert!(
            diags[0].message.contains("整条声明会被丢弃"),
            "{}",
            diags[0].message
        );
        assert!(diags[1].message.contains("`border-radius`"));
        assert!(!diags[1].message.contains("`color`"));
        assert!(
            diags[1].message.contains("其它属性仍然生效"),
            "{}",
            diags[1].message
        );
    }

    #[test]
    fn animation_shorthand_is_not_checked() {
        let (_f, project) = page(
            "transition-animation",
            r#"<view class="a"/><view class="b"/>"#,
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
