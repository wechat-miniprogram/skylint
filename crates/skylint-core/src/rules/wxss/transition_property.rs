//! `transition` / `transition-property` 里的属性名必须落在 `<animatable-property>` 白名单内。
//!
//! 数据来自 `references/animation.md:38-58` 的 BNF；文本/字体类、`visibility`、
//! `pointer-events` 不在表内（同文档 :60-64）。`animation` 简写不判：其中的
//! `animation-name` 是自定义标识符，无法与属性名区分。

use std::collections::HashSet;
use std::sync::OnceLock;

use crate::input::wxss::ValueToken;
use crate::project::Project;
use crate::rules::data_table::{self, Columns};
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

use super::value_util::{at_value, is_dynamic};
use super::visit;

const SOURCE: &str = include_str!("../data/animatable_properties.txt");

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
const TEXT_FONT: [&str; 14] = [
    "text-align",
    "text-shadow",
    "direction",
    "white-space",
    "word-break",
    "color",
    "font-size",
    "font-weight",
    "font-style",
    "font-family",
    "font-feature-settings",
    "line-height",
    "letter-spacing",
    "word-spacing",
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
        let allow = data();
        visit(project, |doc, out| {
            for rule in &doc.rules {
                for decl in &rule.declarations {
                    let name = decl.name.to_ascii_lowercase();
                    let longhand = name == "transition-property";
                    if !longhand && name != "transition" {
                        continue;
                    }
                    if decl.value.trim().is_empty() || is_dynamic(&decl.tokens) {
                        continue;
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
                        if allow.contains(ident.to_ascii_lowercase().as_str()) {
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
                        continue;
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
                }
            }
        })
    }
}

fn data() -> &'static HashSet<&'static str> {
    static TABLE: OnceLock<HashSet<&'static str>> = OnceLock::new();
    TABLE.get_or_init(|| parse(SOURCE).expect("WXSS 可动画属性表格式错误"))
}

/// 解析数据文件。格式错误一律返回带行号的说明，便于同步时定位。
fn parse(source: &'static str) -> Result<HashSet<&'static str>, String> {
    let parsed = data_table::read(source, data_table::whole_line, |_| {
        Columns::new(1, "一个属性名")
    })?;
    let mut names = HashSet::new();
    for row in parsed.rows {
        let line = row.fields[0];
        if line.contains(char::is_whitespace) || line.contains(',') {
            return Err(format!(
                "第 {} 行：一行只能写一个属性名：{line}",
                row.line_no
            ));
        }
        if !is_lowercase(line) {
            return Err(format!("第 {} 行：属性名必须小写：{line}", row.line_no));
        }
        data_table::unique(&mut names, line, row.line_no, || {
            format!("重复的属性名 {line}")
        })?;
    }
    if names.is_empty() {
        return Err("可动画属性表为空".to_string());
    }
    Ok(names)
}

fn is_lowercase(text: &str) -> bool {
    !text.bytes().any(|b| b.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;

    use crate::rules::testutil::wxss::{messages, single};
    use super::TransitionProperty;

    #[test]
    fn properties_outside_the_whitelist_are_reported() {
        let (_f, project) = single(
            "transition-positive",
            r#"
            .a { transition: color .2s; }
            .b { transition-property: color, text-shadow; }
            .c { transition: visibility .2s; }
            .d { transition: color .2s, font-size .2s; }
            "#,
        );
        let diags = TransitionProperty.check(&project);
        assert_eq!(diags.len(), 4);
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Warning));
        assert!(diags[0].message.contains("`color`"));
        assert!(diags[0].message.contains("文本与字体相关属性"));
        assert!(diags[1].message.contains("`color`、`text-shadow`"));
        assert!(diags[2].message.contains("`visibility`"));
        assert!(diags[2].message.contains(super::OTHER_TAIL));
        // 同一声明里多个违规 ident 合成一条。
        assert!(diags[3].message.contains("`color`、`font-size`"));
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
            .g { transition: background .2s steps(4, end); }
            "#,
        );
        assert_eq!(
            messages(&TransitionProperty, &project),
            Vec::<String>::new()
        );
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

    #[test]
    fn data_file_has_no_blank_lines_or_duplicates() {
        let names = super::parse(super::SOURCE).unwrap();
        assert_eq!(names.len(), 65);
        assert!(names.contains("all"));
        assert!(names.contains("none"));
        assert!(names.contains("transform-origin"));
        assert!(names.contains("background-position-x"));
        assert!(names.contains("mask-position-x"));
        assert!(names.contains("mask-position-y"));
        assert!(!super::SOURCE.contains("\n\n"), "数据文件不能有空行");
        assert!(!names.contains("color"));

        let err = super::parse("all\nall\n").unwrap_err();
        assert!(err.contains("重复"), "{err}");
    }
}
