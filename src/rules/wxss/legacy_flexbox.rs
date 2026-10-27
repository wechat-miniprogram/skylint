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

use super::value_util::at_declaration;
use super::visit_rules;

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
        visit_rules(project, |sheet, rule, out| {
            let doc = &sheet.doc;
            let Some(legacy) = legacy_block(rule) else {
                return;
            };
            let names = legacy
                .iter()
                .map(|(label, _)| format!("`{label}`"))
                .collect::<Vec<_>>()
                .join("、");
            out.push(at_declaration(
                Self::ID,
                doc,
                legacy[0].1,
                Severity::Error,
                format!(
                    "这块用了老式 `-webkit-box` 弹性盒：{} 处声明（{names}）在 Skyline 下全部被丢弃，整块布局退回默认 display",
                    legacy.len()
                ),
                "改用 flex 布局（`display: flex` 配 `flex-direction` / `align-items` / `justify-content` / `flex: 1`）；多行省略另用 `<text max-lines>`",
            ));
        })
    }
}

/// 这条规则块依赖老式弹性盒时，返回块内的老式声明（按出现顺序）；否则 `None`。
///
/// `display` 只看最后一条：后面的 `display: flex` 会把前面的 `-webkit-box` 覆盖掉。
fn legacy_block(
    rule: &crate::input::wxss::WxssRule,
) -> Option<Vec<(String, &crate::input::wxss::WxssDeclaration)>> {
    let webkit_display = rule
        .declarations
        .iter()
        .rev()
        .find(|decl| decl.name.eq_ignore_ascii_case("display"))
        .filter(|decl| decl.value.trim().eq_ignore_ascii_case("-webkit-box"))?;
    let mut legacy: Vec<(String, &crate::input::wxss::WxssDeclaration)> = rule
        .declarations
        .iter()
        .filter_map(|decl| {
            let name = decl.name.to_ascii_lowercase();
            if LEGACY_PROPERTIES.contains(&name.as_str()) {
                return Some((name, decl));
            }
            // 值可能是 `{{}}` 之外的其他写法，这里只认生效的那条 display。
            std::ptr::eq(decl, webkit_display).then(|| ("display: -webkit-box".to_string(), decl))
        })
        .collect();
    legacy.sort_by_key(|(_, decl)| decl.offset);
    Some(legacy)
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testutil::wxss::{messages, single};

    use super::LegacyFlexbox;

    #[test]
    fn one_rule_block_reports_once_with_every_declaration() {
        let (_f, project) = single(
            "legacy-one-block",
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
        // 行内省略仍归 unsupported-property，不在这条规则里重复。
        assert!(!message.contains("line-clamp"), "{message}");
    }

    #[test]
    fn later_supported_display_wins_over_webkit_box() {
        // autoprefixer 的渐进增强栈：最后一条 display 是 flex，这块不依赖老式弹性盒。
        let (_f, project) = single(
            "legacy-prefixed-stack",
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
        let (_f, project) = single(
            "legacy-child-properties",
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
        let (_f, project) = single(
            "legacy-spelling",
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
    fn line_clamp_alone_is_left_to_unsupported_property() {
        let (_f, project) = single(
            "legacy-only-clamp",
            r#"
            .a { -webkit-line-clamp: 2; }
            "#,
        );
        assert_eq!(messages(&LegacyFlexbox, &project), Vec::<String>::new());
    }
}
