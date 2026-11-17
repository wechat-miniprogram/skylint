//! 诊断定位辅助：把源码区间 / 元素标签区间换算成行列。
//!
//! 所有规则都用这两个函数产出诊断，行列口径只在这里定义一次。

use glass_easel_template_compiler::parse::Position;

use crate::diag::{Diagnostic, Severity};
use crate::input::wxml::{WxmlDoc, WxmlElement};

/// 定位在 WXML 源文本任意一段区间上的诊断。
pub(crate) fn span_diagnostic(
    id: &'static str,
    doc: &WxmlDoc,
    start: Position,
    end: Position,
    path: &str,
    severity: Severity,
    message: impl Into<String>,
) -> Diagnostic {
    let (line, column) = doc.position(start);
    let (end_line, end_column) = doc.position(end);
    Diagnostic::new(id, severity, message, path, line, column).end(end_line, end_column)
}

/// 定位在元素标签起止位置上的诊断。
pub(crate) fn element_diagnostic(
    id: &'static str,
    doc: &WxmlDoc,
    element: &WxmlElement,
    path: &str,
    severity: Severity,
    message: impl Into<String>,
) -> Diagnostic {
    span_diagnostic(
        id,
        doc,
        element.tag_start,
        element.tag_end,
        path,
        severity,
        message,
    )
}
