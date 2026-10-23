//! `display: flex` 在 Skyline 下默认 `flex-direction: column`，WebView 为 `row`。
//!
//! 同一个选择器下没有 `flex-direction` 时，迁移后布局方向会翻转，报 warning。
//!
//! 声明可以分散在多条同名规则里（`.a { display: flex }` 与 `.a { flex-direction: row }`
//! 分开写、或其中一条在 `@media` 里），所以按「整份样式表里同选择器出现过 flex-direction」
//! 判断，而不是只看单条规则。

use crate::project::Project;
use crate::{Diagnostic, Severity};

use crate::input::wxss::{WxssDoc, WxssRule, WxssSelector};
use crate::rules::{Rule, RuleMeta};

pub struct FlexDirection;

impl FlexDirection {
    pub const ID: &'static str = "skyline/flex-direction";
}

impl Rule for FlexDirection {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "display: flex 未声明 flex-direction，迁移后默认方向会从 row 变为 column",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        for sheet in &project.styles {
            // `defaultDisplayBlock: true` 时 Skyline 的 flex-direction 默认值就是 row，
            // 与 WebView 一致，这条警告不成立。
            if sheet.scopes.iter().all(|scope| project.renders_as_block(scope)) {
                continue;
            }
            let doc = &sheet.doc;
            for rule in &doc.rules {
                // 无选择器的块是预处理器的 mixin 函数体：它只是片段，`display: flex`
                // 和 `flex-direction` 可能分散在定义处与调用处，判不了也不该判。
                if rule.selectors.is_empty() {
                    continue;
                }
                // `flex-flow` 同时设置 flex-direction 与 flex-wrap，算已显式声明方向。
                if rule.declaration("flex-direction").is_some()
                    || rule.declaration("flex-flow").is_some()
                    || declares_flex_direction_elsewhere(doc, rule)
                {
                    continue;
                }
                // 同一条规则里 display 写多次时以后面的为准；最后一条不是 flex 就不报。
                let Some(display) = rule
                    .declarations
                    .iter()
                    .rev()
                    .find(|decl| decl.name.eq_ignore_ascii_case("display"))
                else {
                    continue;
                };
                if !is_flex_value(&display.value) {
                    continue;
                }
                let (line, column) = doc.position(display.offset);
                let (end_line, end_column) = doc.position(display.value_end());
                out.push(
                    Diagnostic::new(
                        Self::ID,
                        Severity::Warning,
                        "display: flex 未显式声明 flex-direction；Skyline 默认 column，WebView 默认 row，迁移后布局方向会翻转",
                        doc.path(),
                        line,
                        column,
                    )
                    .end(end_line, end_column)
                    .help("为这条规则补上 `flex-direction: row`，或按 column 重新设计布局"),
                );
            }
        }
        out
    }
}

/// 同一份样式表里是否有**同名选择器**的另一条规则声明了方向（`flex-direction` 或 `flex-flow`）。
///
/// 只看选择器原文，不解析 at-rule 上下文：`@media` 里声明的 `flex-direction` 未必
/// 在所有条件下生效，但漏报好过误报。
fn declares_flex_direction_elsewhere(doc: &WxssDoc, rule: &WxssRule) -> bool {
    doc.rules.iter().any(|other| {
        !std::ptr::eq(other, rule)
            && (other.declaration("flex-direction").is_some()
                || other.declaration("flex-flow").is_some())
            && other
                .selectors
                .iter()
                .any(|selector| shares_selector(selector, &rule.selectors))
    })
}

fn shares_selector(selector: &WxssSelector, selectors: &[WxssSelector]) -> bool {
    let key = selector.text.trim();
    selectors
        .iter()
        .any(|other| other.text.trim() == key)
}

fn is_flex_value(value: &str) -> bool {
    let lower = value.trim().to_ascii_lowercase();
    if lower.contains("var(") || lower.contains("env(") || lower.contains("{{") {
        return false;
    }
    matches!(lower.as_str(), "flex" | "inline-flex")
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;

    use super::super::testutil::{messages, single, Fixture};
    use super::FlexDirection;

    #[test]
    fn flex_direction_reports_flex_without_direction() {
        let (_f, project) = single(
            "flex-positive",
            r#"
            .a { display: flex; }
            .b { display: inline-flex; color: red; }
            @media (min-width: 10px) { .c { display: flex; } }
            .d { display: FLEX; }
            "#,
        );
        let diags = FlexDirection.check(&project);
        assert_eq!(diags.len(), 4);
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Warning));
        // 嵌套在 @media 里的规则同样要报。
        assert!(diags.iter().any(|d| d.message.contains("flex-direction")));
    }

    #[test]
    fn flex_direction_is_found_in_a_separate_rule_with_the_same_selector() {
        // 声明可以拆开写：`.a` 在两条规则里各写一半，不能只看单条。
        let (_f, project) = single(
            "flex-split",
            r#"
            .a { display: flex; }
            .a { flex-direction: row; }
            .b, .c { display: flex; }
            .c { flex-direction: column; }
            "#,
        );
        assert!(messages(&FlexDirection, &project).is_empty());
    }

    #[test]
    fn flex_flow_counts_as_direction() {
        let (_f, project) = single(
            "flex-flow",
            r#"
            .a { display: flex; flex-flow: row wrap; }
            .b { display: flex; }
            .b { flex-flow: column; }
            "#,
        );
        assert!(messages(&FlexDirection, &project).is_empty());
    }

    /// 同一条规则里 display 写两次时，后一条生效。
    #[test]
    fn the_last_display_declaration_wins() {
        let (_f, project) = single(
            "flex-last-display",
            r#"
            .a { display: flex; display: block; }
            .b { display: block; display: flex; }
            "#,
        );
        let diags = FlexDirection.check(&project);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert_eq!(diags[0].line, 3);
    }

    #[test]
    fn default_display_block_silences_the_warning() {
        // `defaultDisplayBlock: true` 时 Skyline 的 flex-direction 默认值就是 row。
        let f = Fixture::new("flex-block");
        f.write(
            "app.json",
            r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"defaultDisplayBlock":true}}}"#,
        )
        .write("pages/index/index.json", "{}")
        .write("pages/index/index.wxml", "<view/>")
        .write("pages/index/index.wxss", ".a { display: flex; }");
        let project = f.project();
        assert!(messages(&FlexDirection, &project).is_empty());
    }

    #[test]
    fn page_level_override_can_restore_the_column_default() {
        // app 级开着 block 布局，但这一页覆盖回 false：默认仍是 column，要报。
        let f = Fixture::new("flex-block-page");
        f.write(
            "app.json",
            r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"defaultDisplayBlock":true}}}"#,
        )
        .write(
            "pages/index/index.json",
            r#"{"rendererOptions":{"skyline":{"defaultDisplayBlock":false}}}"#,
        )
        .write("pages/index/index.wxml", "<view/>")
        .write("pages/index/index.wxss", ".a { display: flex; }");
        let project = f.project();
        assert_eq!(messages(&FlexDirection, &project).len(), 1);
    }

    #[test]
    fn flex_direction_ignores_declared_direction_block_and_vars() {
        let (_f, project) = single(
            "flex-negative",
            r#"
            .a { display: flex; flex-direction: column; }
            .b { display: block; }
            .c { flex-direction: row; }
            .d { display: var(--display); }
            .f { display: flex; flex-direction: var(--dir); }
            "#,
        );
        assert_eq!(messages(&FlexDirection, &project), Vec::<String>::new());
    }
}
