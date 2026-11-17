//! 合成工程：AS_IS 复制构建产物里与框架无关的文件，只把 WXML 换成
//! 「TSX 还原出来的结构」。
//!
//! 为什么必须复制而不是指一个虚拟文件系统：skylint 的 `Project` 直接操作磁盘
//! （`Project::load` → `Loaded::read`），适配器要保持零侵入，就只能给它一个
//! 真实目录。复制的部分都是逐字节相同的：样式、JSON 配置、第三方组件 WXML。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::emit::{self, FileMap};
use crate::jsx;

/// Taro 运行时的渲染入口，不是任何页面/组件的结构。
const INTERNAL_WXML: &[&str] = &["base.wxml", "comp.wxml"];
const SOURCE_EXTS: &[&str] = &["tsx", "jsx", "ts", "js"];

pub struct Synth {
    pub dir: PathBuf,
    /// 合成文件（相对合成根）→ 位置映射。
    pub maps: BTreeMap<String, FileMap>,
    /// 原样复制的文件（相对合成根）→ 项目相对路径。
    pub copied: BTreeMap<String, String>,
    pub notices: Vec<String>,
    pub synthesized: usize,
    pub copied_wxml: usize,
    pub failed: usize,
}

pub struct Layout {
    pub project_root: PathBuf,
    pub src_root: PathBuf,
    /// 构建产物绝对路径。
    pub dist_root: PathBuf,
    /// 构建产物相对项目根的路径（posix，末尾无 `/`，可能为空串）。
    pub dist_rel: String,
}

impl Layout {
    /// 依次尝试：`project.config.json` 的 `miniprogramRoot` → `dist` →
    /// `dist/build/<platform>` → `dist/dev/<platform>`。
    pub fn detect(
        project_root: &Path,
        src_override: Option<PathBuf>,
        dist_override: Option<PathBuf>,
    ) -> Result<Self> {
        let project_root = project_root
            .canonicalize()
            .with_context(|| format!("项目根不存在：{}", project_root.display()))?;

        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Ok(source) = std::fs::read_to_string(project_root.join("project.config.json"))
            && let Ok(json) = serde_json::from_str::<serde_json::Value>(&source)
            && let Some(root) = json.get("miniprogramRoot").and_then(|v| v.as_str())
        {
            candidates.push(project_root.join(root));
        }
        for fallback in [
            "dist",
            "dist/build/mp-weixin",
            "dist/dev/mp-weixin",
            "dist/production/weapp",
            "dist/dev/weapp",
        ] {
            candidates.push(project_root.join(fallback));
        }
        if let Some(dist) = dist_override {
            candidates.insert(0, project_root.join(dist));
        }

        let mut dist_root = candidates
            .into_iter()
            .find(|path| {
                path.join("app.json").is_file() && path.join("base.wxml").is_file()
            });
        if dist_root.is_none() {
            // 其余产物布局（`dist/production/weapp` 之类）：在 dist 下找带 base.wxml 的目录。
            dist_root = scan_for_output(&project_root.join("dist"));
        }

        let dist_root = dist_root
            .with_context(|| {
                format!(
                    "找不到构建产物（app.json）：请先执行 `taro build --type weapp`。\n\
                     已尝试：{}",
                    project_root.display()
                )
            })?;
        let dist_root = dist_root.canonicalize().unwrap_or(dist_root);
        let dist_rel = dist_root
            .strip_prefix(&project_root)
            .map(Path::to_string_lossy)
            .unwrap_or_default()
            .replace('\\', "/");

        let src_root = match src_override {
            Some(path) => project_root.join(path),
            None if project_root.join("src").is_dir() => project_root.join("src"),
            None => project_root.clone(),
        };

        Ok(Self {
            project_root,
            src_root,
            dist_root,
            dist_rel,
        })
    }

    /// 相对项目根的写法，诊断里对外只用这种路径。
    fn rel(&self, path: &Path) -> String {
        path.strip_prefix(&self.project_root)
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| path.to_string_lossy().to_string())
    }

    fn project_rel(&self, dist_rel_path: &str) -> String {
        if self.dist_rel.is_empty() {
            dist_rel_path.to_string()
        } else {
            format!("{}/{}", self.dist_rel, dist_rel_path)
        }
    }
}

/// 在 `dist` 下找 Taro 产物目录（含 `app.json` 与 `base.wxml`）。
fn scan_for_output(root: &Path) -> Option<PathBuf> {
    if !root.is_dir() {
        return None;
    }
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut found = Vec::new();
    while let Some((dir, depth)) = stack.pop() {
        if depth > 3 {
            continue;
        }
        if dir.join("app.json").is_file() && dir.join("base.wxml").is_file() {
            found.push(dir);
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && !path.ends_with("node_modules") {
                stack.push((path, depth + 1));
            }
        }
    }
    found.sort();
    found.into_iter().next()
}

/// 构建合成工程。`temp_dir` 由调用方决定，便于 `--keep` 时原样保留。
pub fn build(layout: &Layout, temp_dir: &Path) -> Result<Synth> {
    if temp_dir.exists() {
        std::fs::remove_dir_all(temp_dir)
            .with_context(|| format!("清理旧合成目录失败：{}", temp_dir.display()))?;
    }
    std::fs::create_dir_all(temp_dir)?;

    let mut synth = Synth {
        dir: temp_dir.to_path_buf(),
        maps: BTreeMap::new(),
        copied: BTreeMap::new(),
        notices: Vec::new(),
        synthesized: 0,
        copied_wxml: 0,
        failed: 0,
    };

    let mut files = Vec::new();
    collect_files(&layout.dist_root, &layout.dist_root, &mut files)?;
    files.sort();

    let mut deferred = Vec::new();
    for rel in &files {
        let name = rel.rsplit('/').next().unwrap_or(rel);
        if name == "project.config.json" {
            continue;
        }
        match ext(name) {
            "json" | "wxss" | "wxs" => {
                copy(layout, &mut synth, rel, None)?;
            }
            "wxml" if INTERNAL_WXML.contains(&name) => {}
            "wxml" => deferred.push(rel.clone()),
            _ => {}
        }
    }

    for rel in deferred {
        synth_wxml(layout, &mut synth, &rel)?;
    }

    write_project_config(layout, &mut synth)?;
    Ok(synth)
}

fn synth_wxml(layout: &Layout, synth: &mut Synth, rel: &str) -> Result<()> {
    // Taro 运行时的页面 WXML 只有 `<import base.wxml>` + `<template taro_tmpl>`；
    // 不含它的 WXML（第三方 / 原生 `usingComponents` 组件）本身就是静态结构，原样可用。
    if !is_runtime_template(&layout.dist_root.join(rel)) {
        copy(layout, synth, rel, None)?;
        synth.copied_wxml += 1;
        return Ok(());
    }

    let source = find_source(layout, rel);
    let Some(source) = source else {
        // 第三方组件（miniprogram_npm）与原生组件的 WXML 是手写的，原样可用。
        copy(layout, synth, rel, None)?;
        synth.copied_wxml += 1;
        return Ok(());
    };

    let source_rel = layout.rel(&source);
    let code = std::fs::read_to_string(&source)
        .with_context(|| format!("读取源文件失败：{}", source.display()))?;

    if source.extension().is_some_and(|ext| ext == "vue") {
        synth.notices.push(format!(
            "{source_rel} 是 Vue 单文件组件，Taro Vue 版本暂不支持还原，该单元的结构类规则不会执行"
        ));
        synth.failed += 1;
        copy(layout, synth, rel, None)?;
        return Ok(());
    }

    let declared = declared_components(&layout.dist_root, rel);
    match jsx::parse_page(&layout.src_root, &source, &code, &declared) {
        Ok(parsed) if !parsed.roots.is_empty() => {
            synth.notices.extend(parsed.notices);
            let mut map = emit::emit(&parsed.roots, parsed.sources);
            for source in &mut map.sources {
                source.path = layout.rel(Path::new(&source.path));
            }
            write(synth, rel, &map.text)?;
            synth.maps.insert(rel.to_string(), map);
            synth.synthesized += 1;
        }
        Ok(parsed) => {
            synth.notices.extend(parsed.notices);
            synth.notices.push(format!(
                "{source_rel} 没有找到 JSX 元素（可能全部由运行时拼装），该单元的结构类规则不会执行"
            ));
            synth.failed += 1;
            write(synth, rel, "")?;
        }
        Err(error) => {
            synth.notices.push(format!("{source_rel} 解析失败：{error}"));
            synth.failed += 1;
            write(synth, rel, "")?;
        }
    }
    Ok(())
}

fn is_runtime_template(path: &Path) -> bool {
    let Ok(source) = std::fs::read_to_string(path) else {
        return false;
    };
    source.contains("taro_tmpl") || source.contains("base.wxml")
}

/// 该单元的 `usingComponents` 标签名（含 app 级），小写。
///
/// 只有这些组件在 Taro 里是真正的小程序自定义组件；其余 import 进来的
/// React 组件由运行时内联渲染，结构必须从源码展开。
fn declared_components(dist_root: &Path, wxml_rel: &str) -> std::collections::HashSet<String> {
    let mut names = std::collections::HashSet::new();
    let mut sources = vec![dist_root.join("app.json")];
    if let Some(stem) = wxml_rel.strip_suffix(".wxml") {
        sources.push(dist_root.join(format!("{stem}.json")));
    }
    for path in sources {
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&source) else {
            continue;
        };
        if let Some(components) = json.get("usingComponents").and_then(|value| value.as_object()) {
            names.extend(components.keys().map(|name| name.to_ascii_lowercase()));
        }
    }
    names
}

fn find_source(layout: &Layout, wxml_rel: &str) -> Option<PathBuf> {
    let stem = wxml_rel.strip_suffix(".wxml")?;
    let base = layout.src_root.join(stem);
    for ext in SOURCE_EXTS {
        let candidate = base.with_extension(ext);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    // `index.wxml` 也可能是 `index.vue`（Taro Vue）。
    let vue = base.with_extension("vue");
    if vue.is_file() {
        return Some(vue);
    }
    // 目录里的入口文件（`pages/foo/index.tsx`）。
    for ext in SOURCE_EXTS {
        let candidate = base.join("index").with_extension(ext);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn copy(layout: &Layout, synth: &mut Synth, rel: &str, source_rel: Option<String>) -> Result<()> {
    let from = layout.dist_root.join(rel);
    let to = synth.dir.join(rel);
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(&from, &to)
        .with_context(|| format!("复制失败：{} → {}", from.display(), to.display()))?;
    synth
        .copied
        .insert(rel.to_string(), source_rel.unwrap_or_else(|| layout.project_rel(rel)));
    Ok(())
}

fn write(synth: &Synth, rel: &str, text: &str) -> Result<()> {
    let path = synth.dir.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, text).with_context(|| format!("写入失败：{}", path.display()))?;
    Ok(())
}

fn write_project_config(layout: &Layout, synth: &mut Synth) -> Result<()> {
    let original = layout.project_root.join("project.config.json");
    let mut json: serde_json::Value = match std::fs::read_to_string(&original) {
        Ok(source) => serde_json::from_str(&source).unwrap_or_else(|_| serde_json::json!({})),
        Err(_) => serde_json::json!({}),
    };
    json["miniprogramRoot"] = serde_json::json!("./");
    std::fs::write(
        synth.dir.join("project.config.json"),
        serde_json::to_string_pretty(&json)?,
    )?;
    synth
        .copied
        .insert("project.config.json".to_string(), "project.config.json".to_string());

    // 开发者工具的个人配置会覆盖 setting/libVersion，原样带过来。
    let private = layout.project_root.join("project.private.config.json");
    if private.is_file() {
        std::fs::copy(&private, synth.dir.join("project.private.config.json"))?;
        synth.copied.insert(
            "project.private.config.json".to_string(),
            "project.private.config.json".to_string(),
        );
    }
    Ok(())
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("读取目录失败：{}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, out)?;
            continue;
        }
        // sourcemap 对检查没有意义，跳过可以省掉大量复制。
        if path.extension().is_some_and(|ext| ext == "map") {
            continue;
        }
        if let Ok(rel) = path.strip_prefix(root) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}

fn ext(name: &str) -> &str {
    name.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("")
}
