//! 属性可用，但该**整值**在 Skyline 下不支持，声明会被静默忽略。

use crate::project::Project;
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

use super::dict_table::{normalize_value, table};
use super::value_util::at_declaration;
use super::visit_declarations;

pub struct UnsupportedValue;

/// 尺寸类属性在 fp 里都是 `<length_percentage_auto>`：只接受 number / percentage /
/// dimension / `auto` / `env()` / `calc()` / `min()` / `max()` / `clamp()`
/// （`float-pigment-css/src/property.rs:781-786`、`parser/property_value/mod.rs:141-146,215-249`）。
/// 内在尺寸关键字不在其中，解析失败后整条声明被回滚（`parser/mod.rs:1075`）。
const INTRINSIC_SIZE_PROPERTIES: &[&str] = &[
    "width",
    "height",
    "min-width",
    "min-height",
    "max-width",
    "max-height",
    "flex-basis",
];

/// `stretch` 与 `-webkit-fill-available` 同样不在语法里，但 WebView 的支持面也随版本变，
/// 两个渲染器一起失效就不构成迁移差异，故不报。
const INTRINSIC_SIZE_KEYWORDS: &[&str] = &["fit-content", "min-content", "max-content"];

fn intrinsic_size_keyword(name: &str, value: &str) -> Option<&'static str> {
    if !INTRINSIC_SIZE_PROPERTIES.contains(&name) {
        return None;
    }
    INTRINSIC_SIZE_KEYWORDS.iter().copied().find(|keyword| {
        value == *keyword
            || value
                .strip_prefix(keyword)
                .is_some_and(|rest| rest.starts_with('('))
    })
}

impl UnsupportedValue {
    pub const ID: &'static str = "skyline/unsupported-value";
}

impl Rule for UnsupportedValue {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Error,
            description: "属性在 Skyline 下可用，但该值不支持，声明会被静默忽略",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let table = table();
        visit_declarations(project, |doc, _, decl, out| {
            let name = decl.name.to_ascii_lowercase();
            let Some(value) = normalize_value(&decl.value) else {
                return;
            };
            let raw = decl.value.trim();
            let (message, advice) = if let Some(keyword) = intrinsic_size_keyword(&name, &value) {
                (
                    format!(
                        "`{name}: {raw}` 用了内在尺寸关键字 `{keyword}`：Skyline 的 `{name}` 只接受长度 / 百分比 / `auto`，整条声明会被丢弃"
                    ),
                    "换成明确长度或 `auto`；要按内容宽度收缩可用 `display: inline-flex`（父级是 flex 时用 `align-items: flex-start`）",
                )
            } else if let Some(advice) = table.value(&name, &value) {
                (
                    format!("`{name}: {raw}` 这个取值在 Skyline 下不受支持，声明会被忽略"),
                    advice,
                )
            } else if let Some(advice) = table.length_valued(&name, &value) {
                // 该属性只认关键字：长度值连解析都过不了，整条声明被丢弃。
                (
                    format!("`{name}: {raw}` 使用了长度值，Skyline 下该值不支持，整条声明会被丢弃"),
                    advice,
                )
            } else {
                // 多值形态：引擎按项解析（分轴 overflow、分边 border-style），
                // 不支持的项被丢弃，其余项照常生效。
                let items = table.unsupported_items(&name, &value);
                let Some((_, advice)) = items.first().copied() else {
                    return;
                };
                let names = items
                    .iter()
                    .map(|(item, _)| format!("`{item}`"))
                    .collect::<Vec<_>>()
                    .join("、");
                (
                    format!("`{name}: {raw}` 里的 {names} 在 Skyline 下不受支持，这些项会被丢弃"),
                    advice,
                )
            };
            out.push(at_declaration(
                Self::ID,
                doc,
                decl,
                Severity::Error,
                message,
                advice,
            ));
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;

    use super::UnsupportedValue;
    use crate::rules::testutil::wxss::{messages, single};

    #[test]
    fn value_reports_unsupported_items_of_multi_value_forms() {
        // 引擎按项解析（overflow 分轴、border-style 分边），不支持的项被丢弃，
        // 其余项照常生效。
        let (_f, project) = single(
            "value-multi",
            r#"
            .a { overflow: hidden auto; }
            .b { border-style: solid double; }
            .c { border-top-style: double groove; }
            "#,
        );
        let diags = UnsupportedValue.check(&project);
        assert_eq!(diags.len(), 3, "{diags:?}");
        assert!(diags[0].message.contains("`auto`"), "{}", diags[0].message);
        assert!(
            diags[1].message.contains("`double`"),
            "{}",
            diags[1].message
        );
        assert!(
            diags[2].message.contains("`double`") && diags[2].message.contains("`groove`"),
            "{}",
            diags[2].message
        );
    }

    #[test]
    fn multi_value_forms_keep_supported_items_silent() {
        let (_f, project) = single(
            "value-multi-negative",
            r#"
            .a { overflow: hidden visible; }
            .b { border-style: solid dashed none dotted; }
            .c { padding: 1px 2px 3px 4px; }
            "#,
        );
        assert!(UnsupportedValue.check(&project).is_empty());
    }

    #[test]
    fn value_reports_unsupported_values() {
        let (_f, project) = single(
            "value-positive",
            r#"
            .a { display: grid; }
            .b { position: sticky; }
            .c { overflow: auto; }
            .d { visibility: collapse; }
            .e { white-space: pre-wrap; }
            .f { font-weight: bolder; }
            .g { border-top-style: double; }
            .h { DISPLAY: GRID !important; }
            "#,
        );
        let diags = UnsupportedValue.check(&project);
        assert_eq!(diags.len(), 8);
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Error));
        assert!(diags.iter().any(|d| d.message.contains("display: grid")));
        assert!(diags.iter().any(|d| d.message.contains("position: sticky")));
        assert!(diags.iter().any(|d| d.message.contains("overflow: auto")));
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("border-top-style: double"))
        );
        assert_eq!(diags[0].help.as_deref(), Some("改用 flex 布局替代 grid"));
    }

    #[test]
    fn value_ignores_supported_values_vars_and_partial_matches() {
        let (_f, project) = single(
            "value-negative",
            r#"
            .a { display: flex; }
            .b { position: relative; }
            .c { overflow: hidden; }
            .d { display: var(--d); }
            .e { margin: 0 auto; }
            .f { background: url(grid.png); }
            .g { overflow: visible hidden; }
            .h { content: "pre-wrap"; }
            .i { display: env(display); }
            .j { width: calc(100% - 1px); }
            .k { text-decoration-line: underline line-through; }
            .l { --grid: 1; }
            .m { border-style: hidden; }
            .n { border-top-style: hidden; }
            "#,
        );
        assert_eq!(messages(&UnsupportedValue, &project), Vec::<String>::new());
    }

    #[test]
    fn value_reports_intrinsic_size_keywords() {
        // fp 的 width/height/min/max/flex-basis 都是 `<length_percentage_auto>`，
        // `fit-content` 一类关键字解析失败 → 整条声明回滚。
        let (_f, project) = single(
            "value-intrinsic-size",
            r#"
            .a { width: fit-content; }
            .b { height: fit-content; }
            .c { min-width: min-content; }
            .d { max-height: max-content; }
            .e { flex-basis: fit-content(200px); }
            .f { WIDTH: FIT-CONTENT !important; }
            "#,
        );
        let diags = UnsupportedValue.check(&project);
        assert_eq!(diags.len(), 6, "{diags:?}");
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Error));
        assert!(diags[0].message.contains("width: fit-content"));
        assert!(diags[2].message.contains("min-content"));
        assert!(diags[4].message.contains("fit-content(200px)"));
        assert!(diags[0].help.as_deref().unwrap().contains("inline-flex"));
    }

    #[test]
    fn intrinsic_size_keywords_stay_silent_outside_size_properties() {
        let (_f, project) = single(
            "value-intrinsic-size-negative",
            r#"
            .a { width: 100%; }
            .b { height: auto; }
            .c { min-width: calc(100% - 24px); }
            .d { max-width: var(--w); }
            .e { width: fit-content-grid; }
            .f { line-height: max-content; }
            .g { padding: 0 fit-content; }
            "#,
        );
        assert_eq!(messages(&UnsupportedValue, &project), Vec::<String>::new());
    }

    #[test]
    fn value_reports_length_values_of_vertical_align() {
        // fp 的 vertical-align 值语法只有关键字分支（property.rs:426-433），长度值
        // 解析失败后属性被回滚（parser/mod.rs:1768-1781）；引擎 ParseValue 也只认关键字。
        let (_f, project) = single(
            "value-vertical-align-length",
            r#"
            .a { vertical-align: 4px; }
            .b { vertical-align: -2px; }
            .c { vertical-align: 50%; }
            .d { vertical-align: 1.5em; }
            .e { VERTICAL-ALIGN: 4rpx !important; }
            "#,
        );
        let diags = UnsupportedValue.check(&project);
        assert_eq!(diags.len(), 5, "{diags:?}");
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Error));
        assert!(diags.iter().all(|d| d.message.contains("长度值")));
        assert!(diags[0].message.contains("vertical-align: 4px"));
        assert!(diags[2].message.contains("50%"));
        assert!(
            diags[0]
                .help
                .as_deref()
                .unwrap()
                .contains("baseline/top/middle/bottom")
        );
    }

    #[test]
    fn length_values_of_other_properties_and_keywords_stay_silent() {
        let (_f, project) = single(
            "value-vertical-align-length-negative",
            r#"
            .a { vertical-align: middle; }
            .b { vertical-align: baseline; }
            .c { vertical-align: top; }
            .d { vertical-align: bottom; }
            .e { vertical-align: var(--align); }
            .f { vertical-align: env(safe-area-inset-bottom); }
            .g { padding: 4px; }
            .h { margin-top: -2px; }
            .i { line-height: 1.5em; }
            .j { width: 50%; }
            "#,
        );
        assert_eq!(messages(&UnsupportedValue, &project), Vec::<String>::new());
    }
}
