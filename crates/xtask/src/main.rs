//! 发版维护脚本：`cargo run -p xtask -- release [--upload]`。
//!
//! 为什么不放在 GitHub Actions：`skylint-core` 依赖三个内网 crate，公开 runner 拉不到，
//! 所以构建与打包在能访问内网的机器上跑，最后一步用 GitHub API 上传产物。
//! 不带 `--upload` 时只构建、打包、算校验和，并打印将要调用的接口——不联网。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use sha2::{Digest, Sha256};

/// 四个发布平台。一台机器通常只能构建其中一部分：按 `--target` 逐个跑、逐个上传即可。
const TARGETS: &[&str] = &[
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
];

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Command_,
}

#[derive(Subcommand)]
enum Command_ {
    /// 构建、打包并（可选）上传
    Release {
        /// Release tag，默认 `v<Cargo.toml 的版本>`
        #[arg(long)]
        tag: Option<String>,
        /// 目标平台三元组，可重复；默认当前主机
        #[arg(long = "target", value_name = "TRIPLE")]
        targets: Vec<String>,
        /// 真的创建 Release 并上传产物（需要 GITHUB_TOKEN）
        #[arg(long)]
        upload: bool,
        /// 仓库 `owner/name`，默认读 `Cargo.toml` 的 repository
        #[arg(long, value_name = "OWNER/NAME")]
        repo: Option<String>,
        /// Release 说明文件；默认自动生成
        #[arg(long, value_name = "PATH")]
        notes: Option<PathBuf>,
    },
}

fn main() -> ExitCode {
    let Cli { command } = Cli::parse();
    let result = match command {
        Command_::Release {
            tag,
            targets,
            upload,
            repo,
            notes,
        } => run_release(tag, targets, upload, repo, notes),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("xtask: {err:#}");
            ExitCode::FAILURE
        }
    }
}

struct Workspace {
    root: PathBuf,
    version: String,
    tag: String,
    repo: String,
}

fn run_release(
    tag: Option<String>,
    targets: Vec<String>,
    upload: bool,
    repo: Option<String>,
    notes: Option<PathBuf>,
) -> Result<()> {
    let root = workspace_root()?;
    let version = workspace_version(&root)?;
    let tag = tag.unwrap_or_else(|| format!("v{version}"));
    if tag.trim_start_matches('v') != version {
        bail!("tag {tag} 与 Cargo.toml 的版本 {version} 不一致");
    }
    let ws = Workspace {
        repo: repo.unwrap_or_else(|| repository(&root)),
        root,
        version,
        tag,
    };

    ensure_clean(&ws.root)?;
    ensure_tag(&ws.root, &ws.tag)?;
    let targets = resolve_targets(targets)?;

    fs::create_dir_all(ws.root.join("dist"))?;
    let mut archives = Vec::new();
    for target in &targets {
        build(&ws, target)?;
        archives.push(package(&ws, target)?);
    }
    let mut assets = archives.clone();
    assets.push(write_checksums(&ws, &archives)?);

    let body = match notes {
        Some(path) => {
            fs::read_to_string(&path).with_context(|| format!("读不到 {}", path.display()))?
        }
        None => default_notes(&ws, &targets),
    };

    if !upload {
        println!("\n干跑：产物已就绪，未上传。加 --upload（需要 GITHUB_TOKEN）执行：");
        println!("  POST https://api.github.com/repos/{}/releases", ws.repo);
        for asset in &assets {
            println!(
                "  POST https://uploads.github.com/repos/{}/releases/<id>/assets?name={asset}",
                ws.repo
            );
        }
        return Ok(());
    }
    upload_release(&ws, &body, &assets)
}

fn resolve_targets(targets: Vec<String>) -> Result<Vec<String>> {
    if targets.is_empty() {
        return Ok(vec![host_target()?]);
    }
    for target in &targets {
        if !TARGETS.contains(&target.as_str()) {
            bail!("{target} 不在发布平台列表里：{}", TARGETS.join(", "));
        }
    }
    Ok(targets)
}

fn build(ws: &Workspace, target: &str) -> Result<()> {
    println!("构建 {target}");
    let status = Command::new("cargo")
        .args([
            "build",
            "--release",
            "--locked",
            "--target",
            target,
            "-p",
            "skylint",
        ])
        .env("CARGO_TARGET_DIR", ws.root.join("target"))
        .current_dir(&ws.root)
        .status()?;
    if !status.success() {
        bail!(
            "{target} 构建失败。交叉编译先装目标：rustup target add {target}；\
             没有对应工具链就在该平台的机器上单独跑这一条 --target"
        );
    }
    Ok(())
}

/// 打成 `dist/skylint-<版本>-<target>.tar.gz`，内含二进制与 README / LICENSE。
fn package(ws: &Workspace, target: &str) -> Result<String> {
    let exe = if target.contains("windows") {
        ".exe"
    } else {
        ""
    };
    let name = format!("skylint-{}-{target}", ws.version);
    let dist = ws.root.join("dist");
    let stage = dist.join(&name);
    fs::create_dir_all(&stage)?;

    let binary = ws
        .root
        .join("target")
        .join(target)
        .join("release")
        .join(format!("skylint{exe}"));
    if !binary.is_file() {
        bail!("构建产物不存在：{}", binary.display());
    }
    fs::copy(&binary, stage.join(format!("skylint{exe}")))
        .with_context(|| format!("复制 {}", binary.display()))?;
    for file in ["README.md", "LICENSE"] {
        let from = ws.root.join(file);
        if from.is_file() {
            fs::copy(&from, stage.join(file))?;
        }
    }

    let archive = format!("{name}.tar.gz");
    let status = Command::new("tar")
        .args(["-czf", &archive, &name])
        .current_dir(&dist)
        .status()
        .context("调用 tar 失败（Windows 10 以下需要自备 tar）")?;
    if !status.success() {
        bail!("tar 打包失败：{archive}");
    }
    fs::remove_dir_all(&stage)?;
    println!("  打包 dist/{archive}");
    Ok(archive)
}

fn write_checksums(ws: &Workspace, archives: &[String]) -> Result<String> {
    let mut lines = String::new();
    for archive in archives {
        let bytes = fs::read(ws.root.join("dist").join(archive))?;
        let digest = Sha256::digest(&bytes);
        lines.push_str(&format!("{digest:x}  {archive}\n"));
    }
    fs::write(ws.root.join("dist").join("SHA256SUMS"), lines)?;
    println!("  校验和 dist/SHA256SUMS");
    Ok("SHA256SUMS".to_string())
}

fn upload_release(ws: &Workspace, body: &str, assets: &[String]) -> Result<()> {
    let token = std::env::var("GITHUB_TOKEN").context("缺少 GITHUB_TOKEN")?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(300)))
        .build()
        .into();
    let release = ensure_release(&agent, &token, ws, body)?;
    for asset in assets {
        let bytes = fs::read(ws.root.join("dist").join(asset))?;
        delete_asset(&agent, &token, ws, release, asset)?;
        let response = agent
            .post(&format!(
                "https://uploads.github.com/repos/{}/releases/{release}/assets?name={asset}",
                ws.repo
            ))
            .header("Authorization", format!("Bearer {token}"))
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("Content-Type", content_type(asset))
            .send(&bytes)
            .with_context(|| format!("上传 {asset} 失败"))?;
        println!("  上传 {asset}（HTTP {}）", response.status());
    }
    println!(
        "\n完成：https://github.com/{}/releases/tag/{}",
        ws.repo, ws.tag
    );
    Ok(())
}

fn content_type(asset: &str) -> &'static str {
    if asset.ends_with(".tar.gz") {
        "application/gzip"
    } else {
        "text/plain"
    }
}

/// 已有同名 Release 就复用——多台机器分别上传各自平台的产物时靠它。
fn ensure_release(agent: &ureq::Agent, token: &str, ws: &Workspace, body: &str) -> Result<u64> {
    let existing = agent
        .get(&format!(
            "https://api.github.com/repos/{}/releases/tags/{}",
            ws.repo, ws.tag
        ))
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .call();
    if let Ok(response) = existing {
        let json: serde_json::Value = response.into_body().read_json()?;
        let id = json["id"].as_u64().context("Release 响应里没有 id")?;
        println!("复用已有 Release {}（id {id}）", ws.tag);
        return Ok(id);
    }
    match existing {
        Err(ureq::Error::StatusCode(404)) => {}
        Err(err) => return Err(err).context("查询 Release 失败（检查 token 权限）"),
        Ok(_) => unreachable!("上面已处理成功响应"),
    }

    let response = agent
        .post(&format!(
            "https://api.github.com/repos/{}/releases",
            ws.repo
        ))
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send_json(serde_json::json!({
            "tag_name": ws.tag,
            "name": ws.tag,
            "body": body,
            "draft": false,
            "prerelease": ws.tag.contains('-'),
        }))
        .context("创建 Release 失败")?;
    let json: serde_json::Value = response.into_body().read_json()?;
    let id = json["id"].as_u64().context("Release 响应里没有 id")?;
    println!("创建 Release {}（id {id}）", ws.tag);
    Ok(id)
}

/// 重跑时同名产物会 422：先删掉，保证线上就是这次构建的结果。
fn delete_asset(
    agent: &ureq::Agent,
    token: &str,
    ws: &Workspace,
    release: u64,
    name: &str,
) -> Result<()> {
    let assets: Vec<serde_json::Value> = agent
        .get(&format!(
            "https://api.github.com/repos/{}/releases/{release}/assets",
            ws.repo
        ))
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .call()?
        .into_body()
        .read_json()?;
    let Some(id) = assets
        .iter()
        .find(|asset| asset["name"].as_str() == Some(name))
        .and_then(|asset| asset["id"].as_u64())
    else {
        return Ok(());
    };
    agent
        .delete(&format!(
            "https://api.github.com/repos/{}/releases/assets/{id}",
            ws.repo
        ))
        .header("Authorization", format!("Bearer {token}"))
        .call()?;
    Ok(())
}

fn default_notes(ws: &Workspace, targets: &[String]) -> String {
    let mut out = format!("skylint {}\n\n## 产物\n\n", ws.version);
    for target in targets {
        out.push_str(&format!("- `skylint-{}-{target}.tar.gz`\n", ws.version));
    }
    out.push_str("\n校验和见 `SHA256SUMS`，规则清单见 `docs/RULES.md`。\n");
    out
}

fn ensure_clean(root: &Path) -> Result<()> {
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(root)
        .output()?;
    let dirty = String::from_utf8_lossy(&output.stdout);
    if !dirty.trim().is_empty() {
        bail!("工作区不干净，发版必须从已提交的状态构建：\n{dirty}");
    }
    Ok(())
}

fn ensure_tag(root: &Path, tag: &str) -> Result<()> {
    let status = Command::new("git")
        .args(["rev-parse", "--verify", "--quiet"])
        .arg(format!("refs/tags/{tag}"))
        .current_dir(root)
        .status()?;
    if !status.success() {
        bail!("tag {tag} 不存在，先 git tag {tag} && git push origin {tag}");
    }
    Ok(())
}

fn workspace_root() -> Result<PathBuf> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .context("定位 workspace 根目录失败")
}

fn workspace_version(root: &Path) -> Result<String> {
    let manifest = fs::read_to_string(root.join("Cargo.toml"))?;
    let mut in_package = false;
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[workspace.package]";
            continue;
        }
        if !in_package {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("version")
            && let Some(value) = rest.trim_start().strip_prefix('=')
        {
            return Ok(value.trim().trim_matches('"').to_string());
        }
    }
    bail!("Cargo.toml 里没找到 [workspace.package] version")
}

fn repository(root: &Path) -> String {
    let manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap_or_default();
    manifest
        .lines()
        .find_map(|line| line.trim().strip_prefix("repository = "))
        .map(|value| {
            value
                .trim()
                .trim_matches('"')
                .trim_end_matches(".git")
                .trim_start_matches("https://github.com/")
                .to_string()
        })
        .unwrap_or_else(|| "wechat-miniprogram/skylint".to_string())
}

fn host_target() -> Result<String> {
    let output = Command::new("rustc")
        .arg("-vV")
        .output()
        .context("调用 rustc 失败")?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("host: ").map(str::to_string))
        .context("rustc -vV 输出里没有 host")
}
