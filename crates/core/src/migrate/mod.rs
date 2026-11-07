//! 迁移接口：单元划分（`plan`）、作业单（`job`）、元素事实（`facts`）的契约。
//!
//! 契约本身写在 `docs/MIGRATE.md`；这里只负责把已有的 [`Facts`] / 诊断组装成
//! 可下发的数据，**不新增任何语义**——渲染器、可达性、级联、祖先链全部来自
//! `inspect` 与 `report`，避免两套实现漂移。

mod guidance;

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::diag::{Diagnostic, Severity};
use crate::inspect::{Facts, LayoutFacts, LoadedDoc, Renderer, UnitKind};
use crate::project::Project;
use crate::report::Report;
use crate::rules::wxml::Reach;

pub use guidance::{Guidance, family_of, guidance_for};

/// 产物格式版本。破坏性变更才 +1。
pub const SCHEMA_VERSION: u32 = 1;

// ---------------------------------------------------------------- 公共类型

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Generator {
    pub name: &'static str,
    pub version: &'static str,
    /// 生成时的提交；未知为 `null`（不影响两次 `plan` 的字节一致性）。
    pub commit: Option<String>,
}

impl Default for Generator {
    fn default() -> Self {
        Self {
            name: "skylint",
            version: env!("CARGO_PKG_VERSION"),
            commit: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BaselineRef {
    pub file: Option<String>,
    /// 基线内容摘要。M1 不计算，留给 `verify`（M2）填。
    pub sha256: Option<String>,
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

#[derive(Debug, Clone, Serialize)]
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
    /// 命令模板：`{unit}` / `{unitPath}` / `{baseline}` / `{job}` 由 runner 替换。
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
                    files: unit_files(prefix, &page.files),
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
                    files: unit_files(prefix, &component.files),
                });
            }
        }
    }

    // 诊断归属：先按单元文件，剩下的进 shared。
    let mut owner: BTreeMap<&str, usize> = BTreeMap::new();
    for (idx, seed) in seeds.iter().enumerate() {
        for file in &seed.files {
            owner.insert(file.as_str(), idx);
        }
    }
    let mut counts: Vec<DiagCounts> = vec![DiagCounts::default(); seeds.len()];
    let mut shared = DiagCounts::default();
    for diagnostic in &report.diagnostics {
        let slot = match owner.get(diagnostic.path.as_str()) {
            Some(&idx) => &mut counts[idx],
            None => &mut shared,
        };
        slot.total += 1;
        match diagnostic.severity {
            Severity::Error => slot.error += 1,
            Severity::Warning => slot.warning += 1,
            Severity::Info => slot.info += 1,
        }
        *slot.by_rule.entry(diagnostic.rule.clone()).or_default() += 1;
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

fn unit_files(prefix: &str, files: &crate::project::UnitFiles) -> Vec<String> {
    [&files.wxml, &files.wxss, &files.js, &files.json]
        .into_iter()
        .flatten()
        .map(|file| format!("{prefix}{file}"))
        .collect()
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
    let unit = match &loaded {
        Some(doc) => {
            let facts = doc.facts(project);
            let unit = facts.unit();
            JobUnit {
                id: plan_unit.id.clone(),
                path: plan_unit.path.clone(),
                kind: kind_str(&unit.kind),
                renderer: renderer_str(unit.renderer),
                reach: reach_str(unit.reach),
                style_isolation: unit
                    .style_isolation
                    .map(|iso| format!("{iso:?}").to_lowercase()),
                default_display_block: unit.default_display_block,
                default_content_box: unit.default_content_box,
                scroll_view_auto_size: unit.scroll_view_auto_size,
            }
        }
        None => JobUnit {
            id: plan_unit.id.clone(),
            path: plan_unit.path.clone(),
            kind: plan_unit.kind,
            renderer: plan_unit.renderer,
            reach: plan_unit.reach,
            style_isolation: None,
            default_display_block: false,
            default_content_box: false,
            scroll_view_auto_size: false,
        },
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
                run: "skylint verify --unit {unit} --baseline {baseline} --json",
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
        let anchor = anchor_for(&facts, element);
        let mut cascade = BTreeMap::new();
        for property in guidance.facts {
            let entries = facts.cascade(element, property);
            if !entries.is_empty() {
                cascade.insert(property.to_string(), entries);
            }
        }
        Some((
            anchor,
            FindingFacts {
                layout: facts.layout(element),
                cascade,
                ancestors: facts.ancestors(element),
                handlers: facts.handlers(element),
            },
        ))
    });

    let id = match &resolved {
        Some((anchor, _)) => anchor_id(&diagnostic.rule, &diagnostic.path, anchor),
        None => format!(
            "{}|{}|{}:{}",
            diagnostic.rule, diagnostic.path, diagnostic.line, diagnostic.column
        ),
    };

    Finding {
        id,
        rule: diagnostic.rule.clone(),
        severity: diagnostic.severity,
        message: diagnostic.message.clone(),
        help: diagnostic.help.clone(),
        span,
        anchor: resolved.as_ref().map(|(anchor, _)| anchor.clone()),
        facts: resolved.map(|(_, facts)| facts),
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
    pub span: Span,
    pub tag: String,
    pub classes: Vec<String>,
    pub id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ElementDetail {
    pub span: Span,
    pub tag: String,
    pub classes: Vec<String>,
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
                id: anchor_id("<element>", facts.path(), &anchor),
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
        id: anchor_id("<element>", facts.path(), &anchor),
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

/// 诊断路径是否属于这个单元（同名资源 + 常见扩展名）。
fn owns(unit: &PlanUnit, path: &str) -> bool {
    let base = unit.path.strip_suffix(".wxml").unwrap_or(&unit.path);
    if path == base {
        return true;
    }
    path.strip_prefix(base)
        .and_then(|rest| rest.strip_prefix('.'))
        .is_some_and(|ext| matches!(ext, "wxml" | "wxss" | "js" | "ts" | "wxs" | "json"))
}

/// 产物序列化。放在引擎侧，集成层不必依赖 serde。
pub fn to_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).expect("migrate 产物总是可序列化")
}
