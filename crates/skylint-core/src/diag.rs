use std::fmt;

use serde::{Deserialize, Serialize};

/// 严重级别。三档：error 是「一定会坏」，warning 是「大概率语义变了」，
/// info 是「建议改」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 一条检查结果。
///
/// 位置约定与用户可见输出一致：`line`/`column` 均 **1-based**，`column` 按
/// Unicode 字符计（不是字节，也不是 UTF-16 code unit）。跨行元素用
/// `end_line`/`end_column` 表达区间，跨行时 `end_column` 为结束行内的列。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    /// 规则 id，如 `skyline/unsupported-component`。
    pub rule: String,
    pub severity: Severity,
    pub message: String,
    /// 项目相对路径（posix 分隔）。
    pub path: String,
    pub line: u32,
    pub column: u32,
    pub end_line: u32,
    pub end_column: u32,
    /// 迁移建议。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
}

impl Diagnostic {
    pub fn new(
        rule: impl Into<String>,
        severity: Severity,
        message: impl Into<String>,
        path: impl Into<String>,
        line: u32,
        column: u32,
    ) -> Self {
        let (line, column) = (line.max(1), column.max(1));
        Self {
            rule: rule.into(),
            severity,
            message: message.into(),
            path: path.into(),
            line,
            column,
            end_line: line,
            end_column: column,
            help: None,
        }
    }

    pub fn end(mut self, end_line: u32, end_column: u32) -> Self {
        self.end_line = end_line.max(self.line);
        self.end_column = end_column.max(1);
        self
    }

    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }
}

/// minidev 的诊断只有「起点行列 + 字节区间」，跨 crate 边界时补齐成我们的形状。
/// 它的 `column` 是 0-based，这里转成 1-based。
pub fn from_compiler_diag(d: &diag::Diagnostic) -> Diagnostic {
    let severity = match d.severity {
        diag::Severity::Error => Severity::Error,
        diag::Severity::Warning => Severity::Warning,
    };
    let line = d.line.unwrap_or(1);
    let column = d.column.unwrap_or(0) + 1;
    let mut diag = Diagnostic::new(
        d.code.clone(),
        severity,
        d.message.clone(),
        d.path.clone().unwrap_or_default(),
        line,
        column,
    );
    if let Some(help) = &d.help {
        diag = diag.help(help.clone());
    }
    diag
}
