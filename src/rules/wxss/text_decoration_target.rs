//! `text-decoration*` 在 Skyline 下只对文本节点生效。
//!
//! 引擎证据（`skyline/style/style_builder.cc`）：装饰只在文字节点上应用——`TextSpan`
//! 分支 1298-1316 处理 line/color/style/thickness，`Input` 分支 1486-1494 处理输入框；
//! 而 `text_decoration_line_` 在 `ComputedStyle` 的 `non_inherited_data_`
//! （`style/computed_style.h:831`、`:1734`），**不会从父节点继承**。所以 WebView 上给
//! `<view>` 加下划线（浏览器会把装饰绘制到后代文本上）在 Skyline 下完全消失。
//!
//! 判定范围是「声明所在选择器命中的节点」——这里与 `text-overflow` 不同：装饰不继承，
//! 所以命中的容器里有 `<text>` 也不能豁免。命中节点是自定义组件、或属性带插值时静态
//! 判断不了，一律放过（宁可漏报）。

use std::collections::HashMap;

use crate::input::wxml::{self, WxmlElement};
use crate::project::Project;
use crate::rules::RuleMeta;
use crate::rules::wxml::is_builtin;
use crate::{Diagnostic, Severity};

use super::value_util::at_declaration;
use super::visit_matching_rules;

pub const ID: &str = "skyline/text-decoration-target";

/// 会应用装饰的属性。`text-decoration-thickness` 不在内——属性本身就不生效，由
/// `skyline/unsupported-property` 负责。
const PROPERTIES: [&str; 4] = [
    "text-decoration",
    "text-decoration-line",
    "text-decoration-style",
    "text-decoration-color",
];

pub struct TextDecorationTarget;

impl TextDecorationTarget {
    pub const ID: &'static str = ID;
}

impl crate::rules::Rule for TextDecorationTarget {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Warning,
            description: "text-decoration 在 Skyline 下只对 `<text>` / `<span>` / `<input>` / `<navigator>` 生效",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut docs: HashMap<String, Option<wxml::WxmlDoc>> = HashMap::new();

        visit_matching_rules(project, |sheet, rule, out| {
            let doc = &sheet.doc;
            for decl in &rule.declarations {
                let name = decl.name.to_ascii_lowercase();
                if !PROPERTIES.contains(&name.as_str()) {
                    continue;
                }
                // `text-decoration: none` 是「取消」，写在谁身上都无害。
                if decl.value.trim().eq_ignore_ascii_case("none") {
                    continue;
                }
                let offenders = super::broken_selectors(
                    project,
                    sheet,
                    &rule.selectors,
                    &mut docs,
                    |_, element| can_decorate(element),
                );
                if offenders.is_empty() {
                    continue;
                }
                let selectors = offenders.join("`, `");
                out.push(
                        at_declaration(
                            ID,
                            doc,
                            decl,
                            Severity::Warning,
                            format!(
                                "`{}` 在 Skyline 下只作用于 `<text>` / `<span>` / `<input>` / `<navigator>`；选择器 `{selectors}` 在某处命中的节点都不是文本节点，装饰不会显示",
                                decl.name
                            ),
                            "把文本包进 `<text>` 并在它上面写装饰；`<view>` 上的装饰在 Skyline 下不会绘制",
                        ),
                    );
            }
        })
    }
}

/// 装饰会应用到这个节点上吗。自定义组件内部结构不可见，按「可能」处理。
fn can_decorate(element: &WxmlElement) -> bool {
    if !is_builtin(&element.tag) {
        return true;
    }
    matches!(
        element.tag.as_str(),
        "text" | "span" | "input" | "navigator"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::Rule;
    use crate::rules::testutil::wxss::Fixture;

    fn check(name: &str, wxml: &str, wxss: &str) -> Vec<Diagnostic> {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", wxml)
            .write("pages/index/index.wxss", wxss);
        TextDecorationTarget.check(&fixture.project())
    }

    // ---- positive ----

    #[test]
    fn decoration_on_view_is_reported() {
        let diagnostics = check(
            "view",
            r#"<view class="link">详情</view>"#,
            ".link { text-decoration: underline; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].rule, ID);
        assert!(diagnostics[0].message.contains("text-decoration"));
    }

    #[test]
    fn decoration_on_container_with_text_descendant_is_still_reported() {
        // 装饰不继承：容器里的 <text> 拿不到这条声明。
        let diagnostics = check(
            "container",
            r#"<view class="link"><text>详情</text></view>"#,
            ".link { text-decoration-line: underline; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    }

    // ---- negative ----

    #[test]
    fn decoration_on_text_is_fine() {
        let diagnostics = check(
            "text",
            r#"<text class="link">详情</text>"#,
            ".link { text-decoration: underline; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn decoration_on_navigator_is_fine() {
        let diagnostics = check(
            "navigator",
            r#"<navigator class="link">详情</navigator>"#,
            ".link { text-decoration: underline; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn decoration_on_input_is_fine() {
        let diagnostics = check(
            "input",
            r#"<input class="field"/>"#,
            ".field { text-decoration-color: red; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn none_is_fine() {
        let diagnostics = check(
            "none",
            r#"<view class="link">详情</view>"#,
            ".link { text-decoration: none; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn dynamic_class_with_static_token_is_still_matched() {
        // `link` 是原文里写死的 token，运行时一定在 class 列表里；元素是 view，装饰不会绘制。
        let diagnostics = check(
            "dynamic",
            r#"<view class="link {{active}}">详情</view>"#,
            ".link { text-decoration: underline; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].rule, "skyline/text-decoration-target");
    }

    #[test]
    fn fully_dynamic_class_is_fine() {
        let diagnostics = check(
            "dynamic",
            r#"<view class="{{cls}}">详情</view>"#,
            ".link { text-decoration: underline; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn complex_selector_is_fine() {
        // 组合符 / 伪类的命中范围静态判不了，放过。
        let diagnostics = check(
            "complex",
            r#"<view class="link">详情</view>"#,
            ".link:hover { text-decoration: underline; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn custom_component_is_fine() {
        let fixture = Fixture::new("component");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                r#"{"usingComponents":{"c":"/components/c/c"}}"#,
            )
            .write("pages/index/index.wxml", r#"<c class="link"/>"#)
            .write(
                "pages/index/index.wxss",
                ".link { text-decoration: underline; }",
            )
            .write("components/c/c.json", "{}")
            .write("components/c/c.wxml", "<text>x</text>")
            .write("components/c/c.wxss", "");
        let diagnostics = TextDecorationTarget.check(&fixture.project());
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }
}
