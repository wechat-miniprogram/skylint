//! 简写属性本身生效，但值里属于不受支持子属性的那部分会被静默丢弃。
//!
//! 与 [`super::unsupported_property`] 的分界：那里按**属性名**判（`background-attachment: fixed`
//! 是 error），这里按**简写值里的 token** 判（`background: … fixed` 只是 warning，其余子属性照常）。
//! 数据在 `data/shorthand_partial.txt`。

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use crate::input::wxss::{ValueToken, WxssDeclaration, WxssDoc};
use crate::project::Project;
use crate::rules::data_table::{self, Columns};
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

use super::value_util::{at_value, is_dynamic, token_text};
use super::visit;

const SOURCE: &str = include_str!("../data/shorthand_partial.txt");

const HELP: &str = "改用受支持的子属性，或从简写里去掉这一段";

/// 伪 token：匹配任意长度 / 百分比词元。
const LENGTH: &str = "length";

pub struct ShorthandPartial;

impl ShorthandPartial {
    pub const ID: &'static str = "skyline/shorthand-partial";
}

impl Rule for ShorthandPartial {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "简写属性在 Skyline 下部分生效：值里属于不受支持子属性的那一段被丢弃",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let table = data();
        visit(project, |doc, out| {
            for rule in &doc.rules {
                for decl in &rule.declarations {
                    check_decl(table, doc, decl, out);
                }
            }
        })
    }
}

fn check_decl(table: &Table, doc: &WxssDoc, decl: &WxssDeclaration, out: &mut Vec<Diagnostic>) {
    let name = decl.name.to_ascii_lowercase();
    let Some(rows) = table.get(name.as_str()) else {
        return;
    };
    if is_dynamic(&decl.tokens) {
        return;
    }
    // 一行数据最多报一条：同一行命中多个 token（`fixed border-box`）时合并，避免刷屏。
    for row in rows {
        let mut hits: Vec<String> = Vec::new();
        for token in &decl.tokens {
            let Some(text) = row.trigger(token) else {
                continue;
            };
            if !hits.iter().any(|seen| seen.eq_ignore_ascii_case(&text)) {
                hits.push(text);
            }
        }
        if hits.is_empty() {
            continue;
        }
        let list = hits
            .iter()
            .map(|text| format!("`{text}`"))
            .collect::<Vec<_>>()
            .join("、");
        out.push(at_value(
            ShorthandPartial::ID,
            doc,
            decl,
            format!(
                "`{name}: {}` 里的 {list} 属于 `{}`，{}（其余子属性正常生效）",
                decl.value.trim(),
                row.dropped,
                row.note
            ),
            HELP,
        ));
    }
}

#[derive(Debug)]
struct Row {
    triggers: Vec<&'static str>,
    dropped: &'static str,
    note: &'static str,
}

impl Row {
    /// 命中时返回 token 原文（消息里引用用）。
    fn trigger(&self, token: &ValueToken) -> Option<String> {
        match token {
            ValueToken::Ident(ident) => self
                .triggers
                .iter()
                .find(|trigger| trigger.eq_ignore_ascii_case(ident))
                .map(|_| ident.clone()),
            ValueToken::Dimension { .. } | ValueToken::Percentage(_)
                if self.triggers.contains(&LENGTH) =>
            {
                Some(token_text(token))
            }
            _ => None,
        }
    }
}

type Table = HashMap<&'static str, Vec<Row>>;

fn data() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| parse(SOURCE).expect("WXSS 简写部分生效表格式错误"))
}

/// 解析数据文件。格式错误一律返回带行号的说明，便于同步时定位。
fn parse(source: &'static str) -> Result<Table, String> {
    let parsed = data_table::read(source, data_table::split_on('|'), |_| {
        Columns::new(4, "`简写属性 | 触发的 token | 被丢弃的子属性 | 说明` 四列")
    })?;

    let mut table: Table = HashMap::new();
    let mut seen = HashSet::new();
    for row in parsed.rows {
        let (property, triggers, dropped, note) =
            (row.fields[0], row.fields[1], row.fields[2], row.fields[3]);
        if property.is_empty() || triggers.is_empty() || dropped.is_empty() || note.is_empty() {
            return Err(format!("第 {} 行：四列都不能为空", row.line_no));
        }
        if !is_lowercase(property) || !is_lowercase(triggers) {
            return Err(format!(
                "第 {} 行：属性与 token 必须小写：{property} | {triggers}",
                row.line_no
            ));
        }
        let triggers: Vec<&'static str> = triggers
            .split(',')
            .map(str::trim)
            .filter(|trigger| !trigger.is_empty())
            .collect();
        for trigger in &triggers {
            data_table::unique(
                &mut seen,
                (property, trigger.to_ascii_lowercase()),
                row.line_no,
                || format!("重复的 token {property} | {trigger}"),
            )?;
        }
        table.entry(property).or_default().push(Row {
            triggers,
            dropped,
            note,
        });
    }

    Ok(table)
}

fn is_lowercase(text: &str) -> bool {
    !text.bytes().any(|b| b.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;

    use crate::rules::testutil::wxss::{messages, single};
    use super::ShorthandPartial;

    #[test]
    fn unsupported_sub_properties_in_shorthands_are_reported() {
        let (_f, project) = single(
            "shorthand-positive",
            r#"
            .a { background: url(a) no-repeat fixed; }
            .b { background: red border-box; }
            .c { mask: url(a) no-repeat content-box; }
            .d { text-decoration: underline 2px; }
            .e { MASK: NO-CLIP; }
            "#,
        );
        let diags = ShorthandPartial.check(&project);
        assert_eq!(diags.len(), 5);
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Warning));
        assert!(diags[0].message.contains("`fixed`"));
        assert!(diags[0].message.contains("background-attachment"));
        assert!(diags[1].message.contains("`border-box`"));
        assert!(
            diags[1]
                .message
                .contains("background-origin / background-clip")
        );
        assert!(diags[2].message.contains("mask-origin / mask-clip"));
        assert!(diags[3].message.contains("`2px`"));
        assert!(diags[3].message.contains("text-decoration-thickness"));
        assert!(diags[4].message.contains("`NO-CLIP`"));
        assert_eq!(diags[0].help.as_deref(), Some(super::HELP));
    }

    #[test]
    fn one_row_reports_once_and_lists_every_hit_token() {
        let (_f, project) = single(
            "shorthand-merge",
            r#"
            .a { background: url(a) local fixed; }
            .b { background: url(a) fixed border-box; }
            "#,
        );
        let diags = ShorthandPartial.check(&project);
        // `.a` 只命中 attachment 一行（两个 token 合并成一条）；`.b` 命中两行。
        assert_eq!(diags.len(), 3);
        assert!(diags[0].message.contains("`local`、`fixed`"));
        assert_eq!(diags[0].message.matches("background-attachment").count(), 1);
    }

    /// 与初始值相同的 token 不报：`background: scroll` / `mask: border-box` 写了等于没写。
    #[test]
    fn initial_values_are_not_reported() {
        let (_f, project) = single(
            "shorthand-initial",
            r#"
            .a { background: url(a) scroll; }
            .b { mask: url(a) border-box; }
            "#,
        );
        assert!(ShorthandPartial.check(&project).is_empty());
    }

    #[test]
    fn supported_shorthands_and_longhands_are_not_reported() {
        let (_f, project) = single(
            "shorthand-negative",
            r#"
            .a { background: url(a) no-repeat center/cover; }
            .b { mask: url(a) no-repeat; }
            .c { text-decoration: underline red; }
            .d { background-attachment: fixed; }
            .e { background: var(--bg) fixed; }
            .f { mask: url(a) local; }
            .g { transition: color .2s; }
            "#,
        );
        assert_eq!(messages(&ShorthandPartial, &project), Vec::<String>::new());
    }

    #[test]
    fn data_file_parses_without_duplicate_triggers() {
        let table = super::parse(super::SOURCE).unwrap();
        assert_eq!(table["background"].len(), 2);
        assert_eq!(table["mask"].len(), 2);
        assert!(table["background"][0].triggers.contains(&"fixed"));
        assert!(table["text-decoration"][0].triggers.contains(&"length"));

        let err =
            super::parse("background | fixed | x | y\nbackground | fixed | z | w\n").unwrap_err();
        assert!(err.contains("重复的 token"), "{err}");
        let err = super::parse("background | FIXED | x | y\n").unwrap_err();
        assert!(err.contains("必须小写"), "{err}");
        let err = super::parse("background | fixed | x\n").unwrap_err();
        assert!(err.contains("四列"), "{err}");
    }
}
