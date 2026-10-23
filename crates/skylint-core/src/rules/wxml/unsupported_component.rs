//! Skyline 下不可用的组件（如 web-view / movable-view / editor）。

use std::collections::{HashMap, HashSet};

use crate::diag::{Diagnostic, Severity};
use crate::project::Project;
use crate::rules::data_table::{self, Columns};
use crate::rules::{Rule, RuleMeta};

use super::{COMPONENTS_DATA, element_diagnostic, load_doc, wxml_files};

struct ComponentTable {
    supported: HashSet<&'static str>,
    unsupported: HashMap<&'static str, &'static str>,
}

impl ComponentTable {
    fn load() -> Self {
        let parsed = data_table::read(COMPONENTS_DATA, split, |section| match section {
            Some("unsupported") => Columns::new(2, "`标签 | 替代方案` 两列"),
            _ => Columns::new(1, "一个标签"),
        })
        .expect("组件支持表格式错误");

        let mut supported = HashSet::new();
        let mut unsupported = HashMap::new();
        for row in parsed.rows {
            match row.section {
                Some("supported") => {
                    supported.insert(row.fields[0]);
                }
                Some("unsupported") => {
                    unsupported.insert(row.fields[0], row.fields[1]);
                }
                _ => {}
            }
        }
        Self {
            supported,
            unsupported,
        }
    }

    /// 黑名单命中才返回替代方案；白名单与未知自定义组件都不报。
    fn replacement(&self, tag: &str) -> Option<&'static str> {
        if self.supported.contains(tag) {
            return None;
        }
        self.unsupported.get(tag).copied()
    }
}

/// `[supported]` 一行一个标签；`[unsupported]` 按 `|` 切 `标签 | 替代方案`。
fn split<'a>(section: Option<&'a str>, line: &'a str) -> Vec<&'a str> {
    match section {
        Some("unsupported") => line.split('|').map(str::trim).collect(),
        _ => vec![line],
    }
}

pub struct UnsupportedComponent;

impl UnsupportedComponent {
    pub const ID: &'static str = "skyline/unsupported-component";
}

impl Rule for UnsupportedComponent {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Error,
            description: "Skyline 下不可用的组件（如 web-view / movable-view / editor）",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let table = ComponentTable::load();
        let mut diagnostics = Vec::new();
        for path in wxml_files(project) {
            let Some(doc) = load_doc(project, path) else {
                continue;
            };
            for element in doc.elements() {
                let tag = element.tag.to_ascii_lowercase();
                let Some(replacement) = table.replacement(&tag) else {
                    continue;
                };
                diagnostics.push(
                    element_diagnostic(
                        Self::ID,
                        &doc,
                        element,
                        path,
                        Severity::Error,
                        format!("Skyline 不支持组件 `<{}>`", element.tag),
                    )
                    .help(replacement),
                );
            }
        }
        diagnostics
    }
}

#[cfg(test)]
mod tests {
    use crate::rules::testutil::wxml::{Fixture, check_rule, project};
    use super::UnsupportedComponent;

    /// 页面 json 显式 `"renderer": "webview"`：这页不迁 Skyline，整页跳过。
    #[test]
    fn explicit_webview_page_is_skipped() {
        let fixture = Fixture::new("opt-out");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index","pages/webview/webview"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", "<view>skyline</view>")
            .write("pages/webview/webview.json", r#"{"renderer":"webview"}"#)
            .write(
                "pages/webview/webview.wxml",
                r#"<web-view src="https://x"/>"#,
            );
        let diagnostics = check_rule(&fixture, &UnsupportedComponent);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    /// app 级 webview、页面自己没表态：只是还没迁，仍然检查。
    #[test]
    fn inherited_webview_page_is_still_checked() {
        let fixture = Fixture::new("not-migrated");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"webview"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", r#"<web-view src="https://x"/>"#);
        let diagnostics = check_rule(&fixture, &UnsupportedComponent);
        assert_eq!(diagnostics.len(), 1);
    }

    /// 内联 `<template name>` 的定义体同样要查：它是文件里真实存在的标签。
    #[test]
    fn component_inside_inline_template_is_reported() {
        let fixture = Fixture::new("inline-template");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write(
                "pages/index/index.wxml",
                r#"<template name="card"><web-view src="https://x"/></template><template is="card"/>"#,
            );
        let diagnostics = check_rule(&fixture, &UnsupportedComponent);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(diagnostics[0].message.contains("web-view"));
    }

    /// 跳过是按页的：同一工程里没表态的页面照样检查。
    #[test]
    fn opt_out_does_not_leak_to_sibling_pages() {
        let fixture = Fixture::new("opt-out-sibling");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/a/a","pages/b/b"],"renderer":"skyline"}"#,
            )
            .write("pages/a/a.json", r#"{"renderer":"webview"}"#)
            .write("pages/a/a.wxml", r#"<web-view src="https://a"/>"#)
            .write("pages/b/b.json", "{}")
            .write("pages/b/b.wxml", r#"<web-view src="https://b"/>"#);
        let diagnostics = check_rule(&fixture, &UnsupportedComponent);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].path, "pages/b/b.wxml");
    }

    #[test]
    fn unsupported_components_are_reported() {
        let diagnostics = check_rule(
            &project(
                "unsupported",
                r#"<view>
  <web-view src="https://example.com"/>
  <movable-view/>
  <editor/>
  <progress/>
  <xr-frame/>
</view>"#,
            ),
            &UnsupportedComponent,
        );
        assert_eq!(diagnostics.len(), 5);
        assert!(
            diagnostics
                .iter()
                .all(|d| d.severity == crate::Severity::Error)
        );

        let web_view = diagnostics
            .iter()
            .find(|d| d.message.contains("web-view"))
            .unwrap();
        assert!(
            web_view
                .help
                .as_deref()
                .unwrap()
                .contains(r#""renderer": "webview""#)
        );

        let movable_view = diagnostics
            .iter()
            .find(|d| d.message.contains("movable-view"))
            .unwrap();
        assert!(movable_view.help.as_deref().unwrap().contains("worklet"));
    }

    /// 知识库把组件名拼成 `keyboard-accessary`；官方名是 keyboard-accessory，
    /// 按错拼收录等于这条永远命中不了。
    #[test]
    fn keyboard_accessory_is_reported_under_its_official_name() {
        let diagnostics = check_rule(
            &project("keyboard", r#"<keyboard-accessory/>"#),
            &UnsupportedComponent,
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(diagnostics[0].message.contains("keyboard-accessory"));
    }

    #[test]
    fn every_unsupported_occurrence_is_reported() {
        let diagnostics = check_rule(
            &project("unsupported-twice", "<view><web-view/><web-view/></view>"),
            &UnsupportedComponent,
        );
        assert_eq!(diagnostics.len(), 2);
    }

    #[test]
    fn supported_components_are_allowed() {
        let diagnostics = check_rule(
            &project(
                "supported",
                "<view><form/><scroll-view/><navigator/><rich-text/><page-meta/><span/><sticky-header/><list-view/></view>",
            ),
            &UnsupportedComponent,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn unknown_custom_component_is_ignored() {
        let diagnostics = check_rule(
            &project("custom-component", "<my-widget/>"),
            &UnsupportedComponent,
        );
        assert!(diagnostics.is_empty());
    }

    /// `<import>` 引入的模板不属于任何单元，但结构规则仍要检查它。
    #[test]
    fn imported_template_is_scanned() {
        let fixture = Fixture::new("imported-template");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write(
                "pages/index/index.wxml",
                r#"<import src="../../templates/pagination" /><view/>"#,
            )
            .write(
                "templates/pagination.wxml",
                r#"<web-view src="https://x"/>"#,
            );
        let diagnostics = check_rule(&fixture, &UnsupportedComponent);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].path, "templates/pagination.wxml");
    }

    /// `<include>` 同样覆盖；模板 import 模板（A→B→C）要递归到最里层。
    #[test]
    fn included_templates_are_scanned_recursively() {
        let fixture = Fixture::new("included-template");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write(
                "pages/index/index.wxml",
                r#"<include src="../../templates/a"/>"#,
            )
            .write("templates/a.wxml", r#"<include src="b"/>"#)
            .write("templates/b.wxml", r#"<import src="./c.wxml"/>"#)
            .write("templates/c.wxml", r#"<web-view src="https://x"/>"#);
        let diagnostics = check_rule(&fixture, &UnsupportedComponent);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].path, "templates/c.wxml");
    }

    /// 同一模板被多个页面引用时只扫一次。
    #[test]
    fn template_shared_by_two_pages_is_scanned_once() {
        let fixture = Fixture::new("shared-template");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/a/a","pages/b/b"],"renderer":"skyline"}"#,
            )
            .write("pages/a/a.json", "{}")
            .write("pages/a/a.wxml", r#"<import src="../../tpl/x"/>"#)
            .write("pages/b/b.json", "{}")
            .write("pages/b/b.wxml", r#"<import src="../../tpl/x"/>"#)
            .write("tpl/x.wxml", r#"<web-view src="https://x"/>"#);
        let diagnostics = check_rule(&fixture, &UnsupportedComponent);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    }
}
