//! Skyline 下完全不生效的属性。

use crate::project::Project;
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

use super::dict_table::table;
use super::visit_matching_rules;
use crate::input::wxml::WxmlDoc;

use std::collections::HashMap;

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
        let mut docs: HashMap<String, Option<WxmlDoc>> = HashMap::new();
        visit_matching_rules(project, |sheet, rule, out| {
            let doc = &sheet.doc;
            for decl in &rule.declarations {
                let name = decl.name.to_ascii_lowercase();
                let Some(advice) = table.property(&name) else {
                    continue;
                };
                if name == "line-clamp" && rule.declaration("-webkit-line-clamp").is_some() {
                    // 同一块里两条都写时只报一次（autoprefixer 产物很常见）。
                    continue;
                }
                // `word-break: break-all` 在两端都生效，且它允许在任意字符处断行，
                // 已经覆盖了 `word-wrap: break-word` 的效果，丢掉后者没有渲染差异。
                if name == "word-wrap"
                    && rule
                        .declaration("word-break")
                        .is_some_and(|decl| decl.value.trim().eq_ignore_ascii_case("break-all"))
                {
                    continue;
                }
                if super::is_css_wide_keyword(&decl.value) {
                    continue;
                }
                // 声明恰好等于初始值（`object-fit: fill`）：丢弃与不丢弃渲染一致。
                if super::dict_table::is_initial_value(&name, &decl.value) {
                    continue;
                }
                // `-webkit-line-clamp` 被丢弃，但宿主是带同一行数的 `<text max-lines>` 时，
                // 截断由属性完成，两端结果一致。
                if name == "-webkit-line-clamp"
                    && super::truncation_by_max_lines(
                        project,
                        sheet,
                        &rule.selectors,
                        &mut docs,
                        &decl.value,
                    )
                {
                    continue;
                }
                if super::overridden_in_renderer_branch(
                    project,
                    sheet,
                    rule,
                    &name,
                    self.target(),
                    &mut docs,
                ) {
                    continue;
                }
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
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;

    use super::UnsupportedProperty;
    use crate::rules::testutil::wxss::{messages, page};

    #[test]
    fn property_reports_unsupported_properties() {
        let (_f, project) = page(
            "property-positive",
            r#"<view class="a"/><view class="b"/><view class="c"/><view class="d"/><view class="e"/><view class="f"/><view class="g"/><view class="h"/><view class="i"/><view class="j"/>"#,
            r#"
            .a { float: left; }
            .b { writing-mode: vertical-rl; }
            .c { text-indent: 2em; }
            .d { mask-clip: content-box; }
            .e { justify-items: center; }
            .f { list-style-type: none; }
            .g { FLOAT: left; }
            .h { -webkit-line-clamp: 2; }
            .i { object-fit: cover; }
            .j { object-position: left top; }
            "#,
        );
        let diags = UnsupportedProperty.check(&project);
        assert_eq!(diags.len(), 10);
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Error));
        assert!(diags.iter().any(|d| d.message.contains("`float`")));
        assert!(diags.iter().any(|d| d.message.contains("`writing-mode`")));
        assert!(diags.iter().any(|d| d.message.contains("`text-indent`")));
        assert!(diags.iter().any(|d| d.message.contains("`mask-clip`")));
        assert!(diags.iter().any(|d| d.message.contains("`justify-items`")));
        assert_eq!(diags[0].help.as_deref(), Some("使用 flex 布局"));
    }

    /// 声明恰好是属性的初始值（`object-fit: fill`、`background-clip: border-box`）时，
    /// 丢弃与不丢弃渲染结果相同。
    #[test]
    fn initial_values_are_not_reported() {
        let (_f, project) = page(
            "property-initial-values",
            r#"<view class="a"/><view class="b"/><view class="c"/><view class="d"/><view class="e"/><view class="f"/><view class="g"/>"#,
            r#"
            .a { object-fit: fill; }
            .b { background-clip: border-box; }
            .c { background-origin: padding-box; }
            .d { mask-clip: border-box; }
            .e { float: none; }
            .f { list-style-type: disc; }
            .g { object-fit: cover; }
            "#,
        );
        let diags = UnsupportedProperty.check(&project);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert!(diags[0].message.contains("`object-fit`"));
    }

    #[test]
    fn property_ignores_custom_properties_values_and_prefixed_names() {
        let (_f, project) = page(
            "property-negative",
            r#"<view class="a"/><view class="b"/><view class="c"/><view class="d"/><view class="e"/><view class="f"/><view class="g"/><view class="h"/><view class="i"/>"#,
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
        let (_f, project) = page(
            "property-unconditional",
            r#"<view class="c"/><view class="d"/>"#,
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

    /// `word-wrap` 是 `overflow-wrap` 的旧名，引擎属性表里同样没有；`normal` 是初始值，
    /// 同一块又写了 `word-break: break-all` 时断行效果已被覆盖，两者都不报。
    #[test]
    fn word_wrap_is_reported_unless_redundant() {
        let (_f, project) = page(
            "property-word-wrap",
            r#"<view class="a"/><view class="b"/><view class="c"/>"#,
            r#"
            .a { word-wrap: break-word; }
            .b { word-wrap: normal; }
            .c { word-wrap: break-word; word-break: break-all; }
            "#,
        );
        let diags = UnsupportedProperty.check(&project);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert!(diags[0].message.contains("`word-wrap`"));
        assert_eq!(diags[0].help.as_deref(), Some("使用 word-break"));
    }

    /// `clip-path` 在引擎里没有对应属性 id，描边式擦除动画会整条失效；`none` 是初始值。
    #[test]
    fn clip_path_is_reported_unless_none() {
        let (_f, project) = page(
            "property-clip-path",
            r#"<view class="a"/><view class="b"/>"#,
            r#"
            .a { clip-path: inset(0 100% 0 0); }
            .b { clip-path: none; }
            "#,
        );
        let diags = UnsupportedProperty.check(&project);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert!(diags[0].message.contains("`clip-path`"));
    }

    #[test]
    fn animation_play_state_takes_effect_statically() {
        // 引擎会为它发 Diag，但 Parse 返回的是有效值（animation_play_state.cc:51-86），
        // 且运行时读 PlayStateList 判 kPaused（css_animations.cc:415-416）——静态写法生效。
        let (_f, project) = page(
            "property-animation",
            r#"<view class="a"/><view class="b"/>"#,
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
    fn css_wide_keyword_is_not_reported() {
        // `unset` 是迁移工程关掉 WebView 侧写法的手段，不是「用了不支持的值」。
        let (_f, project) = page(
            "property-wide-keyword",
            r#"<view class="a"/><view class="b"/><view class="c"/><view class="d"/>"#,
            r#"
            .a { float: unset; }
            .b { -webkit-line-clamp: unset; }
            .c { writing-mode: initial; }
            .d { text-indent: revert; }
            "#,
        );
        assert_eq!(
            messages(&UnsupportedProperty, &project),
            Vec::<String>::new()
        );
    }

    #[test]
    fn skyline_branch_override_exempts_the_base_declaration() {
        let (_f, project) = page(
            "property-skyline-branch",
            r#"<text class="name {{renderer}}">x</text>"#,
            r#"
            .name { -webkit-line-clamp: 2; float: left; }
            .name.skyline { -webkit-line-clamp: unset; float: unset; }
            "#,
        );
        assert_eq!(
            messages(&UnsupportedProperty, &project),
            Vec::<String>::new()
        );
    }

    #[test]
    fn skyline_branch_without_renderer_binding_is_still_reported() {
        // 元素 class 里没有 renderer：那个分支不会生效，基础声明就是真问题。
        let (_f, project) = page(
            "property-skyline-static",
            r#"<text class="name">x</text>"#,
            r#"
            .name { -webkit-line-clamp: 2; }
            .name.skyline { -webkit-line-clamp: unset; }
            "#,
        );
        assert_eq!(messages(&UnsupportedProperty, &project).len(), 1);
    }

    #[test]
    fn descendant_written_branch_does_not_exempt_the_base_declaration() {
        // `.name .skyline` 是后代关系，不是同元素的变体：`.name` 的声明没被覆盖。
        let (_f, project) = page(
            "property-skyline-descendant",
            r#"<text class="name {{renderer}}">x</text>"#,
            r#"
            .name { -webkit-line-clamp: 2; }
            .name .skyline { -webkit-line-clamp: unset; }
            "#,
        );
        assert_eq!(messages(&UnsupportedProperty, &project).len(), 1);
    }

    #[test]
    fn property_names_are_matched_exactly_not_by_substring() {
        let (_f, project) = page(
            "property-attribution",
            r#"<view class="a"/>"#,
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
