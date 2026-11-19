//! `general/transparent-background`：组件把背景色绑在属性上，调用处没传 → 背景透明。
//!
//! 典型形态是导航容器：组件 wxml 里 `<view class="nav-bar" style="background: {{background}}">`，
//! `background` 属性默认空串，调用方 `<nav-bar title="…">` 不传时 `background: ''`
//! 是无效值，导航区透出页面底色。WebView 时代这块被原生导航条盖着，迁到
//! `navigationStyle: custom` 之后才露出来，所以这条规则不限定渲染器。
//!
//! 只报「样式里确实把背景绑到这个属性」的组件：属性没传时最终背景取决于组件给的默认值
//! （空则透明），提示的是「这里可能少传了一个值」，不是断言一定画错。默认值是否为空不
//! 参与判定——那要读组件 JS 的属性声明，判错就是成片的误报。

use std::collections::BTreeMap;

use crate::diag::{Diagnostic, Severity};
use crate::project::Project;
use crate::rules::wxml::{element_diagnostic, for_each_element, load_doc};
use crate::rules::{Rule, RuleMeta, Target};

pub const ID: &str = "general/transparent-background";

pub struct TransparentBackground;

impl Rule for TransparentBackground {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Info,
            description: "组件把背景色绑在属性上，调用处没传该属性：背景取默认值，为空时透明",
        }
    }

    fn target(&self) -> Target {
        Target::Both
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let bound = bound_background_properties(project);
        if bound.is_empty() {
            return Vec::new();
        }

        let mut out = Vec::new();
        for_each_element(project, self.target(), |site| {
            let Some(targets) = site.project.tag_targets.get(site.doc.path()) else {
                return;
            };
            let Some(component) = targets.get(&site.element.tag.to_ascii_lowercase()) else {
                return;
            };
            let Some(properties) = bound.get(component) else {
                return;
            };
            for property in properties {
                if site
                    .element
                    .attributes
                    .iter()
                    .any(|attribute| attribute.name == *property)
                {
                    continue;
                }
                out.push(
                    element_diagnostic(
                        ID,
                        site.doc,
                        site.element,
                        site.doc.path(),
                        Severity::Info,
                        format!(
                            "`<{}>` 没传 `{property}`：组件把背景色绑在这个属性上，最终背景取决于组件默认值（为空时透明）",
                            site.element.tag
                        ),
                    )
                    .help(format!(
                        "显式传 `{property}=\"#fff\"`（或页面实际底色）；导航类组件建议在组件里给非空默认值"
                    )),
                );
            }
        });
        out
    }
}

/// 组件 WXML 里「背景色绑到属性」的声明：组件路径 → 属性名列表。
fn bound_background_properties(project: &Project) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for component in &project.components {
        let Some(doc) = load_doc(project, &format!("{}.wxml", component.path)) else {
            continue;
        };
        let mut properties: Vec<String> = doc
            .elements()
            .flat_map(|element| element.attributes.iter())
            .filter(|attribute| attribute.name == "style")
            .flat_map(|attribute| match attribute.value.as_deref() {
                Some(value) => bound_property(value).into_iter().collect::<Vec<_>>(),
                // 动态属性（`style="… background: {{x}}"`）`value` 是 None，回源码取原文。
                None => bound_property(raw_value(doc.slice(attribute.start, attribute.end))),
            })
            .collect();
        if !properties.is_empty() {
            properties.sort();
            properties.dedup();
            out.insert(component.path.clone(), properties);
        }
    }
    out
}

/// 从属性的源码原文（`style="…"`）里取出取值部分。
fn raw_value(attribute: &str) -> &str {
    attribute
        .split_once('=')
        .map(|(_, value)| value.trim().trim_matches(['"', '\'']))
        .unwrap_or("")
}

/// 从一条 `style` 里取出 `background: {{prop}}` / `background-color: {{prop}}` 的属性名。
fn bound_property(style: &str) -> Vec<String> {
    let mut out = Vec::new();
    for declaration in style.split(';') {
        let Some((name, value)) = declaration.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        if name != "background" && name != "background-color" {
            continue;
        }
        let Some(inner) = value.trim().strip_prefix("{{") else {
            continue;
        };
        let Some(inner) = inner.strip_suffix("}}") else {
            continue;
        };
        let inner = inner.trim();
        if !inner.is_empty()
            && inner
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '_' | '$'))
        {
            out.push(inner.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{ID, TransparentBackground, bound_property};
    use crate::rules::testutil::wxml::{Fixture, check_rule};

    fn counts(fixture: &Fixture) -> usize {
        check_rule(fixture, &TransparentBackground)
            .iter()
            .filter(|diagnostic| diagnostic.rule == ID)
            .count()
    }

    fn fixture(name: &str, call: &str) -> Fixture {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                r#"{"navigationStyle":"custom","usingComponents":{"nav-bar":"/components/nav/index"}}"#,
            )
            .write("pages/index/index.wxml", call)
            .write("components/nav/index.json", r#"{"component":true}"#)
            .write(
                "components/nav/index.wxml",
                r#"<view style="height: 88rpx; background: {{background}}"><slot/></view>"#,
            )
            .write(
                "components/nav/index.js",
                "Component({ properties: { background: { type: String, value: '' } } });",
            );
        fixture
    }

    #[test]
    fn omitted_background_is_reported() {
        let fixture = fixture("nav-omitted", r#"<nav-bar title="标题"/>"#);
        assert_eq!(counts(&fixture), 1);
    }

    #[test]
    fn passed_background_is_skipped() {
        let fixture = fixture(
            "nav-passed",
            r##"<nav-bar title="标题" background="#fff"/>"##,
        );
        assert_eq!(counts(&fixture), 0);
    }

    #[test]
    fn plain_element_is_skipped() {
        let fixture = fixture("nav-plain", r#"<view class="x"/>"#);
        assert_eq!(counts(&fixture), 0);
    }

    #[test]
    fn parses_background_binding_only() {
        assert_eq!(
            bound_property("height: 88rpx; background: {{background}}"),
            vec!["background".to_string()]
        );
        assert_eq!(
            bound_property("background-color:{{ color }}"),
            vec!["color".to_string()]
        );
        assert!(bound_property("background: #fff").is_empty());
        assert!(bound_property("background: rgba(0,0,0,.5)").is_empty());
        assert!(bound_property("color: {{textColor}}").is_empty());
    }
}
