use std::collections::HashSet;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};
use skylint::migrate::{self, Plan, PlanUnit};
use skylint::render::{self, Style};
use skylint::rules::Target;
use skylint::{Project, Registry, Report};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum TargetArg {
    /// WebView 代码迁到 Skyline 时哪里会坏（默认）
    Skyline,
    /// 按 Skyline 写的代码退回 WebView 渲染时哪些能力静默降级
    Webview,
}

impl From<TargetArg> for Target {
    fn from(arg: TargetArg) -> Self {
        match arg {
            TargetArg::Skyline => Self::Skyline,
            TargetArg::Webview => Self::Webview,
        }
    }
}

#[derive(Debug, Parser)]
#[command(name = "skylint", version, about = "Skyline 小程序迁移检查工具")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// 项目根目录（含 project.config.json），或单个 .wxml / .wxss / .js 文件
    #[arg(value_name = "PATH")]
    path: Option<PathBuf>,

    /// 输出 JSON（机器可读）
    #[arg(long)]
    json: bool,

    #[command(flatten)]
    ignored: IgnoreRules,

    /// 检查方向
    #[arg(long, value_enum, default_value = "skyline")]
    target: TargetArg,
}

// 顶层只有两类命令：不带子命令是 lint，`migration` 下面全是迁移。
// 迁移命令的使用者是 agent，名字长短不重要；把它们和 lint 分开，是为了让
// `skylint --help` 一眼看出「这是个 linter，附带一套迁移工具」。
#[derive(Debug, Subcommand)]
enum Command {
    /// 迁移：单元清单 / 作业单 / 门禁 / 台账（Agent 调用）
    #[command(subcommand)]
    Migration(MigrationCommand),
}

#[derive(Debug, Subcommand)]
enum MigrationCommand {
    /// 生成单元清单（plan.json）：有哪些单元、什么顺序、各有多少问题
    Plan(PlanArgs),
    /// 生成单个单元的作业单（job.json）：改什么、怎么算过
    Job(JobArgs),
    /// 查看单元与元素事实
    Facts(FactsArgs),
    /// 冻结基线：当前全部诊断的稳定身份
    Baseline(BaselineArgs),
    /// 门禁：与基线对比，新增 error/warning 即失败
    Verify(VerifyArgs),
    /// 台账：看迁移进度
    Ledger(LedgerArgs),
    /// 记一条单元状态
    Record(RecordArgs),
    /// 下一个该做的单元
    Next(NextArgs),
}

/// 迁移子命令都接受的「看哪个工程、怎么输出」。
#[derive(Debug, Args)]
struct Projection {
    /// 项目根目录
    #[arg(default_value = ".", value_name = "PATH")]
    path: PathBuf,
    /// 输出 JSON（机器可读）
    #[arg(long)]
    json: bool,
}

/// `--ignore`：跳过的规则 id（可重复）。
#[derive(Debug, Args)]
struct IgnoreRules {
    /// 跳过的规则 id（可重复）
    #[arg(long = "ignore", value_name = "RULE")]
    ignore: Vec<String>,
}

#[derive(Debug, Args)]
struct PlanArgs {
    #[command(flatten)]
    project: Projection,
    /// 写入文件（等价于 `--json > FILE`）
    #[arg(long, value_name = "FILE")]
    out: Option<PathBuf>,
    /// 基线报告，用于对账
    #[arg(long, value_name = "FILE")]
    baseline: Option<PathBuf>,
    #[command(flatten)]
    ignored: IgnoreRules,
}

#[derive(Debug, Args)]
struct JobArgs {
    #[command(flatten)]
    project: Projection,
    /// 单元 id（`unit:components/btn/btn`）或 WXML 路径
    #[arg(long, value_name = "UNIT")]
    unit: String,
    /// 写入文件
    #[arg(long, value_name = "FILE")]
    out: Option<PathBuf>,
    /// 基线报告
    #[arg(long, value_name = "FILE")]
    baseline: Option<PathBuf>,
    #[command(flatten)]
    ignored: IgnoreRules,
}

#[derive(Debug, Args)]
struct BaselineArgs {
    #[command(flatten)]
    project: Projection,
    /// 写入位置（默认 `<PATH>/.skylint/baseline.json`）
    #[arg(long, value_name = "FILE")]
    out: Option<PathBuf>,
    /// 在已有基线上增量吸收「冻结后新增规则」的诊断（`--out` 必须指向已存在的基线）
    #[arg(long)]
    augment: bool,
    #[command(flatten)]
    ignored: IgnoreRules,
}

#[derive(Debug, Args)]
struct VerifyArgs {
    #[command(flatten)]
    project: Projection,
    /// 基线文件（`skylint migration baseline` 生成）
    #[arg(long, value_name = "FILE")]
    baseline: PathBuf,
    /// 只看这个单元（id 或 WXML 路径）
    #[arg(long, value_name = "UNIT")]
    unit: Option<String>,
    #[command(flatten)]
    ignored: IgnoreRules,
}

#[derive(Debug, Args)]
struct LedgerArgs {
    #[command(flatten)]
    project: Projection,
    /// 删掉计划里已不存在的单元（改名/删除组件后的陈旧条目）
    #[arg(long)]
    prune: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum StatusArg {
    Pending,
    Planned,
    Migrated,
    Verified,
    Accepted,
    Blocked,
    Skipped,
}

impl From<StatusArg> for migrate::Status {
    fn from(arg: StatusArg) -> Self {
        match arg {
            StatusArg::Pending => Self::Pending,
            StatusArg::Planned => Self::Planned,
            StatusArg::Migrated => Self::Migrated,
            StatusArg::Verified => Self::Verified,
            StatusArg::Accepted => Self::Accepted,
            StatusArg::Blocked => Self::Blocked,
            StatusArg::Skipped => Self::Skipped,
        }
    }
}

#[derive(Debug, Args)]
struct RecordArgs {
    /// 项目根目录
    #[arg(default_value = ".", value_name = "PATH")]
    path: PathBuf,
    /// 单元 id 或 WXML 路径
    #[arg(long, value_name = "UNIT")]
    unit: String,
    /// 新状态
    #[arg(long, value_enum)]
    status: StatusArg,
    /// 卡在哪道门禁（配合 `--reason`，表示这次尝试失败了）
    #[arg(long, value_name = "GATE")]
    gate: Option<String>,
    /// 失败原因；给了就记一次失败
    #[arg(long, value_name = "TEXT")]
    reason: Option<String>,
    /// 备注
    #[arg(long, value_name = "TEXT")]
    note: Option<String>,
}

#[derive(Debug, Args)]
struct NextArgs {
    #[command(flatten)]
    project: Projection,
    /// 顺便生成这个单元的作业单
    #[arg(long, value_name = "FILE")]
    job_out: Option<PathBuf>,
    /// 基线报告
    #[arg(long, value_name = "FILE")]
    baseline: Option<PathBuf>,
    #[command(flatten)]
    ignored: IgnoreRules,
}

#[derive(Debug, Args)]
struct FactsArgs {
    /// 项目根目录
    #[arg(value_name = "PATH")]
    path: PathBuf,
    /// 单元 WXML（项目相对，可带插件前缀）
    #[arg(value_name = "WXML")]
    unit: String,
    /// 元素位置 `行:列`（1-based，按包含关系取最深元素；输出 span 是该元素的范围）
    #[arg(long, value_name = "LINE:COL")]
    element: Option<String>,
    /// 只看某个属性（配合 `--element`）
    #[arg(long, value_name = "PROPERTY")]
    property: Option<String>,
    /// 输出 JSON（机器可读）
    #[arg(long)]
    json: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Migration(command)) => run_migration(command),
        None => run_check(
            cli.path.unwrap_or_else(|| PathBuf::from(".")),
            cli.json,
            cli.ignored.ignore,
            cli.target.into(),
        ),
    }
}

fn run_migration(command: MigrationCommand) -> ExitCode {
    match command {
        MigrationCommand::Plan(args) => run_plan(args),
        MigrationCommand::Job(args) => run_job(args),
        MigrationCommand::Facts(args) => run_facts(args),
        MigrationCommand::Baseline(args) => run_baseline(args),
        MigrationCommand::Verify(args) => run_verify(args),
        MigrationCommand::Ledger(args) => run_ledger(args),
        MigrationCommand::Record(args) => run_record(args),
        MigrationCommand::Next(args) => run_next(args),
    }
}

fn run_check(path: PathBuf, json: bool, ignore: Vec<String>, target: Target) -> ExitCode {
    let registry = Registry::builtin().for_target(target);
    let projects = match load(&path) {
        Ok(projects) => projects,
        Err(code) => return code,
    };

    let ignored: HashSet<String> = ignore.into_iter().collect();
    let views: Vec<&Project> = projects.iter().collect();
    let diagnostics = registry.run_views(&views, &ignored);
    let report = Report::from_views(&projects, diagnostics);

    if json {
        println!("{}", report.to_json());
    } else {
        print!(
            "{}",
            render::text(
                &report,
                |path| {
                    // 宿主路径相对 miniprogramRoot、插件路径相对项目根，
                    // project.config.json 这类配置又在项目根，依次回落。
                    projects
                        .iter()
                        .find_map(|project| std::fs::read_to_string(project.abs(path)).ok())
                        .or_else(|| std::fs::read_to_string(projects[0].root.join(path)).ok())
                },
                Style {
                    colored: std::io::stdout().is_terminal(),
                    // 片段折行阈值；没有 COLUMNS 时给一个宽于常见终端的值
                    width: std::env::var("COLUMNS")
                        .ok()
                        .and_then(|c| c.parse().ok())
                        .unwrap_or(100),
                }
            )
        );
    }

    if report.has_errors() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn run_plan(args: PlanArgs) -> ExitCode {
    let (projects, report) = match analyse(&args.project.path, &args.ignored.ignore) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let baseline = baseline_ref(args.baseline.as_deref(), report.diagnostics.len());
    let plan = migrate::plan(&projects, &report, baseline);

    if args.project.json || args.out.is_some() {
        let json = migrate::to_json(&plan);
        if let Some(out) = args.out {
            if let Err(err) = write_out(&out, &json) {
                eprintln!("skylint: {err}");
                return ExitCode::from(2);
            }
            eprintln!("wrote {}", out.display());
        } else {
            println!("{json}");
        }
    } else {
        print_plan(&plan);
    }
    ExitCode::SUCCESS
}

fn run_job(args: JobArgs) -> ExitCode {
    let (projects, report) = match analyse(&args.project.path, &args.ignored.ignore) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let baseline = baseline_ref(args.baseline.as_deref(), report.diagnostics.len());
    let plan = migrate::plan(&projects, &report, baseline.clone());

    let Some(unit) = find_unit(&plan, &args.unit) else {
        eprintln!(
            "skylint: 找不到单元 `{}`（先跑 `skylint migration plan` 看清单）",
            args.unit
        );
        return ExitCode::from(1);
    };
    let Some(job) = build_job(&projects, &report, &plan, unit, baseline) else {
        eprintln!("skylint: 单元 `{}` 不属于任何视图", unit.id);
        return ExitCode::from(1);
    };

    if args.project.json || args.out.is_some() {
        let json = migrate::to_json(&job);
        if let Some(out) = args.out {
            if let Err(err) = write_out(&out, &json) {
                eprintln!("skylint: {err}");
                return ExitCode::from(2);
            }
            eprintln!("wrote {}", out.display());
        } else {
            println!("{json}");
        }
    } else {
        print_job(&job);
    }
    ExitCode::SUCCESS
}

fn run_facts(args: FactsArgs) -> ExitCode {
    let projects = match load(&args.path) {
        Ok(projects) => projects,
        Err(code) => return code,
    };
    let Some((project, rel)) = migrate::find_view(&projects, &args.unit) else {
        eprintln!("skylint: `{}` 不属于任何视图", args.unit);
        return ExitCode::from(1);
    };
    let rel = rel.to_string();

    let element = match args.element.as_deref() {
        Some(spec) => match parse_position(spec) {
            Some(position) => Some(position),
            None => {
                eprintln!("skylint: `--element` 要写成 `行:列`，例如 `87:3`");
                return ExitCode::from(2);
            }
        },
        None => None,
    };

    match element {
        Some((line, column)) => {
            let Some(detail) =
                migrate::element_detail(project, &rel, line, column, args.property.as_deref())
            else {
                eprintln!("skylint: `{rel}` 在 {line}:{column} 没有元素");
                return ExitCode::from(1);
            };
            if args.json {
                println!("{}", migrate::to_json(&detail));
            } else {
                print_element(&detail);
            }
        }
        None => {
            let Some(doc) = migrate::facts_doc(project, &rel) else {
                eprintln!("skylint: 读不到 `{rel}`");
                return ExitCode::from(1);
            };
            if args.json {
                println!("{}", migrate::to_json(&doc));
            } else {
                print_facts(&doc);
            }
        }
    }
    ExitCode::SUCCESS
}

fn run_baseline(args: BaselineArgs) -> ExitCode {
    let (projects, report) = match analyse(&args.project.path, &args.ignored.ignore) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let plan = migrate::plan(&projects, &report, None);
    let identified = migrate::identify(&projects, &plan, &report);

    if args.augment {
        return run_augment(args, &identified);
    }

    let baseline = migrate::baseline(&identified);
    let json = migrate::to_json(&baseline);
    // 默认落在工程根下，不落在当前目录——否则从别处调就会写错地方。
    let out = args
        .out
        .unwrap_or_else(|| args.project.path.join(".skylint/baseline.json"));

    if let Err(err) = write_out(&out, &json) {
        eprintln!("skylint: {err}");
        return ExitCode::from(2);
    }
    if args.project.json {
        println!("{json}");
    } else {
        eprintln!(
            "wrote {}（{} 条：e{} w{} i{}）",
            out.display(),
            baseline.summary.total,
            baseline.summary.error,
            baseline.summary.warning,
            baseline.summary.info
        );
    }
    ExitCode::SUCCESS
}

/// 在已有基线上只吸收「新规则」的诊断，不重算整份基线。
fn run_augment(args: BaselineArgs, identified: &[migrate::Identified]) -> ExitCode {
    let Some(out) = args.out else {
        eprintln!("skylint: --augment 需要 --out 指向已存在的基线文件");
        return ExitCode::from(2);
    };
    let text = match std::fs::read_to_string(&out) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("skylint: 读不到基线 {}: {err}", out.display());
            return ExitCode::from(2);
        }
    };
    let mut baseline = match migrate::Baseline::load(&text) {
        Ok(baseline) => baseline,
        Err(err) => {
            eprintln!("skylint: 基线格式不对 {}: {err}", out.display());
            return ExitCode::from(2);
        }
    };

    // 写回前自检：追加逻辑正确时，拿新基线再跑一遍 verify，新增必须是零；
    // fixed / changed 只能来自基线冻结后的代码/规则变化，augment 不能增加它们。
    let before = migrate::verify(&baseline, identified, None, Some(out.display().to_string()));
    let report = migrate::augment(&mut baseline, identified);
    let check = migrate::verify(&baseline, identified, None, Some(out.display().to_string()));
    if check.new != 0
        || check.new_info != 0
        || check.fixed != before.fixed
        || check.changed != before.changed
    {
        eprintln!(
            "skylint: augment 自检失败（new {} / newInfo {} / fixed {} → {} / changed {} → {}），未写回基线",
            check.new, check.new_info, before.fixed, check.fixed, before.changed, check.changed
        );
        print_verify(&check, &[]);
        return ExitCode::FAILURE;
    }

    let json = migrate::to_json(&baseline);
    if let Err(err) = write_out(&out, &json) {
        eprintln!("skylint: {err}");
        return ExitCode::from(2);
    }
    print_augment(&out, &report, &before, &check);
    if args.project.json {
        println!("{json}");
    }
    ExitCode::SUCCESS
}

fn print_augment(
    out: &Path,
    report: &migrate::AugmentReport,
    before: &migrate::VerifyReport,
    check: &migrate::VerifyReport,
) {
    eprintln!(
        "augment {}：吸收 {} 条，基线共 {} 条",
        out.display(),
        report.added,
        report.total
    );
    for rule in &report.rules {
        let files = rule.files.join("、");
        let more = if rule.file_count > rule.files.len() {
            format!("，等 {} 个文件", rule.file_count)
        } else {
            String::new()
        };
        eprintln!("  {}  {} 条  {files}{more}", rule.rule, rule.count);
    }
    eprintln!(
        "自检 verify：new {} / newInfo {} / fixed {}（augment 前 {}）/ changed {}（augment 前 {}）",
        check.new, check.new_info, check.fixed, before.fixed, check.changed, before.changed
    );
}

fn run_verify(args: VerifyArgs) -> ExitCode {
    let text = match std::fs::read_to_string(&args.baseline) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("skylint: 读不到基线 {}: {err}", args.baseline.display());
            return ExitCode::from(2);
        }
    };
    let baseline: migrate::Baseline = match migrate::Baseline::load(&text) {
        Ok(baseline) => baseline,
        Err(err) => {
            eprintln!("skylint: 基线格式不对 {}: {err}", args.baseline.display());
            return ExitCode::from(2);
        }
    };

    let (projects, report) = match analyse(&args.project.path, &args.ignored.ignore) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let plan = migrate::plan(&projects, &report, None);

    // `--unit` 既可以是 id，也可以是 WXML 路径。
    let unit = match args.unit.as_deref() {
        Some(needle) => match find_unit(&plan, needle) {
            Some(unit) => Some(unit.id.clone()),
            None => {
                eprintln!("skylint: 找不到单元 `{needle}`（先跑 `skylint migration plan` 看清单）");
                return ExitCode::from(1);
            }
        },
        None => None,
    };

    let identified = migrate::identify(&projects, &plan, &report);
    let result = migrate::verify(
        &baseline,
        &identified,
        unit.as_deref(),
        Some(args.baseline.display().to_string()),
    );
    let unknown = migrate::unknown_rules(&baseline, &migrate::builtin_rule_ids());

    if args.project.json {
        println!("{}", migrate::to_json(&result));
    } else {
        print_verify(&result, &unknown);
    }

    if result.blocks() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn run_ledger(args: LedgerArgs) -> ExitCode {
    let (projects, report) = match analyse(&args.project.path, &[]) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let plan = migrate::plan(&projects, &report, None);
    let path = ledger_path(&args.project.path);
    let mut ledger = match load_ledger(&path) {
        Ok(ledger) => ledger,
        Err(err) => {
            eprintln!("skylint: {err}");
            return ExitCode::from(2);
        }
    };

    if args.prune {
        let removed = ledger.prune(&plan);
        if let Err(err) = save_ledger(&path, &ledger) {
            eprintln!("skylint: {err}");
            return ExitCode::from(2);
        }
        eprintln!("清理了 {removed} 个计划里已不存在的单元");
    }

    if args.project.json {
        println!(
            "{}",
            migrate::to_json(&migrate::LedgerReport {
                ledger: &ledger,
                shared: &plan.shared,
            })
        );
    } else {
        print_ledger(&plan, &ledger);
    }
    ExitCode::SUCCESS
}

fn run_record(args: RecordArgs) -> ExitCode {
    let (projects, report) = match analyse(&args.path, &[]) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let plan = migrate::plan(&projects, &report, None);
    let Some(unit) = find_unit(&plan, &args.unit) else {
        eprintln!(
            "skylint: 计划里没有单元 `{}`（先跑 `skylint migration plan`，或 `skylint migration ledger --prune`）",
            args.unit
        );
        return ExitCode::from(1);
    };

    let path = ledger_path(&args.path);
    let mut ledger = match load_ledger(&path) {
        Ok(ledger) => ledger,
        Err(err) => {
            eprintln!("skylint: {err}");
            return ExitCode::from(2);
        }
    };

    let failure = args.reason.map(|reason| migrate::Failure {
        gate: args.gate.clone(),
        reason,
    });
    ledger.record(&unit.id.clone(), args.status.into(), failure, args.note);

    if let Err(err) = save_ledger(&path, &ledger) {
        eprintln!("skylint: {err}");
        return ExitCode::from(2);
    }
    let state = ledger.units.get(&unit.id).cloned().unwrap_or_default();
    eprintln!(
        "{} → {}（失败 {} 次）",
        unit.id,
        state.status.as_str(),
        state.failures
    );
    ExitCode::SUCCESS
}

fn run_next(args: NextArgs) -> ExitCode {
    let (projects, report) = match analyse(&args.project.path, &args.ignored.ignore) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let baseline = baseline_ref(args.baseline.as_deref(), report.diagnostics.len());
    let plan = migrate::plan(&projects, &report, baseline.clone());
    let path = ledger_path(&args.project.path);
    let ledger = match load_ledger(&path) {
        Ok(ledger) => ledger,
        Err(err) => {
            eprintln!("skylint: {err}");
            return ExitCode::from(2);
        }
    };

    let next = migrate::next_unit(&plan, &ledger);

    if let (Some(unit), Some(out)) = (&next.unit, &args.job_out) {
        let Some(job) = build_job(&projects, &report, &plan, unit, baseline) else {
            eprintln!("skylint: 单元 `{}` 不属于任何视图", unit.id);
            return ExitCode::from(1);
        };
        if let Err(err) = write_out(out, &migrate::to_json(&job)) {
            eprintln!("skylint: {err}");
            return ExitCode::from(2);
        }
        eprintln!("wrote {}", out.display());
    }

    if args.project.json {
        println!("{}", migrate::to_json(&next));
    } else {
        print_next(&next);
    }
    ExitCode::SUCCESS
}

fn build_job(
    projects: &[Project],
    report: &Report,
    plan: &Plan,
    unit: &PlanUnit,
    baseline: Option<migrate::BaselineRef>,
) -> Option<migrate::Job> {
    let (project, rel) = migrate::find_view(projects, &unit.path)?;
    let diagnostics = migrate::diagnostics_for(plan, unit, report);
    Some(migrate::job(
        project,
        rel,
        plan,
        unit,
        &diagnostics,
        baseline,
    ))
}

fn ledger_path(root: &Path) -> PathBuf {
    root.join(".skylint/ledger.json")
}

fn load_ledger(path: &Path) -> Result<migrate::Ledger, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => migrate::from_json(&text)
            .map_err(|err| format!("台账格式不对 {}: {err}", path.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(migrate::Ledger::default()),
        Err(err) => Err(format!("读不到台账 {}: {err}", path.display())),
    }
}

fn save_ledger(path: &Path, ledger: &migrate::Ledger) -> Result<(), String> {
    write_out(path, &format!("{}\n", migrate::to_json(ledger)))
        .map_err(|err| format!("写不了台账 {}: {err}", path.display()))
}

fn print_ledger(plan: &Plan, ledger: &migrate::Ledger) {
    let progress = migrate::next_unit(plan, ledger).progress;
    println!(
        "需要迁移 {} 个：accepted {} / 待接受 {} / 进行中 {} / pending {} / blocked {} / skipped {}",
        progress.total,
        progress.accepted,
        progress.verified,
        progress.in_progress(),
        progress.pending,
        progress.blocked,
        progress.skipped
    );
    if progress.not_needed > 0 {
        println!("另有 {} 个单元无诊断，不需要迁移", progress.not_needed);
    }
    if progress.shared > 0 {
        println!(
            "共享资源上还有 {} 条诊断（`unit:shared`），不自动排队，需单独审批",
            progress.shared
        );
        if let Some(files) = plan.shared.files_summary() {
            println!("  文件：{files}");
        }
        println!(
            "  只含不属于任何单元的文件；被多个单元引用的文件（如 styles/common.wxss）按 `shared: true` 记在各单元作业单里，不在其中"
        );
    }
    for (index, batch) in plan.batches.iter().enumerate() {
        println!("\nbatch {index}");
        for id in batch {
            let Some(unit) = plan.units.iter().find(|unit| &unit.id == id) else {
                continue;
            };
            if unit.diagnostics.total == 0 || unit.kind == "shared" {
                continue;
            }
            let status = ledger.state(&unit.id);
            let mark = match status {
                migrate::Status::Accepted => "✓",
                migrate::Status::Verified => "~",
                migrate::Status::Migrated => "~",
                migrate::Status::Planned => "~",
                migrate::Status::Blocked => "✗",
                migrate::Status::Skipped => "-",
                migrate::Status::Pending => "·",
            };
            let failures = ledger
                .units
                .get(&unit.id)
                .map(|state| state.failures)
                .unwrap_or(0);
            let tail = if failures > 0 {
                format!("  失败 {failures} 次")
            } else {
                String::new()
            };
            println!(
                "  {mark} {:<48} {:<9} {} 条{tail}",
                unit.path,
                status.as_str(),
                unit.diagnostics.total
            );
        }
    }
}

fn print_next(next: &migrate::Next) {
    match &next.unit {
        Some(unit) => {
            println!(
                "下一个：{}（batch {}，{} 条诊断）",
                unit.id, unit.batch, unit.diagnostics.total
            );
            println!("  {}", unit.path);
            if next.failures > 0 {
                println!("  已失败 {} 次", next.failures);
                if let Some(failure) = &next.last_failure {
                    println!(
                        "  上次：{}{}",
                        failure
                            .gate
                            .as_deref()
                            .map(|gate| format!("[{gate}] "))
                            .unwrap_or_default(),
                        failure.reason
                    );
                }
            }
        }
        None => println!("全部完成"),
    }
    let progress = &next.progress;
    println!(
        "进度：accepted {} / 待接受 {} / 进行中 {} / pending {} / blocked {} / skipped {}（共 {}）",
        progress.accepted,
        progress.verified,
        progress.in_progress(),
        progress.pending,
        progress.blocked,
        progress.skipped,
        progress.total
    );
    if progress.shared > 0 {
        println!(
            "共享资源另有 {} 条诊断（`unit:shared`），需单独审批",
            progress.shared
        );
    }
}

// ---------------------------------------------------------------- 共用

fn load(path: &Path) -> Result<Vec<Project>, ExitCode> {
    // 传文件时走单文件视图：不要求工程结构，但跨文件规则也没东西可比。
    let loaded = if path.is_file() {
        Project::single_file(path).map(|project| vec![project])
    } else {
        // 插件工程的宿主与插件是两套页面，各自一个视图。
        Project::load_views(path)
    };
    loaded.map_err(|err| {
        eprintln!("skylint: {err}");
        ExitCode::from(2)
    })
}

fn analyse(path: &Path, ignore: &[String]) -> Result<(Vec<Project>, Report), ExitCode> {
    let projects = load(path)?;
    let registry = Registry::builtin().for_target(Target::Skyline);
    let ignored: HashSet<String> = ignore.iter().cloned().collect();
    let views: Vec<&Project> = projects.iter().collect();
    let diagnostics = registry.run_views(&views, &ignored);
    let report = Report::from_views(&projects, diagnostics);
    Ok((projects, report))
}

fn baseline_ref(path: Option<&Path>, diagnostics: usize) -> Option<migrate::BaselineRef> {
    path.map(|path| migrate::BaselineRef {
        file: Some(path.display().to_string()),
        diagnostics,
    })
}

fn write_out(path: &Path, json: &str) -> std::io::Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, json)
}

fn find_unit<'a>(plan: &'a Plan, needle: &str) -> Option<&'a PlanUnit> {
    plan.units
        .iter()
        .find(|unit| unit.id == needle)
        .or_else(|| {
            let base = needle.strip_suffix(".wxml").unwrap_or(needle);
            plan.units
                .iter()
                .find(|unit| unit.path.strip_suffix(".wxml").unwrap_or(&unit.path) == base)
                .or_else(|| {
                    plan.units
                        .iter()
                        .find(|unit| !unit.path.is_empty() && unit.path.ends_with(needle))
                })
        })
}

fn parse_position(spec: &str) -> Option<(u32, u32)> {
    let (line, column) = spec.split_once(':')?;
    Some((line.trim().parse().ok()?, column.trim().parse().ok()?))
}

// ---------------------------------------------------------------- 人读输出

fn print_plan(plan: &Plan) {
    println!(
        "{} 个单元，{} 个有诊断（共 {} 条）",
        plan.summary.units, plan.summary.with_diagnostics, plan.summary.diagnostics
    );
    if let Some(baseline) = &plan.baseline {
        println!(
            "基线：{}（{} 条）",
            baseline.file.as_deref().unwrap_or("-"),
            baseline.diagnostics
        );
    }
    for (index, batch) in plan.batches.iter().enumerate() {
        println!("\nbatch {index}（{} 个）", batch.len());
        for id in batch {
            let Some(unit) = plan.units.iter().find(|unit| &unit.id == id) else {
                continue;
            };
            let counts = &unit.diagnostics;
            let marker = if counts.total == 0 {
                String::new()
            } else {
                format!(
                    "  {} 条（e{} w{} i{}）",
                    counts.total, counts.error, counts.warning, counts.info
                )
            };
            println!(
                "  {:<52} {:<9} {:<8}{marker}",
                unit.path, unit.kind, unit.renderer
            );
        }
    }
}

fn print_job(job: &migrate::Job) {
    println!("{}", job.unit.id);
    println!(
        "  {}  {}  {}  {} 条诊断",
        job.unit.kind,
        job.unit.renderer,
        job.unit.path,
        job.findings.len()
    );
    for (index, finding) in job.findings.iter().enumerate() {
        println!(
            "\n  {}. [{}] {}  {}:{}:{}",
            index + 1,
            finding.severity,
            finding.rule,
            job.unit.path,
            finding.span.line,
            finding.span.column
        );
        println!("     {}", finding.message);
        if let Some(help) = &finding.help {
            println!("     → {help}");
        }
    }
}

fn print_facts(doc: &migrate::FactsDoc) {
    println!(
        "{}  {}  {}  {}",
        doc.unit.id, doc.unit.kind, doc.unit.renderer, doc.unit.reach
    );
    println!("  {} 个元素", doc.elements.len());
    for element in &doc.elements {
        println!(
            "  {:>5}:{:<4} {:<16} {}",
            element.span.line,
            element.span.column,
            element.tag,
            element.classes.join(".")
        );
    }
}

fn print_element(detail: &migrate::ElementDetail) {
    println!(
        "<{}> {}  @{}:{}",
        detail.tag,
        detail.classes.join("."),
        detail.span.line,
        detail.span.column
    );
    println!(
        "  display={:?} position={:?} out_of_flow={} definite_height={}",
        detail.layout.display,
        detail.layout.position,
        detail.layout.out_of_flow,
        detail.layout.definite_height
    );
    for (property, entries) in &detail.cascade {
        println!("  {property}:");
        for entry in entries {
            println!(
                "    {}  (spec {}{}{})  {}:{}",
                entry.value,
                entry.specificity,
                if entry.important { " !important" } else { "" },
                if entry.inline { " inline" } else { "" },
                entry.stylesheet.as_deref().unwrap_or("-"),
                entry
                    .line
                    .map(|line| line.to_string())
                    .unwrap_or_else(|| "-".to_string())
            );
        }
    }
    if !detail.ancestors.nodes.is_empty() {
        let chain: Vec<&str> = detail
            .ancestors
            .nodes
            .iter()
            .map(|node| node.tag.as_str())
            .collect();
        println!(
            "  ancestors: {}{}",
            chain.join(" < "),
            if detail.ancestors.open { " …" } else { "" }
        );
    }
    for handler in &detail.handlers {
        println!(
            "  {} → {}{}",
            handler.attribute,
            handler.name.as_deref().unwrap_or("<dynamic>"),
            handler
                .function
                .as_ref()
                .map(|function| format!("  {}:{}", function.script, function.line))
                .unwrap_or_default()
        );
    }
}

fn print_verify(report: &migrate::VerifyReport, unknown_rules: &[String]) {
    if report.version_mismatch {
        eprintln!("⚠ 基线与当前 skylint 版本不同：规则集可能变了，新增/修复的数量要人看一眼");
    }
    println!(
        "新增 {} 条（error+warning），新增 info {} 条，修复 {} 条，调档 {} 条（变严 {} / 变松 {}）",
        report.new,
        report.new_info,
        report.fixed,
        report.changed,
        report.changed_up,
        report.changed_down
    );
    if let Some(unit) = &report.unit {
        println!("只看：{unit}");
    }
    for finding in &report.findings {
        let sign = match finding.kind {
            "new" => "+",
            "fixed" => "-",
            // `changed`：方向由当前 severity 与基线里的 previousSeverity 比较得出。
            _ => match finding.previous_severity {
                Some(previous) if finding.severity > previous => "↑",
                Some(_) => "↓",
                None => "~",
            },
        };
        println!(
            "  {sign} [{}] {}  {}:{}:{}",
            finding.severity, finding.rule, finding.path, finding.span.line, finding.span.column
        );
        if let Some(message) = &finding.message {
            println!("     {message}");
        }
    }
    if !unknown_rules.is_empty() {
        println!(
            "基线冻结时还没有 {} 条规则（{}），这些规则的命中可以用 baseline --augment 吸收",
            unknown_rules.len(),
            unknown_rules.join("、")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn baseline_args(extra: &[&str]) -> BaselineArgs {
        let mut argv = vec!["skylint", "migration", "baseline", "."];
        argv.extend_from_slice(extra);
        match Cli::try_parse_from(argv).expect("参数应合法").command {
            Some(Command::Migration(MigrationCommand::Baseline(args))) => args,
            other => panic!("不是 baseline 命令：{other:?}"),
        }
    }

    /// `--augment` 必须显式给出；不给时仍走整份重算。
    #[test]
    fn baseline_augment_is_opt_in() {
        let plain = baseline_args(&["--out", "b.json"]);
        assert!(!plain.augment);
        assert_eq!(plain.out.as_deref(), Some(Path::new("b.json")));

        let augment = baseline_args(&["--out", "b.json", "--augment"]);
        assert!(augment.augment);
    }
}
