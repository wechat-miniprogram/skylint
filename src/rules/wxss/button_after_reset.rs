//! `button::after` 上的边框 / 背景重置在 Skyline 下不生效。
//!
//! WebView 下小程序 `<button>` 的默认描边由框架的伪元素 `::after` 绘制，工程里因此
//! 常见 `button::after { border: 0 }`。Skyline 下伪元素本身能画，但按钮的默认边框
//! 不是它画的：这条重置不会去掉按钮上那圈 1px 描边（背景同理）。
//!
//! 判据只看选择器与块内声明，不做 WXML 可达性推断：主体是标签 `button`（`.noborder`、
//! `button.foo` 都算，`view:after`、`.icon:after` 不算），伪元素是 `::after` / `:after`，
//! 且块里确实有 `border` / `border-*` 或 `background` / `background-*` 声明。只写
//! `content` / `display` 的块在 Skyline 下没有副作用，不报。
//!
//! 修法：在按钮选择器上直接写 `border: 0`，背景写 `background: transparent`。

use crate::input::wxss::{SelectorPart, WxssDeclaration, WxssPseudo, WxssRule, WxssSelector};
use crate::project::Project;
use crate::{Diagnostic, Severity};

use super::visit_matching_rules;
use crate::rules::{Rule, RuleMeta};

pub struct ButtonAfterReset;

impl ButtonAfterReset {
    pub const ID: &'static str = "skyline/button-after-reset";
}

const HELP: &str = "在按钮选择器上直接写 `border: 0`；背景同理写 `background: transparent`（或 `background: none`），不要依赖 `::after` 的重置";

impl Rule for ButtonAfterReset {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "`button::after` 上的边框/背景重置在 Skyline 下不生效，按钮默认描边/背景仍在",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit_matching_rules(project, |sheet, rule, out| {
            let Some(pseudo) = after_pseudo(rule) else {
                return;
            };
            let Some(declaration) = reset_declaration(rule) else {
                return;
            };
            let (line, column) = sheet.doc.position(pseudo.offset);
            let (end_line, end_column) = sheet.doc.position(pseudo.offset + pseudo.len);
            out.push(
                Diagnostic::new(
                    Self::ID,
                    Severity::Warning,
                    format!(
                        "`button::after` 上的 `{}: {}` 在 Skyline 下不生效：按钮的默认描边/背景不是由伪元素绘制，重置后按钮仍带默认边框/背景",
                        declaration.name, declaration.value
                    ),
                    sheet.doc.path(),
                    line,
                    column,
                )
                .end(end_line, end_column)
                .help(HELP),
            );
        })
    }
}

/// 找到带 `::after` / `:after`、且主体是 `button` 的选择器上的伪元素位置。
fn after_pseudo(rule: &WxssRule) -> Option<&WxssPseudo> {
    rule.selectors
        .iter()
        .filter(|selector| is_button_after(selector))
        .find_map(|selector| {
            selector
                .pseudos
                .iter()
                .find(|pseudo| pseudo.name == "after")
        })
}

/// 选择器主体（最后一个复合选择器）是不是标签 `button`。
fn is_button_after(selector: &WxssSelector) -> bool {
    if !selector.parts.contains(&SelectorPart::Other) {
        // 单 compound：部件顺序稳定，第一个部件就是选择器开头。
        return matches!(selector.parts.first(), Some(SelectorPart::Tag(tag)) if tag == "button");
    }
    // 组合 / 嵌套写法：按原文取最后一个 compound，去掉结尾的伪元素再看标签。
    last_compound(&selector.text)
        .and_then(strip_after)
        .and_then(tag_of)
        .is_some_and(|tag| tag.eq_ignore_ascii_case("button"))
}

/// 块里第一条 `border` / `background`（含 `border-*` / `background-*`）声明。
fn reset_declaration(rule: &WxssRule) -> Option<&WxssDeclaration> {
    rule.declarations.iter().find(|declaration| {
        let name = declaration.name.to_ascii_lowercase();
        name == "border"
            || name.starts_with("border-")
            || name == "background"
            || name.starts_with("background-")
    })
}

/// 按最后一个顶层组合符取最后一段。
fn last_compound(text: &str) -> Option<&str> {
    let mut depth = 0i32;
    let mut split = None;
    for (index, ch) in text.char_indices().rev() {
        match ch {
            ')' | ']' => depth += 1,
            '(' | '[' => depth -= 1,
            ' ' | '>' | '+' | '~' if depth == 0 => {
                split = Some(index);
                break;
            }
            _ => {}
        }
    }
    let last = match split {
        Some(index) => &text[index..],
        None => text,
    };
    let last = last
        .trim_start_matches(|ch: char| ch.is_whitespace() || matches!(ch, '>' | '+' | '~'))
        .trim();
    (!last.is_empty()).then_some(last)
}

/// 去掉 compound 结尾的 `::after` / `:after`。
fn strip_after(compound: &str) -> Option<&str> {
    let trimmed = compound.trim_end();
    ["::after", ":after"].into_iter().find_map(|suffix| {
        let head = trimmed.len().checked_sub(suffix.len())?;
        trimmed[head..]
            .eq_ignore_ascii_case(suffix)
            .then(|| trimmed[..head].trim_end())
    })
}

/// compound 开头的标签名；以 `.` / `#` / `:` / `[` / `*` 开头时没有标签。
fn tag_of(compound: &str) -> Option<&str> {
    let end = compound
        .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'))
        .unwrap_or(compound.len());
    (end > 0).then(|| &compound[..end])
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testutil::wxss::{messages, page};

    use super::ButtonAfterReset;

    fn report(name: &str, wxss: &str) -> Vec<String> {
        let (_fixture, project) = page(name, r#"<button class="noborder"/>"#, wxss);
        messages(&ButtonAfterReset, &project)
    }

    #[test]
    fn resets_on_button_after_are_reported() {
        for (name, source) in [
            ("button-after-border", "button::after { border: 0; }"),
            (
                "button-class-after-border",
                "button.noborder:after { border: 0; }",
            ),
            (
                "button-after-background",
                "button:after { background: transparent; }",
            ),
        ] {
            let out = report(name, source);
            assert_eq!(out.len(), 1, "{source}: {out:?}");
            assert!(out[0].contains("Skyline 下不生效"), "{out:?}");
            assert!(out[0].contains("默认描边/背景"), "{out:?}");
        }
    }

    #[test]
    fn non_reset_or_non_button_pseudos_are_ignored() {
        for (name, source) in [
            (
                "button-after-content-only",
                "button::after { content: none; display: none; }",
            ),
            ("icon-after-border", ".icon::after { border: 0; }"),
            ("view-after-border", "view:after { border: 0; }"),
        ] {
            assert_eq!(report(name, source), Vec::<String>::new(), "{source}");
        }
    }

    /// 一次选择器一条：块里同时重置边框和背景也只报一条。
    #[test]
    fn one_diagnostic_per_selector() {
        let out = report(
            "button-after-many",
            "button::after { border: 0; background: none; }",
        );
        assert_eq!(out.len(), 1, "{out:?}");
    }

    /// 组合选择器的主体仍是 `button`；指到行、指到 `::after`。
    #[test]
    fn complex_selector_subject_and_position() {
        let (_fixture, project) = page(
            "button-after-complex",
            r#"<view class="list"><button/></view>"#,
            ".list button::after { border-width: 0; }\n",
        );
        let diags = ButtonAfterReset.check(&project);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert_eq!((diags[0].line, diags[0].column), (1, 13));
        assert_eq!(diags[0].end_column, 20);
    }
}
