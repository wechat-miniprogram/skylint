//! `skyline/lib-version`：项目配置的基础库版本低于工程实际用到的 Skyline 特性的最低版本。
//!
//! `libVersion` 以 `project.private.config.json` 为准（开发者工具合并时 private 覆盖
//! public），两处都没有则跳过。
//!
//! 为什么需要这条规则：`libVersion` 不参与小程序运行时的 schema 校验，`minidev
//! build` 对版本偏低的工程完全静默（实测 `libVersion: "2.20.0"` + 全套 Skyline
//! 配置照样构建通过）。版本不够时 Skyline 配置不生效、页面退回 WebView，而这种
//! 失效在构建日志和开发者工具里都没有痕迹。
//!
//! 判定只覆盖能静态比较的写法：`latest` / `trial` / `widelyUsed` / `development`
//! 都是 schema 允许的值，无法判断「够不够」，一律跳过。

use crate::input::json::JsonPositions;
use crate::project::Project;
use crate::rules::{Rule, RuleMeta, Target};
use crate::{Diagnostic, Severity};

/// `(major, minor, patch)`，按字典序比较。
type Version = (u32, u32, u32);

pub struct LibVersion;

impl LibVersion {
    pub const ID: &'static str = "skyline/lib-version";
}

impl Rule for LibVersion {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Error,
            description: "项目配置的 libVersion 低于所用 Skyline 特性的最低要求",
        }
    }

    fn target(&self) -> Target {
        // `libVersion` 低于 Skyline 要求只会让页面继续用 WebView——那正是反向的目标环境。
        Target::Skyline
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        if !super::applicable_project_level(project) {
            return Vec::new();
        }
        let Some((min, feature)) = requirement(project) else {
            return Vec::new();
        };
        let Some((raw, path, source)) = project.effective_lib_version() else {
            return Vec::new();
        };
        let Some(actual) = parse_version(raw) else {
            return Vec::new();
        };
        if actual >= min {
            return Vec::new();
        }

        let floor = version_text(min);
        // 中文与西文之间留空格，中文与中文之间不留。
        let space_before = if feature.starts_with(|c: char| c.is_ascii_alphanumeric()) {
            " "
        } else {
            ""
        };
        let space_after = if feature.ends_with(|c: char| c.is_ascii_alphanumeric()) {
            " "
        } else {
            ""
        };
        // 指向值本身（开引号处），而不是 `libVersion` 这个键。
        let (line, column) = JsonPositions::new(source)
            .value_after("libVersion")
            .unwrap_or((1, 1));
        vec![
            Diagnostic::new(
                Self::ID,
                Severity::Error,
                format!("libVersion \"{raw}\" 低于{space_before}{feature}{space_after}要求的 {floor}"),
                path,
                line,
                column,
            )
            .help(format!(
                "低于该版本时这部分配置不生效（Skyline 渲染会退回 WebView），而 project.config.json / project.private.config.json 没有 schema 校验、构建也不会提示；真机运行 Skyline 还需要 3.0.2 以上。请把 libVersion 提到 {floor} 或更高"
            )),
        ]
    }
}

/// 工程实际用到的最高要求。同一工程只报一条，所以这里直接返回阈值最高的那项。
fn requirement(project: &Project) -> Option<(Version, &'static str)> {
    let skyline = project
        .pages
        .iter()
        .any(|page| page.is_skyline && !page.opted_out);

    let mut required: Option<(Version, &'static str)> = None;
    let mut bump = |candidate: (Version, &'static str)| {
        if required.is_none_or(|(current, _)| candidate.0 > current) {
            required = Some(candidate);
        }
    };
    if skyline {
        bump(((2, 30, 4), "Skyline 渲染"));
    }
    if project.app_json.contains_key("rendererOptions") {
        bump(((2, 31, 1), "app 级 rendererOptions"));
    }
    // 页面级 rendererOptions 只有 Skyline 页会读；WebView 页上的是死配置，不抬阈值。
    if project.pages.iter().any(|page| {
        page.is_skyline
            && project
                .page_json
                .get(&page.route)
                .is_some_and(|json| json.contains_key("rendererOptions"))
    }) {
        bump(((3, 1, 0), "页面级 rendererOptions"));
    }
    // 只认 app.json 顶层：写在 rendererOptions 里的 convertRpxToVw 运行时不会读，
    // 位置错误由 `skyline/renderer-options` 报。
    if project.app_flag("convertRpxToVw") {
        bump(((3, 3, 0), "convertRpxToVw"));
    }
    required
}

/// `x.y.z`（各段纯数字）。其余写法（`latest` 等）返回 `None`。
fn parse_version(raw: &str) -> Option<Version> {
    let mut parts = raw.trim().split('.');
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = parts.next()?.parse().ok()?;
    let patch: u32 = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some((major, minor, patch))
}

fn version_text(version: Version) -> String {
    format!("{}.{}.{}", version.0, version.1, version.2)
}

#[cfg(test)]
mod tests {
    use crate::Severity;
    use crate::project::Project;
    use crate::rules::Rule;
    use crate::rules::testutil::Fixture;

    use super::LibVersion;

    const SKYLINE_APP: &str = r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#;

    fn project(name: &str, config: &str, app: &str, page: &str) -> (Fixture, Project) {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", config)
            .write("app.json", app)
            .write("pages/index/index.json", page)
            .write("pages/index/index.wxml", "<view/>");
        let project = fixture.project();
        (fixture, project)
    }

    fn config(lib_version: &str) -> String {
        format!(r#"{{"miniprogramRoot":"","libVersion":"{lib_version}"}}"#)
    }

    #[test]
    fn reports_version_below_skyline_render_floor() {
        let (_f, project) = project(
            "lib-version-skyline",
            "{\n  \"miniprogramRoot\": \"\",\n  \"libVersion\": \"2.20.0\"\n}",
            SKYLINE_APP,
            "{}",
        );
        let diags = LibVersion.check(&project);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(diags[0].path, "project.config.json");
        assert_eq!((diags[0].line, diags[0].column), (3, 17), "应指向值本身");
        assert!(
            diags[0].message.contains("\"2.20.0\""),
            "{}",
            diags[0].message
        );
        assert!(diags[0].message.contains("2.30.4"), "{}", diags[0].message);
        assert!(
            diags[0].message.contains("Skyline 渲染"),
            "{}",
            diags[0].message
        );
    }

    #[test]
    fn reports_version_below_app_renderer_options_floor() {
        let (_f, project) = project(
            "lib-version-app-options",
            &config("2.31.0"),
            r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"defaultDisplayBlock":true}}}"#,
            "{}",
        );
        let diags = LibVersion.check(&project);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("2.31.1"), "{}", diags[0].message);
        assert!(diags[0].message.contains("app 级 rendererOptions"));
    }

    #[test]
    fn reports_version_below_page_renderer_options_floor() {
        let (_f, project) = project(
            "lib-version-page-options",
            &config("3.0.0"),
            SKYLINE_APP,
            r#"{"rendererOptions":{"skyline":{"defaultDisplayBlock":true}}}"#,
        );
        let diags = LibVersion.check(&project);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("3.1.0"), "{}", diags[0].message);
        assert!(diags[0].message.contains("页面级 rendererOptions"));
    }

    #[test]
    fn reports_version_below_convert_rpx_to_vw_floor() {
        let (_f, project) = project(
            "lib-version-rpx-to-vw",
            &config("3.2.0"),
            r#"{"pages":["pages/index/index"],"renderer":"skyline","convertRpxToVw":true}"#,
            "{}",
        );
        let diags = LibVersion.check(&project);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("3.3.0"), "{}", diags[0].message);
        assert!(diags[0].message.contains("convertRpxToVw"));
    }

    #[test]
    fn ignores_convert_rpx_to_vw_written_inside_renderer_options() {
        // 位置错由 renderer-options 报，这里不抬阈值。
        let (_f, project) = project(
            "lib-version-rpx-to-vw-misplaced",
            &config("3.2.0"),
            r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"convertRpxToVw":true}}}"#,
            "{}",
        );
        assert!(LibVersion.check(&project).is_empty());
    }

    #[test]
    fn private_lib_version_overrides_public() {
        // public 低、private 高：实际生效的是 private，不报。
        let fixture = Fixture::new("lib-version-private-high");
        fixture
            .write("project.config.json", &config("2.20.0"))
            .write("project.private.config.json", r#"{"libVersion":"3.3.0"}"#)
            .write("app.json", SKYLINE_APP)
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", "<view/>");
        let project = fixture.project();
        assert!(LibVersion.check(&project).is_empty());
    }

    #[test]
    fn reports_private_lib_version_even_when_public_is_high() {
        // public 高、private 低：实际生效的是 private，诊断落在 private 文件上。
        let fixture = Fixture::new("lib-version-private-low");
        fixture
            .write("project.config.json", &config("3.3.0"))
            .write(
                "project.private.config.json",
                "{\n  \"libVersion\": \"2.20.0\"\n}",
            )
            .write("app.json", SKYLINE_APP)
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", "<view/>");
        let project = fixture.project();
        let diags = LibVersion.check(&project);
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].path, "project.private.config.json");
        assert_eq!((diags[0].line, diags[0].column), (2, 17));
        assert!(diags[0].message.contains("\"2.20.0\""));
    }

    #[test]
    fn ignores_page_renderer_options_on_webview_pages() {
        // WebView 页的 rendererOptions 是死配置，不应把阈值抬到 3.1.0。
        let (_f, project) = project(
            "lib-version-webview-page-options",
            &config("3.0.5"),
            r#"{"pages":["pages/index/index"],"renderer":"webview"}"#,
            r#"{"rendererOptions":{"skyline":{"defaultDisplayBlock":true}}}"#,
        );
        assert!(LibVersion.check(&project).is_empty());
    }

    #[test]
    fn reports_only_the_highest_requirement() {
        let (_f, project) = project(
            "lib-version-highest",
            &config("2.20.0"),
            r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"defaultDisplayBlock":true}}}"#,
            r#"{"rendererOptions":{"skyline":{"defaultDisplayBlock":true}}}"#,
        );
        let diags = LibVersion.check(&project);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("3.1.0"), "{}", diags[0].message);
    }

    #[test]
    fn accepts_version_at_or_above_every_floor() {
        let (_f, project) = project(
            "lib-version-ok",
            &config("3.3.0"),
            r#"{"pages":["pages/index/index"],"renderer":"skyline","convertRpxToVw":true}"#,
            r#"{"rendererOptions":{"skyline":{"defaultDisplayBlock":true}}}"#,
        );
        assert!(LibVersion.check(&project).is_empty());
    }

    #[test]
    fn accepts_newer_than_app_renderer_options_floor() {
        // 2.40.0 高于 2.31.1：app 级 rendererOptions 在这个版本上已经生效。
        let (_f, project) = project(
            "lib-version-newer",
            &config("2.40.0"),
            r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"defaultDisplayBlock":true}}}"#,
            "{}",
        );
        assert!(LibVersion.check(&project).is_empty());
    }

    #[test]
    fn skips_non_version_values_and_missing_field() {
        for (name, config_text) in [
            ("latest", config("latest")),
            ("empty", config("")),
            ("trial", config("trial")),
            ("missing", r#"{"miniprogramRoot":""}"#.to_string()),
        ] {
            let (_f, project) = project(
                &format!("lib-version-{name}"),
                &config_text,
                SKYLINE_APP,
                "{}",
            );
            assert!(
                LibVersion.check(&project).is_empty(),
                "{config_text} 不应判定"
            );
        }
    }

    #[test]
    fn ignores_projects_without_skyline_features() {
        let (_f, project) = project(
            "lib-version-webview",
            &config("1.0.0"),
            r#"{"pages":["pages/index/index"]}"#,
            r#"{"renderer":"webview"}"#,
        );
        assert!(LibVersion.check(&project).is_empty());
    }
}
