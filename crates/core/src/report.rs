use serde::Serialize;

use crate::config::Excludes;
use crate::project::Project;
use crate::{Diagnostic, Severity};

/// 一次检查的完整结果。
///
/// 只承载数据：终端/JSON 等展示由集成层决定（`skylint` 的 CLI 在 `render` 里做
/// miette 渲染，迁移工具直接读字段）。[`Report::to_json`] 因为只是序列化，留在这里。
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub summary: Summary,
    pub diagnostics: Vec<Diagnostic>,
    /// `.skylint/config.json` 排除掉的路径与条数。
    ///
    /// 排除不能静默：报告里说清楚「这几份文件被跳过了、跳掉了多少条」，
    /// 否则「零命中」与「被排掉了」在输出上无法区分。
    pub excluded: Excluded,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Excluded {
    pub files: Vec<String>,
    pub diagnostics: usize,
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
    /// 工程里是否出现过任何 `renderer` 声明；`false` 表示整份结果是在
    /// 「按 Skyline 默认检查」的前提下得出的。
    pub renderer_declared: bool,
}

impl Report {
    /// 多视图（宿主 + 插件）的结果合并。
    ///
    /// 调用方负责把每个视图的路径前缀补上（[`crate::Project::report_prefix`]）；
    /// 这里只做排序、去重与计数，summary 是各视图之和。
    pub fn from_views(projects: &[Project], mut diagnostics: Vec<Diagnostic>) -> Self {
        // 工程级排除（`.skylint/config.json`）：第三方样式这类「本来就不该手改」的
        // 文件不进入报告，但排除本身要被记下来。多视图共用同一个工程根，取第一个。
        let excludes = projects
            .first()
            .map(|project| Excludes::load(&project.root))
            .unwrap_or_default();
        let mut excluded = Excluded::default();
        if !excludes.is_empty() {
            let before = diagnostics.len();
            diagnostics.retain(|diagnostic| !excludes.matches(&diagnostic.path));
            excluded.diagnostics = before - diagnostics.len();
            excluded.files = projects
                .iter()
                .flat_map(|project| project.styles.iter())
                .map(|sheet| sheet.doc.path().to_string())
                .filter(|path| excludes.matches(path))
                .collect();
            excluded.files.sort();
            excluded.files.dedup();
        }

        // 统一排序：路径 → 行 → 列 → 规则 id，保证同一工程多次运行输出稳定。
        diagnostics.sort_by(|a, b| {
            (&a.path, a.line, a.column, &a.rule).cmp(&(&b.path, b.line, b.column, &b.rule))
        });
        // 预处理器嵌套会让同一选择器出现多次（`.a { &:hover {} @media { &:hover {} } }`
        // 摊平后 `.a:hover` 有两条规则），选择器类规则于是重复报同一条。全字段相同的
        // 诊断不携带额外信息，直接去掉。
        diagnostics.dedup();

        let count = |s: Severity| diagnostics.iter().filter(|d| d.severity == s).count();
        let pages = projects.iter().map(|p| p.pages.len()).sum();
        Self {
            summary: Summary {
                pages,
                skyline_pages: projects
                    .iter()
                    .flat_map(|p| &p.pages)
                    .filter(|page| page.is_skyline)
                    .count(),
                components: projects.iter().map(|p| p.components.len()).sum(),
                errors: count(Severity::Error),
                warnings: count(Severity::Warning),
                infos: count(Severity::Info),
                renderer_declared: projects.iter().any(|p| p.declares_renderer()),
            },
            diagnostics,
            excluded,
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("report is always serializable")
    }

    /// CI 判据：出现任何 error 级结果即失败。
    pub fn has_errors(&self) -> bool {
        self.summary.errors > 0
    }
}
