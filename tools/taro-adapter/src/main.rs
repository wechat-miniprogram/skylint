//! `skylint-taro`：把 Taro 构建产物里的 WXML 换回 TSX 还原出来的结构，
//! 再交给 skylint 检查。
//!
//! 设计约束：核心 crate 一行不改。Taro 的语义差异（运行时模板、组件默认值、
//! 属性名映射）全部留在这一层；只有发现「原生 WXML 上也成立、但核心没覆盖」
//! 的通用语义缺口时，才回头改核心。

use std::collections::HashSet;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Parser, ValueEnum};
use skylint::render::{self, Style};
use skylint::rules::Target;
use skylint::{Project, Registry, Report};

use skylint_taro::remap;
use skylint_taro::synth::{self, Layout, Synth};

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
#[command(
    name = "skylint-taro",
    about = "Taro 适配器：还原 TSX 结构后按 Skyline 规则检查",
    long_about = None
)]
struct Cli {
    /// Taro 工程根目录（含 project.config.json）
    #[arg(default_value = ".", value_name = "PATH")]
    path: PathBuf,

    /// 源码根目录（默认自动识别 `src`，可用 `--src` 覆盖）
    #[arg(long, value_name = "DIR")]
    src: Option<PathBuf>,

    /// 构建产物目录（默认从 `project.config.json` / `dist` 自动识别）
    #[arg(long, value_name = "DIR")]
    dist: Option<PathBuf>,

    /// 输出 JSON（机器可读）
    #[arg(long)]
    json: bool,

    /// 跳过的规则 id（可重复）
    #[arg(long = "ignore", value_name = "RULE")]
    ignore: Vec<String>,

    /// 检查方向
    #[arg(long, value_enum, default_value = "skyline")]
    target: TargetArg,

    /// 一并报告死选择器（默认跳过，见 [`DEFAULT_IGNORES`]）
    #[arg(long)]
    dead_selector: bool,

    /// 保留合成工程目录，便于排查还原结果
    #[arg(long)]
    keep: bool,
}

/// 依赖「完整组件树」才能判定的规则。
///
/// 第三方组件（node_modules 里的 taro-ui / echarts 之类）的 JSX 读不到，
/// 它们运行时才会出现的类名与标签在合成树里不存在，`dead-selector`
/// 会把真命中报成死代码（jiezhang 的 `.setting-chart-page canvas` 就是 echarts 画的）。
const DEFAULT_IGNORES: &[&str] = &["skyline/dead-selector"];

fn main() -> ExitCode {
    let cli = Cli::parse();

    let layout = match Layout::detect(&cli.path, cli.src.clone(), cli.dist.clone()) {
        Ok(layout) => layout,
        Err(error) => {
            eprintln!("skylint-taro: {error}");
            return ExitCode::from(2);
        }
    };
    if !is_taro(&layout) {
        eprintln!(
            "skylint-taro: 未在 {} 找到 Taro 产物（base.wxml / taro 依赖），仍按 Taro 规则尝试",
            layout.dist_root.display()
        );
    }

    let temp = temp_dir(&layout.project_root);
    let synth = match synth::build(&layout, &temp) {
        Ok(synth) => synth,
        Err(error) => {
            eprintln!("skylint-taro: {error}");
            return ExitCode::from(2);
        }
    };
    if synth.synthesized == 0 {
        print_notices(&synth);
        eprintln!(
            "skylint-taro: 没有还原出任何页面结构（源码根：{}）。检查 `--src`，或该工程不是 React 版 Taro。",
            layout.src_root.display()
        );
        cleanup(&temp, cli.keep);
        return ExitCode::from(2);
    }

    let report = run(&synth, &cli);
    print_notices(&synth);

    if cli.json {
        println!("{}", report.to_json());
    } else {
        let root = layout.project_root.clone();
        print!(
            "{}",
            render::text(
                &report,
                |path| std::fs::read_to_string(root.join(path)).ok(),
                Style {
                    colored: std::io::stdout().is_terminal(),
                    width: std::env::var("COLUMNS")
                        .ok()
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(100),
                },
            )
        );
    }

    cleanup(&temp, cli.keep);
    if report.has_errors() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn run(synth: &Synth, cli: &Cli) -> Report {
    let project = match Project::load(&synth.dir) {
        Ok(project) => project,
        Err(error) => {
            eprintln!("skylint-taro: 合成工程加载失败：{error}");
            std::process::exit(2);
        }
    };
    let registry = Registry::builtin().for_target(cli.target.into());
    let mut ignored: HashSet<String> = cli.ignore.iter().cloned().collect();
    if !cli.dead_selector {
        ignored.extend(DEFAULT_IGNORES.iter().map(|rule| rule.to_string()));
        eprintln!("skylint-taro: 跳过 skyline/dead-selector（类名可达性需要完整组件树；--dead-selector 打开）");
    }
    let diagnostics = registry.run_views(&[&project], &ignored);
    let diagnostics = remap::remap(diagnostics, synth);
    Report::from_views(&[project], diagnostics)
}

fn print_notices(synth: &Synth) {
    eprintln!(
        "skylint-taro: 还原 {} 个单元，{} 个 WXML 原样使用构建产物，{} 个无法还原",
        synth.synthesized, synth.copied_wxml, synth.failed
    );
    eprintln!(
        "skylint-taro: 合成工程不含脚本单元（.js/.ts），依赖脚本的规则（unsupported-api / route-type / worklet 系列）不会产生诊断"
    );
    for notice in &synth.notices {
        eprintln!("  · {notice}");
    }
}

fn is_taro(layout: &Layout) -> bool {
    let base = layout.dist_root.join("base.wxml");
    if let Ok(source) = std::fs::read_to_string(base)
        && source.contains("taro_tmpl")
    {
        return true;
    }
    std::fs::read_to_string(layout.project_root.join("package.json"))
        .map(|source| source.contains("@tarojs/"))
        .unwrap_or(false)
}

fn temp_dir(project_root: &std::path::Path) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let name = format!(
        "skylint-taro-{}-{}",
        project_root
            .file_name()
            .map(|name| name.to_string_lossy().replace(|c: char| !c.is_ascii_alphanumeric(), "-"))
            .unwrap_or_else(|| "project".to_string()),
        stamp
    );
    std::env::temp_dir().join(name)
}

fn cleanup(dir: &std::path::Path, keep: bool) {
    if keep {
        eprintln!("skylint-taro: 合成工程保留在 {}", dir.display());
        return;
    }
    let _ = std::fs::remove_dir_all(dir);
}
