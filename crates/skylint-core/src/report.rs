use serde::Serialize;

use crate::project::Project;
use crate::{Diagnostic, Severity};

/// 一次检查的完整结果。文本与 JSON 两种渲染共用同一份数据。
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub summary: Summary,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub pages: usize,
    pub skyline_pages: usize,
    pub components: usize,
    pub errors: usize,
    pub warnings: usize,
    pub infos: usize,
}

impl Report {
    pub fn new(project: &Project, mut diagnostics: Vec<Diagnostic>) -> Self {
        // warnings 由工程视图产出（json 解析失败、文件缺失），与规则结果合并后统一排序：
        // 路径 → 行 → 列 → 规则 id，保证同一工程多次运行输出稳定。
        diagnostics.extend(project.warnings.iter().cloned());
        diagnostics.sort_by(|a, b| {
            (&a.path, a.line, a.column, &a.rule).cmp(&(&b.path, b.line, b.column, &b.rule))
        });

        let count = |s: Severity| diagnostics.iter().filter(|d| d.severity == s).count();
        Self {
            summary: Summary {
                pages: project.pages.len(),
                skyline_pages: project.pages.iter().filter(|p| p.is_skyline).count(),
                components: project.components.len(),
                errors: count(Severity::Error),
                warnings: count(Severity::Warning),
                infos: count(Severity::Info),
            },
            diagnostics,
        }
    }

    /// `path:line:col  severity  rule  message`，与大多数 lint 工具的单行格式一致
    /// （编辑器/CI 的高亮规则都认这个形状）。
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for d in &self.diagnostics {
            out.push_str(&format!(
                "{}:{}:{}  {:>7}  {}  {}\n",
                d.path,
                d.line,
                d.column,
                d.severity.as_str(),
                d.rule,
                d.message
            ));
            if let Some(help) = &d.help {
                out.push_str(&format!("           help: {help}\n"));
            }
        }
        let s = &self.summary;
        out.push_str(&format!(
            "\n{} error, {} warning, {} info ({} pages, {} skyline, {} components)\n",
            s.errors, s.warnings, s.infos, s.pages, s.skyline_pages, s.components
        ));
        out
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("report is always serializable")
    }

    /// CI 判据：出现任何 error 级结果即失败。
    pub fn has_errors(&self) -> bool {
        self.summary.errors > 0
    }
}
