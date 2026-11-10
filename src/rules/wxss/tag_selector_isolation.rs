//! 标签 / id 选择器在 Skyline 下不跨组件匹配。
//!
//! 引擎侧的证据（`skyline/style/selector_matcher.cc`）：页面/组件样式表走
//! `MatchScopedRules`，命中前要求 `node.StyleScopeId() == rule_scope_id`；
//! 标签选择器在 `tagNameStyleIsolation: legacy` 下改查全局标签规则表放宽，
//! id 选择器那段**没有 legacy 分支**——所以 legacy 只救标签，救不了 id。
//! 而 `app.wxss` 走 `MatchGlobalRules`，本来就不分作用域，不受影响。
//!
//! 于是迁移风险只有一类：页面 / 组件的样式表里，纯标签或纯 id 选择器命中了
//! **别的单元**的节点。WebView 下这层 DOM 是扁平的、能命中；Skyline 下静默不再命中。

use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use serde_json::{Map, Value};

use crate::input::wxml::{WxmlAttribute, WxmlDoc};
use crate::project::{Project, Scope, StyleIsolation};
use crate::rules::RuleMeta;
use crate::{Diagnostic, Severity};

use super::visit_rules;
use crate::matcher::{FlatNode, FlatSelector, flat_selector};
use crate::wxml_graph::{AncestorRef, WxmlGraph};

pub const ID: &str = "skyline/tag-selector-isolation";

pub struct TagSelectorIsolation;

impl TagSelectorIsolation {
    pub const ID: &'static str = ID;

    /// `rendererOptions.skyline.tagNameStyleIsolation == "legacy"`（页面级优先，缺省回落 app.json）。
    fn legacy(project: &Project, route: Option<&str>) -> bool {
        let read = |json: &Map<String, Value>| {
            json.get("rendererOptions")
                .and_then(|v| v.get("skyline"))
                .and_then(|v| v.get("tagNameStyleIsolation"))
                .and_then(Value::as_str)
                .map(|v| v == "legacy")
        };
        route
            .and_then(|route| project.page_json.get(route))
            .and_then(read)
            .or_else(|| read(&project.app_json))
            .unwrap_or(false)
    }
}

/// 一条命中记录，用来去重（同一选择器可能被多个 scope 各判一次）。
struct Hit {
    selector: String,
    tag: bool,
    /// 命中的外部单元，用来在消息里举例。
    unit: String,
    /// 该单元里被命中的第一个标签名。
    foreign_tag: String,
}

impl crate::rules::Rule for TagSelectorIsolation {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Warning,
            description: "标签 / id 选择器在 Skyline 下不跨组件匹配，WebView 上生效的样式会静默失效",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut hits: BTreeMap<(String, usize), Hit> = BTreeMap::new();
        let mut docs: HashMap<&str, Option<Rc<WxmlDoc>>> = HashMap::new();
        // 只有带组合符的选择器才需要渲染树；纯标签 / 纯 id 的单 compound 用不到它。
        let graph: std::cell::OnceCell<WxmlGraph> = std::cell::OnceCell::new();

        visit_rules(project, |sheet, rule, _out| {
            let doc = &sheet.doc;
            // 这份样式表自己就作用于的单元：它们内部的选择器照常命中，不算「外部」。
            // 典型是 default-box-sizing.scss 既被页面 import、又被组件自己 import。
            let in_scope: Vec<&str> = sheet
                .scopes
                .iter()
                .filter_map(|scope| match scope {
                    Scope::Page(route) => Some(route.as_str()),
                    Scope::Component(path) => Some(path.as_str()),
                    Scope::Global | Scope::Loose(_) => None,
                })
                .collect();
            for scope in &sheet.scopes {
                let (foreign, legacy) = match scope {
                    // app.wxss 走全局规则表，不分作用域，不受影响
                    Scope::Global => continue,
                    Scope::Page(route) => (
                        project.page_components(route),
                        Self::legacy(project, Some(route)),
                    ),
                    Scope::Component(path) => {
                        // 组件样式漏到外部：用到它的页面，以及同一页面里的兄弟组件。
                        // WebView 能命中，Skyline 不跨作用域。
                        //
                        // `styleIsolation: shared` 是例外：它的规则拿到的是页面那一份
                        // scope（`space.ts:633-636`），页面节点按主 scope 查规则集时
                        // 就会命中（`selector_matcher.cc:456-460` 用的是节点主 scope），
                        // 与 WebView 一致，只剩兄弟组件还会失效。
                        let shared = project.style_isolation(path) == StyleIsolation::Shared;
                        let pages = if shared {
                            Vec::new()
                        } else {
                            project.component_pages(path)
                        };
                        let mut foreign = pages.clone();
                        for page in &pages {
                            for component in project.page_components(page) {
                                if component != path.as_str() && !foreign.contains(&component) {
                                    foreign.push(component);
                                }
                            }
                        }
                        (foreign, Self::legacy(project, None))
                    }
                    // 样式图不会产生 Loose。
                    Scope::Loose(_) => continue,
                };
                let foreign: Vec<&str> = foreign
                    .into_iter()
                    .filter(|unit| !in_scope.contains(unit))
                    .collect();
                if foreign.is_empty() {
                    continue;
                }
                for selector in &rule.selectors {
                    let Some(flat) = flat_selector(selector.text.trim()) else {
                        continue;
                    };
                    if !flat.subject_is_tag_or_id_only() {
                        continue;
                    }
                    let is_tag = flat.subject_is_tag();
                    // legacy 放开的是标签选择器；id 选择器无论配置如何都不跨组件
                    if is_tag && legacy {
                        continue;
                    }
                    let Some((unit, foreign_tag)) =
                        find_foreign(project, &graph, &foreign, &flat, &mut docs)
                    else {
                        continue;
                    };
                    let key = (doc.path().to_string(), selector.offset);
                    hits.entry(key).or_insert(Hit {
                        selector: selector.text.trim().to_string(),
                        tag: is_tag,
                        unit,
                        foreign_tag,
                    });
                }
            }
        });

        hits.into_iter()
            .map(|((path, offset), hit)| {
                let doc = project
                    .styles
                    .iter()
                    .map(|sheet| &sheet.doc)
                    .find(|doc| doc.path() == path)
                    .expect("hit 来自 project.styles");
                let (line, column) = doc.position(offset);
                let message = if hit.tag {
                    format!(
                        "标签选择器 `{}` 命中了 `{}` 里的 `<{}>` 节点；Skyline 默认 `tagNameStyleIsolation: \"isolated\"`，标签选择器不跨组件匹配，这些样式会静默失效",
                        hit.selector, hit.unit, hit.foreign_tag
                    )
                } else {
                    format!(
                        "id 选择器 `{}` 命中了 `{}` 里的 `<{}>` 节点；Skyline 下 id 选择器不跨组件匹配（`tagNameStyleIsolation: \"legacy\"` 也不放开），这些样式会静默失效",
                        hit.selector, hit.unit, hit.foreign_tag
                    )
                };
                let help = if hit.tag {
                    "在 app.json 设 rendererOptions.skyline.tagNameStyleIsolation: \"legacy\"（把所有标签选择器恢复成全局匹配），或改用 class 选择器"
                } else {
                    "改用 class 选择器（id 选择器跨组件在 Skyline 下没有配置开关）"
                };
                Diagnostic::new(ID, Severity::Warning, message, path, line, column).help(help)
            })
            .collect()
    }
}

/// 在外部单元里找第一个命中该选择器的节点。
///
/// 主体的祖先可能跨在组件边界另一侧（`page view` 的 `page` 不在组件文档里），
/// 所以带组合符的选择器走 [`WxmlGraph::ancestor_paths`] 的逐条路径；单 compound
/// 不需要路径，直接匹配主体。
fn find_foreign<'a>(
    project: &'a Project,
    graph: &std::cell::OnceCell<WxmlGraph>,
    units: &[&'a str],
    selector: &FlatSelector,
    docs: &mut HashMap<&'a str, Option<Rc<WxmlDoc>>>,
) -> Option<(String, String)> {
    for unit in units {
        let Some(wxml_path) = project
            .components
            .iter()
            .find(|c| &c.path == unit)
            .and_then(|c| c.files.wxml.as_deref())
            .or_else(|| {
                project
                    .pages
                    .iter()
                    .find(|p| &p.route == unit)
                    .and_then(|p| p.files.wxml.as_deref())
            })
        else {
            continue;
        };
        let Some(doc) = docs
            .entry(wxml_path)
            .or_insert_with(|| super::super::wxml::load_doc(project, wxml_path))
            .as_ref()
        else {
            // 该单元 WXML 解析失败只跳过它，不影响其它外部单元。
            continue;
        };
        for element in doc.elements() {
            // 「WebView 能不能命中」才是这条规则关心的方向。
            let node = node_of(&element.tag, &element.attributes);
            if !selector.matches_subject(&node) {
                continue;
            }
            let matched = selector.is_simple() || {
                let graph = graph.get_or_init(|| WxmlGraph::new(project));
                graph.ancestor_paths(wxml_path, element).iter().any(|path| {
                    let ancestors: Vec<FlatNode> = path.iter().map(ancestor_node).collect();
                    selector.matches(&node, &ancestors)
                })
            };
            if matched {
                return Some((unit.to_string(), element.tag.clone()));
            }
        }
    }
    None
}

/// 扁平节点 → 匹配用的描述。动态 class 只取「任何取值下都出现」的词，
/// 动态 id 当作未知（按不命中处理）——宁可漏报也不要凭空报一条。
fn node_of(tag: &str, attributes: &[WxmlAttribute]) -> FlatNode {
    let id = attributes
        .iter()
        .find(|attr| attr.name.eq_ignore_ascii_case("id"))
        .filter(|attr| attr.is_static)
        .and_then(|attr| attr.value.clone());
    let classes = attributes
        .iter()
        .find(|attr| attr.name.eq_ignore_ascii_case("class"))
        .map(|attr| match &attr.class_tokens {
            Some(tokens) => tokens.definite.clone(),
            None if attr.is_static => attr
                .value
                .as_deref()
                .unwrap_or("")
                .split_whitespace()
                .map(str::to_string)
                .collect(),
            None => Vec::new(),
        })
        .unwrap_or_default();
    FlatNode {
        tag: tag.to_ascii_lowercase(),
        id,
        classes,
    }
}

fn ancestor_node(ancestor: &AncestorRef<'_>) -> FlatNode {
    match ancestor {
        AncestorRef::Element { element } => node_of(&element.tag, &element.attributes),
        // WebView 的 DOM 根就是 `<page>`。
        AncestorRef::PageRoot => FlatNode {
            tag: "page".to_string(),
            ..FlatNode::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::Rule;
    use crate::rules::testutil::wxss::Fixture;

    /// 页面 + 一个被它引用的组件（组件内部有 `<view>`）。
    fn fixture(name: &str, page_wxss: &str) -> Fixture {
        let fixture = Fixture::new(name);
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
                r#"<view class="page"><c/></view>"#,
            )
            .write("pages/index/index.wxss", page_wxss)
            .write("components/c/c.json", "{}")
            .write("components/c/c.wxml", r#"<view class="inner"/>"#)
            .write("components/c/c.wxss", "");
        fixture
    }

    fn check(fixture: &Fixture) -> Vec<Diagnostic> {
        TagSelectorIsolation.check(&fixture.project())
    }

    // ---- positive ----

    #[test]
    fn preprocessor_style_source_is_checked_too() {
        // 老工程的标签选择器全局重置常写在 .scss / .less 里（default-box-sizing.scss），
        // 按后缀跳过等于这条规则在迁移目标工程上从不生效。
        let fixture = Fixture::new("scss-tag");
        fixture
            .write(
                "project.config.json",
                r#"{"miniprogramRoot":"","setting":{"useCompilerPlugins":["sass"]}}"#,
            )
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
                r#"<view class="page"><c/></view>"#,
            )
            .write("pages/index/index.scss", "view { color: red; }")
            .write("components/c/c.json", "{}")
            .write("components/c/c.wxml", r#"<view class="inner"/>"#)
            .write("components/c/c.scss", "");
        let diagnostics = check(&fixture);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].path, "pages/index/index.scss");
    }

    #[test]
    fn page_tag_selector_matching_component_node_is_reported() {
        let fixture = fixture("page-tag", "view { color: red; }");
        let diagnostics = check(&fixture);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].rule, ID);
        assert!(diagnostics[0].message.contains("view"));
        assert!(diagnostics[0].message.contains("components/c/c"));
    }

    #[test]
    fn shared_stylesheet_imported_by_the_component_itself_is_not_reported() {
        // default-box-sizing.scss 这类全局重置常被页面和组件各 import 一次；
        // 组件自己那份照常生效，不存在隔离失效。
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
                r#"<view class="page"><c/></view>"#,
            )
            .write(
                "pages/index/index.wxss",
                r#"@import "../../styles/base.wxss";"#,
            )
            .write("components/c/c.json", "{}")
            .write("components/c/c.wxml", r#"<view class="inner"/>"#)
            .write(
                "components/c/c.wxss",
                r#"@import "../../styles/base.wxss";"#,
            )
            .write("styles/base.wxss", "view { box-sizing: border-box; }");
        assert_eq!(check(&fixture).len(), 0);
    }

    #[test]
    fn page_id_selector_matching_component_node_is_reported() {
        let fixture = fixture("page-id", "#inner { color: red; }");
        fixture.write("components/c/c.wxml", r#"<view id="inner"/>"#);
        let diagnostics = check(&fixture);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(diagnostics[0].message.contains("#inner"));
    }

    #[test]
    fn component_tag_selector_matching_page_node_is_reported() {
        let fixture = fixture("component-tag", "");
        fixture.write(
            "components/c/c.wxss",
            ".x { color: red; }\nview { margin: 0; }",
        );
        let diagnostics = check(&fixture);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].path, "components/c/c.wxss");
    }

    /// 组件样式漏到同一个页面里的兄弟组件：WebView 下 `view {}` 会命中它，
    /// Skyline 下不跨组件，必须报（之前只对比页面，整类漏报）。
    #[test]
    fn component_tag_selector_matching_sibling_component_is_reported() {
        let fixture = fixture("component-sibling", "");
        fixture
            .write(
                "pages/index/index.json",
                r#"{"usingComponents":{"c":"/components/c/c","d":"/components/d/d"}}"#,
            )
            .write("pages/index/index.wxml", "<c/><d/>")
            .write("components/c/c.wxml", r#"<view class="inner"/>"#)
            .write("components/c/c.wxss", "view { margin: 0; }")
            .write("components/d/d.json", "{}")
            .write("components/d/d.wxml", "<view class=\"other\"/>")
            .write("components/d/d.wxss", "");
        let diagnostics = check(&fixture);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].path, "components/c/c.wxss");
        assert!(diagnostics[0].message.contains("components/d/d"));
    }

    /// `styleIsolation: shared` 的组件规则拿到页面那份 scope，页面节点照样命中，
    /// 与 WebView 一致；`apply-shared` 只放开 class，标签选择器仍然不跨作用域。
    #[test]
    fn shared_isolation_reaches_page_but_apply_shared_does_not() {
        let shared = fixture("component-shared", "");
        shared
            .write("components/c/c.json", r#"{"styleIsolation":"shared"}"#)
            .write("components/c/c.wxss", "view { margin: 0; }");
        assert!(check(&shared).is_empty(), "{:?}", check(&shared));

        let applied = fixture("component-apply-shared", "");
        applied
            .write(
                "components/c/c.json",
                r#"{"styleIsolation":"apply-shared"}"#,
            )
            .write("components/c/c.wxss", "view { margin: 0; }");
        assert_eq!(check(&applied).len(), 1);
    }

    /// 组件自己的标签选择器命中的是自己内部的节点，不算跨作用域。
    #[test]
    fn component_tag_selector_matching_own_node_is_fine() {
        let fixture = fixture("component-own", "");
        fixture
            .write("pages/index/index.wxml", "<c/>")
            .write("components/c/c.wxml", r#"<view class="inner"/>"#)
            .write("components/c/c.wxss", "view { margin: 0; }");
        let diagnostics = check(&fixture);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    // ---- 带祖先组合符的选择器 ----

    /// `page view` 是真实工程里的全局重置写法（`weapp-skit-player-plugin` 的
    /// `playlet.wxss:2`）：WebView 的 DOM 根是 `<page>`，这条会命中也组件内部的
    /// `<view>`；Skyline 下页面样式表不跨组件，必须报。注意 `page` 在 WXML 里没有
    /// 对应标签，只能靠页面根补上。
    #[test]
    fn page_descendant_tag_selector_reaches_component_nodes() {
        let fixture = fixture("page-descendant", "page view { box-sizing: content-box; }");
        let diagnostics = check(&fixture);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(diagnostics[0].message.contains("page view"));
        assert!(diagnostics[0].message.contains("components/c/c"));
    }

    /// 类祖先只在组件真的挂在它下面时才算：`.other view` 在 `.other` 外面不该报。
    #[test]
    fn class_ancestor_must_really_apply() {
        let unrelated = fixture("class-ancestor-miss", ".other view { color: red; }");
        assert!(check(&unrelated).is_empty(), "{:?}", check(&unrelated));

        let related = fixture("class-ancestor-hit", "page .page view { color: red; }");
        assert_eq!(check(&related).len(), 1);
    }

    /// 子代组合符要求紧邻的父节点匹配：组件宿主 `c` 夹在中间，`.page > view` 不算。
    #[test]
    fn child_combinator_respects_the_component_host() {
        let fixture = fixture("child-combinator", ".page > view { color: red; }");
        assert!(check(&fixture).is_empty(), "{:?}", check(&fixture));
    }

    /// 主体带 class 的选择器不归这条规则管（隔离只对纯标签 / 纯 id 主体生效）。
    #[test]
    fn class_subject_with_ancestor_is_not_this_rule() {
        let fixture = fixture("class-subject", "page view.inner { color: red; }");
        assert!(check(&fixture).is_empty(), "{:?}", check(&fixture));
    }

    /// 超出受限文法的写法（属性 / 伪类 / 兄弟组合符）跳过，不报。
    #[test]
    fn unsupported_shapes_are_skipped() {
        for (name, selector) in [
            ("shape-attr", "page [data-x] view { color: red; }"),
            ("shape-pseudo", "page view:hover { color: red; }"),
            ("shape-sibling", ".page + view { color: red; }"),
        ] {
            let fixture = fixture(name, selector);
            assert!(check(&fixture).is_empty(), "{selector} 不该报");
        }
    }

    // ---- negative ----

    #[test]
    fn class_selectors_are_fine() {
        assert!(check(&fixture("class", ".page { color: red; }")).is_empty());
    }

    #[test]
    fn app_wxss_is_global_and_never_reported() {
        let fixture = fixture("app-wxss", "");
        fixture.write("app.wxss", "view { color: red; }");
        assert!(check(&fixture).is_empty());
    }

    #[test]
    fn unused_component_is_not_considered() {
        let fixture = fixture("unused", "view { color: red; }");
        fixture.write("pages/index/index.json", r#"{"usingComponents":{}}"#);
        assert!(check(&fixture).is_empty());
    }

    #[test]
    fn page_root_selector_is_fine() {
        assert!(check(&fixture("page-root", "page { color: red; }")).is_empty());
    }

    #[test]
    fn bare_text_tag_without_foreign_match_is_fine() {
        let fixture = fixture("no-foreign", "text { color: red; }");
        fixture.write("components/c/c.wxml", r#"<view class="inner"/>"#);
        assert!(check(&fixture).is_empty());
    }

    #[test]
    fn legacy_disables_tag_reporting_but_not_id() {
        let fixture = fixture("legacy", "view { color: red; }\n#inner { color: blue; }");
        fixture.write("components/c/c.wxml", r#"<view id="inner"/>"#);
        fixture.write(
            "app.json",
            r#"{"pages":["pages/index/index"],"rendererOptions":{"skyline":{"tagNameStyleIsolation":"legacy"}}}"#,
        );
        let diagnostics = check(&fixture);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(diagnostics[0].message.contains("#inner"));
    }
}
