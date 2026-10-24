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
        let root = fixture_root(name);
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
fn global_style_resolves_through_the_resolver() {
    // 全局样式也要过 Resolver：工程可能只有 app.less / app.scss。
    // 旧实现把 `miniprogramRoot` 拼了两次（读成 miniprogram/miniprogram/app.wxss），
    // 于是 app 级样式被静默跳过。
    let f = standard_project("app-style");
    f.write("miniprogram/app.less", ".g{color:red}");
    let project = Project::load(f.path()).unwrap();

    let global = project
        .styles
        .iter()
        .find(|sheet| sheet.scopes.contains(&skylint_core::project::Scope::Global))
        .expect("app.less 必须进样式图");
    assert_eq!(global.doc.path(), "app.less");
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
fn non_declarative_component_entries_join_the_closure() {
    let f = Fixture::new("seeds");
    f.write(
        "project.config.json",
        r#"{"miniprogramRoot":"","setting":{}}"#,
    )
    .write(
        "app.json",
        r#"{"pages":["pages/index/index"],"tabBar":{"custom":true},"appBar":{}}"#,
    )
    .write("pages/index/index.wxml", "<view/>")
    .write("custom-tab-bar/index.json", r#"{"component":true}"#)
    .write("custom-tab-bar/index.wxml", "<view/>")
    .write("app-bar/index.json", r#"{"component":true}"#)
    .write("app-bar/index.wxml", "<view/>");

    let project = Project::load(f.path()).unwrap();
    let paths: Vec<&str> = project.components.iter().map(|c| c.path.as_str()).collect();
    assert_eq!(paths, ["app-bar/index", "custom-tab-bar/index"]);
}

#[test]
fn npm_lookup_walks_up_to_the_main_package() {
    // 分包页面引 npm 裸包名时，`miniprogram_npm` 可能在主包根，也可能在分包目录下。
    let f = Fixture::new("npm-up");
    f.write("project.config.json", r#"{"miniprogramRoot":""}"#)
        .write(
            "app.json",
            r#"{"pages":["pkg/a/a"],"subPackages":[{"root":"pkg","pages":["a/a"]}]}"#,
        )
        .write(
            "pkg/a/a.json",
            r#"{"usingComponents":{"btn":"my-lib/button"}}"#,
        )
        .write("pkg/a/a.wxml", "<btn/>")
        .write(
            "miniprogram_npm/my-lib/button.json",
            r#"{"component":true}"#,
        )
        .write("miniprogram_npm/my-lib/button.wxml", "<view/>");

    let project = Project::load(f.path()).unwrap();
    assert_eq!(project.components.len(), 1);
    assert_eq!(project.components[0].path, "miniprogram_npm/my-lib/button");
}

#[test]
fn lowercase_subpackages_is_accepted() {
    let f = Fixture::new("lower-sub");
    f.write("project.config.json", r#"{"miniprogramRoot":""}"#)
        .write(
            "app.json",
            r#"{"pages":["pages/a/a"],"subpackages":[{"root":"pkg/","pages":["b/b"]}]}"#,
        )
        .write("pages/a/a.wxml", "<view/>")
        .write("pkg/b/b.wxml", "<view/>");

    let project = Project::load(f.path()).unwrap();
    let routes: Vec<&str> = project.pages.iter().map(|p| p.route.as_str()).collect();
    assert_eq!(routes, ["pages/a/a", "pkg/b/b"]);
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

#[test]
fn unparsable_stylesheet_is_reported_not_silently_skipped() {
    let f = standard_project("unparsed-style");
    // 这个工程开了 less 插件，样式源是 index.less（wxss 不参与）
    f.write("miniprogram/pages/index/index.less", ".a { color red; }");
    let project = Project::load(f.path()).unwrap();
    let reported: Vec<_> = project
        .warnings
        .iter()
        .filter(|w| w.rule == "skylint/unparsed-style")
        .collect();
    assert_eq!(reported.len(), 1, "{:?}", project.warnings);
    assert_eq!(reported[0].path, "pages/index/index.less");

    // 空文件是正常情况，不该产生任何告警
    let g = standard_project("empty-style");
    g.write("miniprogram/pages/index/index.wxss", "/* nothing */");
    let project = Project::load(g.path()).unwrap();
    assert!(
        project
            .warnings
            .iter()
            .all(|w| w.rule != "skylint/unparsed-style")
    );
}

/// 脚本图是全域的：单元脚本 + 磁盘上其余 `.js` / `.ts`（`.d.ts`、`miniprogram_npm` 除外）。
#[test]
fn scripts_cover_every_source_file_on_disk() {
    let f = Fixture::new("script-domain");
    f.write("project.config.json", r#"{"miniprogramRoot":""}"#)
        .write(
            "app.json",
            r#"{"pages":["pages/a/a","pages/b/b","pages/c/c"]}"#,
        );
    for page in ["a", "b", "c"] {
        f.write(&format!("pages/{page}/{page}.json"), "{}")
            .write(&format!("pages/{page}/{page}.wxml"), "<view/>")
            .write(&format!("pages/{page}/{page}.js"), "Page({});");
    }
    f.write("utils/one.js", "export const one = 1;")
        .write("utils/two.ts", "export const two = 2;")
        .write("utils/types.d.ts", "export declare const x: number;");

    let project = Project::load(f.path()).unwrap();
    let paths: Vec<&str> = project
        .scripts
        .iter()
        .map(|script| script.path.as_str())
        .collect();
    assert_eq!(
        paths,
        [
            "pages/a/a.js",
            "pages/b/b.js",
            "pages/c/c.js",
            "utils/one.js",
            "utils/two.ts",
        ]
    );
    let loose: Vec<&str> = project
        .scripts
        .iter()
        .filter(|script| matches!(script.scope, skylint_core::project::Scope::Loose(_)))
        .map(|script| script.path.as_str())
        .collect();
    assert_eq!(loose, ["utils/one.js", "utils/two.ts"]);
}

fn fixture_root(name: &str) -> std::path::PathBuf {
    let thread = std::thread::current();
    let test = thread.name().unwrap_or("shared").replace("::", "-");
    std::env::temp_dir().join(format!("skylint-{name}-{test}-{}", std::process::id()))
}
