//! 用了 worklet 就必须开 `setting.compileWorklet`（编译 worklet 代码）。
//!
//! worklet 代码要被搬到 UI 线程执行，只有显式开启「编译 worklet 代码」时构建才会做
//! worklet 转换（`minidev compiler-js/src/babel_pipeline.rs:140-142`：
//! `support_worklet = compile_worklet && 源码含 "worklet"`）。关闭时 worklet 不会被
//! 转换，运行时表现为动画直接不生效，而构建过程没有任何提示。
//!
//! 这是工程级判定：只要有一处用到 worklet 就要求开关打开。「用到」包括脚本里任何
//! 带 `'worklet'` 指令的函数，以及 WXML 上的 `worklet:` 属性——后者在函数定义于
//! mixin / 外部模块时是唯一痕迹。
//!
//! 证据：`skyline-overview/references/migration/getting-started.md:17`（使用 worklet 时
//! 勾选「编译 worklet 代码」）、`skyline-config/references/project-config.md`
//! （`compileWorklet` = 是否编译 worklet 代码）。

use serde_json::Value;

use crate::input::json::JsonPositions;
use crate::project::Project;
use crate::rules::wxml::load_doc;
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

use super::unit_wxml;

const MESSAGE: &str = "工程使用了 worklet，但没有开启 setting.compileWorklet（编译 worklet 代码），worklet 代码不会被转换";
const HELP: &str = "在开发者工具 详情 > 本地设置 勾选「编译 worklet 代码」，或把 project.config.json（或 project.private.config.json）的 setting.compileWorklet 设为 true";

pub struct WorkletCompileSetting;

impl WorkletCompileSetting {
    pub const ID: &'static str = "skyline/worklet-compile-setting";
}

impl Rule for WorkletCompileSetting {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "工程使用了 worklet，但 project.config.json 没有开启 setting.compileWorklet",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        // 两份配置都不存在时不判：那是「缺文件」，不是「开关关着」。
        if project.project_config_source.is_empty()
            && project.project_private_config_source.is_empty()
        {
            return Vec::new();
        }
        if !uses_worklet(project) || compile_worklet_enabled(project) {
            return Vec::new();
        }
        // 诊断落在实际写了 `setting` 的那份文件上；private 优先（同 `Project::setting`）。
        let in_private = project.project_private_config.contains_key("setting");
        let (path, source) = if in_private {
            (
                "project.private.config.json",
                &project.project_private_config_source,
            )
        } else {
            ("project.config.json", &project.project_config_source)
        };
        let positions = JsonPositions::new(source);
        let (line, column) = positions
            .key("compileWorklet")
            .or_else(|| positions.key("setting"))
            .unwrap_or((1, 1));
        vec![Diagnostic::new(Self::ID, Severity::Warning, MESSAGE, path, line, column).help(HELP)]
    }
}

fn compile_worklet_enabled(project: &Project) -> bool {
    project.setting("compileWorklet").and_then(Value::as_bool) == Some(true)
}

/// 工程里是否出现 worklet：脚本里的 `'worklet'` 指令，或 WXML 上的 `worklet:` 属性。
///
/// `<import>` / `<include>` 的片段也是工程的一部分：手势绑定常写在被复用的模板里，
/// 只看单元自己的 WXML 会把「用到了 worklet」判成没用到，`compileWorklet` 就漏报。
fn uses_worklet(project: &Project) -> bool {
    let in_scripts = project.scripts.iter().any(|file| {
        file.doc
            .functions()
            .iter()
            .any(|function| function.has_worklet)
    });
    in_scripts
        || unit_wxml(project)
            .into_iter()
            .map(|(path, _)| path)
            .chain(project.template_files.iter().map(String::as_str))
            .any(|path| {
                load_doc(project, path).is_some_and(|doc| {
                    doc.elements().any(|element| {
                        element
                            .attributes
                            .iter()
                            .any(|attribute| attribute.name.starts_with("worklet:"))
                    })
                })
            })
}

#[cfg(test)]
mod tests {
    use crate::project::Project;
    use crate::rules::Rule;
    use crate::rules::testutil::js::{Fixture, messages, single};

    use super::WorkletCompileSetting;

    const WORKLET_JS: &str = "function tick() { 'worklet'; return 1; }";

    fn project(name: &str, config: &str, wxml: &str, js: &str) -> (Fixture, Project) {
        let f = Fixture::new(name);
        f.write("project.config.json", config)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", wxml)
            .write("pages/index/index.js", js);
        let project = f.project();
        (f, project)
    }

    #[test]
    fn worklet_without_compile_worklet_is_reported() {
        let (_f, project) = project(
            "worklet-missing",
            "{\n  \"miniprogramRoot\": \"\",\n  \"setting\": {\n    \"compileWorklet\": false\n  }\n}",
            "<view/>",
            WORKLET_JS,
        );
        let diagnostics = WorkletCompileSetting.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].severity, crate::Severity::Warning);
        assert_eq!(diagnostics[0].path, "project.config.json");
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (4, 5));
        assert!(diagnostics[0].message.contains("setting.compileWorklet"));
        assert!(
            diagnostics[0]
                .help
                .as_deref()
                .unwrap()
                .contains("编译 worklet 代码")
        );
    }

    /// private 配置覆盖 project.config.json（开发者工具的合并顺序）：
    /// public 写着 true、private 写着 false 时实际是关着的。
    #[test]
    fn private_config_overrides_public() {
        let (f, _) = project(
            "worklet-private-override",
            r#"{"miniprogramRoot":"","setting":{"compileWorklet":true}}"#,
            "<view/>",
            WORKLET_JS,
        );
        f.write(
            "project.private.config.json",
            "{\n  \"setting\": {\n    \"compileWorklet\": false\n  }\n}",
        );
        let project = f.project();
        let diagnostics = WorkletCompileSetting.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].path, "project.private.config.json");
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (3, 5));
    }

    /// worklet 的门控与 `es6` 无关：es6 关着但 compileWorklet 开着时正常。
    #[test]
    fn worklet_with_compile_worklet_enabled_is_accepted() {
        let (_f, project) = project(
            "worklet-enabled",
            r#"{"miniprogramRoot":"","setting":{"es6":false,"compileWorklet":true}}"#,
            "<view/>",
            WORKLET_JS,
        );
        assert_eq!(
            messages(&WorkletCompileSetting, &project),
            Vec::<String>::new()
        );
    }

    /// es6 开着也不能替代 compileWorklet。
    #[test]
    fn es6_enabled_does_not_replace_compile_worklet() {
        let (_f, project) = project(
            "worklet-es6-only",
            r#"{"miniprogramRoot":"","setting":{"es6":true}}"#,
            "<view/>",
            WORKLET_JS,
        );
        assert_eq!(messages(&WorkletCompileSetting, &project).len(), 1);
    }

    #[test]
    fn no_worklet_anywhere_is_ignored() {
        let (_f, project) = single("worklet-none", "Page({ onLoad() {} });");
        assert_eq!(
            messages(&WorkletCompileSetting, &project),
            Vec::<String>::new()
        );
    }

    #[test]
    fn missing_setting_is_reported_at_the_top() {
        let (_f, project) = project(
            "worklet-no-setting",
            r#"{"miniprogramRoot":""}"#,
            "<view/>",
            WORKLET_JS,
        );
        let diagnostics = WorkletCompileSetting.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (1, 1));
    }

    #[test]
    fn setting_without_compile_worklet_points_at_the_setting_key() {
        let (_f, project) = project(
            "worklet-setting-only",
            "{\n  \"setting\": {\n    \"minified\": true\n  }\n}",
            "<view/>",
            WORKLET_JS,
        );
        let diagnostics = WorkletCompileSetting.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (2, 3));
    }

    #[test]
    fn wxml_worklet_attribute_alone_counts_as_usage() {
        // 函数定义在 mixin / 外部模块，脚本里看不到 `'worklet'`。
        let (_f, project) = project(
            "worklet-wxml-only",
            r#"{"miniprogramRoot":""}"#,
            r#"<pan-gesture-handler worklet:ongesture="onPan"/>"#,
            "Behavior({});",
        );
        assert_eq!(messages(&WorkletCompileSetting, &project).len(), 1);
    }

    #[test]
    fn worklet_attribute_in_an_imported_template_counts_as_usage() {
        // 手势绑定写在被 `<import>` 复用的模板里，页面自己的 wxml 是干净的。
        let f = Fixture::new("worklet-imported-template");
        f.write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write(
                "pages/index/index.wxml",
                r#"<import src="/templates/pan"/><view/>"#,
            )
            .write("pages/index/index.js", "Page({});")
            .write(
                "templates/pan.wxml",
                r#"<pan-gesture-handler worklet:ongesture="onPan"/>"#,
            );
        let project = f.project();
        assert!(
            project
                .template_files
                .iter()
                .any(|p| p.contains("templates/pan"))
        );
        assert_eq!(messages(&WorkletCompileSetting, &project).len(), 1);
    }

    #[test]
    fn missing_config_file_is_not_reported() {
        let f = Fixture::new("worklet-no-config");
        f.write(
            "app.json",
            r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
        )
        .write("pages/index/index.json", "{}")
        .write("pages/index/index.wxml", "<view/>")
        .write("pages/index/index.js", WORKLET_JS);
        let project = f.project();
        assert_eq!(
            messages(&WorkletCompileSetting, &project),
            Vec::<String>::new()
        );
    }

    #[test]
    fn compile_worklet_as_a_string_is_not_enabled() {
        let (_f, project) = project(
            "worklet-string",
            r#"{"miniprogramRoot":"","setting":{"compileWorklet":"true"}}"#,
            "<view/>",
            WORKLET_JS,
        );
        assert_eq!(messages(&WorkletCompileSetting, &project).len(), 1);
    }
}
