//! 单元源文件解析：`{unit}.{target}` 对应哪个真实文件。
//!
//! 后缀优先级由 `project.config.json` 的 `setting.useCompilerPlugins` 决定——
//! 未声明插件时只认原生后缀；`typescript` 让 `.ts` 优先于 `.js`；`less` 让 `.less`
//! 优先于 `.wxss`；`sass` 让 `.scss` 优先于 `.sass` 优先于 `.wxss`。

use serde_json::Value;

use super::fs::LocalFs;
use crate::project::UnitFiles;

pub struct Exts {
    /// 单个目标的候选后缀，**高优先级在前**。
    wxss: Vec<&'static str>,
    js: Vec<&'static str>,
}

impl Exts {
    pub fn from_config(config: &Value) -> Self {
        let plugins: Vec<&str> = config
            .pointer("/setting/useCompilerPlugins")
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();

        // 注册序在后的优先级更高，因此按注册顺序 push 后整体反转。
        let mut wxss = vec!["wxss"];
        let mut js = vec!["js"];
        for plugin in plugins {
            match plugin {
                "less" => wxss.push("less"),
                "sass" => wxss.extend(["sass", "scss"]),
                "typescript" => js.push("ts"),
                _ => {}
            }
        }
        wxss.reverse();
        js.reverse();
        Self { wxss, js }
    }
}

pub fn resolve_unit(exts: &Exts, fs: &LocalFs, root: &str, unit: &str) -> UnitFiles {
    let at = |candidates: &[&str]| {
        candidates
            .iter()
            .map(|ext| format!("{unit}.{ext}"))
            .find(|path| !path.ends_with(".d.ts") && fs.exists(root, path))
    };
    UnitFiles {
        json: at(&["json"]),
        wxml: at(&["wxml"]),
        wxss: at(&exts.wxss),
        js: at(&exts.js),
    }
}
