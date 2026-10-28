//! `skyline/legacy-flexbox`：老式 `-webkit-box` 弹性盒按**规则块**合并报告。
//!
//! 这一族属性在引擎里全都不存在：`css/css_property_names.h` 的 194 个属性名里没有任何
//! `-webkit-*`（声明查不到 id、解析阶段整条丢弃），`display` 的取值也只认
//! none/block/inline/inline-block/flex/grid/flow-root/inline-flex
//! （`float-pigment-css-0.7.0/src/property.rs:193-200`），`-webkit-box` 解析失败同样丢弃。
//! WebView 认这套老式弹性盒（weui 用它做垂直居中与横排），所以迁移后整块布局会退回
//! 默认 display，是真实的可见差异。
//!
//! 按块合并、而不是逐条声明报：一块布局通常是 3～5 条声明一起出现（businessview 扫描
//! 范围内 `-webkit-box-flex` 171 处、`-webkit-box-align` 147 处、`-webkit-box-pack` 50 处），
//! 逐条报会把一条「这块布局要重写」淹没成几百条 error。`-webkit-line-clamp` 不在这里：
//! 它自己有一条可执行的迁移建议（`<text max-lines>`），仍归 `skyline/unsupported-property`。
//!
//! 判据只看**这条规则块最终生效的 `display`**（同名声明后写覆盖前写）：autoprefixer 产物是
//! `display:-webkit-box; display:-webkit-flex; display:flex` 的渐进增强栈，最后一条是 flex，
//! 这块在现代渲染器上不依赖老式弹性盒——businessview 里 139 个含老式声明的块有 128 个是这种
//! 形态。老式属性写在 `display` 已生效为 flex 的块里同样是死声明，但静态判不出它是不是
//! 无害的兜底（`-webkit-box-flex` 与 `flex:1` 并存、或父级才是老式容器），按「宁可漏报」跳过。

use crate::project::Project;
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

use std::collections::HashMap;

use super::value_util::at_declaration;
use super::visit_matching_rules;
use crate::input::wxml::WxmlDoc;

pub struct LegacyFlexbox;

/// 引擎里不存在的属性名（`-webkit-line-clamp` 除外，见模块注释）。
const LEGACY_PROPERTIES: &[&str] = &[
    "-webkit-box-orient",
    "-webkit-box-direction",
    "-webkit-box-align",
    "-webkit-box-pack",
    "-webkit-box-flex",
    "-webkit-box-lines",
];

/// fp 的 `display` 能解析的取值（`float-pigment-css/src/property.rs:203-212`）。
/// 其余取值（`box` / `-webkit-box` / `-webkit-flex`）整条声明被丢弃。
const PARSEABLE_DISPLAY: &[&str] = &[
    "none",
    "block",
    "inline",
    "inline-block",
    "flex",
    "inline-flex",
    "grid",
    "flow-root",
];

impl LegacyFlexbox {
    pub const ID: &'static str = "skyline/legacy-flexbox";
}

impl Rule for LegacyFlexbox {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Error,
            description: "规则块用了老式 `-webkit-box` 弹性盒，Skyline 不认识这些声明，整块布局会退回默认 display",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut docs: HashMap<String, Option<WxmlDoc>> = HashMap::new();
        visit_matching_rules(project, |sheet, rule, out| {
            let doc = &sheet.doc;
            // `.x.skyline { display: unset }` 这种渲染器分支已经把这块关掉了：
            // 基础块只服务 WebView，在 Skyline 下本来就不生效。
            if super::overridden_in_renderer_branch(
                project,
                sheet,
                rule,
                "display",
                self.target(),
                &mut docs,
            ) {
                return;
            }
            let Some(legacy) = legacy_block(rule) else {
                return;
            };
            let clamp = rule
                .declaration("-webkit-line-clamp")
                .map(|decl| &decl.value);
            // 命中节点全是带同一行数的 `<text max-lines>`：截断由属性完成，
            // 这套堆叠只是 WebView 那一份，两端结果一致。
            if let Some(lines) = clamp
                && super::truncation_by_max_lines(project, sheet, &rule.selectors, &mut docs, lines)
            {
                return;
            }
            let names = legacy
                .iter()
                .map(|(label, _)| format!("`{label}`"))
                .collect::<Vec<_>>()
                .join("、");
            let (message, help) = if clamp.is_some() {
                (
                    format!(
                        "这块的多行省略用了老式 `-webkit-box`：{} 处声明（{names}）在 Skyline 下全部被丢弃，`-webkit-line-clamp` 不起作用，文本会完整铺开",
                        legacy.len()
                    ),
                    "Skyline 用 `<text max-lines=\"N\" overflow=\"ellipsis\">` 截断；要两端都兼容就另加 WebView 兜底（`-webkit-line-clamp` 保留在独立的 `.webview` 分支里）",
                )
            } else {
                (
                    format!(
                        "这块用了老式 `-webkit-box` 弹性盒：{} 处声明（{names}）在 Skyline 下全部被丢弃，整块布局退回默认 display",
                        legacy.len()
                    ),
                    "改用 flex 布局（`display: flex` 配 `flex-direction` / `align-items` / `justify-content` / `flex: 1`）",
                )
            };
            out.push(at_declaration(
                Self::ID,
                doc,
                legacy[0].1,
                Severity::Error,
                message,
                help,
            ));
        })
    }
}

/// 这条规则块依赖老式弹性盒或私有 `display` 值时，返回块内应报的声明（按出现顺序）；
/// 否则 `None`。
///
/// `display` 按 **WebView 实际生效的那一条**（最后一条合法的）看，再对比 Skyline
/// 能解析的最后一条：
/// - 生效值是 `-webkit-box`：WebView 走老式弹性盒，配套的 `-webkit-box-*` 也生效，
///   而 Skyline 全部丢弃 → 报。
/// - 生效值是 `-webkit-flex`（= flex）：WebView 是新式 flex；若块里还有 Skyline 认得的
///   `display: flex`，两端都是 flex，老式声明在两端都是空转 → 不报。只有整块没有任何
///   可解析 `display` 时，Skyline 才退回默认 → 报。
/// - 生效值是普通值（`block` / `none` …）：两端一致 → 不报。
///
/// `-ms-flexbox` 在 Chromium 里是非法值，当作没写。
fn legacy_block(
    rule: &crate::input::wxss::WxssRule,
) -> Option<Vec<(String, &crate::input::wxss::WxssDeclaration)>> {
    let display =
        |decl: &&crate::input::wxss::WxssDeclaration| decl.name.eq_ignore_ascii_case("display");
    let webview = rule.declarations.iter().rev().find(|decl| {
        if !display(decl) {
            return false;
        }
        let value = decl.value.trim();
        PARSEABLE_DISPLAY.contains(&value) || is_webkit_flex_display(value)
    })?;
    let value = webview.value.trim();
    let losing = if value.eq_ignore_ascii_case("-webkit-box") {
        true
    } else if value.eq_ignore_ascii_case("-webkit-flex")
        || matches!(value.to_ascii_lowercase().as_str(), "flex" | "inline-flex")
    {
        !rule
            .declarations
            .iter()
            .filter(display)
            .any(|decl| PARSEABLE_DISPLAY.contains(&decl.value.trim()))
    } else {
        false
    };
    if !losing {
        return None;
    }
    let mut legacy: Vec<(String, &crate::input::wxss::WxssDeclaration)> = rule
        .declarations
        .iter()
        .filter_map(|decl| {
            let name = decl.name.to_ascii_lowercase();
            if LEGACY_PROPERTIES.contains(&name.as_str()) {
                return Some((name, decl));
            }
            std::ptr::eq(decl, webview).then(|| (format!("display: {value}"), decl))
        })
        .collect();
    legacy.sort_by_key(|(_, decl)| decl.offset);
    Some(legacy)
}

fn is_webkit_flex_display(value: &str) -> bool {
    value.eq_ignore_ascii_case("-webkit-box") || value.eq_ignore_ascii_case("-webkit-flex")
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testutil::wxss::{messages, page};

    use super::LegacyFlexbox;

    #[test]
    fn one_rule_block_reports_once_with_every_declaration() {
        let (_f, project) = page(
            "legacy-one-block",
            r#"<view class="a"/>"#,
            r#"
            .a {
              display: -webkit-box;
              -webkit-box-orient: vertical;
              -webkit-box-flex: 1;
              -webkit-line-clamp: 2;
            }
            "#,
        );
        let diags = LegacyFlexbox.check(&project);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        let message = &diags[0].message;
        assert!(message.contains("3 处声明"), "{message}");
        assert!(message.contains("`display: -webkit-box`"), "{message}");
        assert!(message.contains("`-webkit-box-orient`"), "{message}");
        assert!(message.contains("`-webkit-box-flex`"), "{message}");
        // 同时有 `-webkit-line-clamp` 时讲清真实后果：多行省略失效，而不是整块布局坏掉。
        assert!(
            message.contains("`-webkit-line-clamp` 不起作用"),
            "{message}"
        );
    }

    #[test]
    fn layout_block_without_clamp_gets_layout_message() {
        let (_f, project) = page(
            "legacy-layout-only",
            r#"<view class="a"/>"#,
            r#"
            .a {
              display: -webkit-box;
              -webkit-box-orient: vertical;
              -webkit-box-pack: center;
            }
            "#,
        );
        let diags = LegacyFlexbox.check(&project);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert!(
            diags[0].message.contains("整块布局退回默认 display"),
            "{}",
            diags[0].message
        );
    }

    #[test]
    fn autoprefixer_stack_ending_in_webkit_flex_is_not_reported() {
        // `display: flex` 写在最前面、`-webkit-flex` 写在最后的写法：WebView 生效的是
        // 最后一条合法的 `-webkit-flex`（= flex），Skyline 保留前面可解析的 `flex`，
        // 两端都是 flex。
        let (_f, project) = page(
            "legacy-stack-webkit-flex-last",
            r#"<view class="a"/>"#,
            r#"
            .a {
              display: flex;
              display: -ms-flexbox;
              display: -webkit-box;
              display: -webkit-flex;
              -webkit-box-align: center;
            }
            "#,
        );
        assert_eq!(messages(&LegacyFlexbox, &project), Vec::<String>::new());
    }

    #[test]
    fn webkit_flex_without_any_parseable_display_is_reported() {
        // 整块没有 Skyline 认得的 `display`：WebView 是 flex，Skyline 退回默认。
        let (_f, project) = page(
            "legacy-webkit-flex-only",
            r#"<view class="a"/>"#,
            r#"
            .a { display: -webkit-flex; -webkit-box-align: center; }
            "#,
        );
        let diags = LegacyFlexbox.check(&project);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert!(
            diags[0].message.contains("`display: -webkit-flex`"),
            "{}",
            diags[0].message
        );
    }

    #[test]
    fn later_supported_display_wins_over_webkit_box() {
        // autoprefixer 的渐进增强栈：最后一条 display 是 flex，这块不依赖老式弹性盒。
        let (_f, project) = page(
            "legacy-prefixed-stack",
            r#"<view class="a"/>"#,
            r#"
            .a {
              display: -webkit-box;
              display: -webkit-flex;
              display: flex;
              -webkit-box-orient: vertical;
            }
            "#,
        );
        assert_eq!(messages(&LegacyFlexbox, &project), Vec::<String>::new());
    }

    #[test]
    fn legacy_properties_without_a_webkit_display_are_skipped() {
        // 老式属性在 display 已生效为 flex 的块里是死声明，但可能是无害兜底，
        // 也可能父级才是老式容器——静态判不了，按宁可漏报跳过。
        let (_f, project) = page(
            "legacy-child-properties",
            r#"<view class="a"/><view class="b"/>"#,
            r#"
            .a { -webkit-box-align: center; -webkit-box-pack: center; }
            .b { display: flex; -webkit-box-flex: 1; }
            "#,
        );
        assert_eq!(messages(&LegacyFlexbox, &project), Vec::<String>::new());
    }

    #[test]
    fn value_and_name_spelling_is_normalized() {
        // autoprefixer 产物与手写混用的常见形态：大写属性名、`!important`。
        let (_f, project) = page(
            "legacy-spelling",
            r#"<view class="a"/>"#,
            r#"
            .a { display: -webkit-box !important; -WEBKIT-BOX-ORIENT: vertical; }
            "#,
        );
        let diags = LegacyFlexbox.check(&project);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert!(
            diags[0].message.contains("2 处声明"),
            "{}",
            diags[0].message
        );
    }

    #[test]
    fn clamp_stack_on_text_with_same_max_lines_is_not_reported() {
        // 两端各写一份：WebView 用 clamp 堆叠，Skyline 用 `max-lines`，结果一致。
        let (_f, project) = page(
            "legacy-text-max-lines",
            r#"<text class="intro" max-lines="2" overflow="ellipsis">x</text>"#,
            r#"
            .intro {
              display: -webkit-box;
              -webkit-box-orient: vertical;
              -webkit-line-clamp: 2;
              overflow: hidden;
            }
            "#,
        );
        assert_eq!(messages(&LegacyFlexbox, &project), Vec::<String>::new());
    }

    #[test]
    fn clamp_stack_with_different_max_lines_is_still_reported() {
        // 行数对不上：Skyline 会截三行，WebView 截两行，是真实差异。
        let (_f, project) = page(
            "legacy-text-max-lines-mismatch",
            r#"<text class="intro" max-lines="3" overflow="ellipsis">x</text>"#,
            r#"
            .intro {
              display: -webkit-box;
              -webkit-box-orient: vertical;
              -webkit-line-clamp: 2;
            }
            "#,
        );
        assert_eq!(messages(&LegacyFlexbox, &project).len(), 1);
    }

    #[test]
    fn skyline_branch_disables_the_legacy_block() {
        // 迁移工程的常见写法：基础块服务 WebView，`.skyline` 分支把 display 关掉，
        // 截断交给 `<text max-lines>`。基础块在 Skyline 下本来就不生效。
        let (_f, project) = page(
            "legacy-skyline-branch",
            r#"<text class="name {{renderer}}">x</text>"#,
            r#"
            .name {
              display: -webkit-box;
              -webkit-box-orient: vertical;
              -webkit-line-clamp: 2;
            }
            .name.skyline {
              display: unset;
              -webkit-line-clamp: unset;
            }
            "#,
        );
        assert_eq!(messages(&LegacyFlexbox, &project), Vec::<String>::new());
    }

    #[test]
    fn skyline_branch_without_renderer_binding_is_still_reported() {
        // 元素 class 是静态的：`.skyline` 分支永远不会生效，基础块就是真问题。
        let (_f, project) = page(
            "legacy-skyline-static",
            r#"<text class="name">x</text>"#,
            r#"
            .name { display: -webkit-box; -webkit-box-orient: vertical; }
            .name.skyline { display: unset; }
            "#,
        );
        assert_eq!(messages(&LegacyFlexbox, &project).len(), 1);
    }

    #[test]
    fn other_class_branch_does_not_disable_the_block() {
        let (_f, project) = page(
            "legacy-other-branch",
            r#"<text class="name {{renderer}}">x</text>"#,
            r#"
            .name { display: -webkit-box; -webkit-box-orient: vertical; }
            .other.skyline { display: unset; }
            "#,
        );
        assert_eq!(messages(&LegacyFlexbox, &project).len(), 1);
    }

    #[test]
    fn line_clamp_alone_is_left_to_unsupported_property() {
        let (_f, project) = page(
            "legacy-only-clamp",
            r#"<view class="a"/>"#,
            r#"
            .a { -webkit-line-clamp: 2; }
            "#,
        );
        assert_eq!(messages(&LegacyFlexbox, &project), Vec::<String>::new());
    }
}
