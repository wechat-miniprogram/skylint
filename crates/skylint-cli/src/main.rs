use std::collections::HashSet;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, ValueEnum};
use skylint_core::{Project, Registry, Report, suppress};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Format {
    Text,
    Json,
}

#[derive(Debug, Parser)]
#[command(name = "skylint", version, about = "Skyline 小程序迁移检查工具")]
struct Cli {
    /// 项目根目录（含 project.config.json）
    #[arg(default_value = ".", value_name = "PATH")]
    project: PathBuf,

    /// 输出格式
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,

    /// 跳过的规则 id（可重复）
    #[arg(long = "ignore", value_name = "RULE")]
    ignore: Vec<String>,

    /// 只列出全部规则（markdown 表格），不检查工程
    #[arg(long)]
    rules: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let registry = Registry::builtin();

    if cli.rules {
        print!("{}", registry.to_markdown());
        return ExitCode::SUCCESS;
    }

    let project = match Project::load(&cli.project) {
        Ok(project) => project,
        Err(err) => {
            eprintln!("skylint: {err}");
            return ExitCode::from(2);
        }
    };

    let ignored: HashSet<String> = cli.ignore.into_iter().collect();
    let diagnostics = suppress::apply(&project, registry.run(&project, &ignored));
    let report = Report::new(&project, diagnostics);

    match cli.format {
        Format::Text => print!("{}", report.to_text()),
        Format::Json => println!("{}", report.to_json()),
    }

    if report.has_errors() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
