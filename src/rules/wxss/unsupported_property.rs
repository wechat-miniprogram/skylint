//! Skyline 下完全不生效的属性。

use crate::project::Project;
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

use super::dict_table::table;
use super::visit_declarations;

pub struct UnsupportedProperty;

impl UnsupportedProperty {
    pub const ID: &'static str = "skyline/unsupported-property";
}

impl Rule for UnsupportedProperty {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Error,
            description: "声明了 Skyline 完全不支持的属性，该属性不会生效",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let table = table();
        visit_declarations(project, |doc, decl, out| {
            let name = decl.name.to_ascii_lowercase();
            let Some(advice) = table.property(&name) else {
                return;
            };
            let (line, column) = doc.position(decl.offset);
            let (end_line, end_column) = doc.position(decl.offset + decl.name.len());
            out.push(
                Diagnostic::new(
                    Self::ID,
                    Severity::Error,
                    format!("`{name}` 属性在 Skyline 下不生效，该声明会被忽略"),
                    doc.path(),
                    line,
                    column,
                )
                .end(end_line, end_column)
                .help(advice),
            );
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;

    use super::UnsupportedProperty;
    use crate::rules::testutil::wxss::{messages, single};

    #[test]
    fn property_reports_unsupported_properties() {
        let (_f, project) = single(
            "property-positive",
            r#"
            .a { float: left; }
            .b { writing-mode: vertical-rl; }
            .c { text-indent: 2em; }
            .d { mask-clip: border-box; }
            .e { justify-items: center; }
            .f { list-style-type: none; }
            .g { FLOAT: left; }
            "#,
        );
        let diags = UnsupportedProperty.check(&project);
        assert_eq!(diags.len(), 7);
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Error));
        assert!(diags.iter().any(|d| d.message.contains("`float`")));
        assert!(diags.iter().any(|d| d.message.contains("`writing-mode`")));
        assert!(diags.iter().any(|d| d.message.contains("`text-indent`")));
        assert!(diags.iter().any(|d| d.message.contains("`mask-clip`")));
        assert!(diags.iter().any(|d| d.message.contains("`justify-items`")));
        assert_eq!(diags[0].help.as_deref(), Some("使用 flex 布局"));
    }

    #[test]
    fn property_ignores_custom_properties_values_and_prefixed_names() {
        let (_f, project) = single(
            "property-negative",
            r#"
            .a { --float: left; }
            .b { -wx-contain: size; }
            .c { content: "float"; }
            .d { transition-property: float; }
            .e { -webkit-mask-clip: border-box; }
            .f { color: red; }
            .g { --writing-mode: vertical-rl; }
            .h { border-left: 1px solid red; }
            .i { mask-position: center; }
            "#,
        );
        assert_eq!(
            messages(&UnsupportedProperty, &project),
            Vec::<String>::new()
        );
    }

    #[test]
    fn property_reports_missing_property_ids() {
        let (_f, project) = single(
            "property-unconditional",
            r#"
            .c { text-transform: uppercase; }
            .d { user-select: none; }
            "#,
        );
        let diags = UnsupportedProperty.check(&project);
        assert_eq!(diags.len(), 2, "{diags:#?}");
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Error));
        assert!(diags[0].message.contains("`text-transform`"));
        assert!(diags[1].message.contains("`user-select`"));
        assert!(diags[1].help.as_deref().unwrap().contains("user-select"));
    }

    #[test]
    fn animation_play_state_takes_effect_statically() {
        // 引擎会为它发 Diag，但 Parse 返回的是有效值（animation_play_state.cc:51-86），
        // 且运行时读 PlayStateList 判 kPaused（css_animations.cc:415-416）——静态写法生效。
        let (_f, project) = single(
            "property-animation",
            r#"
            .a { animation-play-state: paused; }
            .b { animation-play-state: running; }
            "#,
        );
        assert_eq!(
            messages(&UnsupportedProperty, &project),
            Vec::<String>::new()
        );
    }

    #[test]
    fn property_names_are_matched_exactly_not_by_substring() {
        let (_f, project) = single(
            "property-attribution",
            r#"
            .a { background-clip: padding-box; }
            "#,
        );
        let diags = UnsupportedProperty.check(&project);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("`background-clip`"));
        assert!(!diags[0].message.contains("mask-clip"));
    }
}
