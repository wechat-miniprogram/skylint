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

    /// 跳过的规则 id（可重复）
    #[arg(long = "ignore", value_name = "RULE")]
    ignore: Vec<String>,

    /// 检查方向
    #[arg(long, value_enum, default_value = "skyline")]
    target: TargetArg,
}

#[derive(Debug, Subcommand)]
enum Command {
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
}

#[derive(Debug, Args)]
struct PlanArgs {
    /// 项目根目录
    #[arg(default_value = ".", value_name = "PATH")]
    path: PathBuf,
    /// 输出 JSON（机器可读）
    #[arg(long)]
    json: bool,
    /// 写入文件（等价于 `--json > FILE`）
    #[arg(long, value_name = "FILE")]
    out: Option<PathBuf>,
    /// 基线报告，用于对账
    #[arg(long, value_name = "FILE")]
    baseline: Option<PathBuf>,
    /// 跳过的规则 id（可重复）
    #[arg(long = "ignore", value_name = "RULE")]
    ignore: Vec<String>,
}

#[derive(Debug, Args)]
struct JobArgs {
    /// 项目根目录
    #[arg(default_value = ".", value_name = "PATH")]
    path: PathBuf,
    /// 单元 id（`unit:components/btn/btn`）或 WXML 路径
    #[arg(long, value_name = "UNIT")]
    unit: String,
    /// 输出 JSON（机器可读）
    #[arg(long)]
    json: bool,
    /// 写入文件
    #[arg(long, value_name = "FILE")]
    out: Option<PathBuf>,
    /// 基线报告
    #[arg(long, value_name = "FILE")]
    baseline: Option<PathBuf>,
    /// 跳过的规则 id（可重复）
    #[arg(long = "ignore", value_name = "RULE")]
    ignore: Vec<String>,
}

#[derive(Debug, Args)]
struct BaselineArgs {
    /// 项目根目录
    #[arg(default_value = ".", value_name = "PATH")]
    path: PathBuf,
    /// 写入位置
    #[arg(long, value_name = "FILE", default_value = ".skylint/baseline.json")]
    out: PathBuf,
    /// 输出 JSON（同时写到 `--out`）
    #[arg(long)]
    json: bool,
    /// 跳过的规则 id（可重复）
    #[arg(long = "ignore", value_name = "RULE")]
    ignore: Vec<String>,
}

#[derive(Debug, Args)]
struct VerifyArgs {
    /// 项目根目录
    #[arg(default_value = ".", value_name = "PATH")]
    path: PathBuf,
    /// 基线文件（`skylint baseline` 生成）
    #[arg(long, value_name = "FILE")]
    baseline: PathBuf,
    /// 只看这个单元（id 或 WXML 路径）
    #[arg(long, value_name = "UNIT")]
    unit: Option<String>,
    /// 输出 JSON（机器可读）
    #[arg(long)]
    json: bool,
    /// 跳过的规则 id（可重复）
    #[arg(long = "ignore", value_name = "RULE")]
    ignore: Vec<String>,
}

#[derive(Debug, Args)]
struct FactsArgs {
    /// 项目根目录
    #[arg(value_name = "PATH")]
    path: PathBuf,
    /// 单元 WXML（项目相对，可带插件前缀）
    #[arg(value_name = "WXML")]
    unit: String,
    /// 元素位置 `行:列`（1-based，按包含关系取最深元素）
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
        Some(Command::Plan(args)) => run_plan(args),
        Some(Command::Job(args)) => run_job(args),
        Some(Command::Facts(args)) => run_facts(args),
        Some(Command::Baseline(args)) => run_baseline(args),
        Some(Command::Verify(args)) => run_verify(args),
        None => run_check(
            cli.path.unwrap_or_else(|| PathBuf::from(".")),
            cli.json,
            cli.ignore,
            cli.target.into(),
        ),
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
    let (projects, report) = match analyse(&args.path, &args.ignore) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let baseline = baseline_ref(args.baseline.as_deref(), report.diagnostics.len());
    let plan = migrate::plan(&projects, &report, baseline);

    if args.json || args.out.is_some() {
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
    let (projects, report) = match analyse(&args.path, &args.ignore) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let baseline = baseline_ref(args.baseline.as_deref(), report.diagnostics.len());
    let plan = migrate::plan(&projects, &report, baseline.clone());

    let Some(unit) = find_unit(&plan, &args.unit) else {
        eprintln!(
            "skylint: 找不到单元 `{}`（先跑 `skylint plan` 看清单）",
            args.unit
        );
        return ExitCode::from(1);
    };
    let Some((project, rel)) = migrate::find_view(&projects, &unit.path) else {
        eprintln!("skylint: 单元 `{}` 不属于任何视图", unit.id);
        return ExitCode::from(1);
    };

    let diagnostics = migrate::diagnostics_for(&plan, unit, &report);
    let job = migrate::job(project, rel, unit, &diagnostics, baseline);

    if args.json || args.out.is_some() {
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
    let (projects, report) = match analyse(&args.path, &args.ignore) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let plan = migrate::plan(&projects, &report, None);
    let identified = migrate::identify(&projects, &plan, &report);
    let baseline = migrate::baseline(&identified);
    let json = migrate::to_json(&baseline);

    if let Err(err) = write_out(&args.out, &json) {
        eprintln!("skylint: {err}");
        return ExitCode::from(2);
    }
    if args.json {
        println!("{json}");
    } else {
        eprintln!(
            "wrote {}（{} 条：e{} w{} i{}）",
            args.out.display(),
            baseline.summary.total,
            baseline.summary.error,
            baseline.summary.warning,
            baseline.summary.info
        );
    }
    ExitCode::SUCCESS
}

fn run_verify(args: VerifyArgs) -> ExitCode {
    let text = match std::fs::read_to_string(&args.baseline) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("skylint: 读不到基线 {}: {err}", args.baseline.display());
            return ExitCode::from(2);
        }
    };
    let baseline: migrate::Baseline = match migrate::from_json(&text) {
        Ok(baseline) => baseline,
        Err(err) => {
            eprintln!("skylint: 基线格式不对 {}: {err}", args.baseline.display());
            return ExitCode::from(2);
        }
    };

    let (projects, report) = match analyse(&args.path, &args.ignore) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let plan = migrate::plan(&projects, &report, None);

    // `--unit` 既可以是 id，也可以是 WXML 路径。
    let unit = match args.unit.as_deref() {
        Some(needle) => match find_unit(&plan, needle) {
            Some(unit) => Some(unit.id.clone()),
            None => {
                eprintln!("skylint: 找不到单元 `{needle}`（先跑 `skylint plan` 看清单）");
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

    if args.json {
        println!("{}", migrate::to_json(&result));
    } else {
        print_verify(&result);
    }

    if result.blocks() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
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
        // 摘要留给 `verify`（M2）算，这里不假装有。
        sha256: None,
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

fn print_verify(report: &migrate::VerifyReport) {
    if report.version_mismatch {
        eprintln!("⚠ 基线与当前 skylint 版本不同：规则集可能变了，新增/修复的数量要人看一眼");
    }
    println!(
        "新增 {} 条（error+warning），新增 info {} 条，修复 {} 条，调档 {} 条",
        report.new, report.new_info, report.fixed, report.changed
    );
    if let Some(unit) = &report.unit {
        println!("只看：{unit}");
    }
    for finding in &report.findings {
        let sign = match finding.kind {
            "new" => "+",
            "fixed" => "-",
            _ => "~",
        };
        println!(
            "  {sign} [{}] {}  {}:{}:{}",
            finding.severity, finding.rule, finding.path, finding.span.line, finding.span.column
        );
        if let Some(message) = &finding.message {
            println!("     {message}");
        }
    }
}
