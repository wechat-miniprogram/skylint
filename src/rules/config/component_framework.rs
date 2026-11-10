//! `skyline/component-framework`：Skyline 页面的 `componentFramework` 解析结果不是
//! `glass-easel`，开发者工具会拒绝编译（`minidev build` 不校验这条）。
//!
//! 判据来自 minidev `compiler-graph/src/json/mutate.rs:496-540`
//! （`is_glass_easel_component` / `check_component_framework`），解析顺序：
//!
//! 1. 页面 json 自己的 `componentFramework`（是字符串时直接决定）；
//! 2. 所属分包的 `subPackages[].componentFramework`：按 `root` 前缀匹配、只看声明了
//!    该键的分包，多个命中取 `root` 字典序第一个；
//! 3. app.json 的 `componentFramework`。
//!
//! 页面级优先，所以不能只看 app 级——businessview 就有 4 个页面在页面 json 里单独
//! 写了 `componentFramework`。插件视图不判：插件页走 `compile_plugin_json`，不经过
//! `compile_page_json`，这条校验在插件视图里不会让构建失败。

use serde_json::Value;

use crate::input::json::JsonPositions;
use crate::project::Project;
use crate::rules::{Rule, RuleMeta, Target};
use crate::{Diagnostic, Severity};

pub struct ComponentFramework;

impl ComponentFramework {
    pub const ID: &'static str = "skyline/component-framework";
}

/// 解析结果来自哪一层，决定消息说明与诊断落点。
///
/// 值随来源一起携带：页面级只可能是字符串（非字符串会继续往下一层解析）。
enum Source<'a> {
    Page(&'a str),
    SubPackage { root: &'a str, value: &'a str },
    App(Option<&'a str>),
}

impl Source<'_> {
    fn value(&self) -> Option<&str> {
        match self {
            Self::Page(value) => Some(value),
            Self::SubPackage { value, .. } => Some(value),
            Self::App(value) => *value,
        }
    }
}

impl Rule for ComponentFramework {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Error,
            description: "Skyline 页面的 componentFramework 解析结果必须是 glass-easel",
        }
    }

    fn target(&self) -> Target {
        Target::Skyline
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        if !super::applicable_project_level(project) {
            return Vec::new();
        }
        let mut out = Vec::new();
        for page in project.pages.iter().filter(|page| page.is_skyline) {
            let source = resolve(project, &page.route);
            if source.value() == Some("glass-easel") {
                continue;
            }

            let page_path = format!("{}.json", page.route);
            let (path, at) = match &source {
                Source::Page(_) => (
                    page_path.as_str(),
                    JsonPositions::new(
                        project
                            .page_json_source
                            .get(&page.route)
                            .map_or("", String::as_str),
                    )
                    .key("componentFramework"),
                ),
                Source::SubPackage { .. } | Source::App(_) => (
                    project.app_json_path.as_str(),
                    JsonPositions::new(&project.app_json_source).key("componentFramework"),
                ),
            };

            let detail = match &source {
                Source::Page(value) => {
                    format!("页面 json 写了 `componentFramework: \"{value}\"`")
                }
                Source::SubPackage { root, value } => {
                    format!("分包 `{root}` 声明了 `componentFramework: \"{value}\"`")
                }
                Source::App(Some(value)) => {
                    format!("app.json 写了 `componentFramework: \"{value}\"`")
                }
                Source::App(None) => {
                    "app.json 未声明 `componentFramework`（缺省值不是 `glass-easel`）".to_string()
                }
            };
            super::error(
                &mut out,
                Self::ID,
                path,
                at,
                format!(
                    "页面 `{page_path}` 使用 Skyline 渲染，但{detail}，开发者工具会拒绝编译（`minidev build` 不校验这条）"
                ),
                "写 `componentFramework: \"glass-easel\"`（页面 json 优先，其次所属分包的 subPackages[].componentFramework，最后 app.json）；Skyline 只有 glass-easel 一套节点树实现",
            );
        }
        out
    }
}

/// 页面 json → 所属分包 → app.json，与 minidev 的解析顺序逐行对应。
fn resolve<'a>(project: &'a Project, route: &str) -> Source<'a> {
    if let Some(value) = project
        .page_json
        .get(route)
        .and_then(|json| json.get("componentFramework"))
        .and_then(Value::as_str)
    {
        return Source::Page(value);
    }
    if let Some((root, value)) = sub_package_declaration(project, route) {
        return Source::SubPackage { root, value };
    }
    Source::App(
        project
            .app_json
            .get("componentFramework")
            .and_then(Value::as_str),
    )
}

/// 按 root 前缀匹配声明了 `componentFramework` 的分包；多个命中取字典序第一个。
///
/// minidev 对 `root` 只去前导 `/`（不处理后缀 `/`），`starts_with` 也不是路径边界
/// 匹配——这里照抄，避免和构建行为分叉。
fn sub_package_declaration<'a>(project: &'a Project, route: &str) -> Option<(&'a str, &'a str)> {
    let mut declared: Vec<(&str, &str)> = ["subPackages", "subpackages"]
        .iter()
        .filter_map(|key| project.app_json.get(*key).and_then(Value::as_array))
        .flatten()
        .filter_map(|sub| {
            let root = sub.get("root")?.as_str()?.trim_start_matches('/');
            let value = sub.get("componentFramework")?.as_str()?;
            Some((root, value))
        })
        .collect();
    declared.sort_unstable_by_key(|(root, _)| *root);
    declared
        .into_iter()
        .find(|(root, _)| !root.is_empty() && route.starts_with(root))
}

#[cfg(test)]
mod tests {
    use crate::Severity;
    use crate::project::Project;
    use crate::rules::Rule;
    use crate::rules::testutil::Fixture;

    use super::ComponentFramework;

    #[test]
    fn reports_explicit_exparser_page() {
        let fixture = Fixture::new("framework-exparser");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline","componentFramework":"exparser"}"#,
            )
            .write("pages/index/index.json", r#"{"componentFramework":"exparser"}"#)
            .write("pages/index/index.wxml", "<view/>");
        let diags = ComponentFramework.check(&fixture.project());
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(diags[0].path, "pages/index/index.json");
        assert!(
            diags[0].message.contains("exparser"),
            "{}",
            diags[0].message
        );
    }

    #[test]
    fn accepts_glass_easel_from_app() {
        let fixture = Fixture::new("framework-app-glass");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline","componentFramework":"glass-easel"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", "<view/>");
        assert!(ComponentFramework.check(&fixture.project()).is_empty());
    }

    /// 页面级覆盖 app 级：app 是 exparser，页面写 glass-easel 就不报。
    #[test]
    fn page_level_glass_easel_overrides_app_exparser() {
        let fixture = Fixture::new("framework-page-override");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline","componentFramework":"exparser"}"#,
            )
            .write(
                "pages/index/index.json",
                r#"{"componentFramework":"glass-easel"}"#,
            )
            .write("pages/index/index.wxml", "<view/>");
        assert!(ComponentFramework.check(&fixture.project()).is_empty());
    }

    /// 分包覆盖 app 级：分包声明了 glass-easel，属于它的页面就不报。
    #[test]
    fn sub_package_glass_easel_overrides_app_exparser() {
        let fixture = Fixture::new("framework-subpackage");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":[],"renderer":"skyline","componentFramework":"exparser","subPackages":[{"root":"pkg","pages":["pages/a/a"],"componentFramework":"glass-easel"}]}"#,
            )
            .write("pkg/pages/a/a.json", "{}")
            .write("pkg/pages/a/a.wxml", "<view/>");
        assert!(ComponentFramework.check(&fixture.project()).is_empty());
    }

    /// 分包覆盖成 exparser 时，即使 app 级是 glass-easel 也要报，且落在 app.json。
    #[test]
    fn sub_package_exparser_overrides_app_glass_easel() {
        let fixture = Fixture::new("framework-subpackage-exparser");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":[],"renderer":"skyline","componentFramework":"glass-easel","subPackages":[{"root":"pkg","pages":["pages/a/a"],"componentFramework":"exparser"}]}"#,
            )
            .write("pkg/pages/a/a.json", "{}")
            .write("pkg/pages/a/a.wxml", "<view/>");
        let diags = ComponentFramework.check(&fixture.project());
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].path, "app.json");
        assert!(diags[0].message.contains("pkg"), "{}", diags[0].message);
    }

    /// 多个分包命中时取 root 字典序第一个：`pkg` 排在 `pkg/sub` 前，其 glass-easel 生效。
    #[test]
    fn multiple_matching_sub_packages_take_lexicographically_first_root() {
        let fixture = Fixture::new("framework-subpackage-order");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":[],"renderer":"skyline","subPackages":[{"root":"pkg/sub","pages":["deep/a/a"],"componentFramework":"exparser"},{"root":"pkg","pages":["pages/a/a"],"componentFramework":"glass-easel"}]}"#,
            )
            .write("pkg/sub/deep/a/a.json", "{}")
            .write("pkg/sub/deep/a/a.wxml", "<view/>");
        assert!(ComponentFramework.check(&fixture.project()).is_empty());
    }

    /// 非 Skyline 页不校验；插件页也不校验（不走 `compile_page_json`）。
    #[test]
    fn skips_webview_pages_and_plugin_view() {
        let fixture = Fixture::new("framework-skip");
        fixture
            .write(
                "project.config.json",
                r#"{"miniprogramRoot":"","pluginRoot":"plugin"}"#,
            )
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"webview","componentFramework":"exparser"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", "<view/>")
            .write("plugin/plugin.json", r#"{"pages":{"a":"pages/a/a"}}"#)
            .write(
                "plugin/pages/a/a.json",
                r#"{"renderer":"skyline","componentFramework":"exparser"}"#,
            )
            .write("plugin/pages/a/a.wxml", "<view/>");
        let views = Project::load_views(fixture.root()).unwrap();
        let host = views.iter().find(|view| !view.plugin_view).unwrap();
        let plugin = views.iter().find(|view| view.plugin_view).unwrap();
        assert!(ComponentFramework.check(host).is_empty());
        assert!(ComponentFramework.check(plugin).is_empty());
    }
}
