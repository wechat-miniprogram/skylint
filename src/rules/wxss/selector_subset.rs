//! 选择器不在 Skyline 的选择器子集里：两条分支、同一个根因。
//!
//! Skyline 的规则集按**主体 compound** 的 id / 标签 / class 分桶，匹配也只查这三类桶。
//! 属性条件在转 AST 时被丢弃，`*` 甚至过不了 WXSS 编译。两种写法都落回同一句判断：
//! 「这个选择器在 Skyline 的选择器子集里匹配不了」。
//!
//! 1. **属性条件被丢弃**。主体只有属性（`[foo] {}`）时没有桶，规则不入集，声明在
//!    Skyline 下从不生效；主体另有 id/tag/class（`.a[foo] {}`）时属性条件消失，命中
//!    范围被放宽——`.a[foo]` 等价 `.a`。只在工程里真的能观察到差异时才报：逐元素比对
//!    两种方言，要求存在「WebView 命中而 Skyline 不命中」（失效）或反向（放宽）的节点。
//! 2. **`*` 在编译期被拒绝**。WXSS 编译器直接报 `unexpected token '*'`，整份样式表
//!    编不出来；这是构建阶段失败，不是运行期静默不生效（见 `docs/RULE-VISUAL.md`
//!    的跳过说明）。
//!
//! 同一条选择器只报一条：两条都成立时（`*[foo]`）报更早、更硬的那条——编译期拒绝。

use std::collections::HashMap;
use std::rc::Rc;

use crate::input::wxml::WxmlDoc;
use crate::matcher::{Dialect, Tree, Tri, subject};
use crate::project::{Project, Scope, StyleSheet};
use crate::rules::{Rule, RuleMeta, VisualCase, VisualExpect};
use crate::{Diagnostic, Severity};

use super::visit_matching_rules;

pub struct SelectorSubset;

impl SelectorSubset {
    pub const ID: &'static str = "skyline/selector-subset";
}

/// 属性条件被丢弃后的两种差异落点。
enum Divergence {
    /// WebView 命中、Skyline 不命中：整条声明在 Skyline 下失效。
    Dead,
    /// Skyline 命中、WebView 不命中：属性条件被丢弃后多命中了这个节点。
    Wider,
}

/// 视觉用例：主体只有属性时整条失效，主体带 class 时命中范围被放宽；
/// 第三条 `*` 让整份 WXSS 编译失败，默认不采集（见 `docs/RULE-VISUAL.md`）。
static VISUAL_CASES: &[VisualCase] = &[
    VisualCase {
        wxml: r#"<view class="card" data-theme="dark" id="target">属性选择器命中的卡片</view>"#,
        wxss: "[data-theme=\"dark\"] { padding: 24px; background: #f0a020; color: #fff; }\n",
        js: None,
        expect: VisualExpect::Differ,
        note: "主体只有属性选择器的 `[data-theme=\"dark\"]` 在 WebView 命中、Skyline 下整条丢弃：卡片少了 24px 内边距与橙色背景，白色文字也一起丢（文字区 70→22 高）",
        target: "#target",
    },
    VisualCase {
        wxml: r#"<view class="card" id="target">没有 data-theme</view>
<view class="card" data-theme="dark">有 data-theme</view>"#,
        wxss: ".card[data-theme=\"dark\"] { padding: 16px; background: #3a7afe; color: #fff; }\n",
        js: None,
        expect: VisualExpect::Differ,
        note: "`.card[data-theme=\"dark\"]` 在 Skyline 下丢掉属性条件：第一张没有该属性的卡片也被上色，并多出 16px 内边距（22→54）、第二张整体下移 32 CSS；WebView 只给第二张上色",
        target: "#target",
    },
    VisualCase {
        wxml: r#"<text class="label" id="target">标记文字</text>"#,
        wxss: "* { color: #e00; }\n.label { font-size: 24px; }\n",
        js: None,
        expect: VisualExpect::Differ,
        note: "`*` 让整份 WXSS 在编译期就失败（`unexpected token '*'`），这条用例默认跳过，只登记「构建阶段就被拒绝」这一事实",
        target: "#target",
    },
];

impl Rule for SelectorSubset {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Error,
            description: "选择器不在 Skyline 的选择器子集里：属性选择器被丢弃（整条失效或放宽命中），`*` 在编译期就被拒绝",
        }
    }

    fn visual_cases(&self) -> &'static [VisualCase] {
        VISUAL_CASES
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut docs: HashMap<&str, Option<Rc<WxmlDoc>>> = HashMap::new();
        visit_matching_rules(project, |sheet, rule, out| {
            if !renders_in_skyline(project, sheet) {
                return;
            }
            for selector in &rule.selectors {
                let text = selector.text.trim();
                let Some(subject) = subject(text) else {
                    continue;
                };
                let (line, column) = sheet.doc.position(selector.offset);
                // 两条分支同源；都成立时只报编译期那条，别让 `*[foo]` 收到两条。
                if subject.universal {
                    let extra =
                        if subject.attributes.is_empty() && subject.other_attributes.is_empty() {
                            ""
                        } else {
                            "；同一条选择器里的属性条件在 Skyline 下也会被丢弃，但样式先过不了构建"
                        };
                    out.push(
                        Diagnostic::new(
                            Self::ID,
                            Severity::Error,
                            format!(
                                "`{text}` 命中 `*`（通配选择器）这一类：WXSS 编译期直接报 `unexpected token '*'`，整份样式编不出来——构建阶段就被拒绝，不是运行期失效{extra}"
                            ),
                            sheet.doc.path(),
                            line,
                            column,
                        )
                        .end(line, column + selector.text.chars().count() as u32)
                        .help(UNIVERSAL_HELP),
                    );
                    continue;
                }
                if subject.attributes.is_empty() && subject.other_attributes.is_empty() {
                    continue;
                }
                // 带属性的选择器都走过这一遍：只有真的在工程里命中不同节点才报。
                let Some((divergence, wxml, tag)) =
                    first_divergence(project, sheet, text, &mut docs)
                else {
                    continue;
                };
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

const UNIVERSAL_HELP: &str = "把 `*` 换成具体标签（如 `view`）或类名；`*` 不会被降级成「匹配所有节点」，它让整份 WXSS 构建失败";

/// 这份样式表在 Skyline 下是否真的会被用到：只会渲染在 WebView 的单元里，
/// 选择器子集的差异没有可观测后果。
fn renders_in_skyline(project: &Project, sheet: &StyleSheet) -> bool {
    let skyline_page = |route: &str| {
        project
            .pages
            .iter()
            .any(|page| page.route == route && page.is_skyline && !page.opted_out)
    };
    sheet.scopes.iter().any(|scope| match scope {
        Scope::Global => project
            .pages
            .iter()
            .any(|page| page.is_skyline && !page.opted_out),
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
    docs: &mut HashMap<&'a str, Option<Rc<WxmlDoc>>>,
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
    use crate::Diagnostic;
    use crate::rules::testutil::{Fixture, check_rule};

    use super::SelectorSubset;

    /// 单页纯 WebView 工程：页面 json 显式 opt-out。
    fn webview_fixture(name: &str, wxml: &str, wxss: &str) -> Fixture {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write("app.json", r#"{"pages":["pages/index/index"]}"#)
            .write("pages/index/index.json", r#"{"renderer":"webview"}"#)
            .write("pages/index/index.wxml", wxml)
            .write("pages/index/index.wxss", wxss);
        fixture
    }

    fn check(fixture: &Fixture) -> Vec<Diagnostic> {
        check_rule(fixture, &SelectorSubset)
    }

    /// 视觉用例与单测共用同一份 WXML/WXSS：每条都必须被本规则命中。
    #[test]
    fn visual_cases_are_reported() {
        for case in super::VISUAL_CASES {
            let fixture = Fixture::page("selector-subset-visual", case.wxml, case.wxss);
            let hits = check(&fixture)
                .iter()
                .filter(|diagnostic| diagnostic.rule == SelectorSubset::ID)
                .count();
            assert!(hits >= 1, "{}: {}", case.note, case.wxml);
        }
    }

    #[test]
    fn attributes_only_subject_is_dead_in_skyline() {
        // `[data-theme]` 在 WebView 命中 <view>，Skyline 下整条不生效。
        let fixture = Fixture::page(
            "attr-only",
            r#"<view data-theme="dark"/>"#,
            r#"[data-theme] { color: blue; }"#,
        );
        let diags = check(&fixture);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert_eq!(diags[0].severity, crate::Severity::Warning, "{diags:#?}");
        assert!(diags[0].message.contains("不会进入规则集"), "{diags:#?}");
        assert!(diags[0].message.contains("WebView 命中"), "{diags:#?}");
    }

    #[test]
    fn dead_in_both_renderers_is_not_reported() {
        // 值对不上：WebView 也不命中，两端都是死样式，不是 Skyline 差异。
        let fixture = Fixture::page(
            "attr-dead-both",
            r#"<view data-theme="dark"/>"#,
            r#"[data-theme="light"] { color: red; }"#,
        );
        assert!(check(&fixture).is_empty());
    }

    #[test]
    fn attribute_never_written_only_widens_the_locator() {
        // 属性从未写过：`.card[data-theme]` 在 Skyline 下等价 `.card`（真差异），
        // 而 `[data-theme]` 两端都不命中（不是差异）。
        // 第一行的动态 class 让闸门对 `.card[data-theme]` 判成 Unknown（可能命中），
        // 否则属性从未出现时整块会被可达性闸门跳过。
        let fixture = Fixture::page(
            "attr-unused",
            r#"<view class="{{cls}}"/><view class="card"/>"#,
            r#".card[data-theme] { color: red; }
               [data-theme] { color: blue; }"#,
        );
        let diags = check(&fixture);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert!(
            diags[0].message.contains("`.card[data-theme]`"),
            "{diags:#?}"
        );
    }

    #[test]
    fn dropped_condition_is_reported_when_it_widens_the_match() {
        // 第二个 .card 没有 data-theme：Skyline 丢掉属性条件后多命中它。
        let fixture = Fixture::page(
            "attr-widen",
            r#"<view class="card" data-theme="dark"/><view class="card"/>"#,
            r#".card[data-theme="dark"] { color: red; }"#,
        );
        let diags = check(&fixture);
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
        let fixture = Fixture::page(
            "attr-harmless",
            r#"<view class="card" data-theme="dark"/>"#,
            r#".card[data-theme] { color: red; }"#,
        );
        assert!(check(&fixture).is_empty());
    }

    #[test]
    fn attribute_on_the_ancestor_segment_is_checked_too() {
        // 第一个 .page 带 data-theme（WebView 命中，闸门放行），第二个不带：
        // Skyline 丢掉属性条件后多命中它。
        let fixture = Fixture::page(
            "attr-ancestor",
            r#"<view class="page" data-theme="dark"><view class="title"/></view><view class="page"><view class="title"/></view>"#,
            r#".page[data-theme] .title { color: red; }"#,
        );
        let diags = check(&fixture);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert!(diags[0].message.contains("`[data-theme]`"), "{diags:#?}");
    }

    #[test]
    fn pseudo_class_only_subject_is_left_to_no_pseudo_rule() {
        // 主体只有伪类（`:checked`）在 Skyline 下同样整条失效，但那是 no_pseudo 的判定面。
        let fixture = Fixture::page(
            "attr-pseudo-only",
            r#"<view class="card"/>"#,
            r#".card:checked, [data-x]:checked { color: red; }"#,
        );
        assert!(check(&fixture).is_empty());
    }

    #[test]
    fn universal_subject_is_reported_as_a_build_failure() {
        let fixture = Fixture::page(
            "universal-subject",
            r#"<view class="a"><view/></view>"#,
            "* { box-sizing: border-box; }",
        );
        let diags = check(&fixture);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert_eq!(diags[0].severity, crate::Severity::Error, "{diags:#?}");
        assert!(diags[0].message.contains("`*`"), "{diags:#?}");
        assert!(diags[0].message.contains("编译期"), "{diags:#?}");
        assert!(diags[0].message.contains("构建阶段"), "{diags:#?}");
        assert!(!diags[0].message.contains("不进规则集"), "{diags:#?}");
    }

    #[test]
    fn universal_after_a_class_is_still_reported() {
        // 主体是 `*`，`.a` 只是祖先约束；`*` 同样让整份 WXSS 编译失败。
        let fixture = Fixture::page(
            "universal-after-class",
            r#"<view class="a"><view/></view>"#,
            ".a * { margin: 0; }",
        );
        assert_eq!(check(&fixture).len(), 1);
    }

    #[test]
    fn star_with_attribute_is_reported_once() {
        // 合并前两条规则各报一条；现在同一个选择器只报编译期这一条。
        let fixture = Fixture::page(
            "universal-with-attr",
            r#"<view data-theme="dark"/>"#,
            r#"*[data-theme] { color: red; }"#,
        );
        let diags = check(&fixture);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert_eq!(diags[0].severity, crate::Severity::Error, "{diags:#?}");
        assert!(diags[0].message.contains("`*`（通配选择器）"), "{diags:#?}");
        assert!(diags[0].message.contains("属性条件"), "{diags:#?}");
    }

    #[test]
    fn pure_webview_project_is_not_reported() {
        // 只会渲染在 WebView 的单元：选择器子集的差异没有可观测后果。
        let fixture = webview_fixture(
            "webview-only",
            r#"<view class="a" data-theme="dark"><view/></view>"#,
            r#"* { color: red; }
               [data-theme] { color: blue; }
               .a[data-theme] { color: green; }"#,
        );
        assert!(check(&fixture).is_empty());
    }

    #[test]
    fn tag_class_and_host_subjects_are_fine() {
        for (name, source) in [
            ("ok-tag", "view { box-sizing: border-box; }"),
            ("ok-class", ".a { color: red; }"),
            ("ok-id", "#app { color: red; }"),
            ("ok-host", ":host { display: block; }"),
            ("ok-descendant", ".a view { color: red; }"),
        ] {
            let fixture = Fixture::page(name, r#"<view class="a"><view/></view>"#, source);
            let diags = check(&fixture);
            assert!(diags.is_empty(), "{source} 不该报：{diags:?}");
        }
    }
}
