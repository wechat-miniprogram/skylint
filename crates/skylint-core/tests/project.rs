//! 工程视图的端到端测试：真造一个磁盘工程，验证「路由 → 实际源文件」与
//! 「是否 Skyline 页面」两条最容易被上游实现细节带偏的语义。

use std::fs;
use std::path::{Path, PathBuf};

use skylint_core::Project;

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("skylint-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn write(&self, rel: &str, content: &str) -> &Self {
        let path = self.root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
        self
    }

    fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// 一个最小但完整的工程：TS + less、主包 + 分包、app 级 skyline、单页显式 webview。
fn standard_project(name: &str) -> Fixture {
    let f = Fixture::new(name);
    f.write(
        "project.config.json",
        r#"{"miniprogramRoot":"miniprogram/","setting":{"useCompilerPlugins":["typescript","less"]}}"#,
    )
    .write(
        "miniprogram/app.json",
        r#"{"pages":["pages/index/index","pages/legacy/legacy"],"subPackages":[{"root":"pkg","pages":["a"]}],"renderer":"skyline","usingComponents":{"nav":"/components/nav/nav"}}"#,
    )
    .write("miniprogram/pages/index/index.json", r#"{}"#)
    .write("miniprogram/pages/index/index.wxml", "<view/>")
    .write("miniprogram/pages/index/index.ts", "Page({})")
    .write("miniprogram/pages/index/index.less", ".a{color:red}")
    .write("miniprogram/pages/legacy/legacy.json", r#"{"renderer":"webview"}"#)
    .write("miniprogram/pages/legacy/legacy.wxml", "<view/>")
    .write("miniprogram/pages/legacy/legacy.js", "Page({})")
    .write("miniprogram/pkg/a.json", r#"{}"#)
    .write("miniprogram/pkg/a.wxml", "<view/>")
    .write("miniprogram/pkg/a.js", "Page({})")
    .write("miniprogram/components/nav/nav.json", r#"{"component":true}"#)
    .write("miniprogram/components/nav/nav.wxml", "<view/>")
    .write("miniprogram/components/nav/nav.js", "Component({})")
    .write("miniprogram/components/nav/nav.wxss", ".n{}");
    f
}

#[test]
fn routes_resolve_to_actual_sources() {
    let f = standard_project("routes");
    let project = Project::load(f.path()).unwrap();

    assert_eq!(project.miniprogram_root, "miniprogram");
    assert_eq!(project.pages.len(), 3);

    let index = project
        .pages
        .iter()
        .find(|p| p.route == "pages/index/index")
        .unwrap();
    // .ts/.less 必须解析成真实源文件，而不是假定 .js/.wxss。
    assert_eq!(index.files.js.as_deref(), Some("pages/index/index.ts"));
    assert_eq!(index.files.wxss.as_deref(), Some("pages/index/index.less"));
    assert_eq!(index.files.wxml.as_deref(), Some("pages/index/index.wxml"));
    // 页面 json 是可选文件：list 里没有 json 也不算错，但这里存在。
    assert_eq!(index.files.json.as_deref(), Some("pages/index/index.json"));

    // 分包页在 app.json 里已带 root 前缀。
    let sub = project.pages.iter().find(|p| p.route == "pkg/a").unwrap();
    assert!(sub.files.wxml.is_some());

    assert_eq!(project.components.len(), 1);
    assert_eq!(project.components[0].path, "components/nav/nav");
    assert_eq!(
        project.components[0].files.wxss.as_deref(),
        Some("components/nav/nav.wxss")
    );
}

#[test]
fn skyline_flag_follows_page_then_app_renderer() {
    let f = standard_project("skyline");
    let project = Project::load(f.path()).unwrap();

    let flag = |route: &str| {
        project
            .pages
            .iter()
            .find(|p| p.route == route)
            .unwrap()
            .is_skyline
    };

    // app.renderer = skyline → 未写 renderer 的页面继承 skyline。
    assert!(flag("pages/index/index"));
    assert!(flag("pkg/a"));
    // 页面显式 webview 必须覆盖 app 级 skyline。
    assert!(!flag("pages/legacy/legacy"));
}

#[test]
fn npm_components_join_the_closure() {
    let f = Fixture::new("npm");
    f.write("project.config.json", r#"{"miniprogramRoot":""}"#)
        .write("app.json", r#"{"pages":["pages/index/index"]}"#)
        .write(
            "pages/index/index.json",
            r#"{"usingComponents":{"btn":"my-lib/button"}}"#,
        )
        .write("pages/index/index.wxml", "<btn/>")
        .write(
            "miniprogram_npm/my-lib/button.json",
            r#"{"component":true}"#,
        )
        .write("miniprogram_npm/my-lib/button.wxml", "<view/>")
        .write("miniprogram_npm/my-lib/button.js", "Component({})");

    let project = Project::load(f.path()).unwrap();
    assert_eq!(project.components.len(), 1);
    assert_eq!(project.components[0].path, "miniprogram_npm/my-lib/button");
    assert_eq!(
        project.components[0].files.wxml.as_deref(),
        Some("miniprogram_npm/my-lib/button.wxml")
    );
}

#[test]
fn missing_app_json_is_an_error() {
    let f = Fixture::new("no-app");
    f.write("project.config.json", r#"{"miniprogramRoot":""}"#);
    assert!(Project::load(f.path()).is_err());
}

#[test]
fn broken_page_json_becomes_a_warning() {
    let f = Fixture::new("broken-json");
    f.write("project.config.json", r#"{"miniprogramRoot":""}"#)
        .write("app.json", r#"{"pages":["pages/x/x"]}"#)
        .write("pages/x/x.json", "{ not json")
        .write("pages/x/x.wxml", "<view/>");

    let project = Project::load(f.path()).unwrap();
    // 单文件坏了不能终止整次扫描。
    assert_eq!(project.pages.len(), 1);
    assert_eq!(project.warnings.len(), 1);
    assert_eq!(project.warnings[0].severity, skylint_core::Severity::Error);
}
