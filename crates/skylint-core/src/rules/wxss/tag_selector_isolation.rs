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

use serde_json::{Map, Value};

use crate::input::wxml::WxmlDoc;
use crate::project::{Project, Scope};
use crate::rules::RuleMeta;
use crate::{Diagnostic, Severity};

use super::selector::SimpleSelector;
use super::visit_rules;

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
        let mut docs: HashMap<&str, Option<WxmlDoc>> = HashMap::new();

        visit_rules(project, |sheet, rule, _out| {
            let doc = &sheet.doc;
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
                        let pages = project.component_pages(path);
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
                if foreign.is_empty() {
                    continue;
                }
                for selector in &rule.selectors {
                    let Some(simple) = SimpleSelector::parse(selector) else {
                        continue;
                    };
                    let is_tag = simple.is_tag_only();
                    if !is_tag && !simple.is_id_only() {
                        continue;
                    }
                    // legacy 放开的是标签选择器；id 选择器无论配置如何都不跨组件
                    if is_tag && legacy {
                        continue;
                    }
                    let Some((unit, foreign_tag)) =
                        find_foreign(project, &foreign, &simple, &mut docs)
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
fn find_foreign<'a>(
    project: &'a Project,
    units: &[&'a str],
    selector: &SimpleSelector,
    docs: &mut HashMap<&'a str, Option<WxmlDoc>>,
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
            if selector.matches(element) == super::selector::Match::Yes {
                return Some((unit.to_string(), element.tag.clone()));
            }
        }
    }
    None
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
