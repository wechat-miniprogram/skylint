use std::collections::HashMap;
use std::fmt::Write as _;

use miette::{
    GraphicalReportHandler, GraphicalTheme, LabeledSpan, MietteDiagnostic, NamedSource,
    Report as MietteReport,
};
use serde::Serialize;

use crate::project::Project;
use crate::{Diagnostic, Severity};

/// 终端渲染参数。宽度的意义是源码片段的行内折行阈值（超长行 miette 会切成
/// `╭─▶` / `╰─▶` 多段，反而难读），所以宁可比终端略宽。
#[derive(Debug, Clone, Copy)]
pub struct Style {
    pub colored: bool,
    pub width: usize,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            colored: false,
            width: 100,
        }
    }
}

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
        // 预处理器嵌套会让同一选择器出现多次（`.a { &:hover {} @media { &:hover {} } }`
        // 摊平后 `.a:hover` 有两条规则），选择器类规则于是重复报同一条。全字段相同的
        // 诊断不携带额外信息，直接去掉。
        diagnostics.dedup();

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

    /// 逐条渲染成带源码片段的块：miette 的图形报告，与 minidev 的错误出口同一套
    /// （`unicode` / `unicode_nocolor` 主题）。`source` 按项目相对路径取源码，
    /// 取不到（文件被删、非 UTF-8）时退化成单行 `path:line:col`，不静默丢弃。
    pub fn to_text(&self, source: impl Fn(&str) -> Option<String>, style: Style) -> String {
        let handler = GraphicalReportHandler::new_themed(if style.colored {
            GraphicalTheme::unicode()
        } else {
            GraphicalTheme::unicode_nocolor()
        })
        .with_width(style.width)
        // 超长行（WXML 常整段写一行）截断而不是折成多段，否则一条诊断能占半屏
        .with_wrap_lines(false)
        .with_context_lines(1);
        let mut cache: HashMap<&str, Option<String>> = HashMap::new();
        let mut out = String::new();

        for d in &self.diagnostics {
            let text = cache
                .entry(d.path.as_str())
                .or_insert_with(|| source(&d.path));
            let mut diag = MietteDiagnostic::new(d.message.clone())
                .with_code(d.rule.clone())
                .with_severity(severity(d.severity));
            if let Some(help) = &d.help {
                diag = diag.with_help(help.clone());
            }
            match text {
                Some(src) => {
                    diag = diag.with_labels(vec![LabeledSpan::new_with_span(None, span(src, d))]);
                    // NamedSource 带文件名，片段头才会渲染成 `╭─[path:line:col]`
                    let report = MietteReport::new(diag)
                        .with_source_code(NamedSource::new(d.path.clone(), src.clone()));
                    let _ = handler.render_report(&mut out, report.as_ref());
                    out.push('\n');
                }
                None => {
                    let _ = writeln!(
                        out,
                        "{}:{}:{}  {}  {}  {}",
                        d.path,
                        d.line,
                        d.column,
                        d.severity.as_str(),
                        d.rule,
                        d.message
                    );
                    if let Some(help) = &d.help {
                        let _ = writeln!(out, "    help: {help}");
                    }
                }
            }
        }

        let s = &self.summary;
        let _ = writeln!(
            out,
            "{} error, {} warning, {} info ({} pages, {} skyline, {} components)",
            s.errors, s.warnings, s.infos, s.pages, s.skyline_pages, s.components
        );
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

fn severity(s: Severity) -> miette::Severity {
    match s {
        Severity::Error => miette::Severity::Error,
        Severity::Warning => miette::Severity::Warning,
        Severity::Info => miette::Severity::Advice,
    }
}

/// 诊断区间 → `(offset, length)`（miette 的 SourceSpan 就是「起点 + 长度」）。
///
/// 跨行区间收窄到起始行：多行标签（`<scroll-view` 的属性常常换行）整段下划线
/// 读起来像「整块代码错了」，而 linter 只需要指出起点。
fn span(source: &str, d: &Diagnostic) -> (usize, usize) {
    let start = offset(source, d.line, d.column);
    let end = if d.end_line > d.line {
        // 跨行区间收窄到起始行尾
        line_end(source, start)
    } else {
        offset(source, d.end_line, d.end_column)
    };
    // 只给了起点（工程级 warning 等）：标出该字符，caret 才有落点
    let end = if end > start {
        end
    } else {
        let width = source[start..].chars().next().map_or(0, char::len_utf8);
        start + width
    };
    (start, end - start)
}

/// 从 `offset` 所在行的行尾（不含换行符）。
fn line_end(source: &str, offset: usize) -> usize {
    let rest = &source[offset..];
    rest.find('\n').map_or(source.len(), |i| {
        let tail = &rest[..i];
        offset + tail.trim_end_matches('\r').len()
    })
}

/// (1-based 行, 1-based 字符列) → 字节偏移；越界钳到行尾 / 文末。
fn offset(source: &str, line: u32, column: u32) -> usize {
    let mut base = 0;
    for (idx, text) in (1u32..).zip(source.split_inclusive('\n')) {
        if idx == line {
            // 行尾的换行符不属于任何列，钳到行内容末尾
            let content = text.trim_end_matches(['\n', '\r']).len();
            let byte = text
                .char_indices()
                .nth(column.saturating_sub(1) as usize)
                .map_or(content, |(i, _)| i.min(content));
            return base + byte;
        }
        base += text.len();
    }
    source.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    const WXML: &str = "line1\n  <scroll-view class=\"a\" scroll-y enhanced show-scrollbar=\"{{false}}\">\nline3\n";

    fn report(d: Diagnostic) -> Report {
        Report {
            summary: Summary {
                pages: 1,
                skyline_pages: 1,
                components: 0,
                errors: 0,
                warnings: 1,
                infos: 0,
            },
            diagnostics: vec![d],
        }
    }

    #[test]
    fn span_is_offset_and_length() {
        let d = Diagnostic::new("skyline/x", Severity::Warning, "m", "a.wxml", 2, 4).end(2, 17);
        let (start, len) = span(WXML, &d);
        assert_eq!(&WXML[start..start + len], "scroll-view c");
    }

    #[test]
    fn point_span_marks_one_char() {
        let d = Diagnostic::new("skyline/x", Severity::Warning, "m", "a.wxml", 2, 4);
        let (start, len) = span(WXML, &d);
        assert_eq!(&WXML[start..start + len], "s");
    }

    #[test]
    fn multi_line_span_collapses_to_start_line() {
        let d = Diagnostic::new("skyline/x", Severity::Warning, "m", "a.wxml", 2, 4).end(3, 1);
        let (start, len) = span(WXML, &d);
        assert_eq!(start + len, line_end(WXML, start), "应止于起始行行尾");
        assert!(!WXML[start..start + len].contains('\n'));
    }

    #[test]
    fn renders_code_help_and_location() {
        let d =
            Diagnostic::new("skyline/x", Severity::Error, "坏了", "a.wxml", 2, 4).help("这么改");
        let out = report(d).to_text(
            |_| Some(WXML.to_string()),
            Style {
                colored: false,
                width: 120,
            },
        );
        assert!(out.contains("a.wxml:2:4"), "缺少行列信息:\n{out}");
        assert!(out.contains("skyline/x"));
        assert!(out.contains("坏了"));
        assert!(out.contains("help: 这么改"));
        assert!(out.contains("1 error, 0 warning") || out.contains("0 error, 1 warning"));
    }

    #[test]
    fn unreadable_source_falls_back_to_one_line() {
        let d = Diagnostic::new("skyline/x", Severity::Warning, "m", "gone.wxml", 3, 2);
        let out = report(d).to_text(|_| None, Style::default());
        assert!(out.contains("gone.wxml:3:2"), "{out}");
    }
}
