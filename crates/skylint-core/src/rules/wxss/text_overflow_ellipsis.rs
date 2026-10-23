//! `text-overflow: ellipsis` 在 Skyline 下只对 `<text>` 生效。
//!
//! 知识库原文：「`text-overflow: ellipsis` 在 Skyline 下**仅在 `<text>` 组件上生效**，
//! 在 `<view>` 等组件上不生效」。WebView 上 `overflow: hidden` + `white-space: nowrap`
//! + `text-overflow: ellipsis` 对任意块级元素都成立，迁移后静默失效。
//!
//! 判定范围是「声明所在选择器命中的节点 + 其后代」：`text-overflow` 可继承，所以
//! `.list`（`<view>`）下面只要有 `<text>` 就不算失效。命中节点是自定义组件、或属性带
//! 插值时静态判断不了，一律放过（宁可漏报）。

use std::collections::HashMap;

use crate::input::wxml::{self, WxmlElement, WxmlNode};
use crate::project::{Project, Scope};
use crate::rules::RuleMeta;
use crate::rules::wxml::is_builtin;
use crate::{Diagnostic, Severity};

use super::selector::{Match, SimpleSelector};

pub const ID: &str = "skyline/text-overflow-ellipsis";

pub struct TextOverflowEllipsis;

impl TextOverflowEllipsis {
    pub const ID: &'static str = ID;
}

impl crate::rules::Rule for TextOverflowEllipsis {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Warning,
            description: "text-overflow: ellipsis 在 Skyline 下只对 <text> 生效",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        let mut docs: HashMap<&str, Option<wxml::WxmlDoc>> = HashMap::new();

        for sheet in &project.styles {
            let doc = &sheet.doc;
            for rule in &doc.rules {
                let Some(decl) = rule.declaration("text-overflow") else {
                    continue;
                };
                if !decl.value.eq_ignore_ascii_case("ellipsis") {
                    continue;
                }
                // 逗号分隔的选择器各自独立：`.a, .b` 里只要有一条命中的节点承载不了
                // ellipsis，这条声明对那批节点就是失效的。
                let mut offenders: Vec<&str> = Vec::new();
                for selector in &rule.selectors {
                    let Some(simple) = SimpleSelector::parse(selector) else {
                        continue;
                    };
                    let (mut matched, mut hostable) = (false, false);
                    for scope in &sheet.scopes {
                        for wxml_path in scope_wxml(project, scope) {
                            let Some(wxml_doc) = docs
                                .entry(wxml_path)
                                .or_insert_with(|| super::super::wxml::load_doc(project, wxml_path))
                                .as_ref()
                            else {
                                continue;
                            };
                            for element in wxml_doc.elements() {
                                match simple.matches(element) {
                                    // 判不了就当命中、也当能承载：不漏报变误报
                                    Match::Unknown => (matched, hostable) = (true, true),
                                    Match::Yes => {
                                        matched = true;
                                        hostable |= can_host_text(element);
                                    }
                                    Match::No => {}
                                }
                            }
                        }
                    }
                    if matched && !hostable {
                        offenders.push(selector.text.trim());
                    }
                }
                if offenders.is_empty() {
                    continue;
                }
                let selectors = offenders.join("`, `");
                let (line, column) = doc.position(decl.offset);
                out.push(
                    Diagnostic::new(
                        ID,
                        Severity::Warning,
                        format!("`text-overflow: ellipsis` 在 Skyline 下只作用于 `<text>`；选择器 `{selectors}` 命中的节点及其后代都没有 `<text>`，声明不会生效"),
                        doc.path(),
                        line,
                        column,
                    )
                    .help("用 `<text>` 包裹文本（Skyline 下可配合 `overflow` / `max-lines` 截断），或删掉该声明"),
                );
            }
        }
        out
    }
}

/// 该声明的作用域里需要一起看的 WXML。
fn scope_wxml<'a>(project: &'a Project, scope: &Scope) -> Vec<&'a str> {
    match scope {
        Scope::Global => project.global_wxml().collect(),
        // 样式图不会产生 Loose。
        Scope::Loose(_) => Vec::new(),
        _ => project.scope_wxml(scope).into_iter().collect(),
    }
}

/// 这个节点（或它的后代）上能否有 `<text>` 承载 ellipsis。
fn can_host_text(element: &WxmlElement) -> bool {
    if element.tag == "text" || !is_builtin(&element.tag) {
        return true;
    }
    element
        .children
        .iter()
        .any(|child| matches!(child, WxmlNode::Element(el) if can_host_text(el)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::Rule;
    use crate::rules::wxss::testutil::Fixture;

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
        let project = fixture.project();
        TextOverflowEllipsis.check(&project)
    }

    // ---- positive ----

    #[test]
    fn preprocessor_style_source_is_checked_too() {
        // 真实工程的样式源常常只有 .scss / .less；按后缀跳过等于这条规则从未生效。
        let fixture = Fixture::new("scss-source");
        fixture
            .write(
                "project.config.json",
                r#"{"miniprogramRoot":"","setting":{"useCompilerPlugins":["sass"]}}"#,
            )
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write(
                "pages/index/index.wxml",
                r#"<view class="title">标题</view>"#,
            )
            .write(
                "pages/index/index.scss",
                ".title { text-overflow: ellipsis; }",
            );
        let diagnostics = TextOverflowEllipsis.check(&fixture.project());
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].path, "pages/index/index.scss");
    }

    #[test]
    fn view_with_plain_text_is_reported() {
        let diagnostics = check(
            "plain",
            r#"<view class="title">标题</view>"#,
            ".title { text-overflow: ellipsis; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].rule, ID);
        assert_eq!(diagnostics[0].path, "pages/index/index.wxss");
    }

    #[test]
    fn view_without_any_text_child_is_reported() {
        let diagnostics = check(
            "nested-view",
            r#"<view class="title"><view class="inner">x</view></view>"#,
            ".title { text-overflow: ellipsis; }",
        );
        assert_eq!(diagnostics.len(), 1);
    }

    // ---- negative ----

    #[test]
    fn text_element_is_fine() {
        let diagnostics = check(
            "text",
            r#"<text class="title">标题</text>"#,
            ".title { text-overflow: ellipsis; }",
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn inherited_through_text_descendant_is_fine() {
        let diagnostics = check(
            "descendant",
            r#"<view class="list"><view class="row"><text>标题</text></view></view>"#,
            ".list { text-overflow: ellipsis; }",
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn custom_component_is_fine() {
        let diagnostics = check(
            "custom",
            r#"<my-text class="title"/>"#,
            ".title { text-overflow: ellipsis; }",
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn unmatched_selector_is_fine() {
        let diagnostics = check(
            "unmatched",
            r#"<view class="other">x</view>"#,
            ".title { text-overflow: ellipsis; }",
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn clip_is_fine() {
        let diagnostics = check(
            "clip",
            r#"<view class="title">x</view>"#,
            ".title { text-overflow: clip; }",
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn complex_selector_is_skipped() {
        let diagnostics = check(
            "complex",
            r#"<view class="a"><view class="b">x</view></view>"#,
            ".a .b { text-overflow: ellipsis; }",
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn dynamic_class_is_skipped() {
        let diagnostics = check(
            "dynamic",
            r#"<view class="{{cls}}">x</view>"#,
            ".title { text-overflow: ellipsis; }",
        );
        assert!(diagnostics.is_empty());
    }

    /// 逗号分隔的选择器各判各的：`.a` 命中 view，`.b` 命中的是 text。
    #[test]
    fn selectors_in_a_list_are_judged_individually() {
        let diagnostics = check(
            "selector-list",
            r#"<view class="a">x</view><text class="b">y</text>"#,
            ".a, .b { text-overflow: ellipsis; }",
        );
        assert_eq!(diagnostics.len(), 1);
        assert!(
            diagnostics[0].message.contains("`.a`"),
            "{}",
            diagnostics[0].message
        );
        assert!(!diagnostics[0].message.contains("`.b`"));
    }

    #[test]
    fn app_wxss_matches_across_pages() {
        let fixture = Fixture::new("global");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", r#"<view class="title">x</view>"#)
            .write("app.wxss", ".title { text-overflow: ellipsis; }");
        let project = fixture.project();
        assert_eq!(TextOverflowEllipsis.check(&project).len(), 1);
    }
}
