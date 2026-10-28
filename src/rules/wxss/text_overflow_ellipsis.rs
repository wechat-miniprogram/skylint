//! `text-overflow: ellipsis` 在 Skyline 下只对 `<text>` 生效。
//!
//! 知识库原文：「`text-overflow: ellipsis` 在 Skyline 下**仅在 `<text>` 组件上生效**，
//! 在 `<view>` 等组件上不生效」。WebView 上 `overflow: hidden` + `white-space: nowrap`
//! + `text-overflow: ellipsis` 对任意块级元素都成立，迁移后静默失效。
//!
//! `text-overflow` 不继承（`css/css_properties/text_overflow.h:12` 只带 `kProperty`），
//! 所以判定范围就是「声明所在选择器命中的节点自身」：`.list`（`<view>`）下面有
//! `<text>` 也不生效。命中节点是自定义组件、或属性带插值时静态判断不了，一律放过
//! （宁可漏报）。
//!
//! 另一半前提是「漏在 WebView 下真的省了」：没有 `white-space: nowrap`（也没有
//! `-webkit-line-clamp` 那套多行截断）时，WebView 只会换行、不会产生省略号，两边
//! 表现一致就不算迁移差异（`.book_author` 这类 flex 容器、`.intro_content` 这类靠
//! `max-height` 裁剪的多行块都属于这种）。多行 clamp 的弱化另由
//! `skyline/unsupported-property` 与 `skyline/legacy-flexbox` 报。

use std::collections::HashMap;

use crate::input::wxml::{self, WxmlElement};
use crate::matcher::Tree;
use crate::project::Project;
use crate::rules::RuleMeta;
use crate::rules::wxml::is_builtin;
use crate::rules::wxml::resolved_property;
use crate::{Diagnostic, Severity};

use super::visit_rules;

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
            description: "text-overflow: ellipsis 在 Skyline 下只对 `<text>` 生效",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut docs: HashMap<String, Option<wxml::WxmlDoc>> = HashMap::new();

        visit_rules(project, |sheet, rule, out| {
            let doc = &sheet.doc;
            let Some(decl) = rule.declaration("text-overflow") else {
                return;
            };
            if !decl.value.eq_ignore_ascii_case("ellipsis") {
                return;
            }
            // 逗号分隔的选择器各自独立：`.a, .b` 里只要有一条命中的节点承载不了
            // ellipsis，这条声明对那批节点就是失效的。
            let offenders = super::broken_selectors(
                project,
                sheet,
                &rule.selectors,
                &mut docs,
                |tree, element| no_divergence(project, tree, element),
            );
            if offenders.is_empty() {
                return;
            }
            let selectors = offenders.join("`, `");
            let (line, column) = doc.position(decl.offset);
            out.push(
                Diagnostic::new(
                    ID,
                    Severity::Warning,
                    format!("`text-overflow: ellipsis` 在 Skyline 下只作用于 `<text>`；选择器 `{selectors}` 在某处命中的节点不是 `<text>`，声明不会生效"),
                    doc.path(),
                    line,
                    column,
                )
                .help("用 `<text>` 包裹文本（Skyline 下可配合 `overflow` / `max-lines` 截断），或删掉该声明"),
            );
        })
    }
}

/// 这个命中不构成「WebView 能省略、Skyline 不能」的差异。
///
/// - 宿主是 `<text>` / `<span>` / `<navigator>` 或自定义组件：Skyline 下这条声明本来生效；
/// - 其余宿主在 WebView 里也得要 `white-space: nowrap` 才可能省略，没 nowrap 时文本只
///   换行、两端一致。`white-space` 从祖先继承的场景静态判不了，按漏报处理。
fn no_divergence(project: &Project, tree: &Tree, element: &WxmlElement) -> bool {
    if !is_builtin(&element.tag) || matches!(element.tag.as_str(), "text" | "span" | "navigator") {
        return true;
    }
    !resolved_property(project, tree, element, "white-space")
        .is_some_and(|resolved| resolved.value.eq_ignore_ascii_case("nowrap"))
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
                ".title { text-overflow: ellipsis; white-space: nowrap; }",
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
            ".title { text-overflow: ellipsis; white-space: nowrap; }",
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
            ".title { text-overflow: ellipsis; white-space: nowrap; }",
        );
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn no_nowrap_in_webview_means_no_difference() {
        // 没有 `white-space: nowrap` 时 WebView 只会换行，不产生省略号：两端一致。
        let diagnostics = check(
            "no-nowrap",
            r#"<view class="title">标题</view>"#,
            ".title { text-overflow: ellipsis; overflow: hidden; max-height: 4.5em; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn flex_container_without_nowrap_is_fine() {
        // `.book_author` 那种：flex 容器的子项都是 flex item，WebView 也不会省略。
        let diagnostics = check(
            "flex-container",
            r#"<view class="author"><text class="name">阿乙</text></view>"#,
            ".author { display: flex; text-overflow: ellipsis; overflow: hidden; }
             .name { white-space: nowrap; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn dynamic_nowrap_is_skipped() {
        let diagnostics = check(
            "dynamic-nowrap",
            r#"<view class="title">x</view>"#,
            ".title { text-overflow: ellipsis; white-space: {{nowrap}}; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    /// 共享样式在页面里失效、在组件里正常（反过来也一样）也要报：按作用域分别判。
    #[test]
    fn shared_style_broken_in_one_scope_is_reported() {
        let fixture = Fixture::new("shared-scope");
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
            .write(
                "pages/index/index.wxml",
                r#"<view class="title">页面上的标题</view><c/>"#,
            )
            .write("pages/index/index.wxss", "@import \"/styles/shared.wxss\";")
            .write("components/c/c.json", "{}")
            .write(
                "components/c/c.wxml",
                r#"<text class="title">组件里的标题</text>"#,
            )
            .write("components/c/c.wxss", "@import \"/styles/shared.wxss\";")
            .write(
                "styles/shared.wxss",
                ".title { text-overflow: ellipsis; white-space: nowrap; }",
            );
        let diagnostics = TextOverflowEllipsis.check(&fixture.project());
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].path, "styles/shared.wxss");
    }

    // ---- negative ----

    #[test]
    fn text_element_is_fine() {
        let diagnostics = check(
            "text",
            r#"<text class="title">标题</text>"#,
            ".title { text-overflow: ellipsis; white-space: nowrap; }",
        );
        assert!(diagnostics.is_empty());
    }

    /// `text-overflow` 不继承：容器里的 `<text>` 拿不到这条声明。
    #[test]
    fn text_descendant_does_not_exempt_the_container() {
        let diagnostics = check(
            "descendant",
            r#"<view class="list"><view class="row"><text>标题</text></view></view>"#,
            ".list { text-overflow: ellipsis; white-space: nowrap; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    }

    #[test]
    fn span_and_navigator_are_fine() {
        let diagnostics = check(
            "text-nodes",
            r#"<span class="a">x</span><navigator class="b">y</navigator>"#,
            ".a, .b { text-overflow: ellipsis; white-space: nowrap; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn custom_component_is_fine() {
        let diagnostics = check(
            "custom",
            r#"<my-text class="title"/>"#,
            ".title { text-overflow: ellipsis; white-space: nowrap; }",
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn unmatched_selector_is_fine() {
        let diagnostics = check(
            "unmatched",
            r#"<view class="other">x</view>"#,
            ".title { text-overflow: ellipsis; white-space: nowrap; }",
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
    fn descendant_selector_is_matched() {
        // 后代选择器现在能命中视图：`.b` 是 `<view>`，承载不了 text-overflow。
        let diagnostics = check(
            "complex",
            r#"<view class="a"><view class="b">x</view></view>"#,
            ".a .b { text-overflow: ellipsis; white-space: nowrap; }",
        );
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn descendant_selector_on_text_is_fine() {
        let diagnostics = check(
            "complex-text",
            r#"<view class="a"><text class="b">x</text></view>"#,
            ".a .b { text-overflow: ellipsis; white-space: nowrap; }",
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn dynamic_class_is_skipped() {
        let diagnostics = check(
            "dynamic",
            r#"<view class="{{cls}}">x</view>"#,
            ".title { text-overflow: ellipsis; white-space: nowrap; }",
        );
        assert!(diagnostics.is_empty());
    }

    /// 逗号分隔的选择器各判各的：`.a` 命中 view，`.b` 命中的是 text。
    #[test]
    fn selectors_in_a_list_are_judged_individually() {
        let diagnostics = check(
            "selector-list",
            r#"<view class="a">x</view><text class="b">y</text>"#,
            ".a, .b { text-overflow: ellipsis; white-space: nowrap; }",
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
            .write(
                "app.wxss",
                ".title { text-overflow: ellipsis; white-space: nowrap; }",
            );
        let project = fixture.project();
        assert_eq!(TextOverflowEllipsis.check(&project).len(), 1);
    }
}
