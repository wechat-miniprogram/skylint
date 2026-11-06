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

use super::value_util::at_declaration;
use super::visit_matching_rules;

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
        let per_block = visit_matching_rules(project, |sheet, rule, out| {
            // `defaultDisplayBlock: true` 时 Skyline 的 flex-direction 默认值就是 row，
            // 与 WebView 一致，这条警告不成立。
            if sheet
                .scopes
                .iter()
                .all(|scope| project.renders_as_block(scope))
            {
                return;
            }
            let doc = &sheet.doc;
            // 无选择器的块是预处理器的 mixin 函数体：它只是片段，`display: flex`
            // 和 `flex-direction` 可能分散在定义处与调用处，判不了也不该判。
            if rule.selectors.is_empty() {
                return;
            }
            if declares_direction(rule) || declares_flex_direction_elsewhere(doc, rule) {
                return;
            }
            // 同一条规则里 display 写多次时以后面的为准；最后一条不是 flex 就不报。
            let Some(display) = rule
                .declarations
                .iter()
                .rev()
                .find(|decl| decl.name.eq_ignore_ascii_case("display"))
            else {
                return;
            };
            if !crate::rules::wxml::is_flex_value(&display.value) {
                return;
            }
            // 未开 `defaultDisplayBlock` 时这类告警通常是一整片（本工程 97 条），
            // 全部由同一处配置引起，help 先给一次性解法，再说逐条改法。
            let help = if project.app_display_block() {
                "为这条规则补上 `flex-direction: row`，或按 column 重新设计布局"
            } else {
                "整工程对齐：在 `rendererOptions.skyline` 开启 `defaultDisplayBlock: true`，Skyline 的 flex-direction 默认值就变回 row，同类告警一次性消失；保持关闭则需为这条规则补 `flex-direction: row`，或按 column 重新设计布局"
            };
            out.push(at_declaration(
                Self::ID,
                doc,
                display,
                Severity::Warning,
                "display: flex 未显式声明 flex-direction；Skyline 默认 column，WebView 默认 row，迁移后布局方向会翻转",
                help,
            ));
        });
        // 工程级没开开关时，每一处 `display: flex` 都会翻转，但解法只有一个：要么开
        // 开关、要么逐处补 `flex-direction`。逐条报只在按声明检视时有用，迁移报告里
        // 它只是一片同因噪声（参考工程最多 96 条），所以收成一条带条数的提示。
        // 页面级单独关开关的情况不聚合：那时受影响的是个别页面，逐条报才有意义。
        if project.app_display_block() || per_block.is_empty() {
            return per_block;
        }
        vec![aggregated(project, per_block.len())]
    }
}

/// 一处 `display: flex` 都没声明方向时的工程级提示：条数只作影响面说明，解法只有
/// 配置里的一个开关。
///
/// 插件视图没有 app.json 可改（`rendererOptions` 归宿主 app），所以那一侧只描述后果，
/// 并把「插件侧能自己做的」写在 help 里。
fn aggregated(project: &Project, blocks: usize) -> Diagnostic {
    let (line, column) = crate::input::json::JsonPositions::new(&project.app_json_source)
        .key("rendererOptions")
        .unwrap_or((1, 1));
    let message = if project.plugin_view {
        format!(
            "宿主 app 未开启 defaultDisplayBlock：{blocks} 处 `display: flex` 没有显式声明 flex-direction，Skyline 默认 column、WebView 默认 row，宿主的这个开关决定它们会不会翻转（插件自身无法设置 `rendererOptions`）"
        )
    } else {
        format!(
            "app 级未开启 defaultDisplayBlock：{blocks} 处 `display: flex` 没有显式声明 flex-direction，Skyline 默认 column、WebView 默认 row，迁移后这些容器的布局方向都会翻转"
        )
    };
    let help = if project.plugin_view {
        "插件侧要给准话就为每处显式写 `flex-direction: row`（或按 column 重写布局）；想靠开关解决则需宿主 app 在 `rendererOptions.skyline` 开启 `defaultDisplayBlock: true`"
    } else {
        "在 `rendererOptions.skyline` 开启 `defaultDisplayBlock: true`，Skyline 的 flex-direction 默认值就变回 row，这一整类差异一次性消失；保持关闭则需为每处补 `flex-direction: row`，或按 column 重新设计布局"
    };
    Diagnostic::new(
        FlexDirection::ID,
        Severity::Warning,
        message,
        project.app_json_path.clone(),
        line,
        column,
    )
    .help(help)
}

/// 同一份样式表里是否有**同名选择器**的另一条规则声明了方向（`flex-direction` 或 `flex-flow`）。
///
/// 只看选择器原文，不解析 at-rule 上下文：`@media` 里声明的 `flex-direction` 未必
/// 在所有条件下生效，但漏报好过误报。
fn declares_flex_direction_elsewhere(doc: &WxssDoc, rule: &WxssRule) -> bool {
    // 逐选择器判定：`.a .flex, .a .wrap { display:flex }` 里只有 `.a .flex` 在别处
    // 声明了方向时，`.a .wrap` 仍然没有方向。
    rule.selectors.iter().all(|selector| {
        doc.rules.iter().any(|other| {
            !std::ptr::eq(other, rule)
                && declares_direction(other)
                && shares_selector(selector, &other.selectors)
        })
    })
}

/// 这条规则是否显式声明了**具体**方向。
///
/// `flex-direction: initial` / `unset` / `revert` / `inherit` 不算：运行时的
/// `ApplyInitial` 在未开 `defaultDisplayBlock` 时给出 column（`flex_direction.cc:39-46`），
/// 与 WebView 的 row 仍然相反，写这些关键字并没有消除差异。
/// `flex-flow` 一律算已声明：按 CSS 语义 `flex-flow: wrap` 的方向缺省值是 row，
/// fp 的 shorthand 定义里 `flex_direction` 的缺省值也是 `Row`
/// （`float-pigment-css/src/property.rs:48,380`），静态判起来与 WebView 一致。
fn declares_direction(rule: &WxssRule) -> bool {
    if let Some(decl) = rule.declaration("flex-direction") {
        let value = decl.value.trim().to_ascii_lowercase();
        if super::value_util::text_is_dynamic(&value) || is_direction(&value) {
            return true;
        }
    }
    rule.declaration("flex-flow").is_some()
}

fn is_direction(value: &str) -> bool {
    matches!(value, "row" | "row-reverse" | "column" | "column-reverse")
}

fn shares_selector(selector: &WxssSelector, selectors: &[WxssSelector]) -> bool {
    let key = selector.text.trim();
    selectors.iter().any(|other| other.text.trim() == key)
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;

    use super::FlexDirection;
    use crate::rules::testutil::wxss::{Fixture, messages, page, page_opt_out};

    #[test]
    fn flex_direction_reports_flex_without_direction() {
        let (_f, project) = page_opt_out(
            "flex-positive",
            r#"<view class="a"/><view class="b"/><view class="c"/><view class="d"/>"#,
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

    /// 旧实现把选择器里的标识符统一小写，`.Btn` 命不中 `class="Btn"`：整块样式被可达性闸门
    /// 当作「没人用」跳过，块内的真实诊断全部消失。
    #[test]
    fn capitalized_class_selectors_are_reachable() {
        let (_f, project) = page_opt_out(
            "flex-capitalized",
            r#"<view class="Btn"/>"#,
            r#".Btn { display: flex; }"#,
        );
        assert_eq!(messages(&FlexDirection, &project).len(), 1);
    }

    #[test]
    fn flex_direction_is_found_in_a_separate_rule_with_the_same_selector() {
        // 声明可以拆开写：`.a` 在两条规则里各写一半，不能只看单条。
        let (_f, project) = page(
            "flex-split",
            r#"<view class="a"/><view class="b"/>"#,
            r#"
            .a { display: flex; }
            .a { flex-direction: row; }
            .b { display: flex; }
            .b { flex-direction: column; }
            "#,
        );
        assert!(messages(&FlexDirection, &project).is_empty());
    }

    /// 并列选择器逐项判：`.b` 没有方向，不能被 `.c` 的声明一起豁免。
    #[test]
    fn selector_list_is_exempted_one_by_one() {
        let (_f, project) = page(
            "flex-list",
            r#"<view class="b"/><view class="c"/>"#,
            r#"
            .b, .c { display: flex; }
            .c { flex-direction: column; }
            "#,
        );
        let messages = messages(&FlexDirection, &project);
        assert_eq!(messages.len(), 1, "{messages:?}");
        // 消息是工程级聚合（「N 处」），这里只确认 `.b` 那一处被计入了。
        assert!(messages[0].contains("1 处"), "{messages:?}");
    }

    #[test]
    fn flex_flow_counts_as_direction() {
        let (_f, project) = page(
            "flex-flow",
            r#"<view class="a"/><view class="b"/>"#,
            r#"
            .a { display: flex; flex-flow: row wrap; }
            .b { display: flex; }
            .b { flex-flow: column; }
            "#,
        );
        assert!(messages(&FlexDirection, &project).is_empty());
    }

    /// `initial` / `unset` / `inherit` 没有消除方向差异：未开 defaultDisplayBlock 时
    /// Skyline 的初值是 column，WebView 是 row。
    #[test]
    fn global_keywords_do_not_count_as_declaring_a_direction() {
        let (_f, project) = page_opt_out(
            "flex-global-keywords",
            r#"<view class="a"/><view class="b"/><view class="c"/><view class="d"/><view class="e"/>"#,
            r#"
            .a { display: flex; flex-direction: initial; }
            .b { display: flex; flex-direction: unset; }
            .c { display: flex; flex-direction: inherit; }
            .d { display: flex; flex-direction: var(--dir); }
            .e { display: flex; flex-flow: wrap; }
            "#,
        );
        let diags = FlexDirection.check(&project);
        // .d / .e 静态判不了，按已声明处理（宁可漏报）。
        assert_eq!(diags.len(), 3, "{diags:#?}");
    }

    /// 同一条规则里 display 写两次时，后一条生效。
    #[test]
    fn the_last_display_declaration_wins() {
        let (_f, project) = page_opt_out(
            "flex-last-display",
            r#"<view class="a"/><view class="b"/>"#,
            r#"
            .a { display: flex; display: block; }
            .b { display: block; display: flex; }
            "#,
        );
        let diags = FlexDirection.check(&project);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert_eq!(diags[0].line, 3);
    }

    /// app 级没开开关时，逐条 warning 收成一条带条数的提示：解法只有那一个开关。
    #[test]
    fn missing_app_switch_aggregates_the_warnings() {
        let (_f, project) = page(
            "flex-aggregate",
            r#"<view class="a"/><view class="b"/><view class="c"/>"#,
            r#"
            .a { display: flex; }
            .b { display: flex; }
            .c { display: flex; }
            "#,
        );
        let diags = FlexDirection.check(&project);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert_eq!(diags[0].path, "app.json");
        assert!(diags[0].message.contains("3 处"), "{:?}", diags[0].message);
        assert!(
            diags[0]
                .help
                .as_deref()
                .is_some_and(|help| help.contains("defaultDisplayBlock"))
        );
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
        .write("pages/index/index.wxml", r#"<view class="a"/>"#)
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
        .write("pages/index/index.wxml", r#"<view class="a"/>"#)
        .write("pages/index/index.wxss", ".a { display: flex; }");
        let project = f.project();
        assert_eq!(messages(&FlexDirection, &project).len(), 1);
    }

    #[test]
    fn flex_direction_ignores_declared_direction_block_and_vars() {
        let (_f, project) = page(
            "flex-negative",
            r#"<view class="a"/><view class="b"/><view class="c"/><view class="d"/><view class="f"/>"#,
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
