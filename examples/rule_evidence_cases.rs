//! 把规则的 [`EvidenceCase`] 写成互不相干的完整小程序工程。
//!
//! 用法（由 `tools/evidence/run.sh` 调用，也可以手工跑）：
//!
//! ```text
//! cargo run --example rule_evidence_cases -- --out .tmp/evidence-run
//! ```
//!
//! 产物布局：`<out>/cases/<rule>/<case>/` 是一个独立工程，`<out>/evidence.json` 是清单
//! （含 rule、name、工程相对路径、run、expect）。每条用例一个工程，因为证据用例的
//! 变量常常就是 app.json / 页面 json 里的一个键，「同一工程多页」会把缺键变成全局缺键。
//!
//! 与视觉生成器分开写：视觉生成器强制 `renderer` / `componentFramework` /
//! `lazyCodeLoading` 并拒绝 `rendererOptions`，而证据用例恰恰要逐条控制这些键。

use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use skylint::Registry;
use skylint::rules::{EvidenceExpect, EvidenceRun, VisualFile};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut out: Option<PathBuf> = None;
    let mut skips: Vec<String> = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => out = args.next().map(PathBuf::from),
            "--skip" => {
                if let Some(value) = args.next() {
                    skips.push(value);
                }
            }
            other => return Err(format!("未知参数：{other}").into()),
        }
    }
    let out = out.ok_or("必须指定 --out <目录>")?;

    let cases_dir = out.join("cases");
    if cases_dir.exists() {
        fs::remove_dir_all(&cases_dir)?;
    }
    fs::create_dir_all(&cases_dir)?;

    let mut manifest = Vec::new();
    let mut names = HashSet::new();
    for (meta, cases) in Registry::builtin().evidence_cases() {
        if skips.iter().any(|skip| meta.id.contains(skip.as_str())) {
            continue;
        }
        let rule_slug = slugify(meta.id);
        for case in cases {
            validate_name(meta.id, case.name)?;
            if !names.insert((meta.id, case.name)) {
                return Err(format!("规则 `{}` 的用例名重复：{}", meta.id, case.name).into());
            }
            let dir = cases_dir.join(&rule_slug).join(case.name);
            fs::create_dir_all(&dir)?;

            let config = case
                .project_config
                .unwrap_or(r#"{"miniprogramRoot":"","appid":"touristappid"}"#);
            let config_json: serde_json::Value = serde_json::from_str(config).map_err(|error| {
                format!(
                    "规则 `{}` 的 {} project_config 不是 JSON：{error}",
                    meta.id, case.name
                )
            })?;
            if !config_json.is_object() {
                return Err(format!(
                    "规则 `{}` 的 {} project_config 必须是 JSON 对象",
                    meta.id, case.name
                )
                .into());
            }
            write(&dir.join("project.config.json"), config)?;

            for file in case.files {
                validate_path(meta.id, file.path)?;
                write(&dir.join(file.path), file.content)?;
            }

            if let Some(page) = case.run.page() {
                let page_json = format!("{page}.json");
                if !case.files.iter().any(|file| file.path == page_json) {
                    return Err(format!(
                        "规则 `{}` 的 {} 要加载 {page_json}，但 files 里没有",
                        meta.id, case.name
                    )
                    .into());
                }
            }

            manifest.push(serde_json::json!({
                "rule": meta.id,
                "name": case.name,
                "dir": format!("cases/{rule_slug}/{}", case.name),
                "run": run_json(&case.run),
                "expect": expect_json(&case.expect),
                "note": case.note,
                "renderer": renderer_of(case.files, &case.run),
            }));
        }
    }

    write(
        &out.join("evidence.json"),
        &format!("{}\n", serde_json::to_string_pretty(&manifest)?),
    )?;
    println!(
        "wrote {} evidence case(s) → {}",
        manifest.len(),
        out.display()
    );
    Ok(())
}

fn write(path: &Path, content: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)
}

/// 用例名会拼进目录路径，只允许安全字符。
fn validate_name(rule_id: &str, name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let valid = !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-');
    if valid {
        Ok(())
    } else {
        Err(format!("规则 `{rule_id}` 的用例名必须是 [a-z0-9-]：{name:?}").into())
    }
}

/// 工程内文件路径必须是不含 `..` 的相对路径：绝对路径会写出用例目录。
fn validate_path(rule_id: &str, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let safe = !path.is_empty()
        && !Path::new(path).is_absolute()
        && Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_)));
    if safe {
        Ok(())
    } else {
        Err(format!("规则 `{rule_id}` 的工程文件路径必须是不含 `..` 的相对路径：{path:?}").into())
    }
}

fn run_json(run: &EvidenceRun) -> serde_json::Value {
    match run {
        EvidenceRun::Build => serde_json::json!({"kind": "build"}),
        EvidenceRun::Serve { page } => serde_json::json!({"kind": "serve", "page": page}),
    }
}

fn expect_json(expect: &EvidenceExpect) -> serde_json::Value {
    match expect {
        EvidenceExpect::BuildOk => serde_json::json!({"kind": "build-ok"}),
        EvidenceExpect::BuildFails { stderr_contains } => serde_json::json!({
            "kind": "build-fails",
            "stderr_contains": stderr_contains,
        }),
        EvidenceExpect::BuildLog { must, must_not } => serde_json::json!({
            "kind": "build-log",
            "must": must,
            "must_not": must_not,
        }),
        EvidenceExpect::Artifact {
            path,
            must,
            must_not,
        } => serde_json::json!({
            "kind": "artifact",
            "path": path,
            "must": must,
            "must_not": must_not,
        }),
        EvidenceExpect::Console { must, must_not } => serde_json::json!({
            "kind": "console",
            "must": must,
            "must_not": must_not,
        }),
        EvidenceExpect::Runtime { expr, equals } => {
            serde_json::json!({"kind": "runtime", "expr": expr, "equals": equals})
        }
    }
}

/// serve 侧是否要 `--browser`：页面自己写 `renderer: "webview"` 时是 WebView，
/// 其余（含继承 app.json）由 serve 托管 Electron。读不到就留空，由 run.sh 按 skyline 处理。
fn renderer_of(files: &[VisualFile], run: &EvidenceRun) -> Option<String> {
    let page = run.page()?;
    let page_json = files
        .iter()
        .find(|file| file.path == format!("{page}.json"))?;
    let value: serde_json::Value = serde_json::from_str(page_json.content).ok()?;
    match value.get("renderer").and_then(serde_json::Value::as_str) {
        Some(renderer) => Some(renderer.to_string()),
        None => {
            let app = files.iter().find(|file| file.path == "app.json")?;
            let value: serde_json::Value = serde_json::from_str(app.content).ok()?;
            value
                .get("renderer")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        }
    }
}

/// 规则 id → 目录名：只保留 `[a-z0-9-]`，保证跨平台可用。
fn slugify(id: &str) -> String {
    let mut out = String::with_capacity(id.len());
    let mut pending_dash = false;
    for ch in id.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            pending_dash = false;
        } else if !pending_dash && !out.is_empty() {
            out.push('-');
            pending_dash = true;
        }
    }
    out.trim_end_matches('-').to_string()
}
