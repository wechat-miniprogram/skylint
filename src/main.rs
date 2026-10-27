use std::collections::HashSet;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, ValueEnum};
use skylint::report::Style;
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
    /// 项目根目录（含 project.config.json），或单个 .wxml / .wxss / .js 文件
    #[arg(default_value = ".", value_name = "PATH")]
    path: PathBuf,

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

fn main() -> ExitCode {
    let cli = Cli::parse();
    let registry = Registry::builtin().for_target(cli.target.into());

    // 传文件时走单文件视图：不要求工程结构，但跨文件规则也没东西可比。
    let loaded = if cli.path.is_file() {
        Project::single_file(&cli.path).map(|project| vec![project])
    } else {
        // 插件工程的宿主与插件是两套页面，各自一个视图。
        Project::load_views(&cli.path)
    };
    let projects = match loaded {
        Ok(projects) => projects,
        Err(err) => {
            eprintln!("skylint: {err}");
            return ExitCode::from(2);
        }
    };

    let ignored: HashSet<String> = cli.ignore.into_iter().collect();
    let views: Vec<&Project> = projects.iter().collect();
    let diagnostics = registry.run_views(&views, &ignored);
    let report = Report::from_views(&projects, diagnostics);

    if cli.json {
        println!("{}", report.to_json());
    } else {
        print!(
            "{}",
            report.to_text(
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
