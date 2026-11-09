//! 终端渲染：把 [`Report`] 画成带源码片段的文本块。
//!
//! 这是集成层的东西（依赖 miette），引擎不参与；MCP/迁移工具直接用 `Report` 的字段
//! 或 `to_json`，不会经过这里。

use std::collections::HashMap;
use std::fmt::Write as _;

use miette::{
    GraphicalReportHandler, GraphicalTheme, LabeledSpan, MietteDiagnostic, NamedSource,
    Report as MietteReport,
};

use skylint_core::{Diagnostic, Report, Severity};

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

/// 逐条渲染成带源码片段的块：miette 的图形报告，与 minidev 的错误出口同一套
/// （`unicode` / `unicode_nocolor` 主题）。`source` 按项目相对路径取源码，
/// 取不到（文件被删、非 UTF-8）时退化成单行 `path:line:col`，不静默丢弃。
pub fn text(report: &Report, source: impl Fn(&str) -> Option<String>, style: Style) -> String {
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

    for d in &report.diagnostics {
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

    let s = &report.summary;
    let _ = writeln!(
        out,
        "{} error, {} warning, {} info ({} pages, {} skyline, {} components)",
        s.errors, s.warnings, s.infos, s.pages, s.skyline_pages, s.components
    );
    if !s.renderer_declared {
        let _ = writeln!(
            out,
            "note: 工程里没有任何 renderer 声明，以上按 Skyline 默认口径检查；WebView 页面请显式写 \"renderer\": \"webview\""
        );
    }
    // 排除必须说出来：否则「零命中」与「被排掉了」在输出上分不出来。
    if report.excluded.diagnostics > 0 {
        let _ = writeln!(
            out,
            "excluded: {} 份文件（{} 条诊断）来自 {}，见 .skylint/config.json",
            report.excluded.files.len(),
            report.excluded.diagnostics,
            report.excluded.files.join(", ")
        );
    }
    out
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
    use skylint_core::report::Summary;

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
                renderer_declared: true,
            },
            diagnostics: vec![d],
            excluded: Default::default(),
        }
    }

    /// 工程里没有任何 `renderer` 声明时，结果是在「按 Skyline 默认」前提下得出的，
    /// 报告必须把这个前提说出来，否则读者会当成当前渲染器下的 bug。
    #[test]
    fn missing_renderer_declaration_adds_a_note() {
        let mut r = report(Diagnostic::new(
            "skyline/x",
            Severity::Warning,
            "m",
            "a.wxml",
            2,
            4,
        ));
        assert!(!text(&r, |_| None, Style::default()).contains("note:"));
        r.summary.renderer_declared = false;
        let out = text(&r, |_| None, Style::default());
        assert!(out.contains("没有任何 renderer 声明"), "{out}");
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
        let out = text(
            &report(d),
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
        let out = text(&report(d), |_| None, Style::default());
        assert!(out.contains("gone.wxml:3:2"), "{out}");
    }
}
