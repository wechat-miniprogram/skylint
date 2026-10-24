use std::collections::HashSet;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use skylint_core::report::Style;
use skylint_core::{Project, Registry, Report, suppress};

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
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let registry = Registry::builtin();

    // 传文件时走单文件视图：不要求工程结构，但跨文件规则也没东西可比。
    let loaded = if cli.path.is_file() {
        Project::single_file(&cli.path)
    } else {
        Project::load(&cli.path)
    };
    let project = match loaded {
        Ok(project) => project,
        Err(err) => {
            eprintln!("skylint: {err}");
            return ExitCode::from(2);
        }
    };

    let ignored: HashSet<String> = cli.ignore.into_iter().collect();
    let diagnostics = suppress::apply(&project, registry.run(&project, &ignored));
    let report = Report::new(&project, diagnostics);

    if cli.json {
        println!("{}", report.to_json());
    } else {
        print!(
            "{}",
            report.to_text(
                |path| {
                    // 多数路径相对 miniprogramRoot；project.config.json 在项目根，补一次回落。
                    std::fs::read_to_string(project.abs(path))
                        .or_else(|_| std::fs::read_to_string(project.root.join(path)))
                        .ok()
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
