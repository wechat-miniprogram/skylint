//! 迁移接口的端到端测试：真造工程，验证三件容易漂的事——批次顺序、诊断归属、
//! 以及「元素挪了位置诊断 id 不变」。

use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use skylint::migrate;
use skylint::rules::Target;
use skylint::{Project, Registry, Report, Severity};

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("skylint-migrate-{name}-{}", std::process::id()));
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

fn analyse(root: &Path) -> (Vec<Project>, Report) {
    let projects = Project::load_views(root).expect("project");
    let registry = Registry::builtin().for_target(Target::Skyline);
    let views: Vec<&Project> = projects.iter().collect();
    let diagnostics = registry.run_views(&views, &HashSet::new());
    let report = Report::from_views(&projects, diagnostics);
    (projects, report)
}

/// 页面 → a → b：b 必须在第 0 批，a 在第 1 批，页面最后。
#[test]
fn batches_put_leaves_before_their_users() {
    let f = Fixture::new("batches");
    f.write(
        "project.config.json",
        r#"{"miniprogramRoot":"miniprogram/"}"#,
    )
    .write(
        "miniprogram/app.json",
        r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
    )
    .write(
        "miniprogram/pages/index/index.json",
        r#"{"usingComponents":{"a":"/components/a/a"}}"#,
    )
    .write("miniprogram/pages/index/index.wxml", "<a/>")
    .write("miniprogram/pages/index/index.js", "Page({})")
    .write(
        "miniprogram/components/a/a.json",
        r#"{"component":true,"usingComponents":{"b":"../b/b"}}"#,
    )
    .write("miniprogram/components/a/a.wxml", "<b/>")
    .write("miniprogram/components/a/a.js", "Component({})")
    .write("miniprogram/components/b/b.json", r#"{"component":true}"#)
    .write("miniprogram/components/b/b.wxml", "<view/>")
    .write("miniprogram/components/b/b.js", "Component({})");

    let (projects, report) = analyse(f.path());
    let plan = migrate::plan(&projects, &report, None);
    let batch = |suffix: &str| {
        plan.units
            .iter()
            .find(|unit| unit.path.ends_with(suffix))
            .unwrap_or_else(|| panic!("missing unit {suffix}"))
            .batch
    };

    assert_eq!(batch("components/b/b.wxml"), 0, "叶子组件先做");
    assert_eq!(batch("components/a/a.wxml"), 1);
    assert_eq!(batch("pages/index/index.wxml"), 2, "页面最后");
    assert_eq!(plan.batches.len(), 3);
}

/// `path` 是报告路径（相对 `miniprogramRoot`），`fsPath` 是工程根相对路径。
///
/// runner 要拿它去做 `git diff` / `checkout`，两者弄混就会在「工程根不是
/// miniprogramRoot」的所有真实工程上静默失效。
#[test]
fn plan_carries_filesystem_paths() {
    let f = Fixture::new("fspath");
    f.write(
        "project.config.json",
        r#"{"miniprogramRoot":"miniprogram/"}"#,
    )
    .write(
        "miniprogram/app.json",
        r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
    )
    .write("miniprogram/pages/index/index.json", r#"{}"#)
    .write("miniprogram/pages/index/index.wxml", "<view/>")
    .write("miniprogram/pages/index/index.js", "Page({})");

    let (projects, report) = analyse(f.path());
    let plan = migrate::plan(&projects, &report, None);
    let page = plan
        .units
        .iter()
        .find(|unit| unit.kind == "page")
        .expect("page unit");

    assert_eq!(page.path, "pages/index/index.wxml");
    assert_eq!(page.fs_path, "miniprogram/pages/index/index.wxml");
    assert!(
        f.path().join(&page.fs_path).is_file(),
        "fs_path 要能直接落到磁盘"
    );
}

/// 样式源是 `.scss` 时，作业单也要拿到这条诊断。
///
/// 以前 `owns()` 按后缀白名单（wxml/wxss/js/ts/wxs/json）判归属，`.scss` / `.less`
/// 的命中被算进 plan 的计数、却在 `job` 里被丢掉——agent 拿到的是 `findings: []`。
#[test]
fn job_keeps_findings_from_non_wxss_style_sources() {
    let f = Fixture::new("scss-owner");
    f.write(
        "project.config.json",
        r#"{"miniprogramRoot":"miniprogram/","setting":{"useCompilerPlugins":["sass"]}}"#,
    )
    .write(
        "miniprogram/app.json",
        r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
    )
    .write("miniprogram/pages/index/index.json", r#"{}"#)
    .write("miniprogram/pages/index/index.wxml", "<view class=\"sw\"/>")
    .write("miniprogram/pages/index/index.js", "Page({})")
    .write(
        "miniprogram/pages/index/index.scss",
        ".sw { transition: background-color 0.1s, border 0.1s; }\n",
    );

    let (projects, report) = analyse(f.path());
    let plan = migrate::plan(&projects, &report, None);
    let unit = plan
        .units
        .iter()
        .find(|unit| unit.kind == "page")
        .expect("page unit");

    assert_eq!(
        unit.diagnostics.total, 1,
        "plan 要把 .scss 上的命中算进单元"
    );
    let diagnostics = migrate::diagnostics_for(&plan, unit, &report);
    assert_eq!(diagnostics.len(), 1, "job 也要拿到同一条");
    assert!(diagnostics[0].path.ends_with("index.scss"));

    let (project, rel) = migrate::find_view(&projects, &unit.path).expect("view");
    let job = migrate::job(project, rel, &plan, unit, &diagnostics, None);
    assert_eq!(job.findings.len(), 1);
    assert_eq!(
        job.findings[0].path, "pages/index/index.scss",
        "非 WXML 诊断也要带 path"
    );
    assert!(job.findings[0].anchor.is_none());
}

/// `.skylint/config.json` 的 `exclude` 把第三方样式排除掉，但要把排除本身记下来。
///
/// 现实数据：businessview 的 `unit:shared` 98 条里有 90 条来自三份字节相同的
/// `weui.wxss`（第三方库，本来就不该手改）。排除后共享资源只剩 8 条。
#[test]
fn exclude_drops_vendor_styles_but_records_them() {
    let f = Fixture::new("exclude");
    f.write(
        "project.config.json",
        r#"{"miniprogramRoot":"miniprogram/"}"#,
    )
    .write(
        "miniprogram/app.json",
        r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
    )
    .write("miniprogram/app.wxss", ".x { float: left; }\n")
    .write("miniprogram/pages/index/index.json", r#"{}"#)
    .write("miniprogram/pages/index/index.wxml", "<view class=\"x\"/>")
    .write("miniprogram/pages/index/index.js", "Page({})");

    let (_, before) = analyse(f.path());
    assert!(
        before.diagnostics.iter().any(|d| d.path == "app.wxss"),
        "排除前 app.wxss 上的命中应该在"
    );
    assert_eq!(before.excluded.diagnostics, 0);

    f.write(
        ".skylint/config.json",
        r#"{"schemaVersion":1,"exclude":["*.wxss"]}"#,
    );
    let (projects, after) = analyse(f.path());
    assert!(
        !after.diagnostics.iter().any(|d| d.path == "app.wxss"),
        "排除后 app.wxss 上的命中应该消失"
    );
    assert_eq!(after.excluded.diagnostics, 1);
    assert_eq!(after.excluded.files, vec!["app.wxss".to_string()]);

    // 排除要一路传到 plan：shared 不能再算这些条数。
    let plan = migrate::plan(&projects, &after, None);
    let shared = plan.units.iter().find(|unit| unit.kind == "shared");
    assert!(
        shared.is_none_or(|unit| !unit
            .diagnostics
            .by_rule
            .contains_key("skyline/unsupported-property")),
        "被排除文件的诊断不应计入 unit:shared"
    );
}

/// 不属于任何单元文件的诊断（app.json）进 `unit:shared`。
#[test]
fn shared_unit_collects_diagnostics_outside_any_unit() {
    let f = Fixture::new("shared");
    f.write(
        "project.config.json",
        r#"{"miniprogramRoot":"miniprogram/"}"#,
    )
    .write(
        "miniprogram/app.json",
        r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
    )
    .write("miniprogram/pages/index/index.json", r#"{}"#)
    .write("miniprogram/pages/index/index.wxml", "<view/>")
    .write("miniprogram/pages/index/index.js", "Page({})");

    let (projects, report) = analyse(f.path());
    let plan = migrate::plan(&projects, &report, None);
    let shared = plan
        .units
        .iter()
        .find(|unit| unit.kind == "shared")
        .expect("app.json 的诊断不属于任何单元，必须有 shared");

    assert!(
        shared
            .diagnostics
            .by_rule
            .contains_key("skyline/default-layout-drift"),
        "app 级开关的诊断应落在 shared：{:?}",
        shared.diagnostics.by_rule
    );
    // 页面单元不该把这些诊断算到自己头上。
    let page = plan
        .units
        .iter()
        .find(|unit| unit.path == "pages/index/index.wxml")
        .expect("page unit");
    assert_eq!(page.diagnostics.total, 0);
}

/// `<include>` 引用的模板属于真正引用它的页面：模板上的诊断要进 plan 的
/// `files` 与 job 的 `findings`，不能落在 `unit:shared`。
#[test]
fn included_template_diagnostics_belong_to_the_page() {
    let f = Fixture::new("include-owner");
    f.write(
        "project.config.json",
        r#"{"miniprogramRoot":"miniprogram/"}"#,
    )
    .write(
        "miniprogram/app.json",
        r#"{"pages":["pages/index/index","pages/other/other"],"renderer":"skyline","rendererOptions":{"skyline":{"defaultDisplayBlock":true,"defaultContentBox":true}}}"#,
    )
    .write("miniprogram/pages/index/index.json", r#"{}"#)
    .write(
        "miniprogram/pages/index/index.wxml",
        r#"<include src="../../tpl/item"/>"#,
    )
    .write("miniprogram/pages/index/index.js", "Page({})")
    .write("miniprogram/pages/other/other.json", r#"{}"#)
    .write(
        "miniprogram/pages/other/other.wxml",
        r#"<include src="../../tpl/item"/>"#,
    )
    .write("miniprogram/pages/other/other.js", "Page({})")
    .write(
        "miniprogram/tpl/item.wxml",
        r#"<view class="item">hello <view class="inner">x</view></view>"#,
    );

    let (projects, report) = analyse(f.path());
    let plan = migrate::plan(&projects, &report, None);
    let unit = plan
        .units
        .iter()
        .find(|unit| unit.path == "pages/index/index.wxml")
        .expect("page unit");
    assert!(
        unit.files.iter().any(|file| file == "tpl/item.wxml"),
        "被 include 的模板要进单元 files：{:?}",
        unit.files
    );
    let other = plan
        .units
        .iter()
        .find(|unit| unit.path == "pages/other/other.wxml")
        .expect("second page unit");
    assert!(
        other.files.iter().any(|file| file == "tpl/item.wxml"),
        "同一模板被多个页面引用时同时属于它们"
    );

    let diagnostics = migrate::diagnostics_for(&plan, unit, &report);
    let template: Vec<_> = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.path == "tpl/item.wxml")
        .collect();
    assert_eq!(template.len(), 1, "模板上只有一条 no-inline-text");

    let (project, rel) = migrate::find_view(&projects, &unit.path).expect("view");
    let job = migrate::job(project, rel, &plan, unit, &diagnostics, None);
    assert!(
        job.findings
            .iter()
            .any(|finding| finding.rule == template[0].rule),
        "作业单要带上模板上的诊断：{:?}",
        job.findings.iter().map(|f| &f.rule).collect::<Vec<_>>()
    );
    assert!(
        !plan
            .units
            .iter()
            .filter(|unit| unit.kind == "shared")
            .any(|unit| unit.diagnostics.by_rule.contains_key(&template[0].rule)),
        "模板诊断不能落进 unit:shared"
    );
}

/// 样式诊断要能直接告诉 agent「改哪个选择器、这块钱还有什么、命中本单元哪些节点」。
#[test]
fn style_findings_carry_selector_declarations_and_matches() {
    let f = style_fixture(
        "style-context",
        r#"<view class="box"><view class="card"/></view>"#,
        ".card { display: block; float: left; color: red; }\n",
    );
    let job = job_for_page(f.path());
    let finding = job
        .findings
        .iter()
        .find(|finding| finding.rule == "skyline/unsupported-property")
        .expect("float 应该被报出来");

    assert_eq!(finding.selector.as_deref(), Some(".card"));
    assert_eq!(finding.property.as_deref(), Some("float"));
    let declarations: Vec<_> = finding
        .declarations
        .iter()
        .map(|declaration| declaration.property.as_str())
        .collect();
    assert_eq!(declarations, ["display", "float", "color"]);
    assert_eq!(finding.declarations[1].value, "left");

    assert_eq!(finding.match_count, 1);
    assert_eq!(finding.matches.len(), 1);
    assert_eq!(finding.matches[0].path, "pages/index/index.wxml");
    assert_eq!(finding.matches[0].tag, "view");
    assert_eq!(finding.matches[0].line, 1);

    assert!(!finding.shared, "普通页面自带样式不是共享文件");
    assert_eq!(finding.affected_units, 0);
    assert!(job.shared_findings.is_empty());
}

/// 匹配只算本单元文件：别的页面也有 `.card` 时不能算进来，否则 agent 会改错页面。
#[test]
fn style_matches_stay_within_the_unit() {
    let f = Fixture::new("style-matches-unit");
    f.write(
        "project.config.json",
        r#"{"miniprogramRoot":"miniprogram/"}"#,
    )
    .write(
        "miniprogram/app.json",
        r#"{"pages":["pages/index/index","pages/other/other"],"renderer":"skyline"}"#,
    )
    .write("miniprogram/pages/index/index.json", r#"{}"#)
    .write(
        "miniprogram/pages/index/index.wxml",
        r#"<view class="box"><view class="card"/></view>"#,
    )
    .write("miniprogram/pages/index/index.js", "Page({})")
    .write(
        "miniprogram/pages/index/index.wxss",
        ".card { float: left; }\n",
    )
    .write("miniprogram/pages/other/other.json", r#"{}"#)
    .write(
        "miniprogram/pages/other/other.wxml",
        r#"<view class="card"/><view class="card"/>"#,
    )
    .write("miniprogram/pages/other/other.js", "Page({})");

    let (projects, report) = analyse(f.path());
    let plan = migrate::plan(&projects, &report, None);
    let unit = plan
        .units
        .iter()
        .find(|unit| unit.path == "pages/index/index.wxml")
        .expect("page unit");
    let (project, rel) = migrate::find_view(&projects, &unit.path).expect("view");
    let diagnostics = migrate::diagnostics_for(&plan, unit, &report);
    let job = migrate::job(project, rel, &plan, unit, &diagnostics, None);

    let finding = job
        .findings
        .iter()
        .find(|finding| finding.rule == "skyline/unsupported-property")
        .expect("float 应该被报出来");
    assert_eq!(
        finding.match_count, 1,
        "另一个页面里的两个 .card 不能算进本单元"
    );
    assert!(
        finding
            .matches
            .iter()
            .all(|node| node.path == "pages/index/index.wxml"),
        "命中节点必须只来自本单元文件：{:?}",
        finding.matches
    );
}

/// 被多个单元 `@import` 的文件：`shared: true` + 引用单元数，且作业单顶部有汇总。
#[test]
fn shared_style_findings_are_marked_for_separate_approval() {
    let f = Fixture::new("shared-style");
    f.write(
        "project.config.json",
        r#"{"miniprogramRoot":"miniprogram/"}"#,
    )
    .write(
        "miniprogram/app.json",
        r#"{"pages":["pages/index/index","pages/other/other"],"renderer":"skyline"}"#,
    )
    .write("miniprogram/pages/index/index.json", r#"{}"#)
    .write("miniprogram/pages/index/index.wxml", r#"<view class="x"/>"#)
    .write("miniprogram/pages/index/index.js", "Page({})")
    .write(
        "miniprogram/pages/index/index.wxss",
        "@import \"../../styles/common.wxss\";\n",
    )
    .write("miniprogram/pages/other/other.json", r#"{}"#)
    .write("miniprogram/pages/other/other.wxml", r#"<view class="x"/>"#)
    .write("miniprogram/pages/other/other.js", "Page({})")
    .write(
        "miniprogram/pages/other/other.wxss",
        "@import \"../../styles/common.wxss\";\n",
    )
    .write("miniprogram/styles/common.wxss", ".x { float: left; }\n");

    let (projects, report) = analyse(f.path());
    let plan = migrate::plan(&projects, &report, None);
    for path in ["pages/index/index.wxml", "pages/other/other.wxml"] {
        let unit = plan
            .units
            .iter()
            .find(|unit| unit.path == path)
            .expect("page unit");
        let (project, rel) = migrate::find_view(&projects, &unit.path).expect("view");
        let diagnostics = migrate::diagnostics_for(&plan, unit, &report);
        let job = migrate::job(project, rel, &plan, unit, &diagnostics, None);

        let finding = job
            .findings
            .iter()
            .find(|finding| finding.path == "styles/common.wxss")
            .expect("共享样式上的诊断");
        assert!(finding.shared, "{path} 也要标记共享");
        assert_eq!(finding.affected_units, 2, "被两个页面引用");
        assert_eq!(
            job.shared_findings.len(),
            1,
            "{path} 的共享汇总应列出 common.wxss"
        );
        assert_eq!(job.shared_findings[0].path, "styles/common.wxss");
        assert_eq!(job.shared_findings[0].count, 1);
    }
}

/// `@import` 引入的样式（含传递闭包）同样属于引入它的页面。
#[test]
fn imported_style_diagnostics_belong_to_the_page() {
    let f = Fixture::new("import-owner");
    f.write(
        "project.config.json",
        r#"{"miniprogramRoot":"miniprogram/"}"#,
    )
    .write(
        "miniprogram/app.json",
        r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
    )
    .write("miniprogram/pages/index/index.json", r#"{}"#)
    .write("miniprogram/pages/index/index.wxml", r#"<view class="x"/>"#)
    .write("miniprogram/pages/index/index.js", "Page({})")
    .write(
        "miniprogram/pages/index/index.wxss",
        "@import \"../../styles/common.wxss\";\n",
    )
    .write("miniprogram/styles/common.wxss", ".x { float: left; }\n");

    let (projects, report) = analyse(f.path());
    let plan = migrate::plan(&projects, &report, None);
    let unit = plan
        .units
        .iter()
        .find(|unit| unit.path == "pages/index/index.wxml")
        .expect("page unit");
    assert!(
        unit.files.iter().any(|file| file == "styles/common.wxss"),
        "被 @import 的样式要进单元 files：{:?}",
        unit.files
    );

    let diagnostics = migrate::diagnostics_for(&plan, unit, &report);
    let style: Vec<_> = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.path == "styles/common.wxss")
        .collect();
    assert_eq!(style.len(), 1, "样式上只有一条 unsupported-property");
    assert_eq!(unit.diagnostics.total, 1, "plan 要把样式命中算进单元");

    let (project, rel) = migrate::find_view(&projects, &unit.path).expect("view");
    let job = migrate::job(project, rel, &plan, unit, &diagnostics, None);
    assert_eq!(job.findings.len(), 1, "作业单要带上样式上的诊断");
    assert_eq!(job.findings[0].rule, "skyline/unsupported-property");
    assert_eq!(job.findings[0].path, "styles/common.wxss");
    assert_eq!(
        job.unit.files, unit.files,
        "job.unit.files 要与 plan 单元文件同源"
    );
    assert!(
        job.unit
            .files
            .iter()
            .any(|file| file == "styles/common.wxss")
    );
}

fn inline_text_fixture(name: &str, wxss: &str) -> Fixture {
    let f = Fixture::new(name);
    f.write(
        "project.config.json",
        r#"{"miniprogramRoot":"miniprogram/"}"#,
    )
    .write(
        "miniprogram/app.json",
        r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"defaultDisplayBlock":true,"defaultContentBox":true}}}"#,
    )
    .write("miniprogram/pages/index/index.json", r#"{}"#)
    .write(
        "miniprogram/pages/index/index.wxml",
        r#"<view class="box">hello <view class="inner">x</view></view>"#,
    )
    .write("miniprogram/pages/index/index.js", "Page({})")
    .write("miniprogram/pages/index/index.wxss", wxss);
    f
}

fn style_fixture(name: &str, wxml: &str, wxss: &str) -> Fixture {
    let f = Fixture::new(name);
    f.write(
        "project.config.json",
        r#"{"miniprogramRoot":"miniprogram/"}"#,
    )
    .write(
        "miniprogram/app.json",
        r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
    )
    .write("miniprogram/pages/index/index.json", r#"{}"#)
    .write("miniprogram/pages/index/index.wxml", wxml)
    .write("miniprogram/pages/index/index.js", "Page({})")
    .write("miniprogram/pages/index/index.wxss", wxss);
    f
}

fn job_for_page(root: &Path) -> migrate::Job {
    let (projects, report) = analyse(root);
    let plan = migrate::plan(&projects, &report, None);
    let unit = plan
        .units
        .iter()
        .find(|unit| unit.path == "pages/index/index.wxml")
        .expect("page unit");
    let (project, rel) = migrate::find_view(&projects, &unit.path).expect("view");
    let diagnostics = migrate::diagnostics_for(&plan, unit, &report);
    migrate::job(project, rel, &plan, unit, &diagnostics, None)
}

/// 元素整体下移一行，诊断 id 不变（行列只用于展示）。
#[test]
fn finding_id_survives_line_shift() {
    let f = inline_text_fixture("shift", ".box { display: block; }");
    let before = job_for_page(f.path());
    let finding = before
        .findings
        .iter()
        .find(|finding| finding.rule == "skyline/no-inline-text")
        .expect("no-inline-text");
    assert_eq!(finding.span.line, 1);
    assert_eq!(finding.path, "pages/index/index.wxml");
    assert_eq!(
        finding.anchor.as_ref().map(|anchor| anchor.classes.clone()),
        Some(vec!["box".to_string()])
    );

    // 在最上面插一行空行：所有行号 +1，元素本身没变。
    f.write(
        "miniprogram/pages/index/index.wxml",
        "\n<view class=\"box\">hello <view class=\"inner\">x</view></view>",
    );
    let after = job_for_page(f.path());
    let moved = after
        .findings
        .iter()
        .find(|finding| finding.rule == "skyline/no-inline-text")
        .expect("no-inline-text");

    assert_eq!(moved.span.line, 2, "行号应该跟着移动");
    assert_eq!(moved.id, finding.id, "id 不该随行号变化");
}

/// 只内联规则声明关心的属性，不把整份级联塞进作业单。
#[test]
fn cascade_only_carries_declared_properties() {
    let f = inline_text_fixture(
        "cascade",
        ".box { display: block; z-index: 3; color: red; }",
    );
    let job = job_for_page(f.path());
    let finding = job
        .findings
        .iter()
        .find(|finding| finding.rule == "skyline/no-inline-text")
        .expect("no-inline-text");
    let cascade = &finding.facts.as_ref().expect("facts").cascade;

    assert!(cascade.contains_key("display"));
    assert!(
        !cascade.contains_key("z-index"),
        "no-inline-text 没声明 z-index，不该内联"
    );
    assert!(!cascade.contains_key("color"));
    assert!(
        !finding.counterexamples.is_empty(),
        "规则应该带反例，否则 agent 容易过度发挥"
    );
}

fn baseline_of(root: &Path) -> migrate::Baseline {
    let (projects, report) = analyse(root);
    let plan = migrate::plan(&projects, &report, None);
    migrate::baseline(&migrate::identify(&projects, &plan, &report))
}

fn verify_now(root: &Path, baseline: &migrate::Baseline) -> migrate::VerifyReport {
    let (projects, report) = analyse(root);
    let plan = migrate::plan(&projects, &report, None);
    let current = migrate::identify(&projects, &plan, &report);
    migrate::verify(baseline, &current, None, None)
}

#[test]
fn verify_is_clean_on_unchanged_code() {
    let f = inline_text_fixture("verify-same", ".box { display: block; }");
    let baseline = baseline_of(f.path());
    let result = verify_now(f.path(), &baseline);

    assert_eq!(result.new, 0);
    assert_eq!(result.fixed, 0);
    assert!(!result.blocks());
}

/// 门禁的核心性质：挪行不算新增，也不算修复。
#[test]
fn verify_ignores_line_shift() {
    let f = inline_text_fixture("verify-shift", ".box { display: block; }");
    let baseline = baseline_of(f.path());

    f.write(
        "miniprogram/pages/index/index.wxml",
        "\n\n<view class=\"box\">hello <view class=\"inner\">x</view></view>",
    );
    let result = verify_now(f.path(), &baseline);

    assert_eq!(result.new, 0, "挪行不该产生新诊断：{:?}", result.findings);
    assert_eq!(result.fixed, 0, "挪行不该把旧诊断算成修复");
    assert!(!result.blocks());
}

#[test]
fn verify_reports_new_finding_and_blocks() {
    let f = inline_text_fixture("verify-new", ".box { display: block; }");
    let baseline = baseline_of(f.path());

    f.write(
        "miniprogram/pages/index/index.wxml",
        "<view class=\"box\">hello <view class=\"inner\">x</view></view>\n<view class=\"other\">hi <view/></view>",
    );
    let result = verify_now(f.path(), &baseline);

    assert_eq!(result.new, 1);
    assert_eq!(result.fixed, 0);
    assert!(result.blocks());
    assert_eq!(result.findings[0].kind, "new");
    assert!(result.findings[0].id.contains("view.other"));
}

#[test]
fn verify_reports_fixed_finding() {
    let f = inline_text_fixture("verify-fixed", ".box { display: block; }");
    let baseline = baseline_of(f.path());

    // 去掉文本，只剩元素：no-inline-text 不再成立。
    f.write(
        "miniprogram/pages/index/index.wxml",
        "<view class=\"box\"><view class=\"inner\">x</view></view>",
    );
    let result = verify_now(f.path(), &baseline);

    assert_eq!(result.new, 0);
    assert_eq!(result.fixed, 1);
    assert!(!result.blocks(), "修复不该拦门禁");
    assert_eq!(result.findings[0].kind, "fixed");
    let message = result.findings[0]
        .message
        .as_deref()
        .expect("fixed 要带上 message，才能看出修掉的是哪条");
    assert!(message.contains("文本与元素混排"), "{message}");
    assert!(
        result.findings[0].help.is_some(),
        "基线存了 help，就一并带出来"
    );
}

/// `--json` 里不再输出恒为 `null` 的 `baseline.sha256`。
#[test]
fn verify_baseline_ref_has_no_sha256() {
    let f = inline_text_fixture("verify-sha", ".box { display: block; }");
    let baseline = baseline_of(f.path());
    let result = verify_now(f.path(), &baseline);
    let json = migrate::to_json(&result);
    assert!(
        !json.contains("\"sha256\""),
        "baseline 摘要字段应当去掉而不是输出 null：{json}"
    );
}

/// 样式诊断按 selector + property 定身份：顶部插空行、整块挪动都不算变化。
#[test]
fn wxss_line_shift_keeps_identity() {
    let f = style_fixture(
        "wxss-shift",
        r#"<view class="x"/><view class="y"/>"#,
        ".x { float: left; }\n.y { float: right; }\n",
    );
    let baseline = baseline_of(f.path());
    assert_eq!(
        baseline
            .findings
            .iter()
            .filter(|finding| finding.path == "pages/index/index.wxss")
            .count(),
        2
    );

    f.write(
        "miniprogram/pages/index/index.wxss",
        "\n\n\n\n\n.x { float: left; }\n.y { float: right; }\n",
    );
    let shifted = verify_now(f.path(), &baseline);
    assert_eq!(
        shifted.new, 0,
        "插空行不该产生新诊断：{:?}",
        shifted.findings
    );
    assert_eq!(shifted.fixed, 0, "插空行不该把旧诊断算成修复");

    // 整块挪动：两个规则的顺序对调，身份仍按选择器算。
    f.write(
        "miniprogram/pages/index/index.wxss",
        ".y { float: right; }\n.x { float: left; }\n",
    );
    let moved = verify_now(f.path(), &baseline);
    assert_eq!(moved.new, 0, "挪整块不该产生新诊断：{:?}", moved.findings);
    assert_eq!(moved.fixed, 0, "挪整块不该把旧诊断算成修复");
}

/// 真正新增一条违规 → new 1。
#[test]
fn wxss_new_declaration_is_one_new_finding() {
    let f = style_fixture("wxss-new", r#"<view class="x"/>"#, ".x { float: left; }\n");
    let baseline = baseline_of(f.path());

    f.write(
        "miniprogram/pages/index/index.wxss",
        ".x { float: left; }\n.x { float: right; }\n",
    );
    let result = verify_now(f.path(), &baseline);
    assert_eq!(result.new, 1, "新增声明应只算一条：{:?}", result.findings);
    assert_eq!(result.fixed, 0);
}

/// 删掉一条既有声明 → fixed 1。
#[test]
fn wxss_deleted_declaration_is_one_fixed_finding() {
    let f = style_fixture(
        "wxss-delete",
        r#"<view class="x"/>"#,
        ".x { float: left; }\n",
    );
    let baseline = baseline_of(f.path());

    f.write("miniprogram/pages/index/index.wxss", ".x { color: red; }\n");
    let result = verify_now(f.path(), &baseline);
    assert_eq!(result.new, 0);
    assert_eq!(
        result.fixed, 1,
        "删掉声明应只算一条修复：{:?}",
        result.findings
    );
}

/// 同一选择器里两条同类声明：删掉其中一条只算 fixed 1，另一条照旧匹配。
#[test]
fn duplicate_property_occurrence_keeps_the_other_finding() {
    let f = style_fixture(
        "wxss-duplicate",
        r#"<view class="x"/>"#,
        ".x { float: left; float: right; }\n",
    );
    let baseline = baseline_of(f.path());
    assert_eq!(
        baseline
            .findings
            .iter()
            .filter(|finding| finding.path == "pages/index/index.wxss")
            .count(),
        2
    );

    f.write(
        "miniprogram/pages/index/index.wxss",
        ".x { float: left; }\n",
    );
    let result = verify_now(f.path(), &baseline);
    assert_eq!(result.new, 0, "另一条同类声明不该变成新增");
    assert_eq!(result.fixed, 1, "只应报删掉的那一条");
}

/// `--unit` 只比对这个单元，别的单元的变化不算进来。
#[test]
fn verify_filters_by_unit() {
    let f = inline_text_fixture("verify-unit", ".box { display: block; }");
    let baseline = baseline_of(f.path());
    let (projects, report) = analyse(f.path());
    let plan = migrate::plan(&projects, &report, None);
    let page = plan
        .units
        .iter()
        .find(|unit| unit.path == "pages/index/index.wxml")
        .expect("page unit");

    f.write(
        "miniprogram/pages/index/index.wxml",
        "<view class=\"box\">hello <view class=\"inner\">x</view></view>\n<view class=\"other\">hi <view/></view>",
    );
    let (projects, report) = analyse(f.path());
    let plan = migrate::plan(&projects, &report, None);
    let current = migrate::identify(&projects, &plan, &report);

    let only_page = migrate::verify(&baseline, &current, Some(&page.id), None);
    assert_eq!(only_page.new, 1);

    let other = migrate::verify(&baseline, &current, Some("unit:pages/other/other"), None);
    assert_eq!(other.new, 0, "别的单元不受影响");
    assert_eq!(other.fixed, 0);
}

/// 单元图夹具：页面 → a → b。
fn chain_fixture(name: &str) -> Fixture {
    let f = Fixture::new(name);
    f.write(
        "project.config.json",
        r#"{"miniprogramRoot":"miniprogram/"}"#,
    )
    .write(
        "miniprogram/app.json",
        r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
    )
    .write(
        "miniprogram/pages/index/index.json",
        r#"{"usingComponents":{"a":"/components/a/a"}}"#,
    )
    .write("miniprogram/pages/index/index.wxml", "<a/>")
    .write("miniprogram/pages/index/index.js", "Page({})")
    .write(
        "miniprogram/components/a/a.json",
        r#"{"component":true,"usingComponents":{"b":"../b/b"}}"#,
    )
    .write("miniprogram/components/a/a.wxml", "<b/>")
    .write("miniprogram/components/a/a.js", "Component({})")
    .write("miniprogram/components/b/b.json", r#"{"component":true}"#)
    .write("miniprogram/components/b/b.wxml", "<view/>")
    .write("miniprogram/components/b/b.js", "Component({})");
    f
}

/// 给每个单元都标上「有诊断」，好在不看真实规则的情况下测排序。
fn all_units_need_work(root: &Path) -> migrate::Plan {
    let (projects, report) = analyse(root);
    let mut plan = migrate::plan(&projects, &report, None);
    for unit in &mut plan.units {
        unit.diagnostics.total = 1;
    }
    plan
}

#[test]
fn next_walks_batches_from_leaves_to_pages() {
    let f = chain_fixture("next-order");
    let plan = all_units_need_work(f.path());
    let mut ledger = migrate::Ledger::default();

    let first = migrate::next_unit(&plan, &ledger).unit.expect("first");
    assert!(
        first.path.ends_with("components/b/b.wxml"),
        "叶子先做：{}",
        first.path
    );

    ledger.record(&first.id, migrate::Status::Accepted, None, None);
    let second = migrate::next_unit(&plan, &ledger).unit.expect("second");
    assert!(
        second.path.ends_with("components/a/a.wxml"),
        "{}",
        second.path
    );

    ledger.record(&second.id, migrate::Status::Accepted, None, None);
    let third = migrate::next_unit(&plan, &ledger).unit.expect("third");
    assert!(
        third.path.ends_with("pages/index/index.wxml"),
        "{}",
        third.path
    );

    ledger.record(&third.id, migrate::Status::Accepted, None, None);
    assert!(
        migrate::next_unit(&plan, &ledger).unit.is_none(),
        "全部接受后不该再有下一个"
    );
}

#[test]
fn next_skips_units_without_diagnostics() {
    let f = chain_fixture("next-skip");
    let mut plan = all_units_need_work(f.path());
    // 只留页面需要迁移，两个组件无诊断。
    for unit in &mut plan.units {
        if unit.kind == "component" {
            unit.diagnostics.total = 0;
        }
    }
    let next = migrate::next_unit(&plan, &migrate::Ledger::default());

    assert_eq!(next.progress.total, 1);
    assert_eq!(next.progress.not_needed, 2);
    assert!(
        next.unit
            .expect("page")
            .path
            .ends_with("pages/index/index.wxml")
    );
}

#[test]
fn next_reports_failures_and_terminal_states() {
    let f = chain_fixture("next-failures");
    let plan = all_units_need_work(f.path());
    let mut ledger = migrate::Ledger::default();
    let unit = migrate::next_unit(&plan, &ledger).unit.expect("unit");

    ledger.record(
        &unit.id,
        migrate::Status::Migrated,
        Some(migrate::Failure {
            gate: Some("l0-static".to_string()),
            reason: "new=2".to_string(),
        }),
        None,
    );
    let next = migrate::next_unit(&plan, &ledger);
    assert_eq!(
        next.unit.as_ref().map(|unit| unit.id.as_str()),
        Some(unit.id.as_str())
    );
    assert_eq!(next.failures, 1);
    assert_eq!(
        next.last_failure
            .as_ref()
            .map(|failure| failure.reason.as_str()),
        Some("new=2")
    );

    // blocked 是终态，不再返回。
    ledger.record(&unit.id, migrate::Status::Blocked, None, None);
    let after = migrate::next_unit(&plan, &ledger);
    assert_ne!(
        after.unit.as_ref().map(|unit| unit.id.as_str()),
        Some(unit.id.as_str())
    );
    assert_eq!(after.progress.blocked, 1);
}

#[test]
fn record_accept_clears_last_failure_but_keeps_the_count() {
    let mut ledger = migrate::Ledger::default();
    ledger.record(
        "unit:x",
        migrate::Status::Migrated,
        Some(migrate::Failure {
            gate: None,
            reason: "boom".to_string(),
        }),
        None,
    );
    ledger.record("unit:x", migrate::Status::Accepted, None, None);

    let state = &ledger.units["unit:x"];
    assert_eq!(state.status, migrate::Status::Accepted);
    assert_eq!(state.failures, 1, "重试次数是历史事实，接受后仍保留");
    assert!(state.last_failure.is_none(), "接受后不该再挂着上次失败");
}

#[test]
fn ledger_prunes_units_missing_from_the_plan() {
    let f = chain_fixture("ledger-prune");
    let plan = all_units_need_work(f.path());
    let mut ledger = migrate::Ledger::default();
    ledger.record(
        "unit:components/gone/gone",
        migrate::Status::Accepted,
        None,
        None,
    );

    let removed = ledger.prune(&plan);
    assert_eq!(removed, 1);
    assert!(!ledger.units.contains_key("unit:components/gone/gone"));
}

/// `verified` 在等人接受，`next` 不该再把它派给 agent。
#[test]
fn next_skips_units_waiting_for_acceptance() {
    let f = chain_fixture("next-verified");
    let plan = all_units_need_work(f.path());
    let mut ledger = migrate::Ledger::default();

    let leaf = migrate::next_unit(&plan, &ledger).unit.expect("leaf");
    ledger.record(&leaf.id, migrate::Status::Verified, None, None);

    let next = migrate::next_unit(&plan, &ledger);
    assert_ne!(
        next.unit.as_ref().map(|unit| unit.id.as_str()),
        Some(leaf.id.as_str()),
        "已过门禁的单元不该重跑"
    );
    assert_eq!(next.progress.verified, 1);

    // 人打回：显式改回 migrated，它才重新排队。
    ledger.record(&leaf.id, migrate::Status::Migrated, None, None);
    assert_eq!(
        migrate::next_unit(&plan, &ledger)
            .unit
            .as_ref()
            .map(|unit| unit.id.as_str()),
        Some(leaf.id.as_str())
    );
}

/// JSON 工程夹具：页面存在但内容为空，只让配置类规则产生诊断。
fn json_fixture(name: &str, app: &str) -> Fixture {
    let f = Fixture::new(name);
    f.write(
        "project.config.json",
        r#"{"miniprogramRoot":"miniprogram/"}"#,
    )
    .write("miniprogram/app.json", app)
    .write("miniprogram/pages/index/index.json", "{}")
    .write("miniprogram/pages/index/index.wxml", "<view/>")
    .write("miniprogram/pages/index/index.js", "Page({})")
    .write("miniprogram/pages/index/index.wxss", "");
    f
}

/// JSON 重排（多行 / 缩进变化）不改 id：身份是指针 + 值指纹。
#[test]
fn json_reformat_keeps_identity() {
    let f = json_fixture(
        "json-reformat",
        r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"defaultDisplayBlock":"yes","defaultContentBox":true}}}"#,
    );
    let baseline = baseline_of(f.path());
    assert!(
        baseline
            .findings
            .iter()
            .any(|finding| finding.path == "app.json"
                && finding.rule == "skyline/renderer-options")
    );

    f.write(
        "miniprogram/app.json",
        r#"{
  "pages": ["pages/index/index"],
  "renderer": "skyline",
  "rendererOptions": {
    "skyline": {
      "defaultDisplayBlock": "yes",
      "defaultContentBox": true
    }
  }
}"#,
    );
    let result = verify_now(f.path(), &baseline);
    assert_eq!(result.new, 0, "重排不该产生新诊断：{:?}", result.findings);
    assert_eq!(result.fixed, 0, "重排不该把旧诊断算成修复");
}

/// 真加一个会触发诊断的配置项 → 只 new 1；已有条目不漂。
#[test]
fn json_added_config_key_is_one_new_finding() {
    let f = json_fixture(
        "json-add-key",
        r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"defaultDisplayBlock":"yes","defaultContentBox":true}}}"#,
    );
    let baseline = baseline_of(f.path());

    f.write(
        "miniprogram/app.json",
        r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"defaultDisplayBlock":"yes","defaultContentBox":true,"tagNameStyleIsolation":"bogus"}}}"#,
    );
    let result = verify_now(f.path(), &baseline);
    assert_eq!(result.new, 1, "新增坏配置应只算一条：{:?}", result.findings);
    assert_eq!(result.fixed, 0, "已有诊断不该被算成修复");
}

/// 删掉一个开关 → 对应 finding 是 new，不是 new+fixed 成对。
#[test]
fn json_removed_switch_is_not_a_pair() {
    let f = json_fixture(
        "json-remove-switch",
        r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"defaultDisplayBlock":true}}}"#,
    );
    let baseline = baseline_of(f.path());
    let content_box = baseline
        .findings
        .iter()
        .find(|finding| finding.id.contains("defaultContentBox"))
        .expect("baseline 里应只有 defaultContentBox 那条 info");

    f.write(
        "miniprogram/app.json",
        r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{}}}"#,
    );
    let result = verify_now(f.path(), &baseline);
    assert_eq!(result.new, 0, "只有新增 info：{:?}", result.findings);
    assert_eq!(result.new_info, 1, "defaultDisplayBlock 应成为唯一新增");
    assert_eq!(result.fixed, 0, "defaultContentBox 不该漂成修复");
    assert!(
        result
            .findings
            .iter()
            .any(|entry| entry.kind == "new" && entry.id.contains("defaultDisplayBlock")),
        "{:?}",
        result.findings
    );
    // 旧那条 contentBox 仍是同一条（id 没变）。
    let now = baseline_of(f.path());
    assert!(
        now.findings
            .iter()
            .any(|finding| finding.id == content_box.id),
        "contentBox 的 id 不该随相邻键变化"
    );
}

/// `Baseline::load` 只接受当前形态：旧版字段、缺字段都直接报错。
#[test]
fn baseline_load_rejects_old_formats() {
    let f = json_fixture(
        "baseline-load",
        r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"defaultDisplayBlock":true,"defaultContentBox":true}}}"#,
    );
    let baseline = baseline_of(f.path());
    let text = migrate::to_json(&baseline);
    assert!(migrate::Baseline::load(&text).is_ok());

    // 旧版：多出 idFormat 字段。
    let with_id_format = text.replace(
        "  \"rules\":",
        "  \"idFormat\": \"structural-v2\",\n  \"rules\":",
    );
    let error = migrate::Baseline::load(&with_id_format).expect_err("旧字段应被拒绝");
    assert!(error.contains("请重新生成"), "{error}");

    // 更旧的：没有 rules 字段。
    let mut without_rules: serde_json::Value = serde_json::from_str(&text).expect("合法 JSON");
    without_rules.as_object_mut().unwrap().remove("rules");
    let error = migrate::Baseline::load(&serde_json::to_string(&without_rules).unwrap())
        .expect_err("缺字段应被拒绝");
    assert!(error.contains("请重新生成"), "{error}");
}

// ---------------------------------------------------------------- 输出语义

fn identified(id: &str, severity: Severity) -> migrate::Identified {
    identified_with(
        id,
        "skyline/no-inline-text",
        "pages/index/index.wxml",
        severity,
    )
}

fn identified_with(id: &str, rule: &str, path: &str, severity: Severity) -> migrate::Identified {
    migrate::Identified {
        id: id.to_string(),
        unit: "unit:pages/index/index".to_string(),
        rule: rule.to_string(),
        severity,
        path: path.to_string(),
        span: migrate::Span {
            line: 1,
            column: 1,
            end_line: 1,
            end_column: 2,
        },
        message: "m".to_string(),
        help: None,
    }
}

/// 新增 error 仍叫 `new`（拦门禁）。
#[test]
fn verify_new_error_keeps_kind_new() {
    let baseline = migrate::baseline(&[]);
    let current = vec![identified("error", Severity::Error)];
    let report = migrate::verify(&baseline, &current, None, None);

    assert_eq!(report.new, 1);
    assert_eq!(report.new_info, 0);
    assert!(report.blocks());
    assert_eq!(report.findings[0].kind, "new");
    assert_eq!(report.findings[0].previous_severity, None);
}

/// info 级新增不拦门禁；`kind` 仍是 `new`，区别在 severity。
#[test]
fn verify_info_new_finding_does_not_block() {
    let f = inline_text_fixture("verify-info-kind", ".box { display: block; }");
    let baseline = baseline_of(f.path());

    f.write(
        "miniprogram/pages/index/index.wxml",
        "<view class=\"box\"><text overflow=\"ellipsis\" max-lines=\"2\">第一行\n第二行</text></view>",
    );
    let report = verify_now(f.path(), &baseline);

    assert_eq!(report.new, 0, "info 不该计入 new：{:?}", report.findings);
    assert_eq!(report.new_info, 1);
    assert!(!report.blocks(), "info 级新增不能把门禁判死");
    let entry = report
        .findings
        .iter()
        .find(|entry| entry.rule == "skyline/text-hard-break-ellipsis")
        .expect("应有新增的 info 诊断");
    assert_eq!(entry.kind, "new");
    assert_eq!(entry.severity, Severity::Info);
    assert_eq!(entry.previous_severity, None);
}

/// `changed` 用 `previousSeverity` 表达方向，不再是两个 kind。
#[test]
fn verify_severity_change_carries_previous_severity() {
    let raised = migrate::verify(
        &migrate::baseline(&[identified("same", Severity::Info)]),
        &[identified("same", Severity::Error)],
        None,
        None,
    );
    assert_eq!(raised.changed, 1);
    assert_eq!(raised.changed_up, 1);
    assert_eq!(raised.changed_down, 0);
    assert_eq!(raised.findings[0].kind, "changed");
    assert_eq!(raised.findings[0].severity, Severity::Error);
    assert_eq!(raised.findings[0].previous_severity, Some(Severity::Info));

    let lowered = migrate::verify(
        &migrate::baseline(&[identified("same", Severity::Error)]),
        &[identified("same", Severity::Info)],
        None,
        None,
    );
    assert_eq!(lowered.changed, 1);
    assert_eq!(lowered.changed_up, 0);
    assert_eq!(lowered.changed_down, 1);
    assert_eq!(lowered.findings[0].kind, "changed");
    assert_eq!(lowered.findings[0].severity, Severity::Info);
    assert_eq!(lowered.findings[0].previous_severity, Some(Severity::Error));
}

/// `plan.shared` 给出 `unit:shared` 的文件明细，条数与伪单元对得上。
#[test]
fn plan_shared_lists_files_with_counts() {
    let f = Fixture::new("shared-files");
    f.write(
        "project.config.json",
        r#"{"miniprogramRoot":"miniprogram/"}"#,
    )
    .write(
        "miniprogram/app.json",
        r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
    )
    .write("miniprogram/app.wxss", ".x { float: left; }\n")
    .write("miniprogram/pages/index/index.json", r#"{}"#)
    .write("miniprogram/pages/index/index.wxml", "<view class=\"x\"/>")
    .write("miniprogram/pages/index/index.js", "Page({})");

    let (projects, report) = analyse(f.path());
    let plan = migrate::plan(&projects, &report, None);
    let shared = plan
        .units
        .iter()
        .find(|unit| unit.kind == "shared")
        .unwrap();

    assert_eq!(plan.shared.diagnostics, shared.diagnostics.total);
    assert_eq!(
        plan.shared
            .files
            .iter()
            .map(|file| file.count)
            .sum::<usize>(),
        plan.shared.diagnostics
    );
    assert!(plan.shared.files.iter().any(|file| file.path == "app.json"));
    assert!(plan.shared.files.iter().any(|file| file.path == "app.wxss"));
    assert!(
        plan.shared
            .files
            .windows(2)
            .all(|pair| pair[0].count >= pair[1].count),
        "文件按条数降序：{:?}",
        plan.shared.files
    );
}

/// 文本明细只列前 5 个文件，其余折成「等 N 个文件」。
#[test]
fn shared_files_summary_truncates_after_five() {
    let shared = migrate::SharedFiles {
        diagnostics: 28,
        files: (1..=7)
            .map(|index| migrate::SharedFile {
                path: format!("file{index}.json"),
                count: 8 - index,
            })
            .collect(),
    };

    let summary = shared.files_summary().expect("有文件就要有明细");
    for index in 1..=5 {
        assert!(summary.contains(&format!("file{index}.json")), "{summary}");
    }
    assert!(!summary.contains("file6.json"), "只列前 5 个：{summary}");
    assert!(summary.contains("等 7 个文件"), "{summary}");
}

/// `ledger --json` 保留台账原字段，只新增 `shared`。
#[test]
fn ledger_json_adds_shared_without_changing_ledger_fields() {
    let f = Fixture::new("ledger-shared");
    f.write(
        "project.config.json",
        r#"{"miniprogramRoot":"miniprogram/"}"#,
    )
    .write(
        "miniprogram/app.json",
        r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
    )
    .write("miniprogram/pages/index/index.json", r#"{}"#)
    .write("miniprogram/pages/index/index.wxml", "<view/>")
    .write("miniprogram/pages/index/index.js", "Page({})");

    let (projects, report) = analyse(f.path());
    let plan = migrate::plan(&projects, &report, None);
    let ledger = migrate::Ledger::default();
    let json = migrate::to_json(&migrate::LedgerReport {
        ledger: &ledger,
        shared: &plan.shared,
    });
    let value: serde_json::Value = serde_json::from_str(&json).expect("合法 JSON");

    assert!(value.get("schemaVersion").is_some());
    assert!(value.get("generator").is_some());
    assert!(value.get("units").is_some());
    let shared = value.get("shared").expect("新增 shared 字段");
    assert_eq!(
        shared["diagnostics"].as_u64().unwrap() as usize,
        plan.shared.diagnostics
    );
    assert!(shared["files"].is_array());
}

// ---------------------------------------------------------------- 基线规则集与 augment

/// 新基线必须记录冻结时的规则集：字典序、去重、非空。
#[test]
fn baseline_records_rule_set() {
    let rules = migrate::baseline(&[]).rules;
    assert!(!rules.is_empty());
    assert!(rules.windows(2).all(|pair| pair[0] < pair[1]), "{rules:?}");
    assert!(
        rules.iter().any(|id| id == "skyline/no-inline-text"),
        "规则 id 应来自当前 Registry"
    );
    assert_eq!(rules.len(), rules.iter().collect::<HashSet<_>>().len());
    assert_eq!(rules, migrate::builtin_rule_ids());
}

/// augment 只追加「新规则」的诊断；已有条目逐字段不变。
#[test]
fn augment_appends_only_new_rule_findings() {
    let old = identified("old", Severity::Warning);
    let mut baseline = migrate::baseline(std::slice::from_ref(&old));
    baseline.rules = vec!["skyline/no-inline-text".to_string()];
    baseline.findings.retain(|finding| finding.id == old.id);
    let before = migrate::to_json(&baseline.findings[0]);

    let current = vec![
        old.clone(),
        identified_with(
            "new-1",
            "skyline/pseudo-inline-layout",
            "pages/b/index.wxml",
            Severity::Warning,
        ),
        identified_with(
            "new-2",
            "skyline/pseudo-inline-layout",
            "pages/b/index.wxml",
            Severity::Info,
        ),
    ];
    let report = migrate::augment(&mut baseline, &current);

    assert_eq!(report.added, 2);
    assert_eq!(report.rules.len(), 1);
    assert_eq!(report.rules[0].rule, "skyline/pseudo-inline-layout");
    assert_eq!(report.rules[0].count, 2);
    assert_eq!(report.rules[0].file_count, 1);
    assert_eq!(report.rules[0].files, vec!["pages/b/index.wxml"]);

    // 老条目一个字段都不动。
    let kept = baseline
        .findings
        .iter()
        .find(|finding| finding.id == "old")
        .expect("老条目还在");
    assert_eq!(migrate::to_json(kept), before);

    assert_eq!(baseline.findings.len(), 3);
    assert_eq!(baseline.summary.total, 3);
    assert_eq!(baseline.summary.warning, 2);
    assert_eq!(baseline.summary.info, 1);
    assert!(
        baseline
            .rules
            .iter()
            .any(|id| id == "skyline/pseudo-inline-layout")
    );
}

/// augment 之后立刻 verify，四项计数必须全零。
#[test]
fn augment_then_verify_is_clean() {
    let old = identified("old", Severity::Warning);
    let mut baseline = migrate::baseline(std::slice::from_ref(&old));
    baseline.rules = vec!["skyline/no-inline-text".to_string()];
    let current = vec![
        old.clone(),
        identified_with(
            "new-error",
            "skyline/pseudo-inline-layout",
            "pages/b/index.wxml",
            Severity::Error,
        ),
    ];

    migrate::augment(&mut baseline, &current);
    let report = migrate::verify(&baseline, &current, None, None);

    assert_eq!(report.new, 0, "{:?}", report.findings);
    assert_eq!(report.new_info, 0);
    assert_eq!(report.fixed, 0);
    assert_eq!(report.changed, 0);
    assert!(!report.blocks());
}

/// 吸收清单只列前 5 个文件，同时给出文件总数。
#[test]
fn augment_report_lists_first_five_files() {
    let mut baseline = migrate::baseline(&[identified("old", Severity::Warning)]);
    baseline.rules = vec!["skyline/no-inline-text".to_string()];
    let current: Vec<_> = (0..7)
        .map(|index| {
            identified_with(
                &format!("new-{index}"),
                "skyline/pseudo-inline-layout",
                &format!("pages/p{index}/index.wxml"),
                Severity::Warning,
            )
        })
        .collect();

    let report = migrate::augment(&mut baseline, &current);

    assert_eq!(report.rules[0].count, 7);
    assert_eq!(report.rules[0].file_count, 7);
    assert_eq!(report.rules[0].files.len(), 5);
}

/// 只报告「当前规则集里基线没记录的规则」。
#[test]
fn unknown_rules_lists_only_rules_missing_from_baseline() {
    let mut baseline = migrate::baseline(&[]);
    baseline.rules = vec!["skyline/no-inline-text".to_string()];
    let current = vec![
        "skyline/no-inline-text".to_string(),
        "skyline/pseudo-inline-layout".to_string(),
    ];
    assert_eq!(
        migrate::unknown_rules(&baseline, &current),
        vec!["skyline/pseudo-inline-layout"]
    );
}

/// `migration baseline`（不带 `--augment`）仍重算整份基线，并写入 `rules`。
#[test]
fn baseline_command_without_augment_writes_fresh_baseline_with_rules() {
    let f = inline_text_fixture("baseline-cli", ".box { display: block; }");
    let out = f.path().join(".skylint/baseline.json");
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_skylint"))
        .args(["migration", "baseline"])
        .arg(f.path())
        .args(["--out"])
        .arg(&out)
        .status()
        .expect("运行 skylint");
    assert!(status.success());

    let value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&out).expect("基线文件")).expect("合法 JSON");
    assert!(
        value["rules"]
            .as_array()
            .is_some_and(|rules| !rules.is_empty())
    );
}

// ------------------------------------------------------- 逐条豁免（waiver）

/// 豁免是「两边都不算」：不算新增，也不算修复，单独计数。
#[test]
fn waived_finding_is_neither_new_nor_fixed() {
    let f = inline_text_fixture("waive-verify", ".box { display: block; }");
    let baseline = baseline_of(f.path());
    let (projects, report) = analyse(f.path());
    let plan = migrate::plan(&projects, &report, None);
    let current = migrate::identify(&projects, &plan, &report);
    let id = current
        .iter()
        .find(|finding| finding.rule == "skyline/no-inline-text")
        .expect("夹具应有一条 no-inline-text")
        .id
        .clone();

    let waived: BTreeSet<&str> = [id.as_str()].into_iter().collect();
    let result = migrate::verify_with(&baseline, &current, None, None, &waived);

    assert_eq!(result.new, 0, "豁免的条目不该算新增：{:?}", result.findings);
    assert_eq!(result.fixed, 0, "豁免不是修复，不该进 fixed");
    assert_eq!(result.waived, 1);
    assert!(result.findings.is_empty());
    assert!(!result.blocks());
}

/// 豁免后在新代码里被删掉的那条，不该冒充「修复」。
#[test]
fn waived_finding_removed_from_code_is_not_counted_as_fixed() {
    let f = inline_text_fixture("waive-removed", ".box { display: block; }");
    let baseline = baseline_of(f.path());
    let (projects, report) = analyse(f.path());
    let plan = migrate::plan(&projects, &report, None);
    let current = migrate::identify(&projects, &plan, &report);
    let waived: BTreeSet<&str> = current
        .iter()
        .filter(|finding| finding.rule == "skyline/no-inline-text")
        .map(|finding| finding.id.as_str())
        .collect();
    assert_eq!(waived.len(), 1, "夹具应有一条 no-inline-text");

    // 代码里真的修掉了：只剩元素，不再混排。
    f.write(
        "miniprogram/pages/index/index.wxml",
        "<view class=\"box\"><view class=\"inner\">x</view></view>",
    );
    let (projects, report) = analyse(f.path());
    let plan = migrate::plan(&projects, &report, None);
    let current = migrate::identify(&projects, &plan, &report);
    let result = migrate::verify_with(&baseline, &current, None, None, &waived);

    assert_eq!(result.fixed, 0, "豁免过的条目即使被删掉也不算修复成绩");
    assert_eq!(result.waived, 0, "当前已无此诊断，waived 计数归零");
}

/// 豁免从报告里摘掉之后，`plan` 不再为它排队。
#[test]
fn waived_findings_disappear_from_the_plan() {
    let f = inline_text_fixture("waive-plan", ".box { display: block; }");
    let (projects, report) = analyse(f.path());
    let before = migrate::plan(&projects, &report, None);
    let total_before: usize = before.units.iter().map(|unit| unit.diagnostics.total).sum();
    assert!(total_before >= 1, "夹具先要有一条诊断");

    let mut report = report;
    let waived: BTreeSet<String> = migrate::diagnostic_ids(&projects, &report)
        .into_iter()
        .collect();
    let mut keep = migrate::diagnostic_ids(&projects, &report)
        .into_iter()
        .map(|id| !waived.contains(&id));
    report.retain_diagnostics(|_| keep.next().unwrap_or(true));
    assert!(report.diagnostics.is_empty(), "豁免项应被摘掉");

    let after = migrate::plan(&projects, &report, None);
    let total_after: usize = after.units.iter().map(|unit| unit.diagnostics.total).sum();
    assert_eq!(total_after, 0, "摘掉之后不该再有待修诊断");
    assert_eq!(
        report.summary.errors + report.summary.warnings + report.summary.infos,
        0
    );
}

/// 台账里的豁免能存能读，撤销返回原理由。
#[test]
fn ledger_keeps_waivers_with_their_reasons() {
    let mut ledger = migrate::Ledger::default();
    ledger.waive(
        "unit:pages/index/index",
        migrate::Waiver {
            id: "rule|path|anchor".to_string(),
            rule: "skyline/no-inline-text".to_string(),
            path: "pages/index/index.wxml".to_string(),
            reason: "预加载容器，静态不可见，不影响渲染".to_string(),
        },
    );

    let text = migrate::to_json(&ledger);
    let loaded: migrate::Ledger = migrate::from_json(&text).expect("台账应能读回");
    assert_eq!(loaded.waived_count(), 1);
    assert_eq!(
        loaded.waived_ids().into_iter().collect::<Vec<_>>(),
        vec!["rule|path|anchor"]
    );

    let mut loaded = loaded;
    let reason = loaded.unwaive("rule|path|anchor").expect("应该有这条豁免");
    assert!(reason.contains("预加载容器"));
    assert_eq!(loaded.waived_count(), 0);
}
