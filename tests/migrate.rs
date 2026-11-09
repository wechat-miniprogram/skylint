//! 迁移接口的端到端测试：真造工程，验证三件容易漂的事——批次顺序、诊断归属、
//! 以及「元素挪了位置诊断 id 不变」。

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use skylint::migrate;
use skylint::rules::Target;
use skylint::{Project, Registry, Report};

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
    migrate::job(project, rel, unit, &diagnostics, None)
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
