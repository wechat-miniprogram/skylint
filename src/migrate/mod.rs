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
use crate::input::wxss::WxssRule;
use crate::inspect::{Facts, LayoutFacts, LoadedDoc, Renderer, UnitKind};
use crate::project::{Project, Scope, UnitFiles};
use crate::report::Report;
use crate::rules::wxml::Reach;

pub use guidance::{Guidance, family_of, guidance_for};
pub use ledger::{Failure, Ledger, Next, Progress, Status, UnitState, next as next_unit};

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
    for diagnostic in &report.diagnostics {
        match owners.get(diagnostic.path.as_str()) {
            Some(idxs) => {
                for &idx in idxs {
                    count_diagnostic(&mut counts[idx], diagnostic);
                }
            }
            None => count_diagnostic(&mut shared, diagnostic),
        }
    }

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
            findings.push(finding(project, doc.as_ref(), diagnostic));
        }
    }
    findings.sort_by(|a, b| {
        (a.span.line, a.span.column, &a.rule).cmp(&(b.span.line, b.span.column, &b.rule))
    });

    Job {
        schema_version: SCHEMA_VERSION,
        generator: Generator::default(),
        baseline,
        unit,
        findings,
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

fn finding(project: &Project, doc: Option<&LoadedDoc>, diagnostic: &Diagnostic) -> Finding {
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
    }
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

        // WXML 按元素锚点；样式按 selector + property；JS / JSON 等退回行列。
        let (id, _) = match find_view(views, &diagnostic.path) {
            Some((project, rel)) if diagnostic.path.ends_with(".wxml") => {
                let doc = docs
                    .entry(diagnostic.path.clone())
                    .or_insert_with(|| LoadedDoc::load(project, rel));
                identify_one(project, doc.as_ref(), diagnostic)
            }
            Some((project, _)) => identify_one(project, None, diagnostic),
            None => identify_one_fallback(diagnostic),
        };

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
        None => style_identity(project, diagnostic)
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
/// 旧格式基线仍可被 `verify` 兼容匹配，见 [`baseline_format`]。
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

/// 非 WXML 的新格式 id 以 `|#<occurrence>` 结尾；旧格式是 `rule|path|line:column`。
fn baseline_format(baseline: &Baseline) -> &'static str {
    match baseline.id_format {
        IdFormat::Structural => "current",
        IdFormat::Legacy => "legacy",
    }
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
#[serde(rename_all = "camelCase")]
pub struct Baseline {
    pub schema_version: u32,
    pub generator: Generator,
    /// id 格式标记。旧基线没有这个字段，反序列化为 `Legacy`。
    #[serde(default)]
    pub id_format: IdFormat,
    pub summary: BaselineSummary,
    pub findings: Vec<BaselineFinding>,
}

/// 基线里的 id 格式。`Legacy` 是 `rule|path|line:column`（非 WXML）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IdFormat {
    #[default]
    Legacy,
    Structural,
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
        id_format: IdFormat::Structural,
        summary,
        findings,
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
    /// `current`（结构稳定 id）或 `legacy`（旧的行列 id；按 rule+文件+顺序宽松匹配）。
    pub baseline_format: &'static str,
    /// **新增的 error + warning**（门禁判据：`new == 0`）。
    pub new: usize,
    /// 新增的 info（不拦门禁）。
    pub new_info: usize,
    pub fixed: usize,
    /// 同一 id、严重级变了（例如规则被调档）。
    pub changed: usize,
    pub findings: Vec<DiffEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffEntry {
    /// `new` / `fixed` / `severity-changed`
    pub kind: &'static str,
    pub id: String,
    pub rule: String,
    pub severity: Severity,
    pub path: String,
    pub span: Span,
    /// 诊断说明：`new` / `severity-changed` 用当前诊断，`fixed` 用基线里存的。
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
    let format = baseline_format(baseline);
    let legacy = format == "legacy";
    let scoped: Vec<&BaselineFinding> = baseline
        .findings
        .iter()
        .filter(|finding| unit.is_none_or(|wanted| finding.unit == wanted))
        .collect();
    let baseline_by_id: BTreeMap<&str, &BaselineFinding> = scoped
        .iter()
        .map(|finding| (finding.id.as_str(), *finding))
        .collect();

    // 旧基线没有结构信息，只能按 `(rule, path)` 分组、再按源码顺序对齐：插入空行 /
    // 整块挪动不会改变序号，所以不会误报。代价是「删一条 + 加一条」这种同数量
    // 变化分不出来——文档要求升级后重建基线。
    let legacy_by_key: BTreeMap<(&str, &str, usize), &BaselineFinding> = if legacy {
        let mut groups: BTreeMap<(&str, &str), Vec<&BaselineFinding>> = BTreeMap::new();
        for finding in &scoped {
            if !finding.path.ends_with(".wxml") {
                groups
                    .entry((finding.rule.as_str(), finding.path.as_str()))
                    .or_default()
                    .push(finding);
            }
        }
        let mut map = BTreeMap::new();
        for ((rule, path), mut group) in groups {
            group.sort_by_key(|finding| (finding.span.line, finding.span.column));
            for (occurrence, finding) in group.into_iter().enumerate() {
                map.insert((rule, path, occurrence), finding);
            }
        }
        map
    } else {
        BTreeMap::new()
    };
    let current_occurrence: BTreeMap<usize, usize> = if legacy {
        let mut groups: BTreeMap<(&str, &str), Vec<usize>> = BTreeMap::new();
        for (index, finding) in current.iter().enumerate() {
            if unit.is_none_or(|wanted| finding.unit == wanted) && !finding.path.ends_with(".wxml")
            {
                groups
                    .entry((finding.rule.as_str(), finding.path.as_str()))
                    .or_default()
                    .push(index);
            }
        }
        let mut map = BTreeMap::new();
        for (_, mut indexes) in groups {
            indexes.sort_by_key(|&index| (current[index].span.line, current[index].span.column));
            for (occurrence, index) in indexes.into_iter().enumerate() {
                map.insert(index, occurrence);
            }
        }
        map
    } else {
        BTreeMap::new()
    };

    let mut findings = Vec::new();
    let mut new = 0;
    let mut new_info = 0;
    let mut changed = 0;
    let mut seen: BTreeSet<&str> = BTreeSet::new();

    for (index, finding) in current.iter().enumerate() {
        if unit.is_some_and(|wanted| finding.unit != wanted) {
            continue;
        }
        let matched = baseline_by_id
            .get(finding.id.as_str())
            .copied()
            .or_else(|| {
                (legacy && !finding.path.ends_with(".wxml"))
                    .then(|| current_occurrence.get(&index))
                    .flatten()
                    .and_then(|occurrence| {
                        legacy_by_key
                            .get(&(finding.rule.as_str(), finding.path.as_str(), *occurrence))
                            .copied()
                    })
            });
        match matched {
            None => {
                match finding.severity {
                    Severity::Info => new_info += 1,
                    _ => new += 1,
                }
                findings.push(DiffEntry {
                    kind: "new",
                    id: finding.id.clone(),
                    rule: finding.rule.clone(),
                    severity: finding.severity,
                    path: finding.path.clone(),
                    span: finding.span.clone(),
                    message: Some(finding.message.clone()),
                    help: finding.help.clone(),
                });
            }
            Some(previous) if previous.severity != finding.severity => {
                seen.insert(previous.id.as_str());
                changed += 1;
                findings.push(DiffEntry {
                    kind: "severity-changed",
                    id: finding.id.clone(),
                    rule: finding.rule.clone(),
                    severity: finding.severity,
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
        if seen.contains(previous.id.as_str()) {
            continue;
        }
        fixed += 1;
        findings.push(DiffEntry {
            kind: "fixed",
            id: previous.id.clone(),
            rule: previous.rule.clone(),
            severity: previous.severity,
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
        baseline_format: format,
        new,
        new_info,
        fixed,
        changed,
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
