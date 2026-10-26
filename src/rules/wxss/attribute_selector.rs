//! 属性选择器在 Skyline 下不参与匹配。
//!
//! 证据链（全部读自引擎源码）：
//! - fp 解析属性选择器并挂在 `SelectorFragment.attributes` 上
//!   （`float-pigment-css/src/parser/selector.rs:612-627`、`:658`）；
//! - Skyline 转 AST 时只读 `id` / `tag_name` / `classes`
//!   （`css/css_parser_impl.h:191-220`），`CompoundSelector` 没有属性字段
//!   （`css/selector.h:78-91`）；
//! - 入规则集按 id → tag → class 分桶，主体三者都为空且不是 `:host` 时直接返回，
//!   规则不会进入任何桶（`style/rule_set.cc:106-160`）；
//! - 匹配只查 tag / id / class 三类桶（`style/selector_matcher.cc:456-500`）。
//!
//! 于是分两种结果：
//!
//! 1. **整条失效**：主体只有属性选择器（`[foo] {}`、`[foo="a"] {}`），空 compound
//!    不入集，声明在 Skyline 下从不生效。
//! 2. **条件放宽**：主体另有 id/tag/class（`.a[foo] {}`、`view[foo] {}`），属性条件
//!    被丢弃，命中的节点变多——`.a[foo]` 等价 `.a`。
//!
//! 两种都只在工程里真的能观察到差异时才报：逐元素比对两种方言，要求存在
//! 「WebView 命中而 Skyline 不命中」（失效）或「Skyline 命中而 WebView 不命中」
//! （放宽）的节点。工程里所有 `.a` 本来都带 `foo` 时，放宽没有可观测后果；
//! 属性从未被写过时，WebView 也不会命中，两端都是死样式。

use std::collections::HashMap;

use crate::input::wxml::WxmlDoc;
use crate::matcher::{Dialect, Tree, Tri, subject};
use crate::project::{Project, Scope, StyleSheet};
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

use super::visit_rules;

pub struct AttributeSelector;

impl AttributeSelector {
    pub const ID: &'static str = "skyline/attribute-selector";
}

/// 两种差异落点。
enum Divergence {
    /// WebView 命中、Skyline 不命中：整条声明在 Skyline 下失效。
    Dead,
    /// Skyline 命中、WebView 不命中：属性条件被丢弃后多命中了这个节点。
    Wider,
}

impl Rule for AttributeSelector {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "属性选择器在 Skyline 下不参与匹配：主体只有属性时整条声明失效，否则命中范围被放宽",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut docs: HashMap<&str, Option<WxmlDoc>> = HashMap::new();
        visit_rules(project, |sheet, rule, out| {
            if !renders_in_skyline(project, sheet) {
                return;
            }
            for selector in &rule.selectors {
                let Some(subject) = subject(&selector.text) else {
                    continue;
                };
                if subject.attributes.is_empty() && subject.other_attributes.is_empty() {
                    continue;
                }
                let text = selector.text.trim();
                // 带属性的选择器都走过这一遍：只有真的在工程里命中不同节点才报。
                let Some((divergence, wxml, tag)) =
                    first_divergence(project, sheet, text, &mut docs)
                else {
                    continue;
                };
                let (line, column) = sheet.doc.position(selector.offset);
                let conditions = subject
                    .attributes
                    .iter()
                    .chain(&subject.other_attributes)
                    .map(|name| format!("`[{name}]`"))
                    .collect::<Vec<_>>()
                    .join("、");
                let (message, help) = match divergence {
                    Divergence::Dead => (
                        format!(
                            "`{text}` 的属性选择器在 Skyline 下被丢弃，而主体没有标签 / class / id：这条规则不会进入规则集，`{wxml}` 里的 `<{tag}>` 在 WebView 命中、在 Skyline 不命中，整条声明失效"
                        ),
                        "给这条规则补一个标签 / class / id 作为主体（如 `.a[foo]`），或把属性换成 class 表达".to_string(),
                    ),
                    Divergence::Wider => (
                        format!(
                            "`{text}` 里的 {conditions} 在 Skyline 下被丢弃：`{wxml}` 里的 `<{tag}>` 没有这个属性（或值不匹配），WebView 不会命中它，Skyline 会把样式应用上去"
                        ),
                        format!(
                            "把 `[{}]` 表达成 class（例如选中时切换一个类名），属性选择器在 Skyline 下等价于不存在",
                            subject
                                .attributes
                                .first()
                                .or_else(|| subject.other_attributes.first())
                                .map(String::as_str)
                                .unwrap_or_default()
                        ),
                    ),
                };
                out.push(
                    Diagnostic::new(
                        Self::ID,
                        Severity::Warning,
                        message,
                        sheet.doc.path(),
                        line,
                        column,
                    )
                    .help(help),
                );
            }
        })
    }
}

/// 这份样式表在 Skyline 下是否真的会被用到：只会渲染在 WebView 的单元里，
/// 属性选择器的差异没有可观测后果。
fn renders_in_skyline(project: &Project, sheet: &StyleSheet) -> bool {
    let skyline_page = |route: &str| {
        project
            .pages
            .iter()
            .any(|page| page.route == route && !page.opted_out)
    };
    sheet.scopes.iter().any(|scope| match scope {
        Scope::Global => project.pages.iter().any(|page| !page.opted_out),
        Scope::Page(route) => skyline_page(route),
        Scope::Component(path) => project
            .component_pages(path)
            .iter()
            .any(|route| skyline_page(route)),
        Scope::Loose(_) => false,
    })
}

/// 在样式表作用到的单元里找第一个两种方言结论不同的元素。
fn first_divergence<'a>(
    project: &'a Project,
    sheet: &StyleSheet,
    selector: &str,
    docs: &mut HashMap<&'a str, Option<WxmlDoc>>,
) -> Option<(Divergence, String, String)> {
    for unit in candidate_units(project, sheet) {
        let Some(doc) = docs
            .entry(unit)
            .or_insert_with(|| super::super::wxml::load_doc(project, unit))
            .as_ref()
        else {
            continue;
        };
        let tree = Tree::new(doc);
        for element in doc.elements() {
            let skyline = tree.matches(element, selector, Dialect::Skyline);
            let webview = tree.matches(element, selector, Dialect::WebView);
            let divergence = match (skyline, webview) {
                (Tri::No, Tri::Yes) => Divergence::Dead,
                (Tri::Yes, Tri::No) => Divergence::Wider,
                _ => continue,
            };
            return Some((divergence, doc.path().to_string(), element.tag.clone()));
        }
    }
    None
}

/// 这份样式表可能作用到的单元（`app.wxss` 是全局，覆盖所有页面与组件）。
fn candidate_units<'a>(project: &'a Project, sheet: &StyleSheet) -> Vec<&'a str> {
    let mut out: Vec<&str> = Vec::new();
    for scope in &sheet.scopes {
        for path in super::scope_wxml(project, scope) {
            if !out.contains(&path) {
                out.push(path);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use crate::project::Project;
    use crate::rules::Rule;
    use crate::rules::testutil::wxss::Fixture;

    use super::AttributeSelector;

    /// 单页工程；fixture 必须活到检查结束（WXML 是懒加载的）。
    fn fixture(name: &str, wxml: &str, wxss: &str) -> (Fixture, Project) {
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
        (fixture, project)
    }

    #[test]
    fn attributes_only_subject_is_dead_in_skyline() {
        // `[data-theme]` 在 WebView 命中 <view>，Skyline 下整条不生效。
        let (_f, project) = fixture(
            "attr-only",
            r#"<view data-theme="dark"/>"#,
            r#"[data-theme] { color: blue; }"#,
        );
        let diags = AttributeSelector.check(&project);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert!(diags[0].message.contains("不会进入规则集"), "{diags:#?}");
        assert!(diags[0].message.contains("WebView 命中"), "{diags:#?}");
    }

    #[test]
    fn dead_in_both_renderers_is_not_reported() {
        // 值对不上：WebView 也不命中，两端都是死样式，不是 Skyline 差异。
        let (_f, project) = fixture(
            "attr-dead-both",
            r#"<view data-theme="dark"/>"#,
            r#"[data-theme="light"] { color: red; }"#,
        );
        assert!(AttributeSelector.check(&project).is_empty());
    }

    #[test]
    fn attribute_never_written_only_widens_the_locator() {
        // 属性从未写过：`.card[data-theme]` 在 Skyline 下等价 `.card`（真差异），
        // 而 `[data-theme]` 两端都不命中（不是差异）。
        let (_f, project) = fixture(
            "attr-unused",
            r#"<view class="card"/>"#,
            r#".card[data-theme] { color: red; }
               [data-theme] { color: blue; }"#,
        );
        let diags = AttributeSelector.check(&project);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert!(
            diags[0].message.contains("`.card[data-theme]`"),
            "{diags:#?}"
        );
    }

    #[test]
    fn dropped_condition_is_reported_when_it_widens_the_match() {
        // 第二个 .card 没有 data-theme：Skyline 丢掉属性条件后多命中它。
        let (_f, project) = fixture(
            "attr-widen",
            r#"<view class="card" data-theme="dark"/><view class="card"/>"#,
            r#".card[data-theme="dark"] { color: red; }"#,
        );
        let diags = AttributeSelector.check(&project);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert!(diags[0].message.contains("被丢弃"), "{diags:#?}");
        assert!(
            diags[0].message.contains("pages/index/index.wxml"),
            "{diags:#?}"
        );
    }

    #[test]
    fn every_matching_element_carrying_the_attribute_is_not_reported() {
        // 所有 .card 都带属性：放宽之后命中范围不变，没有可观测差异。
        let (_f, project) = fixture(
            "attr-harmless",
            r#"<view class="card" data-theme="dark"/>"#,
            r#".card[data-theme] { color: red; }"#,
        );
        assert!(AttributeSelector.check(&project).is_empty());
    }

    #[test]
    fn attribute_on_the_ancestor_segment_is_checked_too() {
        let (_f, project) = fixture(
            "attr-ancestor",
            r#"<view class="page"><view class="title"/></view>"#,
            r#".page[data-theme] .title { color: red; }"#,
        );
        let diags = AttributeSelector.check(&project);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert!(diags[0].message.contains("`[data-theme]`"), "{diags:#?}");
    }

    #[test]
    fn pseudo_class_only_subject_is_left_to_no_pseudo_rule() {
        // 主体只有伪类（`:checked`）在 Skyline 下同样整条失效，但那是 no_pseudo 的判定面。
        let (_f, project) = fixture(
            "attr-pseudo-only",
            r#"<view class="card"/>"#,
            r#".card:checked, [data-x]:checked { color: red; }"#,
        );
        assert!(AttributeSelector.check(&project).is_empty());
    }
}
