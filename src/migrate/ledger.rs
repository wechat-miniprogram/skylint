//! 迁移台账：哪些单元做到哪一步了。
//!
//! 台账是**纯状态**，不含时间戳——何时改的由 git 提交记录回答，写进文件只会让
//! diff 变噪、让两次运行的结果不可比。它必须能被提交、被 diff、被断点续跑。

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use super::{Generator, Plan, PlanUnit, SCHEMA_VERSION, SharedFiles};

/// 单元状态。终态之后不再由 `next` 返回。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// 还没开始。
    #[default]
    Pending,
    /// 作业单已生成，agent 还没动。
    Planned,
    /// agent 已改完，还没过门禁。
    Migrated,
    /// 静态与构建门禁都过了，等人（或策略）接受。
    Verified,
    /// 接受，不再处理。
    Accepted,
    /// 尝试次数用尽，需要人介入。
    Blocked,
    /// 明确跳过（例如页面声明了 `renderer: "webview"`）。
    Skipped,
}

impl Status {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Accepted | Self::Blocked | Self::Skipped)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Planned => "planned",
            Self::Migrated => "migrated",
            Self::Verified => "verified",
            Self::Accepted => "accepted",
            Self::Blocked => "blocked",
            Self::Skipped => "skipped",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ledger {
    pub schema_version: u32,
    pub generator: Generator,
    pub units: BTreeMap<String, UnitState>,
}

/// `ledger --json` 的输出：台账本身 + `plan` 里的共享清单。
///
/// `shared` 是派生数据，不写进台账文件（台账只存状态）。
#[derive(Debug, Serialize)]
pub struct LedgerReport<'a> {
    #[serde(flatten)]
    pub ledger: &'a Ledger,
    pub shared: &'a SharedFiles,
}

impl Default for Ledger {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            generator: Generator::default(),
            units: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitState {
    pub status: Status,
    /// 记录过几次失败。由 runner 决定上限，台账只记数。
    #[serde(default)]
    pub failures: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_failure: Option<Failure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// 逐条豁免的诊断：认定「不需要处理」，机械判定里当它已经解决。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waived: Vec<Waiver>,
    /// 门禁执行记录：只记事实，不参与排队与判定。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gates: Vec<GateRecord>,
}

impl Default for UnitState {
    fn default() -> Self {
        Self {
            status: Status::Pending,
            failures: 0,
            last_failure: None,
            note: None,
            waived: Vec::new(),
            gates: Vec::new(),
        }
    }
}

/// 迁移门禁。`l0`–`l5` 与 `docs/MIGRATE.md` §9 的定义一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Gate {
    /// 静态字段级 diff（`verify`）。
    L0,
    /// 生产构建。
    L1,
    /// automator 冒烟。
    L2,
    /// 截图对比。
    L3,
    /// AI 判定。
    L4,
    /// 人工。
    L5,
}

impl Gate {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::L0 => "l0",
            Self::L1 => "l1",
            Self::L2 => "l2",
            Self::L3 => "l3",
            Self::L4 => "l4",
            Self::L5 => "l5",
        }
    }
}

impl fmt::Display for Gate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Gate {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "l0" => Ok(Self::L0),
            "l1" => Ok(Self::L1),
            "l2" => Ok(Self::L2),
            "l3" => Ok(Self::L3),
            "l4" => Ok(Self::L4),
            "l5" => Ok(Self::L5),
            other => Err(format!("未知门禁 `{other}`（应为 l0…l5）")),
        }
    }
}

/// 一道门禁的执行结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GateResult {
    Pass,
    Fail,
    Skip,
}

impl GateResult {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::Skip => "skip",
        }
    }
}

impl fmt::Display for GateResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 一次门禁记录。
///
/// 台账不取时钟：`at` 由调用方给出，不填就不写，保持同一输入产出同一份文件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GateRecord {
    pub gate: Gate,
    pub result: GateResult,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// 截图、构建日志等证据的位置。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
}

/// 一条被认定为「不需要处理」的诊断。
///
/// 粒度是**单条诊断**而不是整个单元：单元里往往一半是真问题、一半是刻意保留的
/// 差异（info 级、既定取舍、需要页面级改造才能修）。`reason` 必填——没有理由的
/// 豁免等于静默忽略，复核时无从判断。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Waiver {
    /// 与 baseline / verify 同一套稳定 id（`docs/MIGRATE.md` §4.2）。
    pub id: String,
    pub rule: String,
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Failure {
    /// 卡在哪道门禁（`l0-static` / `l1-build` …）；人工记录时可为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<String>,
    pub reason: String,
}

impl Ledger {
    pub fn state(&self, unit: &str) -> Status {
        self.units
            .get(unit)
            .map(|state| state.status)
            .unwrap_or_default()
    }

    /// 写一条状态。`failure` 有值表示这次尝试失败了（`failures` +1）。
    ///
    /// 不做状态机校验：允许 `pending → accepted` 这种跳步（人工直接判掉），
    /// 语义由 runner 决定，台账只负责持久化。
    pub fn record(
        &mut self,
        unit: &str,
        status: Status,
        failure: Option<Failure>,
        note: Option<String>,
    ) {
        let state = self.units.entry(unit.to_string()).or_default();
        state.status = status;
        if let Some(failure) = failure {
            state.failures += 1;
            state.last_failure = Some(failure);
        }
        if note.is_some() {
            state.note = note;
        }
        if status == Status::Accepted {
            state.last_failure = None;
        }
    }

    /// 豁免一条诊断。同一 id 重复写会覆盖理由（判断可能被修正）。
    pub fn waive(&mut self, unit: &str, waiver: Waiver) {
        let state = self.units.entry(unit.to_string()).or_default();
        match state.waived.iter_mut().find(|slot| slot.id == waiver.id) {
            Some(slot) => *slot = waiver,
            None => state.waived.push(waiver),
        }
    }

    /// 记一道门禁结果。同一门禁重跑是常态，`(unit, gate)` 后者覆盖前者。
    ///
    /// 顺序按门禁固定，保证同一组记录无论写入顺序如何，写出字节一致。
    pub fn record_gate(&mut self, unit: &str, record: GateRecord) {
        let gates = &mut self.units.entry(unit.to_string()).or_default().gates;
        match gates.iter_mut().find(|slot| slot.gate == record.gate) {
            Some(slot) => *slot = record,
            None => gates.push(record),
        }
        gates.sort_by_key(|slot| slot.gate);
    }

    /// 撤销一条豁免，返回原来的理由。
    pub fn unwaive(&mut self, id: &str) -> Option<String> {
        for state in self.units.values_mut() {
            if let Some(pos) = state.waived.iter().position(|waiver| waiver.id == id) {
                return Some(state.waived.remove(pos).reason);
            }
        }
        None
    }

    /// 全部豁免的诊断 id。`plan` / `verify` 据此把豁免项从两侧一起摘掉。
    pub fn waived_ids(&self) -> BTreeSet<&str> {
        self.units
            .values()
            .flat_map(|state| state.waived.iter())
            .map(|waiver| waiver.id.as_str())
            .collect()
    }

    /// 豁免总数（不看单元）。
    pub fn waived_count(&self) -> usize {
        self.units.values().map(|state| state.waived.len()).sum()
    }

    /// 删掉计划里已经不存在的单元（改名、删除组件后留下的陈旧条目）。
    pub fn prune(&mut self, plan: &Plan) -> usize {
        let live: std::collections::BTreeSet<&str> =
            plan.units.iter().map(|unit| unit.id.as_str()).collect();
        let stale: Vec<String> = self
            .units
            .keys()
            .filter(|id| !live.contains(id.as_str()))
            .cloned()
            .collect();
        for id in &stale {
            self.units.remove(id);
        }
        stale.len()
    }
}

/// 进度统计（只数需要迁移的单元：`diagnostics.total > 0`）。
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub total: usize,
    pub pending: usize,
    /// 作业单已生成、agent 还没跑。
    pub planned: usize,
    /// agent 改完了，门禁没过。
    pub migrated: usize,
    /// 过了门禁，**在等人接受**。`next` 不会返回这类单元。
    pub verified: usize,
    pub accepted: usize,
    pub blocked: usize,
    pub skipped: usize,
    /// 计划里没有诊断、不需要迁移的单元数。
    pub not_needed: usize,
    /// 共享资源（`unit:shared`）上的诊断条数。不参与自动排队：
    /// 改共享样式会影响多个单元，必须单独审批。
    pub shared: usize,
}

impl Progress {
    /// 正在流程里（不含 `verified`：那一步在等人）。
    pub fn in_progress(&self) -> usize {
        self.planned + self.migrated
    }
}

/// 这个状态还需要跑 agent 吗。
///
/// `verified` 不在内：它已经过了门禁，只是在等人接受——再跑一次 agent 会把
/// 人的等待变成重复劳动。人要打回时显式 `record --status migrated`。
fn needs_agent(status: Status) -> bool {
    matches!(status, Status::Pending | Status::Planned | Status::Migrated)
}

/// 下一个该做的单元。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Next {
    /// 全部做完时为 `null`。
    pub unit: Option<PlanUnit>,
    pub progress: Progress,
    /// 这个单元已经失败过几次（runner 据此决定是否放弃）。
    pub failures: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_failure: Option<Failure>,
}

/// 按批次顺序取第一个还没到终态的单元。
///
/// 批次本身就是拓扑层（`batch(u) = 1 + max(batch(依赖))`），所以「先做叶子」
/// 由批次顺序保证，不需要再单独做依赖门控。
pub fn next(plan: &Plan, ledger: &Ledger) -> Next {
    let mut progress = Progress::default();

    for unit in &plan.units {
        if unit.kind == "shared" {
            progress.shared += unit.diagnostics.total;
            continue;
        }
        if unit.diagnostics.total == 0 && !ledger.units.contains_key(&unit.id) {
            progress.not_needed += 1;
            continue;
        }
        progress.total += 1;
        match ledger.state(&unit.id) {
            Status::Pending => progress.pending += 1,
            Status::Planned => progress.planned += 1,
            Status::Migrated => progress.migrated += 1,
            Status::Verified => progress.verified += 1,
            Status::Accepted => progress.accepted += 1,
            Status::Blocked => progress.blocked += 1,
            Status::Skipped => progress.skipped += 1,
        }
    }

    let unit = plan
        .batches
        .iter()
        .flatten()
        .filter_map(|id| plan.units.iter().find(|unit| &unit.id == id))
        .find(|unit| {
            unit.kind != "shared"
                && unit.diagnostics.total > 0
                && needs_agent(ledger.state(&unit.id))
        })
        .cloned();

    let state = unit
        .as_ref()
        .and_then(|unit| ledger.units.get(&unit.id).cloned())
        .unwrap_or_default();

    Next {
        unit,
        progress,
        failures: state.failures,
        last_failure: state.last_failure,
    }
}
