//! 用了 worklet 就必须开 `setting.es6`（把 JS 编译成 ES5）。
//!
//! worklet 代码要被搬到 UI 线程执行，编译产物必须是 ES5；开发者工具的「将 JS 编译成
//! ES5」关闭时（`project.config.json` 里 `setting.es6` 不为 `true`），worklet 不会被
//! 正确编译，运行时表现为动画直接不生效，而构建过程没有任何提示。
//!
//! 这是工程级判定：只要有一处用到 worklet 就要求开关打开。「用到」包括脚本里任何
//! 带 `'worklet'` 指令的函数，以及 WXML 上的 `worklet:` 属性——后者在函数定义于
//! mixin / 外部模块时是唯一痕迹。
//!
//! 证据：`skyline-worklet/references/core/worklet-overview.md:11`（前置条件：勾选
//! 「将 JS 编译成 ES5」）、`skyline-overview/references/migration/getting-started.md:17`
//! （使用 worklet 时勾选「编译 worklet 代码」）、`skyline-config/references/project-config.md:42`
//! （`es6` = 是否启用 ES6 转 ES5）。

use serde_json::Value;

use crate::input::json::JsonPositions;
use crate::project::Project;
use crate::rules::wxml::load_doc;
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

use super::unit_wxml;

const MESSAGE: &str = "工程使用了 worklet，但没有开启 setting.es6（将 JS 编译成 ES5），worklet 代码不会被正确编译";
const HELP: &str = "在开发者工具 详情 > 本地设置 勾选「将 JS 编译成 ES5」，或把 project.config.json（或 project.private.config.json）的 setting.es6 设为 true";

pub struct WorkletEs5;

impl WorkletEs5 {
    pub const ID: &'static str = "skyline/worklet-es5";
}

impl Rule for WorkletEs5 {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "工程使用了 worklet，但 project.config.json 没有开启 setting.es6",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        // 两份配置都不存在时不判：那是「缺文件」，不是「开关关着」。
        if project.project_config_source.is_empty()
            && project.project_private_config_source.is_empty()
        {
            return Vec::new();
        }
        if !uses_worklet(project) || es6_enabled(project) {
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
            .key("es6")
            .or_else(|| positions.key("setting"))
            .unwrap_or((1, 1));
        vec![
            Diagnostic::new(Self::ID, Severity::Warning, MESSAGE, path, line, column).help(HELP),
        ]
    }
}

fn es6_enabled(project: &Project) -> bool {
    project.setting("es6").and_then(Value::as_bool) == Some(true)
}

/// 工程里是否出现 worklet：脚本里的 `'worklet'` 指令，或 WXML 上的 `worklet:` 属性。
fn uses_worklet(project: &Project) -> bool {
    let in_scripts = project.scripts.iter().any(|file| {
        file.doc
            .functions()
            .iter()
            .any(|function| function.has_worklet)
    });
    in_scripts
        || unit_wxml(project).into_iter().any(|(path, _)| {
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
    use crate::rules::js::testutil::{Fixture, messages, single};

    use super::WorkletEs5;

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
    fn worklet_without_es6_is_reported() {
        let (_f, project) = project(
            "es5-missing",
            "{\n  \"miniprogramRoot\": \"\",\n  \"setting\": {\n    \"es6\": false\n  }\n}",
            "<view/>",
            WORKLET_JS,
        );
        let diagnostics = WorkletEs5.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].severity, crate::Severity::Warning);
        assert_eq!(diagnostics[0].path, "project.config.json");
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (4, 5));
        assert!(diagnostics[0].message.contains("setting.es6"));
    }

    /// private 配置覆盖 project.config.json（开发者工具的合并顺序）：
    /// public 写着 true、private 写着 false 时实际是关着的。
    #[test]
    fn private_config_overrides_public() {
        let (f, _) = project(
            "es5-private-override",
            r#"{"miniprogramRoot":"","setting":{"es6":true}}"#,
            "<view/>",
            WORKLET_JS,
        );
        f.write(
            "project.private.config.json",
            "{\n  \"setting\": {\n    \"es6\": false\n  }\n}",
        );
        let project = f.project();
        let diagnostics = WorkletEs5.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].path, "project.private.config.json");
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (3, 5));
    }

    #[test]
    fn worklet_with_es6_enabled_is_accepted() {
        let (_f, project) = project(
            "es5-enabled",
            r#"{"miniprogramRoot":"","setting":{"es6":true}}"#,
            "<view/>",
            WORKLET_JS,
        );
        assert_eq!(messages(&WorkletEs5, &project), Vec::<String>::new());
    }

    #[test]
    fn no_worklet_anywhere_is_ignored() {
        let (_f, project) = single("es5-no-worklet", "Page({ onLoad() {} });");
        assert_eq!(messages(&WorkletEs5, &project), Vec::<String>::new());
    }

    #[test]
    fn missing_setting_is_reported_at_the_top() {
        let (_f, project) = project(
            "es5-no-setting",
            r#"{"miniprogramRoot":""}"#,
            "<view/>",
            WORKLET_JS,
        );
        let diagnostics = WorkletEs5.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (1, 1));
    }

    #[test]
    fn setting_without_es6_points_at_the_setting_key() {
        let (_f, project) = project(
            "es5-setting-only",
            "{\n  \"setting\": {\n    \"minified\": true\n  }\n}",
            "<view/>",
            WORKLET_JS,
        );
        let diagnostics = WorkletEs5.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (2, 3));
    }

    #[test]
    fn wxml_worklet_attribute_alone_counts_as_usage() {
        // 函数定义在 mixin / 外部模块，脚本里看不到 `'worklet'`。
        let (_f, project) = project(
            "es5-wxml-only",
            r#"{"miniprogramRoot":""}"#,
            r#"<pan-gesture-handler worklet:ongesture="onPan"/>"#,
            "Behavior({});",
        );
        assert_eq!(messages(&WorkletEs5, &project).len(), 1);
    }

    #[test]
    fn missing_config_file_is_not_reported() {
        let f = Fixture::new("es5-no-config");
        f.write(
            "app.json",
            r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
        )
        .write("pages/index/index.json", "{}")
        .write("pages/index/index.wxml", "<view/>")
        .write("pages/index/index.js", WORKLET_JS);
        let project = f.project();
        assert_eq!(messages(&WorkletEs5, &project), Vec::<String>::new());
    }

    #[test]
    fn es6_as_a_string_is_not_enabled() {
        let (_f, project) = project(
            "es5-string",
            r#"{"miniprogramRoot":"","setting":{"es6":"true"}}"#,
            "<view/>",
            WORKLET_JS,
        );
        assert_eq!(messages(&WorkletEs5, &project).len(), 1);
    }
}
