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

/// 文本/字体相关、且**不在** float-pigment 的 `TransitionPropertyItem` 闭集里
/// （`float-pigment-css/src/parser/property.rs:1028-1101`）的属性名。含它们的
/// `transition` 值在解析层就失败，整条声明回滚。
const TEXT_FONT_FATAL: [&str; 8] = [
    "text-align",
    "text-shadow",
    "direction",
    "white-space",
    "word-break",
    "font-style",
    "font-family",
    "font-feature-settings",
];

/// 在闭集里、但引擎的 `transition_property.cc` 没有对应 case（走 `default:` 只丢这一项）
/// 的属性名——同一行里其它属性照样过渡。
const ITEM_ONLY: [&str; 1] = ["line-height"];

const ITEM_ONLY_TAIL: &str = "该属性不会产生过渡动画，同一声明里的其它属性仍然生效";
const OTHER_TAIL: &str = "该属性不在 Skyline 的可动画属性白名单内";

/// `visibility` 比普通的不在白名单里更糟：float-pigment 的 `TransitionPropertyItem`
/// 枚举里根本没有它（`float-pigment-css/src/parser/property.rs:1028-1101`），含它的
/// `transition`/`transition-property` 值整条解析失败，声明被丢弃——同一行里的
/// `transform`/`opacity` 也一起没了。
const DROPS_DECLARATION: &[&str] = &["visibility"];

/// 解析层就失败的属性名：`visibility` 与上面那批文本/字体属性，外加所有厂商前缀名
/// （fp 的属性名闭集里没有任何 `-webkit-*` 别名）。后果是**整条** `transition`
/// 回滚——`transition: -webkit-transform .35s, transform .35s` 的后半段也会一起消失。
fn is_parse_fatal(ident: &str) -> bool {
    ident.starts_with('-')
        || DROPS_DECLARATION
            .iter()
            .any(|name| name.eq_ignore_ascii_case(ident))
        || TEXT_FONT_FATAL
            .iter()
            .any(|name| name.eq_ignore_ascii_case(ident))
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
        visit_declarations(project, |sheet, _rule, decl, out| {
            let doc = &sheet.doc;
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
            let drops_whole = offenders.iter().any(|ident| is_parse_fatal(ident));
            let tail = if drops_whole {
                // 值是整条解析失败的：连同一行里受支持的属性也不会过渡。
                "解析器不认识这个属性名，整条声明会被丢弃，同一声明里的其它属性也不会产生过渡"
            } else if offenders.iter().any(|ident| {
                ITEM_ONLY
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(ident))
            }) {
                ITEM_ONLY_TAIL
            } else {
                OTHER_TAIL
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
        assert!(!diags[2].message.contains(super::OTHER_TAIL));
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
