//! 文件视图：一次递归枚举 + 相对 `miniprogramRoot` 的读与存在判断。
//!
//! 枚举口径与 miniprogram-ci 的 `project.fs` 一致：跳过 `node_modules` 与点目录，
//! 只收文件，反斜杠归一，`list_files` 的返回值带 `miniprogramRoot` 前缀（无前导 `/`）；
//! `exists` / `read_file` 的路径相对 `miniprogramRoot`，前导 `/` 视为根路径。

use std::path::{Path, PathBuf};

pub struct LocalFs {
    pub project_path: PathBuf,
}

impl LocalFs {
    pub fn new(project_path: impl Into<PathBuf>) -> Self {
        Self {
            project_path: project_path.into(),
        }
    }

    /// 递归枚举 `root` 下的全部文件，返回项目相对路径（含 `root` 前缀）。
    ///
    /// `root` 来自用户可改的 `project.config.json`，越界（`../..`）一律返回空表，
    /// 不把扫描带出项目目录。
    pub fn list_files(&self, root: &str) -> Vec<String> {
        let dir = self.project_path.join(root);
        if !is_within(&self.project_path, &dir) {
            return Vec::new();
        }
        let mut found = Vec::new();
        walk(&dir, &mut found);
        let base = format!("{}/", normalize_path(&self.project_path));
        found
            .into_iter()
            .filter_map(|path| path.strip_prefix(&base).map(str::to_string))
            .collect()
    }

    pub fn exists(&self, root: &str, file: &str) -> bool {
        self.abs(root, file).is_file()
    }

    pub fn read_file(&self, root: &str, file: &str) -> Option<String> {
        std::fs::read_to_string(self.abs(root, file)).ok()
    }

    fn abs(&self, root: &str, file: &str) -> PathBuf {
        self.project_path
            .join(root)
            .join(file.trim_start_matches('/'))
    }
}

fn walk(dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if name == "node_modules" || name.starts_with('.') {
                continue;
            }
            walk(&entry.path(), out);
        } else if kind.is_file() {
            out.push(normalize_path(&entry.path()));
        }
    }
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn is_within(base: &Path, target: &Path) -> bool {
    let Ok(target) = target.canonicalize() else {
        return false;
    };
    let base = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
    target.starts_with(&base)
}
