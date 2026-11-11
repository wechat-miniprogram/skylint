//! `skyline/autoprefix-build`：prod 构建的 autoprefixer 会给 Skyline 样式注入 `-webkit-`。
//!
//! 开发者工具的 `setting.postcss` 与 `setting.minifyWXSS` 是「或」关系：任一为真就注册
//! wxss 插件（`pluginconfig.ts:48-51`），而插件里的 `autoPrefix` 默认 `true`
//! （`plugins/base/wxss.ts:100`），按 `iOS >= 8, Chrome >= 37` 补前缀且 `remove: false`。
//! 更麻烦的是它**只在 prod 构建跑**（`wxss.ts:143-148` 对 `resultType === 'dev'` 直接
//! 返回），所以本地预览与开发者工具里永远复现不出来。
//!
//! 注入结果里真正会坏事的只有两种：`transition` **值**里的 `-webkit-transform`
//! （fp 属性名闭集里没有前缀，整条声明回滚）和 `@-webkit-keyframes`（Skyline 不解析
//! 带前缀的 at-rule）。两者都只在产物里出现，源码看不到——所以这条规则直接看 setting，
//! 给一条**工程级提示**：该工程的 prod 产物带有这个风险。
//!
//! 不报「一定会坏」：前缀是否成对出现取决于源码里有没有对应的标准声明，静态判断不了。

use crate::input::json::JsonPositions;
use crate::project::Project;
use crate::rules::{Rule, RuleMeta, Target};
use crate::{Diagnostic, Severity};

pub struct AutoprefixBuild;

impl AutoprefixBuild {
    pub const ID: &'static str = "skyline/autoprefix-build";
}

impl Rule for AutoprefixBuild {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Info,
            description: "prod 构建的 autoprefixer 会给样式注入 `-webkit-` 前缀，Skyline 不认",
        }
    }

    fn target(&self) -> Target {
        Target::Skyline
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        if !super::applicable_project_level(project) {
            return Vec::new();
        }
        // 全 WebView 工程里这些前缀同样被 WebView 忽略或照常处理，不构成迁移差异。
        if !project
            .pages
            .iter()
            .any(|page| page.is_skyline && !page.opted_out)
        {
            return Vec::new();
        }
        let enabled: Vec<(&str, &str, &str)> = ["postcss", "minifyWXSS"]
            .into_iter()
            .filter_map(|key| setting_flag(project, key))
            .collect();
        if enabled.is_empty() {
            return Vec::new();
        }

        let names = enabled
            .iter()
            .map(|(key, _, _)| format!("`setting.{key}`"))
            .collect::<Vec<_>>()
            .join(" / ");
        let (_, path, source) = enabled[0];
        let (line, column) = JsonPositions::new(source)
            .value_after(enabled[0].0)
            .unwrap_or((1, 1));
        vec![
            Diagnostic::new(
                Self::ID,
                Severity::Info,
                format!(
                    "{names} 开启后，prod 构建会跑 autoprefixer：往 `transition` 值里注入 `-webkit-transform`、并额外生成 `@-webkit-keyframes`。Skyline 不认这些前缀，过渡与关键帧可能整条失效；dev 构建跳过这一步，本地预览看不出来"
                ),
                path,
                line,
                column,
            )
            .help(
                "确认 prod 产物里没有「值内前缀」和「前缀 at-rule」后再上线；临时规避是把 `postcss` 与 `minifyWXSS` 都设为 false（会一并失去 wxss 压缩），并用 `skyline/vendor-prefix` 复查产物",
            )
            .json(crate::diag::JsonAnchor::new(
                "/setting",
                Some(
                    enabled
                        .iter()
                        .map(|(key, _, _)| *key)
                        .collect::<Vec<_>>()
                        .join(","),
                ),
            )),
        ]
    }
}

/// `setting.<key>` 为真时返回 `(key, 配置文件路径, 原文)`；private 覆盖 public。
fn setting_flag<'a>(
    project: &'a Project,
    key: &'static str,
) -> Option<(&'static str, &'a str, &'a str)> {
    let lookup = |config: &'a serde_json::Map<String, serde_json::Value>,
                  source: &'a str|
     -> Option<(&'static str, &'a str, &'a str)> {
        let enabled = config
            .get("setting")
            .and_then(|setting| setting.get(key))
            .and_then(|value| value.as_bool())?;
        enabled.then_some((key, "project.private.config.json", source))
    };
    lookup(
        &project.project_private_config,
        &project.project_private_config_source,
    )
    .or_else(|| {
        let (key, _, source) = lookup(&project.project_config, &project.project_config_source)?;
        Some((key, "project.config.json", source))
    })
}

#[cfg(test)]
mod tests {
    use super::AutoprefixBuild;
    use crate::rules::Rule;
    use crate::rules::testutil::Fixture;
    use crate::{Project, Severity};

    const SKYLINE_APP: &str = r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#;

    fn project(name: &str, config: &str, app: &str) -> (Fixture, Project) {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", config)
            .write("app.json", app)
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", "<view/>");
        let project = fixture.project();
        (fixture, project)
    }

    #[test]
    fn postcss_enabled_on_skyline_project_is_reported() {
        let (_f, project) = project(
            "autoprefix-postcss",
            "{\n  \"miniprogramRoot\": \"\",\n  \"setting\": {\"postcss\": true}\n}",
            SKYLINE_APP,
        );
        let diagnostics = AutoprefixBuild.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].severity, Severity::Info);
        assert_eq!(diagnostics[0].path, "project.config.json");
        assert!(diagnostics[0].message.contains("setting.postcss"));
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (3, 26));
    }

    /// `postcss: false` 也能触发：插件由 `minifyWXSS` 拉起，`autoPrefix` 走默认 true。
    #[test]
    fn minify_wxss_alone_is_enough() {
        let (_f, project) = project(
            "autoprefix-minify",
            r#"{"miniprogramRoot":"","setting":{"postcss":false,"minifyWXSS":true}}"#,
            SKYLINE_APP,
        );
        let diagnostics = AutoprefixBuild.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("setting.minifyWXSS"));
        assert!(!diagnostics[0].message.contains("setting.postcss"));
    }

    #[test]
    fn both_disabled_is_quiet() {
        let (_f, project) = project(
            "autoprefix-off",
            r#"{"miniprogramRoot":"","setting":{"postcss":false,"minifyWXSS":false}}"#,
            SKYLINE_APP,
        );
        assert!(AutoprefixBuild.check(&project).is_empty());
    }

    #[test]
    fn webview_only_project_is_quiet() {
        let (_f, project) = project(
            "autoprefix-webview",
            r#"{"miniprogramRoot":"","setting":{"postcss":true}}"#,
            r#"{"pages":["pages/index/index"],"renderer":"webview"}"#,
        );
        assert!(AutoprefixBuild.check(&project).is_empty());
    }
}
