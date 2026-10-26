//! 简写属性本身生效，但值里属于不受支持子属性的那部分会被静默丢弃。
//!
//! 与 [`super::unsupported_property`] 的分界：那里按**属性名**判（`background-attachment: fixed`
//! 是 error），这里按**简写值里的 token** 判（`background: … fixed` 只是 warning，其余子属性照常）。
//! 数据以 const 形式定义在本模块。

use crate::input::wxss::{ValueToken, WxssDeclaration, WxssDoc};
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

use super::value_util::{at_value, is_dynamic, token_text};
use super::visit_declarations;

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
        visit_declarations(project, |doc, _, decl, out| {
            check_decl(doc, decl, out);
        })
    }
}

fn check_decl(doc: &WxssDoc, decl: &WxssDeclaration, out: &mut Vec<Diagnostic>) {
    let name = decl.name.to_ascii_lowercase();
    if is_dynamic(&decl.tokens) {
        return;
    }
    // 一行数据最多报一条：同一行命中多个 token（`fixed border-box`）时合并，避免刷屏。
    for row in ROWS.iter().filter(|row| row.property == name.as_str()) {
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

struct Row {
    property: &'static str,
    triggers: &'static [&'static str],
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

/// 来源：skyline-skills/skills/skyline-wxss/SKILL.md:132-142、references/visual.md:35、:135、references/text.md:86、:123
///
/// 语义：简写属性本身生效，但值里属于某个不受支持子属性的那部分会被静默丢弃，其余子属性照常。
///       长写这些子属性属于不支持属性表（error），不在这里。
/// 触发的 token = ident（大小写不敏感、整值匹配）；`length` 是伪 token，匹配任意长度 / 百分比词元。
/// 与初始值相同的 token 不收录：写了等于没写，丢弃也不改变渲染（background-attachment
/// 初始值 scroll、mask-origin / mask-clip 初始值 border-box）。
/// 匹配：含 var( / env( / {{ / 预处理器变量的值无法静态判定，整条声明跳过。
/// 同一属性按声明顺序匹配，`(property, trigger)` 不重复。
const ROWS: &[Row] = &[
    Row {
        property: "background",
        triggers: &["fixed", "local"],
        dropped: "background-attachment",
        note: "简写里的 attachment 被丢弃",
    },
    Row {
        property: "background",
        triggers: &["border-box", "padding-box", "content-box"],
        dropped: "background-origin / background-clip",
        note: "简写里的盒关键字被丢弃",
    },
    Row {
        property: "mask",
        triggers: &["padding-box", "content-box"],
        dropped: "mask-origin / mask-clip",
        note: "简写里的盒关键字被丢弃",
    },
    Row {
        property: "mask",
        triggers: &["no-clip"],
        dropped: "mask-clip",
        note: "no-clip 被丢弃",
    },
    Row {
        property: "text-decoration",
        triggers: &[LENGTH],
        dropped: "text-decoration-thickness",
        note: "长度值被当作 thickness 丢弃",
    },
];

#[cfg(test)]
mod tests {

    use crate::rules::Rule;

    use super::ShorthandPartial;
    use crate::rules::testutil::wxss::{messages, single};

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
}
