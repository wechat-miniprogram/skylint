//! `text-decoration-line` 只支持单值。
//!
//! `text-decoration` 简写会展开到 line（`style_property_shorthand.cc:213-223`），
//! 所以 `text-decoration: underline line-through` 同样丢线；fp 的
//! `TextDecorationLineItem` 只有 overline / line-through / underline / blink
//! （`typing.rs:1518-1523`），`spelling-error` / `grammar-error` 整条解析失败。

use crate::project::Project;
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

use super::value_util::{at_value, is_dynamic, is_ident, split_tokens};
use super::visit_declarations;

/// 能落到 `text-decoration-line` 的线型关键字。
const LINE_KEYWORDS: [&str; 4] = ["underline", "overline", "line-through", "blink"];

/// fp 的枚举里没有这两个线上值（`typing.rs:1518-1523`）。
const UNSUPPORTED_LINES: [&str; 2] = ["spelling-error", "grammar-error"];

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
        visit_declarations(project, |sheet, _rule, decl, out| {
            let doc = &sheet.doc;
            let name = decl.name.to_ascii_lowercase();
            if name != "text-decoration-line" && name != "text-decoration" {
                return;
            }
            let value = decl.value.trim();
            if value.is_empty() || is_dynamic(&decl.tokens) {
                return;
            }
            let groups = split_tokens(&decl.tokens);
            // 长写里每个 token 都是线型值；简写只数线型关键字（颜色/样式/厚度不算）。
            let lines = if name == "text-decoration-line" {
                groups.len()
            } else {
                groups
                    .iter()
                    .filter(|group| {
                        group.first().is_some_and(|token| {
                            LINE_KEYWORDS.iter().any(|word| is_ident(token, word))
                        })
                    })
                    .count()
            };
            let invalid = groups.iter().any(|group| {
                group
                    .first()
                    .is_some_and(|token| UNSUPPORTED_LINES.iter().any(|word| is_ident(token, word)))
            });
            if !invalid && lines <= 1 {
                return;
            }
            let message = if invalid {
                format!(
                    "`{name}: {value}` 里的线型不被支持（只有 underline / overline / line-through / blink），整条声明会被丢弃"
                )
            } else {
                format!(
                    "`{name}: {value}` 一次声明了 {lines} 种线型，Skyline 只支持单值，整条声明会被丢弃"
                )
            };
            out.push(at_value(
                Self::ID,
                doc,
                decl,
                message,
                "只保留一个线型，例如 `text-decoration: underline`",
            ));
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;

    use super::TextDecorationLine;
    use crate::rules::testutil::wxss::{messages, single};

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
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Warning));
    }

    /// 简写会展开到 line，多线型与 fp 不认的值同样不生效。
    #[test]
    fn text_decoration_shorthand_is_checked_too() {
        let (_f, project) = single(
            "decoration-shorthand",
            r#"
            .a { text-decoration: underline line-through; }
            .b { text-decoration: underline red; }
            .c { text-decoration: spelling-error; }
            "#,
        );
        let diags = TextDecorationLine.check(&project);
        assert_eq!(diags.len(), 2, "{diags:#?}");
        assert!(diags[0].message.contains("2 种线型"));
        assert!(diags[1].message.contains("不被支持"));
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
}
