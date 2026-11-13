//! 迁移接口：单元划分（`plan`）、作业单（`job`）、元素事实（`facts`）的契约。
//!
//! 契约本身写在 `docs/MIGRATE.md`；这里只负责把已有的 [`Facts`] / 诊断组装成
//! 可下发的数据，**不新增任何语义**——渲染器、可达性、级联、祖先链全部来自
//! `inspect` 与 `report`，避免两套实现漂移。

mod guidance;
mod ledger;

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::diag::{Diagnostic, Severity};
use crate::input::wxss::{WxssDoc, WxssRule};
use crate::inspect::{Facts, LayoutFacts, LoadedDoc, Renderer, UnitKind};
use crate::matcher::{Dialect, Tree, Tri};
use crate::project::{Project, Scope, UnitFiles};
use crate::report::Report;
use crate::rules::wxml::Reach;

pub use guidance::{Guidance, family_of, guidance_for};
pub use ledger::{
    Failure, Ledger, LedgerReport, Next, Progress, Status, UnitState, Waiver, next as next_unit,
};

/// 产物格式版本。破坏性变更才 +1。
pub const SCHEMA_VERSION: u32 = 1;

// ---------------------------------------------------------------- 公共类型

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Generator {
    pub name: String,
    pub version: String,
    /// 生成时的提交；未知为 `null`（不影响两次 `plan` 的字节一致性）。
    pub commit: Option<String>,
}

impl Default for Generator {
    fn default() -> Self {
        Self {
            name: "skylint".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            commit: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BaselineRef {
    pub file: Option<String>,
    /// 基线里的诊断总数，方便和 `plan.summary` 对账。
    pub diagnostics: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub schema_version: u32,
    pub generator: Generator,
    pub baseline: Option<BaselineRef>,
    pub units: Vec<PlanUnit>,
    /// 按依赖深度分层，层内按路径排序。
    pub batches: Vec<Vec<String>>,
    pub summary: PlanSummary,
    /// `unit:shared` 的文件明细（无归属诊断按文件聚合）。
    pub shared: SharedFiles,
}

/// 无归属诊断（`unit:shared`）按文件聚合，条数降序、同数按路径。
///
/// 只含**不属于任何单元**的文件（`app.json` / `project.private.config.json` 等）；
/// 被多个单元引用的文件（`styles/common.wxss`）不属于这里，它们在各自作业单里
/// 以 `shared: true` 出现。
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedFiles {
    pub diagnostics: usize,
    pub files: Vec<SharedFile>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedFile {
    pub path: String,
    pub count: usize,
}

impl SharedFiles {
    /// 一行文件明细：`app.wxss（2 条）、project.private.config.json（1 条）`。
    /// 多于 5 个文件时只列前 5 个，末尾追加 `等 N 个文件`。
    pub fn files_summary(&self) -> Option<String> {
        if self.files.is_empty() {
            return None;
        }
        const SHOWN: usize = 5;
        let mut parts: Vec<String> = self
            .files
            .iter()
            .take(SHOWN)
            .map(|file| format!("{}（{} 条）", file.path, file.count))
            .collect();
        if self.files.len() > SHOWN {
            parts.push(format!("等 {} 个文件", self.files.len()));
        }
        Some(parts.join("、"))
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanUnit {
    pub id: String,
    /// 报告路径（插件视图带前缀），与诊断的 `path` 同源。
    pub path: String,
    /// 工程根相对的**文件系统**路径。
    ///
    /// `path` 是报告路径，宿主视图下它相对 `miniprogramRoot`（不是工程根），
    /// 插件视图下又带 `pluginRoot/` 前缀——两种视图不一致。runner 要拿它去
    /// `git diff` / `checkout`，必须有一份不带歧义的路径，所以这里算好。
    pub fs_path: String,
    /// 这个单元的全部文件（报告路径）：wxml / 样式 / 脚本 / json，以及它实际
    /// 引用（`<import>` / `<include>` / `@import`，含传递闭包）的模板与样式。
    ///
    /// 诊断归属按它判，而不是按「同名的某几个后缀」——样式源可以是 `.scss` /
    /// `.less`，枚举后缀会静默丢诊断（作业单里 findings 为空）。
    pub files: Vec<String>,
    pub kind: &'static str,
    pub renderer: &'static str,
    pub reach: &'static str,
    /// 依赖深度：不引用其他组件的单元是 0。
    pub batch: u32,
    pub usages: usize,
    pub diagnostics: DiagCounts,
    pub rule_families: Vec<&'static str>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagCounts {
    pub total: usize,
    pub error: usize,
    pub warning: usize,
    pub info: usize,
    pub by_rule: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanSummary {
    pub units: usize,
    pub with_diagnostics: usize,
    pub diagnostics: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub schema_version: u32,
    pub generator: Generator,
    pub baseline: Option<BaselineRef>,
    pub unit: JobUnit,
    pub findings: Vec<Finding>,
    /// 本作业单里必须单独审批的共享文件改动汇总：`shared: true` 的 finding 按文件计数。
    pub shared_findings: Vec<SharedFinding>,
    pub acceptance: Vec<Acceptance>,
    pub workarounds: Vec<Workaround>,
    /// 重试时由 runner 追加；生成时为空。
    pub attempts: Vec<Attempt>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobUnit {
    pub id: String,
    pub path: String,
    /// 工程根相对的文件系统路径，见 [`PlanUnit::fs_path`]。
    pub fs_path: String,
    /// 与 [`PlanUnit::files`] 同源：四件套 + 实际引用的模板 / 样式。
    pub files: Vec<String>,
    pub kind: &'static str,
    pub renderer: &'static str,
    pub reach: &'static str,
    pub style_isolation: Option<String>,
    pub default_display_block: bool,
    pub default_content_box: bool,
    pub scroll_view_auto_size: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    /// 跨运行稳定标识，见 `docs/MIGRATE.md` §4.2。
    pub id: String,
    /// 诊断所在文件（报告路径），与 `id` 里的路径同源。
    pub path: String,
    pub rule: String,
    pub severity: Severity,
    pub message: String,
    pub help: Option<String>,
    pub span: Span,
    /// 非 WXML 诊断没有元素。
    pub anchor: Option<Anchor>,
    pub facts: Option<FindingFacts>,
    pub preconditions: Vec<&'static str>,
    pub counterexamples: Vec<&'static str>,

    // ---- 样式诊断的定位上下文（非样式诊断全部为 `null` / 空）。
    /// 所在选择器原文；`@media` 等 at-rule 内的声明给的是**内层**选择器。
    pub selector: Option<String>,
    /// 声明属性名（小写）；选择器级诊断或非样式诊断为 `null`。
    pub property: Option<String>,
    /// 诊断所在规则块的全部声明，用于看到「改这条要顺手改哪些」。
    pub declarations: Vec<Declaration>,
    /// 本单元内该选择器命中的 WXML 节点，最多 5 条。
    pub matches: Vec<MatchNode>,
    /// 命中节点总数（`matches` 截断前）。
    pub match_count: usize,

    // ---- 共享文件标记。
    /// 该文件被 ≥2 个单元引用：改它会影响其他单元的验收范围。
    pub shared: bool,
    /// 引用该文件的单元数；`shared` 为 `false` 时为 0。
    pub affected_units: usize,
}

/// 共享文件汇总：这份作业单里有几条诊断落在同一个跨单元文件上。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedFinding {
    pub path: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Declaration {
    pub property: String,
    pub value: String,
    pub line: u32,
    pub column: u32,
}

/// 选择器在本单元 WXML 里的一个命中节点。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchNode {
    /// 命中的 WXML 文件（报告路径）。
    pub path: String,
    pub line: u32,
    pub column: u32,
    pub tag: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Span {
    pub line: u32,
    pub column: u32,
    pub end_line: u32,
    pub end_column: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Anchor {
    pub tag: String,
    pub classes: Vec<String>,
    /// 同文档内、同 `(tag, classes)` 元素中的序号。
    pub occurrence: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindingFacts {
    pub layout: LayoutFacts,
    /// 只含该规则声明关心的属性（`migrate::guidance`）。
    pub cascade: BTreeMap<String, Vec<crate::inspect::CascadeView>>,
    pub ancestors: crate::inspect::Ancestors,
    pub handlers: Vec<crate::inspect::HandlerView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Acceptance {
    pub id: &'static str,
    pub kind: &'static str,
    /// 命令模板：`{root}` / `{unit}` / `{unitPath}` / `{baseline}` / `{job}` 由 runner 替换。
    pub run: &'static str,
    pub expect: Expect,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Expect {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json_path: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub equals: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Workaround {
    pub id: String,
    pub reason: String,
    /// 什么时候可以删掉这条 workaround。
    pub expires_when: String,
    pub file: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Attempt {
    pub attempt: u32,
    pub agent: String,
    pub exit_code: i32,
    pub failed_gate: Option<String>,
    pub evidence: Option<String>,
}

// ---------------------------------------------------------------- plan

/// 由视图与诊断生成单元清单。
pub fn plan(views: &[Project], report: &Report, baseline: Option<BaselineRef>) -> Plan {
    let mut seeds: Vec<Seed> = Vec::new();
    for (vi, project) in views.iter().enumerate() {
        let prefix = project.report_prefix.as_str();
        for page in &project.pages {
            if let Some(wxml) = page.files.wxml.as_deref() {
                seeds.push(Seed {
                    view: vi,
                    id: unit_id(prefix, wxml),
                    path: format!("{prefix}{wxml}"),
                    rel: wxml.to_string(),
                    kind: "page",
                    dep_key: page.route.clone(),
                    files: unit_files(
                        project,
                        prefix,
                        &Scope::Page(page.route.clone()),
                        &page.files,
                    ),
                });
            }
        }
        for component in &project.components {
            if let Some(wxml) = component.files.wxml.as_deref() {
                seeds.push(Seed {
                    view: vi,
                    id: unit_id(prefix, wxml),
                    path: format!("{prefix}{wxml}"),
                    rel: wxml.to_string(),
                    kind: "component",
                    dep_key: component.path.clone(),
                    files: unit_files(
                        project,
                        prefix,
                        &Scope::Component(component.path.clone()),
                        &component.files,
                    ),
                });
            }
        }
    }

    // 诊断归属：先按单元文件，剩下的进 shared。同一文件被多个单元引用时
    // 每个单元都算一份（作业单各拿一份），summary 仍按报告总数对账。
    let mut owners: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (idx, seed) in seeds.iter().enumerate() {
        for file in &seed.files {
            owners.entry(file.as_str()).or_default().push(idx);
        }
    }
    let mut counts: Vec<DiagCounts> = vec![DiagCounts::default(); seeds.len()];
    let mut shared = DiagCounts::default();
    let mut shared_files: BTreeMap<&str, usize> = BTreeMap::new();
    for diagnostic in &report.diagnostics {
        match owners.get(diagnostic.path.as_str()) {
            Some(idxs) => {
                for &idx in idxs {
                    count_diagnostic(&mut counts[idx], diagnostic);
                }
            }
            None => {
                count_diagnostic(&mut shared, diagnostic);
                *shared_files.entry(diagnostic.path.as_str()).or_default() += 1;
            }
        }
    }
    let mut shared_files: Vec<SharedFile> = shared_files
        .into_iter()
        .map(|(path, count)| SharedFile {
            path: path.to_string(),
            count,
        })
        .collect();
    shared_files.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.path.cmp(&b.path)));

    let depths = dependency_depths(views, &seeds);

    let mut units: Vec<PlanUnit> = Vec::with_capacity(seeds.len() + 1);
    for (idx, seed) in seeds.iter().enumerate() {
        let project = &views[seed.view];
        let (renderer, reach, usages) = match LoadedDoc::load(project, &seed.rel) {
            Some(loaded) => {
                let facts = loaded.facts(project);
                (
                    renderer_str(facts.unit().renderer),
                    reach_str(facts.unit().reach),
                    facts.usages().len(),
                )
            }
            None => ("unknown", "webview", 0),
        };
        units.push(PlanUnit {
            id: seed.id.clone(),
            path: seed.path.clone(),
            fs_path: fs_path(project, &seed.rel),
            files: seed.files.clone(),
            kind: seed.kind,
            renderer,
            reach,
            batch: depths.get(&seed.dep_key).copied().unwrap_or(0),
            usages,
            rule_families: families(&counts[idx]),
            diagnostics: counts[idx].clone(),
        });
    }
    let shared_total = shared.total;
    if shared.total > 0 {
        units.push(PlanUnit {
            id: "unit:shared".to_string(),
            path: String::new(),
            fs_path: String::new(),
            files: Vec::new(),
            kind: "shared",
            renderer: "unknown",
            reach: "webview",
            batch: 0,
            usages: 0,
            rule_families: families(&shared),
            diagnostics: shared,
        });
    }

    units.sort_by(|a, b| (a.batch, &a.path).cmp(&(b.batch, &b.path)));
    let mut grouped: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    for unit in &units {
        grouped.entry(unit.batch).or_default().push(unit.id.clone());
    }
    let batches: Vec<Vec<String>> = grouped.into_values().collect();

    let with_diagnostics = units.iter().filter(|u| u.diagnostics.total > 0).count();
    Plan {
        schema_version: SCHEMA_VERSION,
        generator: Generator::default(),
        baseline,
        summary: PlanSummary {
            units: units.len(),
            with_diagnostics,
            diagnostics: report.diagnostics.len(),
        },
        batches,
        units,
        shared: SharedFiles {
            diagnostics: shared_total,
            files: shared_files,
        },
    }
}

struct Seed {
    view: usize,
    id: String,
    path: String,
    /// 视图相对路径，`LoadedDoc::load` 用。
    rel: String,
    kind: &'static str,
    /// `Project::deps` 里的键：页面用 route，组件用自身路径。
    dep_key: String,
    files: Vec<String>,
}

fn unit_id(prefix: &str, wxml: &str) -> String {
    let base = wxml.strip_suffix(".wxml").unwrap_or(wxml);
    format!("unit:{prefix}{base}")
}

/// 视图相对路径 → 工程根相对路径（`/` 分隔）。
///
/// 宿主视图的 `miniprogram_root` 是 `miniprogram/`，插件视图的根是 `pluginRoot`，
/// 两者都不是工程根，所以不能直接把报告路径当文件路径用。
fn fs_path(project: &Project, rel: &str) -> String {
    project
        .abs(rel)
        .strip_prefix(&project.root)
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| rel.to_string())
}

fn unit_files(project: &Project, prefix: &str, scope: &Scope, files: &UnitFiles) -> Vec<String> {
    let mut out: Vec<String> = [&files.wxml, &files.wxss, &files.js, &files.json]
        .into_iter()
        .flatten()
        .map(|file| format!("{prefix}{file}"))
        .collect();

    // 单元实际引用（含传递闭包）的样式与模板：它们不是四件套，但诊断必须
    // 跟着作业单走，否则 agent 看不到这些文件上的坑。
    let referenced: BTreeSet<String> = project
        .styles
        .iter()
        .filter(|sheet| sheet.scopes.contains(scope))
        .map(|sheet| format!("{prefix}{}", sheet.doc.path()))
        .chain(
            project
                .template_scopes
                .iter()
                .filter(|(_, scopes)| scopes.contains(scope))
                .map(|(path, _)| format!("{prefix}{path}")),
        )
        .collect();
    for file in referenced {
        if !out.contains(&file) {
            out.push(file);
        }
    }
    out
}

fn count_diagnostic(slot: &mut DiagCounts, diagnostic: &Diagnostic) {
    slot.total += 1;
    match diagnostic.severity {
        Severity::Error => slot.error += 1,
        Severity::Warning => slot.warning += 1,
        Severity::Info => slot.info += 1,
    }
    *slot.by_rule.entry(diagnostic.rule.clone()).or_default() += 1;
}

/// 依赖深度：`0` = 不引用其他组件。环按 0 处理（组件图不该有环）。
fn dependency_depths(views: &[Project], seeds: &[Seed]) -> BTreeMap<String, u32> {
    // dep_key 只在单个视图内有意义，按视图分别算，再按 id 回填。
    let mut out: BTreeMap<String, u32> = BTreeMap::new();
    for (vi, project) in views.iter().enumerate() {
        let keys: Vec<&String> = seeds
            .iter()
            .filter(|seed| seed.view == vi)
            .map(|seed| &seed.dep_key)
            .collect();
        let mut memo: BTreeMap<String, u32> = BTreeMap::new();
        for key in keys {
            let mut visiting = BTreeSet::new();
            depth_of(key, project, &mut memo, &mut visiting);
        }
        for seed in seeds.iter().filter(|seed| seed.view == vi) {
            if let Some(depth) = memo.get(&seed.dep_key) {
                out.insert(seed.dep_key.clone(), *depth);
            }
        }
    }
    out
}

fn depth_of(
    key: &str,
    project: &Project,
    memo: &mut BTreeMap<String, u32>,
    visiting: &mut BTreeSet<String>,
) -> u32 {
    if let Some(&depth) = memo.get(key) {
        return depth;
    }
    if !visiting.insert(key.to_string()) {
        return 0;
    }
    let depth = project
        .deps
        .get(key)
        .map(|deps| {
            deps.iter()
                .map(|dep| depth_of(dep, project, memo, visiting) + 1)
                .max()
                .unwrap_or(0)
        })
        .unwrap_or(0);
    visiting.remove(key);
    memo.insert(key.to_string(), depth);
    depth
}

fn families(counts: &DiagCounts) -> Vec<&'static str> {
    let set: BTreeSet<&'static str> = counts
        .by_rule
        .keys()
        .map(|rule| family_of(rule))
        .filter(|family| *family != "other")
        .collect();
    set.into_iter().collect()
}

fn renderer_str(renderer: Renderer) -> &'static str {
    match renderer {
        Renderer::Skyline => "skyline",
        Renderer::Webview => "webview",
        Renderer::Unknown => "unknown",
    }
}

fn reach_str(reach: Reach) -> &'static str {
    match reach {
        Reach::Skyline => "skyline",
        Reach::WebviewOnly => "webview",
    }
}

fn kind_str(kind: &UnitKind) -> &'static str {
    match kind {
        UnitKind::Page { .. } => "page",
        UnitKind::Component => "component",
        UnitKind::Fragment => "fragment",
        UnitKind::Unknown => "unknown",
    }
}

// ---------------------------------------------------------------- job

/// 单个单元的作业单。
pub fn job(
    project: &Project,
    rel_wxml: &str,
    plan: &Plan,
    plan_unit: &PlanUnit,
    diagnostics: &[&Diagnostic],
    baseline: Option<BaselineRef>,
) -> Job {
    let loaded = LoadedDoc::load(project, rel_wxml);
    // 无论能否解析出事实，单元的身份与文件清单都来自 `PlanUnit`；
    // 两分支只差 renderer / 布局事实，避免把同一份字段写两遍。
    let (kind, renderer, reach, style_isolation, display_block, content_box, auto_size) =
        match &loaded {
            Some(doc) => {
                let facts = doc.facts(project);
                let unit = facts.unit();
                (
                    kind_str(&unit.kind),
                    renderer_str(unit.renderer),
                    reach_str(unit.reach),
                    unit.style_isolation
                        .map(|iso| format!("{iso:?}").to_lowercase()),
                    unit.default_display_block,
                    unit.default_content_box,
                    unit.scroll_view_auto_size,
                )
            }
            None => (
                plan_unit.kind,
                plan_unit.renderer,
                plan_unit.reach,
                None,
                false,
                false,
                false,
            ),
        };
    let unit = JobUnit {
        id: plan_unit.id.clone(),
        path: plan_unit.path.clone(),
        fs_path: plan_unit.fs_path.clone(),
        files: plan_unit.files.clone(),
        kind,
        renderer,
        reach,
        style_isolation,
        default_display_block: display_block,
        default_content_box: content_box,
        scroll_view_auto_size: auto_size,
    };

    // 每个文件只解析一次；诊断按文件分组。
    let mut by_file: BTreeMap<&str, Vec<&Diagnostic>> = BTreeMap::new();
    for diagnostic in diagnostics {
        by_file
            .entry(diagnostic.path.as_str())
            .or_default()
            .push(diagnostic);
    }

    let mut findings = Vec::with_capacity(diagnostics.len());
    for (path, group) in &by_file {
        let rel = strip_prefix(project, path);
        let doc = rel.and_then(|rel| LoadedDoc::load(project, rel));
        for &diagnostic in group {
            findings.push(finding(project, plan, plan_unit, doc.as_ref(), diagnostic));
        }
    }
    findings.sort_by(|a, b| {
        (a.span.line, a.span.column, &a.rule).cmp(&(b.span.line, b.span.column, &b.rule))
    });

    let mut shared_counts: BTreeMap<&str, usize> = BTreeMap::new();
    for finding in &findings {
        if finding.shared {
            *shared_counts.entry(finding.path.as_str()).or_default() += 1;
        }
    }
    let shared_findings = shared_counts
        .into_iter()
        .map(|(path, count)| SharedFinding {
            path: path.to_string(),
            count,
        })
        .collect();

    Job {
        schema_version: SCHEMA_VERSION,
        generator: Generator::default(),
        baseline,
        unit,
        findings,
        shared_findings,
        acceptance: vec![
            Acceptance {
                id: "l0-static",
                kind: "command",
                run: "skylint migration verify {root} --unit {unit} --baseline {baseline} --json",
                expect: Expect {
                    exit_code: Some(0),
                    json_path: Some("$.new"),
                    equals: Some(0),
                    threshold: None,
                },
            },
            Acceptance {
                id: "l1-build",
                kind: "command",
                run: "minidev build --no-pack",
                expect: Expect {
                    exit_code: Some(0),
                    json_path: None,
                    equals: None,
                    threshold: None,
                },
            },
        ],
        workarounds: Vec::new(),
        attempts: Vec::new(),
    }
}

fn finding(
    project: &Project,
    plan: &Plan,
    unit: &PlanUnit,
    doc: Option<&LoadedDoc>,
    diagnostic: &Diagnostic,
) -> Finding {
    let guidance = guidance_for(&diagnostic.rule);
    let span = Span {
        line: diagnostic.line,
        column: diagnostic.column,
        end_line: diagnostic.end_line,
        end_column: diagnostic.end_column,
    };

    let resolved = doc.and_then(|doc| {
        let facts = doc.facts(project);
        let element = facts.element_at(diagnostic.line, diagnostic.column)?;
        let mut cascade = BTreeMap::new();
        for property in guidance.facts {
            let entries = facts.cascade(element, property);
            if !entries.is_empty() {
                cascade.insert(property.to_string(), entries);
            }
        }
        Some(FindingFacts {
            layout: facts.layout(element),
            cascade,
            ancestors: facts.ancestors(element),
            handlers: facts.handlers(element),
        })
    });

    // id 与 `identify()` 用同一份计算，避免作业单和门禁对不上。
    let (id, anchor) = identify_one(project, doc, diagnostic);
    let style = style_view(project, unit, diagnostic);

    let affected_units = plan
        .units
        .iter()
        .filter(|other| other.kind != "shared" && owns(other, &diagnostic.path))
        .count();
    let shared = affected_units >= 2;

    Finding {
        id,
        path: diagnostic.path.clone(),
        rule: diagnostic.rule.clone(),
        severity: diagnostic.severity,
        message: diagnostic.message.clone(),
        help: diagnostic.help.clone(),
        span,
        anchor,
        facts: resolved,
        preconditions: guidance.preconditions.to_vec(),
        counterexamples: guidance.counterexamples.to_vec(),
        selector: style.selector,
        property: style.property,
        declarations: style.declarations,
        matches: style.matches,
        match_count: style.match_count,
        shared,
        affected_units: if shared { affected_units } else { 0 },
    }
}

/// 样式诊断的定位上下文。非样式诊断（`.js` / `.json` / WXML）或解析失败时为空。
#[derive(Default)]
struct StyleView {
    selector: Option<String>,
    property: Option<String>,
    declarations: Vec<Declaration>,
    matches: Vec<MatchNode>,
    match_count: usize,
}

struct StyleContext<'a> {
    doc: &'a WxssDoc,
    rule: &'a WxssRule,
    declaration: Option<&'a crate::input::wxss::WxssDeclaration>,
    selector: Option<String>,
}

fn style_view(project: &Project, unit: &PlanUnit, diagnostic: &Diagnostic) -> StyleView {
    let Some(context) = style_context(project, diagnostic) else {
        return StyleView::default();
    };
    let declarations = context
        .rule
        .declarations
        .iter()
        .map(|declaration| {
            let (line, column) = context.doc.position(declaration.offset);
            Declaration {
                property: declaration.name.to_ascii_lowercase(),
                value: declaration.value.clone(),
                line,
                column,
            }
        })
        .collect();
    let (matches, match_count) = context
        .selector
        .as_deref()
        .map(|selector| style_matches(project, unit, selector))
        .unwrap_or_default();
    StyleView {
        selector: context.selector,
        property: context
            .declaration
            .map(|declaration| declaration.name.to_ascii_lowercase()),
        declarations,
        matches,
        match_count,
    }
}

/// 诊断落在哪条规则上：声明命中优先，其次选择器自身；at-rule prelude 不返回。
fn style_context<'a>(project: &'a Project, diagnostic: &Diagnostic) -> Option<StyleContext<'a>> {
    let rel = strip_prefix(project, &diagnostic.path)?;
    let doc = project
        .styles
        .iter()
        .map(|sheet| &sheet.doc)
        .find(|doc| doc.path() == rel)?;
    let offset = doc.offset(diagnostic.line, diagnostic.column);

    for rule in &doc.rules {
        for declaration in &rule.declarations {
            if declaration.offset <= offset && offset <= declaration.value_end() {
                let selector = selector_text(rule);
                return Some(StyleContext {
                    doc,
                    rule,
                    declaration: Some(declaration),
                    selector: (!selector.is_empty()).then_some(selector),
                });
            }
        }
    }

    let mut best: Option<(&WxssRule, &crate::input::wxss::WxssSelector)> = None;
    for rule in &doc.rules {
        for selector in &rule.selectors {
            if selector.offset <= offset && offset <= selector.offset + selector.text.len() {
                best = Some(match best {
                    Some((_, previous)) if previous.offset > selector.offset => (rule, previous),
                    _ => (rule, selector),
                });
            }
        }
    }
    let (rule, selector) = best?;
    Some(StyleContext {
        doc,
        rule,
        declaration: None,
        selector: Some(selector.text.clone()),
    })
}

/// 选择器在本单元 WXML 里的命中节点（只看 `unit.files`，不跨单元）。
fn style_matches(project: &Project, unit: &PlanUnit, selector: &str) -> (Vec<MatchNode>, usize) {
    let mut all = Vec::new();
    for file in &unit.files {
        if !file.ends_with(".wxml") {
            continue;
        }
        let Some(rel) = strip_prefix(project, file) else {
            continue;
        };
        let Some(loaded) = LoadedDoc::load(project, rel) else {
            continue;
        };
        let tree = Tree::new(&loaded.doc);
        for element in loaded.doc.elements() {
            if tree.matches(element, selector, Dialect::WebView) != Tri::Yes {
                continue;
            }
            let (line, column) = loaded.doc.position(element.start);
            all.push(MatchNode {
                path: file.clone(),
                line,
                column,
                tag: element.tag.clone(),
            });
        }
    }
    let total = all.len();
    all.truncate(5);
    (all, total)
}

fn anchor_for(facts: &Facts<'_>, element: &crate::input::wxml::WxmlElement) -> Anchor {
    let classes = classes_of(element);
    let occurrence = facts
        .elements()
        .iter()
        .copied()
        .take_while(|other| !std::ptr::eq(*other, element))
        .filter(|other| other.tag == element.tag && classes_of(other) == classes)
        .count();
    Anchor {
        tag: element.tag.clone(),
        classes,
        occurrence,
    }
}

fn classes_of(element: &crate::input::wxml::WxmlElement) -> Vec<String> {
    element
        .attributes
        .iter()
        .filter(|attribute| attribute.name == "class")
        .flat_map(
            |attribute| match (&attribute.class_tokens, attribute.value.as_deref()) {
                // 动态 class：只取「任何取值下都出现」的词，可能出现的词会让 id 随分支变。
                (Some(tokens), _) => tokens.definite.clone(),
                (None, Some(value)) => value.split_whitespace().map(str::to_string).collect(),
                (None, None) => Vec::new(),
            },
        )
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn anchor_id(rule: &str, unit_path: &str, anchor: &Anchor) -> String {
    format!(
        "{rule}|{unit_path}|{}.{}|#{}",
        anchor.tag,
        anchor.classes.join("."),
        anchor.occurrence
    )
}

/// 报告路径（带视图前缀）→ 视图相对路径。
pub fn strip_prefix<'a>(project: &Project, path: &'a str) -> Option<&'a str> {
    if project.report_prefix.is_empty() {
        Some(path)
    } else {
        path.strip_prefix(project.report_prefix.as_str())
    }
}

/// 报告路径归属哪个视图。前缀为空串的视图匹配任何路径，所以取**最长**前缀。
pub fn find_view<'a>(views: &'a [Project], path: &'a str) -> Option<(&'a Project, &'a str)> {
    views
        .iter()
        .filter_map(|project| strip_prefix(project, path).map(|rel| (project, rel)))
        .max_by_key(|(project, _)| project.report_prefix.len())
}

// ---------------------------------------------------------------- facts

/// `facts --element` 未指定属性时展示的默认属性集。
pub const DEFAULT_PROPERTIES: &[&str] = &[
    "display",
    "position",
    "z-index",
    "width",
    "height",
    "flex",
    "flex-direction",
    "padding",
    "margin",
    "overflow",
    "white-space",
    "transform",
    "box-sizing",
    "transition",
    "animation",
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FactsDoc {
    pub schema_version: u32,
    pub unit: FactsUnit,
    pub elements: Vec<ElementIndex>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FactsUnit {
    pub id: String,
    pub path: String,
    pub kind: &'static str,
    pub renderer: &'static str,
    pub reach: &'static str,
    pub style_isolation: Option<String>,
    pub default_display_block: bool,
    pub default_content_box: bool,
    pub scroll_view_auto_size: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ElementIndex {
    /// 元素自身的范围（不是查询位置）。
    pub span: Span,
    pub tag: String,
    pub classes: Vec<String>,
    /// 元素定位串（`element|<doc>|<tag>.<classes>|#<n>`），不是诊断 id。
    pub id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ElementDetail {
    /// 包含 `--element` 查询位置的那个元素的完整范围，不是查询位置本身。
    pub span: Span,
    pub tag: String,
    pub classes: Vec<String>,
    /// 元素定位串（`element|<doc>|<tag>.<classes>|#<n>`），不是诊断 id。
    pub id: String,
    pub layout: LayoutFacts,
    pub cascade: BTreeMap<String, Vec<crate::inspect::CascadeView>>,
    pub ancestors: crate::inspect::Ancestors,
    pub handlers: Vec<crate::inspect::HandlerView>,
    pub usages: Vec<crate::inspect::UsageView>,
}

/// 单元事实 + 元素索引（文档顺序）。
pub fn facts_doc(project: &Project, rel_wxml: &str) -> Option<FactsDoc> {
    let loaded = LoadedDoc::load(project, rel_wxml)?;
    let facts = loaded.facts(project);
    let unit = facts.unit();
    let elements = facts
        .elements()
        .iter()
        .map(|element| {
            let anchor = anchor_for(&facts, element);
            ElementIndex {
                span: Span {
                    line: element.start.line + 1,
                    column: element.start.utf16_col + 1,
                    end_line: element.end.line + 1,
                    end_column: element.end.utf16_col + 1,
                },
                tag: element.tag.clone(),
                classes: anchor.classes.clone(),
                id: anchor_id("element", facts.path(), &anchor),
            }
        })
        .collect();
    Some(FactsDoc {
        schema_version: SCHEMA_VERSION,
        unit: FactsUnit {
            id: unit_id(&project.report_prefix, rel_wxml),
            path: format!("{}{rel_wxml}", project.report_prefix),
            kind: kind_str(&unit.kind),
            renderer: renderer_str(unit.renderer),
            reach: reach_str(unit.reach),
            style_isolation: unit
                .style_isolation
                .map(|iso| format!("{iso:?}").to_lowercase()),
            default_display_block: unit.default_display_block,
            default_content_box: unit.default_content_box,
            scroll_view_auto_size: unit.scroll_view_auto_size,
        },
        elements,
    })
}

/// 单个元素的完整事实。
pub fn element_detail(
    project: &Project,
    rel_wxml: &str,
    line: u32,
    column: u32,
    property: Option<&str>,
) -> Option<ElementDetail> {
    let loaded = LoadedDoc::load(project, rel_wxml)?;
    let facts = loaded.facts(project);
    let element = facts.element_at(line, column)?;
    let anchor = anchor_for(&facts, element);

    let properties: Vec<&str> = match property {
        Some(property) => vec![property],
        None => DEFAULT_PROPERTIES.to_vec(),
    };
    let mut cascade = BTreeMap::new();
    for property in properties {
        let entries = facts.cascade(element, property);
        if !entries.is_empty() {
            cascade.insert(property.to_string(), entries);
        }
    }

    Some(ElementDetail {
        span: Span {
            line: element.start.line + 1,
            column: element.start.utf16_col + 1,
            end_line: element.end.line + 1,
            end_column: element.end.utf16_col + 1,
        },
        tag: element.tag.clone(),
        classes: anchor.classes.clone(),
        id: anchor_id("element", facts.path(), &anchor),
        layout: facts.layout(element),
        cascade,
        ancestors: facts.ancestors(element),
        handlers: facts.handlers(element),
        usages: facts.usages(),
    })
}

/// 一个单元该收哪些诊断。`shared` 收所有不属于任何具体单元的。
pub fn diagnostics_for<'a>(
    plan: &Plan,
    unit: &PlanUnit,
    report: &'a Report,
) -> Vec<&'a Diagnostic> {
    if unit.kind == "shared" {
        return report
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                !plan
                    .units
                    .iter()
                    .any(|other| other.kind != "shared" && owns(other, &diagnostic.path))
            })
            .collect();
    }
    report
        .diagnostics
        .iter()
        .filter(|diagnostic| owns(unit, &diagnostic.path))
        .collect()
}

/// 诊断路径是否属于这个单元（四件套 + 实际引用的模板 / 样式）。
fn owns(unit: &PlanUnit, path: &str) -> bool {
    unit.files.iter().any(|file| file == path)
}

/// 产物序列化。放在引擎侧，集成层不必依赖 serde。
pub fn to_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).expect("migrate 产物总是可序列化")
}

// ---------------------------------------------------------------- 稳定标识

/// 一条诊断的稳定身份。`baseline` / `verify` / `job` 共用这一份计算。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Identified {
    pub id: String,
    /// 归属单元 id（`unit:shared` 表示不属于任何单元）。
    pub unit: String,
    pub rule: String,
    pub severity: Severity,
    pub path: String,
    pub span: Span,
    pub message: String,
    /// 只写进基线，给 `verify` 的 `fixed` 条目展示用；不参与身份判定。
    pub help: Option<String>,
}

/// 全部诊断的稳定 id，按 `report.diagnostics` 顺序。
///
/// 与 [`identify`] 同一套算法，区别只是不需要 `plan` 就能算（豁免要在建计划之前
/// 生效，那时还没有单元划分）。
pub fn diagnostic_ids(views: &[Project], report: &Report) -> Vec<String> {
    let mut docs: BTreeMap<String, Option<LoadedDoc>> = BTreeMap::new();
    report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic_id(views, &mut docs, diagnostic))
        .collect()
}

/// 单条诊断的 id。`docs` 是同一次运行的 WXML 解析缓存。
fn diagnostic_id(
    views: &[Project],
    docs: &mut BTreeMap<String, Option<LoadedDoc>>,
    diagnostic: &Diagnostic,
) -> String {
    // WXML 按元素锚点；样式按 selector + property；JS / JSON 等退回行列。
    match find_view(views, &diagnostic.path) {
        Some((project, rel)) if diagnostic.path.ends_with(".wxml") => {
            let doc = docs
                .entry(diagnostic.path.clone())
                .or_insert_with(|| LoadedDoc::load(project, rel));
            identify_one(project, doc.as_ref(), diagnostic).0
        }
        Some((project, _)) => identify_one(project, None, diagnostic).0,
        None => identify_one_fallback(diagnostic).0,
    }
}

/// 给全部诊断算稳定 id。同一份代码重复调用结果一致；元素挪行不变。
pub fn identify(views: &[Project], plan: &Plan, report: &Report) -> Vec<Identified> {
    let mut docs: BTreeMap<String, Option<LoadedDoc>> = BTreeMap::new();
    let mut out = Vec::with_capacity(report.diagnostics.len());

    for diagnostic in &report.diagnostics {
        let unit = plan
            .units
            .iter()
            .find(|unit| unit.kind != "shared" && owns(unit, &diagnostic.path))
            .or_else(|| plan.units.iter().find(|unit| unit.kind == "shared"));
        let unit_id = unit.map(|unit| unit.id.clone()).unwrap_or_default();
        let id = diagnostic_id(views, &mut docs, diagnostic);

        out.push(Identified {
            id,
            unit: unit_id,
            rule: diagnostic.rule.clone(),
            severity: diagnostic.severity,
            path: diagnostic.path.clone(),
            span: Span {
                line: diagnostic.line,
                column: diagnostic.column,
                end_line: diagnostic.end_line,
                end_column: diagnostic.end_column,
            },
            message: diagnostic.message.clone(),
            help: diagnostic.help.clone(),
        });
    }
    out
}

/// `finding()` 与 `identify()` 共用的 id 计算。
fn identify_one(
    project: &Project,
    doc: Option<&LoadedDoc>,
    diagnostic: &Diagnostic,
) -> (String, Option<Anchor>) {
    let anchor = doc.and_then(|doc| {
        let facts = doc.facts(project);
        let element = facts.element_at(diagnostic.line, diagnostic.column)?;
        Some(anchor_for(&facts, element))
    });
    let id = match &anchor {
        Some(anchor) => anchor_id(&diagnostic.rule, &diagnostic.path, anchor),
        // 样式诊断按「选择器 + 声明属性」定身份；改上方代码不再让后面全部漂移。
        None => json_identity(diagnostic)
            .or_else(|| style_identity(project, diagnostic))
            .unwrap_or_else(|| identify_one_fallback(diagnostic).0),
    };
    (id, anchor)
}

/// 一条样式规则的全部选择器原文（逗号分隔时合并，保持源码顺序）。
fn selector_text(rule: &WxssRule) -> String {
    rule.selectors
        .iter()
        .map(|selector| selector.text.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// 非 WXML 诊断的结构化身份：`rule|path|selector|property|#occurrence`。
///
/// 定位不到选择器 / 属性时返回 `None`，由调用方退化成 `rule|path|line:column`。
fn style_identity(project: &Project, diagnostic: &Diagnostic) -> Option<String> {
    let rel = strip_prefix(project, &diagnostic.path)?;
    let doc = project
        .styles
        .iter()
        .map(|sheet| &sheet.doc)
        .find(|doc| doc.path() == rel)?;
    let offset = doc.offset(diagnostic.line, diagnostic.column);

    // 1) at-rule：`@media` 这类诊断落在 prelude 上。同一条 at-rule 会在它内部
    // 每一条规则里重复出现，按 prelude 起点去重。
    let mut at_rules: BTreeMap<usize, (&str, &str)> = BTreeMap::new();
    for rule in &doc.rules {
        for at in &rule.at_rules {
            at_rules.entry(at.offset).or_insert((&at.name, &at.prelude));
        }
    }
    let at_rules: Vec<(usize, &str, &str)> = at_rules
        .into_iter()
        .map(|(offset, (name, prelude))| (offset, name, prelude))
        .collect();
    for (index, (start, name, prelude)) in at_rules.iter().enumerate() {
        if *start <= offset && offset <= *start + prelude.len() {
            let occurrence = at_rules[..index]
                .iter()
                .filter(|(_, other_name, other_prelude)| {
                    *other_name == *name && *other_prelude == *prelude
                })
                .count();
            return Some(format!(
                "{}|{}|@{} {}|#{}",
                diagnostic.rule, diagnostic.path, name, prelude, occurrence
            ));
        }
    }

    // 2) 声明：所在规则的完整选择器列表 + 属性名；同选择器同属性按源码顺序编号。
    for rule in &doc.rules {
        for declaration in &rule.declarations {
            if declaration.offset <= offset && offset <= declaration.value_end() {
                let selector = selector_text(rule);
                let property = declaration.name.to_ascii_lowercase();
                let occurrence = doc
                    .rules
                    .iter()
                    .filter(|other| selector_text(other) == selector)
                    .flat_map(|other| other.declarations.iter())
                    .filter(|other| {
                        other.name.eq_ignore_ascii_case(&property)
                            && other.offset < declaration.offset
                    })
                    .count();
                return Some(format!(
                    "{}|{}|{}|{}|#{}",
                    diagnostic.rule, diagnostic.path, selector, property, occurrence
                ));
            }
        }
    }

    // 3) 诊断落在选择器自身上（如 `tag-selector-isolation` / `no-pseudo`）。
    let mut best: Option<&crate::input::wxss::WxssSelector> = None;
    for rule in &doc.rules {
        for selector in &rule.selectors {
            if selector.offset <= offset && offset <= selector.offset + selector.text.len() {
                best = Some(match best {
                    Some(previous) if previous.offset > selector.offset => previous,
                    _ => selector,
                });
            }
        }
    }
    let selector = best?;
    let occurrence = doc
        .rules
        .iter()
        .flat_map(|rule| rule.selectors.iter())
        .filter(|other| other.text == selector.text && other.offset < selector.offset)
        .count();
    Some(format!(
        "{}|{}|{}|#{}",
        diagnostic.rule, diagnostic.path, selector.text, occurrence
    ))
}

/// JSON 诊断的结构化身份：`rule|path|jsonPointer|valueFingerprint`。
///
/// 指针与值都由产生诊断的规则随诊断携带（[`crate::diag::JsonAnchor`]）；拿不到时
/// （解析失败的 JSON、`.js` 诊断）退回 `rule|path|line:column`。
fn json_identity(diagnostic: &Diagnostic) -> Option<String> {
    let anchor = diagnostic.json.as_ref()?;
    let fingerprint = match &anchor.value {
        Some(value) => stable_hash(&format!("v:{value}")),
        None => stable_hash("absent"),
    };
    Some(format!(
        "{}|{}|{}|{}",
        diagnostic.rule, diagnostic.path, anchor.pointer, fingerprint
    ))
}

/// FNV-1a 64，输出 16 位十六进制。
///
/// 指定了算法的哈希，跨进程、跨 Rust 版本都不变；`DefaultHasher` 的算法未承诺
/// 稳定（且 `RandomState` 每次进程随机），不能用来算基线身份。
fn stable_hash(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn identify_one_fallback(diagnostic: &Diagnostic) -> (String, Option<Anchor>) {
    (
        format!(
            "{}|{}|{}:{}",
            diagnostic.rule, diagnostic.path, diagnostic.line, diagnostic.column
        ),
        None,
    )
}

// ---------------------------------------------------------------- baseline

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Baseline {
    pub schema_version: u32,
    pub generator: Generator,
    /// 冻结时工具具备的全部规则 id（字典序、去重）。
    pub rules: Vec<String>,
    pub summary: BaselineSummary,
    pub findings: Vec<BaselineFinding>,
}

impl Baseline {
    /// 读取基线文件内容。
    ///
    /// 只接受当前形态：缺字段、多出旧版字段（如 `idFormat`）或 `schemaVersion`
    /// 不匹配都直接报错，不做任何宽松兼容。
    pub fn load(text: &str) -> Result<Self, String> {
        let baseline: Self = from_json(text)
            .map_err(|err| format!("{err}（这份基线是用旧版 skylint 生成的，请重新生成）"))?;
        if baseline.schema_version != SCHEMA_VERSION {
            return Err(format!(
                "schemaVersion 是 {}，当前版本是 {SCHEMA_VERSION}（这份基线是用旧版 skylint 生成的，请重新生成）",
                baseline.schema_version
            ));
        }
        Ok(baseline)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaselineSummary {
    pub total: usize,
    pub error: usize,
    pub warning: usize,
    pub info: usize,
}

/// 基线里只存身份与位置，不存 message：措辞随版本变，但问题没变。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaselineFinding {
    pub id: String,
    /// 归属单元，`--unit` 过滤用。
    pub unit: String,
    pub rule: String,
    pub severity: Severity,
    pub path: String,
    pub span: Span,
    /// 仅用于让 `verify` 的 `fixed` 条目能展示「修掉的是哪条」；身份仍只看 `id`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
}

pub fn baseline(identified: &[Identified]) -> Baseline {
    let mut summary = BaselineSummary::default();
    let mut findings = Vec::with_capacity(identified.len());
    for finding in identified {
        summary.total += 1;
        match finding.severity {
            Severity::Error => summary.error += 1,
            Severity::Warning => summary.warning += 1,
            Severity::Info => summary.info += 1,
        }
        findings.push(BaselineFinding {
            id: finding.id.clone(),
            unit: finding.unit.clone(),
            rule: finding.rule.clone(),
            severity: finding.severity,
            path: finding.path.clone(),
            span: finding.span.clone(),
            message: Some(finding.message.clone()),
            help: finding.help.clone(),
        });
    }
    findings.sort_by(|a, b| a.id.cmp(&b.id));
    Baseline {
        schema_version: SCHEMA_VERSION,
        generator: Generator::default(),
        rules: builtin_rule_ids(),
        summary,
        findings,
    }
}

/// 迁移命令实际运行的规则集（`Skyline` 方向），字典序、去重。
///
/// `baseline` 把它写进基线，回答「冻结这个基线时工具具备哪些规则」。
pub fn builtin_rule_ids() -> Vec<String> {
    crate::Registry::builtin()
        .for_target(crate::rules::Target::Skyline)
        .metas()
        .into_iter()
        .map(|meta| meta.id.to_string())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// 当前规则集里基线未记录的规则。
pub fn unknown_rules(baseline: &Baseline, current: &[String]) -> Vec<String> {
    let known: BTreeSet<&str> = baseline.rules.iter().map(String::as_str).collect();
    current
        .iter()
        .filter(|id| !known.contains(id.as_str()))
        .cloned()
        .collect()
}

// ---------------------------------------------------------------- augment

/// `baseline --augment` 的吸收结果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AugmentReport {
    pub schema_version: u32,
    /// 追加的条目总数。
    pub added: usize,
    /// 追加后基线总条数。
    pub total: usize,
    /// 按规则分组的吸收明细，规则 id 字典序。
    pub rules: Vec<AugmentedRule>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AugmentedRule {
    pub rule: String,
    pub count: usize,
    /// 涉及文件（字典序，最多 5 个）。
    pub files: Vec<String>,
    /// 涉及文件总数。
    pub file_count: usize,
}

/// 把「基线冻结后才出现的规则」产生的诊断追加进基线。
///
/// 已知规则集的诊断一律不动：不新增、不改 severity、不删除。
pub fn augment(baseline: &mut Baseline, current: &[Identified]) -> AugmentReport {
    let known: BTreeSet<String> = baseline.rules.iter().cloned().collect();

    let mut seen: BTreeSet<String> = baseline
        .findings
        .iter()
        .map(|finding| finding.id.clone())
        .collect();
    let mut added_by_rule: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for finding in current {
        if known.contains(&finding.rule) || !seen.insert(finding.id.clone()) {
            continue;
        }
        added_by_rule
            .entry(finding.rule.clone())
            .or_default()
            .push(finding.path.clone());
        baseline.findings.push(BaselineFinding {
            id: finding.id.clone(),
            unit: finding.unit.clone(),
            rule: finding.rule.clone(),
            severity: finding.severity,
            path: finding.path.clone(),
            span: finding.span.clone(),
            message: Some(finding.message.clone()),
            help: finding.help.clone(),
        });
    }

    // 既有排序口径：按 id；summary 必须跟着条目走。
    baseline.findings.sort_by(|a, b| a.id.cmp(&b.id));
    let mut summary = BaselineSummary::default();
    for finding in &baseline.findings {
        summary.total += 1;
        match finding.severity {
            Severity::Error => summary.error += 1,
            Severity::Warning => summary.warning += 1,
            Severity::Info => summary.info += 1,
        }
    }
    baseline.summary = summary;
    baseline.rules = builtin_rule_ids();

    let added: usize = added_by_rule.values().map(Vec::len).sum();
    let rules = added_by_rule
        .into_iter()
        .map(|(rule, paths)| {
            let files: BTreeSet<&str> = paths.iter().map(String::as_str).collect();
            AugmentedRule {
                rule,
                count: paths.len(),
                file_count: files.len(),
                files: files.iter().take(5).map(|path| path.to_string()).collect(),
            }
        })
        .collect();

    AugmentReport {
        schema_version: SCHEMA_VERSION,
        added,
        total: baseline.summary.total,
        rules,
    }
}

// ---------------------------------------------------------------- verify

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyReport {
    pub schema_version: u32,
    pub baseline: BaselineRef,
    /// 只看这个单元时给出；全量时为 `null`。
    pub unit: Option<String>,
    /// 基线与当前 skylint 版本不同：规则集可能变了，结果要人看一眼。
    pub version_mismatch: bool,
    /// **新增的 error + warning**（门禁判据：`new == 0`）。
    pub new: usize,
    /// 新增的 info（不拦门禁）。
    pub new_info: usize,
    pub fixed: usize,
    /// 同一 id、严重级变了（规则调档）。等于 `changed_up + changed_down`。
    pub changed: usize,
    /// 其中变严的（info→warning/error、warning→error）。
    pub changed_up: usize,
    /// 其中变松的（error/warning→info、error→warning）。
    pub changed_down: usize,
    /// 被豁免、因此不参与新增/修复统计的条数（台账里 `waived` 与当前诊断对上的）。
    pub waived: usize,
    pub findings: Vec<DiffEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffEntry {
    /// `new`：新增（severity 为 info 时不拦门禁）；`fixed`：基线有、当前没有；
    /// `changed`：同一 id 的 severity 变了（不拦门禁）。
    pub kind: &'static str,
    pub id: String,
    pub rule: String,
    pub severity: Severity,
    /// `changed` 时为基线里的 severity；方向由它与 `severity` 比较得出。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_severity: Option<Severity>,
    pub path: String,
    pub span: Span,
    /// 诊断说明：`new` / `changed` 用当前诊断，`fixed` 用基线里存的。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
}

pub fn verify(
    baseline: &Baseline,
    current: &[Identified],
    unit: Option<&str>,
    baseline_file: Option<String>,
) -> VerifyReport {
    verify_with(baseline, current, unit, baseline_file, &BTreeSet::new())
}

/// 带豁免的 `verify`。
///
/// 豁免是「两边都不算」：当前里跳过它（不算 `new` / `changed`），基线里也跳过它
/// （不算 `fixed`）。这样「认定不处理」与「修掉了」在机械判定上等价，但它们在
/// 报告里分得开——`waived` 单独计数，不会被当成修复成绩。
pub fn verify_with(
    baseline: &Baseline,
    current: &[Identified],
    unit: Option<&str>,
    baseline_file: Option<String>,
    waived: &BTreeSet<&str>,
) -> VerifyReport {
    let baseline_by_id: BTreeMap<&str, &BaselineFinding> = baseline
        .findings
        .iter()
        .filter(|finding| unit.is_none_or(|wanted| finding.unit == wanted))
        .map(|finding| (finding.id.as_str(), finding))
        .collect();

    let mut findings = Vec::new();
    let mut new = 0;
    let mut new_info = 0;
    let mut changed = 0;
    let mut changed_up = 0;
    let mut changed_down = 0;
    let mut waived_hits = 0;
    let mut seen: BTreeSet<&str> = BTreeSet::new();

    for finding in current {
        if unit.is_some_and(|wanted| finding.unit != wanted) {
            continue;
        }
        // 豁免项：不进 `new` / `changed`，也不进下面的 `fixed`。
        if waived.contains(finding.id.as_str()) {
            waived_hits += 1;
            continue;
        }
        match baseline_by_id.get(finding.id.as_str()).copied() {
            None => {
                if finding.severity == Severity::Info {
                    new_info += 1;
                } else {
                    new += 1;
                }
                findings.push(DiffEntry {
                    kind: "new",
                    id: finding.id.clone(),
                    rule: finding.rule.clone(),
                    severity: finding.severity,
                    previous_severity: None,
                    path: finding.path.clone(),
                    span: finding.span.clone(),
                    message: Some(finding.message.clone()),
                    help: finding.help.clone(),
                });
            }
            Some(previous) if previous.severity != finding.severity => {
                seen.insert(previous.id.as_str());
                changed += 1;
                // `Severity` 的排序是 Info < Warning < Error，变大即变严。
                if finding.severity > previous.severity {
                    changed_up += 1;
                } else {
                    changed_down += 1;
                }
                findings.push(DiffEntry {
                    kind: "changed",
                    id: finding.id.clone(),
                    rule: finding.rule.clone(),
                    severity: finding.severity,
                    previous_severity: Some(previous.severity),
                    path: finding.path.clone(),
                    span: finding.span.clone(),
                    message: Some(finding.message.clone()),
                    help: finding.help.clone(),
                });
            }
            Some(previous) => {
                seen.insert(previous.id.as_str());
            }
        }
    }

    let mut fixed = 0;
    for previous in baseline_by_id.values() {
        if seen.contains(previous.id.as_str()) || waived.contains(previous.id.as_str()) {
            continue;
        }
        fixed += 1;
        findings.push(DiffEntry {
            kind: "fixed",
            id: previous.id.clone(),
            rule: previous.rule.clone(),
            severity: previous.severity,
            previous_severity: None,
            path: previous.path.clone(),
            span: previous.span.clone(),
            message: previous.message.clone(),
            help: previous.help.clone(),
        });
    }

    findings.sort_by(|a, b| {
        (a.kind, &a.path, a.span.line, a.span.column).cmp(&(
            b.kind,
            &b.path,
            b.span.line,
            b.span.column,
        ))
    });

    VerifyReport {
        schema_version: SCHEMA_VERSION,
        baseline: BaselineRef {
            file: baseline_file,
            diagnostics: baseline.summary.total,
        },
        unit: unit.map(str::to_string),
        version_mismatch: baseline.generator.version != Generator::default().version,
        new,
        new_info,
        fixed,
        changed,
        changed_up,
        changed_down,
        waived: waived_hits,
        findings,
    }
}

impl VerifyReport {
    /// 门禁判据：新增 error/warning 即失败；只新增 info 不拦。
    pub fn blocks(&self) -> bool {
        self.new > 0
    }
}

/// 产物反序列化（基线文件）。同样放在引擎侧，集成层不必依赖 serde。
pub fn from_json<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, String> {
    serde_json::from_str(text).map_err(|err| err.to_string())
}
