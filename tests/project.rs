//! 工程视图的端到端测试：真造一个磁盘工程，验证「路由 → 实际源文件」与
//! 「是否 Skyline 页面」两条最容易被上游实现细节带偏的语义。

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use skylint::Project;
use skylint::project::Scope;

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
        .find(|sheet| sheet.scopes.contains(&skylint::project::Scope::Global))
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
fn single_file_wxml_is_lintable_without_a_project() {
    let f = Fixture::new("single-wxml");
    f.write("one.wxml", "<view>text<image/></view>");

    let project = Project::single_file(f.path().join("one.wxml")).unwrap();
    assert_eq!(project.pages.len(), 1);
    assert!(project.pages[0].is_skyline);

    let diagnostics = skylint::Registry::builtin().run(&project, &HashSet::new());
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule == "skyline/no-inline-text"),
        "单文件也该跑单文件规则：{diagnostics:?}"
    );
    // 没有真工程就没有配置可判：不能对着不存在的 app.json 报 defaultDisplayBlock。
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| !diagnostic.path.ends_with("app.json")),
        "单文件模式不该产出配置类诊断：{diagnostics:#?}"
    );
}

#[test]
fn single_file_wxss_needs_no_project_layout() {
    let f = Fixture::new("single-wxss");
    f.write("one.wxss", ".a { user-select: none; }");

    let project = Project::single_file(f.path().join("one.wxss")).unwrap();
    let diagnostics = skylint::Registry::builtin().run(&project, &HashSet::new());
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule == "skyline/unsupported-property"),
        "单文件也该跑声明级规则：{diagnostics:?}"
    );
}

#[test]
fn malformed_app_json_types_fail_loudly() {
    // 类型写错必须在加载阶段失败：静默当成 0 个页面，等于所有规则都不跑却没人知道。
    for (name, app_json) in [
        ("app-pages-type", r#"{"pages":"pages/a"}"#),
        ("app-subs-type", r#"{"pages":[],"subPackages":"pkg"}"#),
        (
            "app-sub-root",
            r#"{"pages":[],"subPackages":[{"pages":["a"]}]}"#,
        ),
    ] {
        let f = Fixture::new(name);
        f.write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write("app.json", app_json);
        assert!(Project::load(f.path()).is_err(), "{app_json} 应该加载失败");
    }
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
    assert_eq!(project.warnings[0].severity, skylint::Severity::Error);
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

#[test]
fn unparsable_wxml_is_reported_not_silently_skipped() {
    let f = standard_project("unparsed-template");
    f.write("miniprogram/pages/index/index.wxml", "<view>{{ a</view>");
    let project = Project::load(f.path()).unwrap();
    let reported: Vec<_> = project
        .warnings
        .iter()
        .filter(|w| w.rule == "skylint/unparsed-template")
        .collect();
    assert_eq!(reported.len(), 1, "{:?}", project.warnings);
    assert_eq!(reported[0].path, "pages/index/index.wxml");
}

/// project.config.json 解析失败不能静默：miniprogramRoot 读不到时 app.json 必然
/// 找不到，报错必须指向真正的坏文件；项目就在根目录时则退化成一条诊断。
#[test]
fn unparsable_project_config_names_the_real_cause() {
    let f = standard_project("bad-project-config");
    f.write(
        "project.config.json",
        "{\"miniprogramRoot\": \"miniprogram/\",}",
    );
    let error = match Project::load(f.path()) {
        Ok(_) => panic!("配置解析失败不该成功"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("project.config.json"), "{error}");
}

#[test]
fn unparsable_project_config_at_root_is_a_diagnostic() {
    let f = Fixture::new("bad-project-config-root");
    f.write("project.config.json", "{\"libVersion\": \"3.0.0\",}")
        .write(
            "app.json",
            r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
        )
        .write("pages/index/index.wxml", "<view/>");
    let project = Project::load(f.path()).unwrap();
    let reported: Vec<_> = project
        .warnings
        .iter()
        .filter(|w| w.rule == "skylint/invalid-json")
        .collect();
    assert_eq!(reported.len(), 1, "{:?}", project.warnings);
    assert_eq!(reported[0].path, "project.config.json");
}

/// 缺文件等价于没写，不该产出诊断。
#[test]
fn missing_project_config_is_silent() {
    let f = Fixture::new("no-project-config");
    f.write(
        "app.json",
        r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
    )
    .write("pages/index/index.wxml", "<view/>");
    let project = Project::load(f.path()).unwrap();
    assert!(
        project
            .warnings
            .iter()
            .all(|w| w.rule != "skylint/invalid-json"),
        "{:?}",
        project.warnings
    );
}

/// `componentGenerics.{key}.default` 也是组件引用：编译器会加载默认实现，
/// 漏掉它这个组件完全不参与检查（minidev `compiler-graph/src/conf.rs:309-315`）。
#[test]
fn component_generics_default_joins_the_closure() {
    let f = standard_project("generics");
    f.write(
        "miniprogram/pages/index/index.json",
        r#"{"usingComponents":{"card":"/components/card/card"},"componentGenerics":{"slot-comp":{"default":"/components/slot-comp/slot-comp"}}}"#,
    );
    f.write(
        "miniprogram/components/slot-comp/slot-comp.json",
        r#"{"component":true}"#,
    )
    .write("miniprogram/components/slot-comp/slot-comp.wxml", "<view/>");
    let project = Project::load(f.path()).unwrap();
    assert!(
        project
            .components
            .iter()
            .any(|component| component.path == "components/slot-comp/slot-comp"),
        "{:?}",
        project
            .components
            .iter()
            .map(|component| &component.path)
            .collect::<Vec<_>>()
    );
}

/// BOM 是合法字节但不是合法 JSON / CSS 前缀，读入时要去掉。
#[test]
fn utf8_bom_is_stripped_for_config_and_styles() {
    let f = standard_project("bom");
    f.write(
        "project.config.json",
        "\u{feff}{\"miniprogramRoot\":\"miniprogram\"}",
    );
    f.write(
        "miniprogram/pages/index/index.wxss",
        "\u{feff}.a { float: left; }",
    );
    let project = Project::load(f.path()).unwrap();
    assert!(project.project_config.contains_key("miniprogramRoot"));
    let sheet = project
        .styles
        .iter()
        .find(|sheet| sheet.doc.path().ends_with("index.wxss"))
        .expect("样式文件应当进入样式图");
    assert!(!sheet.doc.source().starts_with('\u{feff}'));
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
        .filter(|script| matches!(script.scope, skylint::project::Scope::Loose(_)))
        .map(|script| script.path.as_str())
        .collect();
    assert_eq!(loose, ["utils/one.js", "utils/two.ts"]);
}

/// 插件的宿主示例与插件本身是两个视图：插件页只在 `pluginRoot/plugin.json` 里，
/// 宿主 `app.json` 看不到它们。
fn plugin_project(name: &str) -> Fixture {
    let f = Fixture::new(name);
    f.write(
        "project.config.json",
        r#"{"compileType":"plugin","miniprogramRoot":"dist/miniprogram/","pluginRoot":"dist/plugin/","libVersion":"2.20.0"}"#,
    )
    // 宿主：一个页面，通过 plugin:// 用了插件的 public component。
    .write(
        "dist/miniprogram/app.json",
        r#"{"pages":["pages/index/index"],"renderer":"skyline","usingComponents":{"card":"plugin://demo/card"}}"#,
    )
    .write("dist/miniprogram/pages/index/index.json", r#"{}"#)
    .write("dist/miniprogram/pages/index/index.wxml", "<card/>")
    // 插件：一页一组件，组件只在 publicComponents 里出现。
    .write(
        "dist/plugin/plugin.json",
        r#"{"pages":{"novel":"pages/novel/index"},"renderer":"skyline","publicComponents":{"card":"components/card/card"}}"#,
    )
    .write(
        "dist/plugin/pages/novel/index.json",
        r#"{"usingComponents":{"row":"../../components/row/row"}}"#,
    )
    .write("dist/plugin/pages/novel/index.wxml", "<row/>")
    .write("dist/plugin/pages/novel/index.wxss", ".a:hover{color:red}")
    .write("dist/plugin/components/row/row.json", r#"{"component":true}"#)
    .write("dist/plugin/components/row/row.wxml", "<view/>")
    .write("dist/plugin/components/card/card.json", r#"{"component":true}"#)
    .write("dist/plugin/components/card/card.wxml", "<view/>");
    f
}

#[test]
fn plugin_pages_get_their_own_view() {
    let f = plugin_project("plugin-view");
    let views = Project::load_views(f.path()).unwrap();
    assert_eq!(views.len(), 2, "宿主 + 插件两个视图");

    let [host, plugin] = &views[..] else {
        unreachable!()
    };
    assert_eq!(host.report_prefix, "");
    assert_eq!(host.pages.len(), 1);
    assert!(!host.plugin_view);

    assert_eq!(plugin.report_prefix, "dist/plugin/");
    assert!(plugin.plugin_view);
    let routes: Vec<&str> = plugin.pages.iter().map(|p| p.route.as_str()).collect();
    assert_eq!(routes, ["pages/novel/index"]);
    // 页面 usingComponents 与 publicComponents 都要进闭包。
    let components: Vec<&str> = plugin.components.iter().map(|c| c.path.as_str()).collect();
    assert_eq!(components, ["components/card/card", "components/row/row"]);
    // 插件视图没有 app.wxss，页面样式照样入图。
    let scopes: Vec<&Scope> = plugin
        .styles
        .iter()
        .flat_map(|sheet| &sheet.scopes)
        .collect();
    assert_eq!(scopes, [&Scope::Page("pages/novel/index".to_string())]);
}

#[test]
fn plugin_paths_are_prefixed_and_resolvable() {
    let f = plugin_project("plugin-paths");
    let views = Project::load_views(f.path()).unwrap();
    let plugin = &views[1];
    // 视图内路径相对 pluginRoot，`abs` 才能定位到真实文件。
    assert_eq!(
        plugin.abs("pages/novel/index.wxml"),
        f.path().join("dist/plugin/pages/novel/index.wxml")
    );
    assert!(plugin.abs("pages/novel/index.wxml").is_file());
}

#[test]
fn project_without_plugin_root_has_one_view() {
    let f = Fixture::new("no-plugin");
    f.write("project.config.json", r#"{"miniprogramRoot":""}"#)
        .write("app.json", r#"{"pages":["pages/a/a"]}"#)
        .write("pages/a/a.json", "{}")
        .write("pages/a/a.wxml", "<view/>");
    // pluginRoot 指向不存在的目录，或压根没写，都只有一个视图。
    let views = Project::load_views(f.path()).unwrap();
    assert_eq!(views.len(), 1);
}

#[test]
fn missing_plugin_manifest_has_no_plugin_view() {
    let f = Fixture::new("plugin-missing");
    f.write(
        "project.config.json",
        r#"{"miniprogramRoot":"miniprogram/","pluginRoot":"plugin/"}"#,
    )
    .write("miniprogram/app.json", r#"{"pages":["pages/a/a"]}"#)
    .write("miniprogram/pages/a/a.json", "{}")
    .write("miniprogram/pages/a/a.wxml", "<view/>");
    assert_eq!(Project::load_views(f.path()).unwrap().len(), 1);
}

#[test]
fn plugin_pages_accept_list_shaped_pages() {
    let f = Fixture::new("plugin-list-pages");
    f.write(
        "project.config.json",
        r#"{"miniprogramRoot":"miniprogram/","pluginRoot":"plugin/"}"#,
    )
    .write("miniprogram/app.json", r#"{"pages":[]}"#)
    .write(
        "plugin/plugin.json",
        r#"{"pages":["pages/a/a","pages/a/a"]}"#,
    )
    .write("plugin/pages/a/a.json", "{}")
    .write("plugin/pages/a/a.wxml", "<view/>");
    let views = Project::load_views(f.path()).unwrap();
    let routes: Vec<&str> = views[1].pages.iter().map(|p| p.route.as_str()).collect();
    assert_eq!(routes, ["pages/a/a"], "重复项只保留一次");
}

#[test]
fn project_level_config_rules_run_once() {
    // `libVersion` 偏低在宿主与插件两个视图下是同一个事实，不能报两遍；
    // 也不能被插件视图的路径前缀带偏成 `dist/plugin/project.config.json`。
    let f = plugin_project("plugin-config-once");
    let views = Project::load_views(f.path()).unwrap();
    let refs: Vec<&Project> = views.iter().collect();
    let diagnostics = skylint::Registry::builtin().run_views(&refs, &HashSet::new());
    let lib: Vec<&skylint::Diagnostic> = diagnostics
        .iter()
        .filter(|d| d.rule == "skyline/lib-version")
        .collect();
    assert_eq!(lib.len(), 1, "{lib:#?}");
    assert_eq!(lib[0].path, "project.config.json");
}

#[test]
fn plugin_diagnostics_carry_the_view_prefix() {
    let f = plugin_project("plugin-run");
    let views = Project::load_views(f.path()).unwrap();
    let refs: Vec<&Project> = views.iter().collect();
    let diagnostics = skylint::Registry::builtin().run_views(&refs, &HashSet::new());

    assert!(
        diagnostics.iter().any(
            |d| d.rule == "skyline/no-pseudo" && d.path == "dist/plugin/pages/novel/index.wxss"
        ),
        "插件页的样式要参与检查，路径带 pluginRoot 前缀"
    );
    // 宿主视图的路径保持原样，不带任何前缀。
    assert!(
        diagnostics
            .iter()
            .all(|d| !d.path.starts_with("dist/miniprogram/"))
    );
}

fn fixture_root(name: &str) -> std::path::PathBuf {
    let thread = std::thread::current();
    let test = thread.name().unwrap_or("shared").replace("::", "-");
    std::env::temp_dir().join(format!("skylint-{name}-{test}-{}", std::process::id()))
}
