use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, ValueEnum};
use skylint_core::{Project, Report};

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
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let project = match Project::load(&cli.project) {
        Ok(project) => project,
        Err(err) => {
            eprintln!("skylint: {err}");
            return ExitCode::from(2);
        }
    };

    // 规则尚未接入：先只验证工程视图与报告管道。
    let report = Report::new(&project, Vec::new());

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
