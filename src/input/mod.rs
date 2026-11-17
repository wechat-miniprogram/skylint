//! 输入视图：把源文件解析成规则可消费的结构。
//!
//! 每个视图只做「解析 + 定位」，不含任何 Skyline 语义判断——判断属于 `rules`。
//! 解析失败返回 `Err(String)`，由调用方决定是否降级（单文件失败不得终止全扫描）。

pub mod env;
pub mod expr;
pub mod js;
pub mod json;
pub mod wxml;
pub mod wxss;

/// 字节偏移 → `(line, column)`，均 1-based，`column` 按 Unicode 字符计。
///
/// 偏移落在多字节字符中间时向下取整到该字符起点，超出文件长度时取文件末尾。
/// JS 与 WXSS 文档共用这一份：两边原本各写了一遍，改一处忘一处就会让诊断位置错位。
pub(crate) fn position(source: &str, offset: usize) -> (u32, u32) {
    let mut end = offset.min(source.len());
    while end > 0 && !source.is_char_boundary(end) {
        end -= 1;
    }
    let prefix = &source[..end];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() as u32 + 1;
    let line_start = prefix.rfind('\n').map_or(0, |index| index + 1);
    let column = source[line_start..end].chars().count() as u32 + 1;
    (line, column)
}

/// `(line, column)` → 字节偏移，均 1-based，`column` 按 Unicode 字符计。
///
/// 是 [`position`] 的反向换算；行/列越界时取源文件末尾。
pub(crate) fn offset(source: &str, line: u32, column: u32) -> usize {
    let mut line_start = 0;
    let mut current = 1;
    while current < line {
        let Some(index) = source[line_start..].find('\n') else {
            return source.len();
        };
        line_start += index + 1;
        current += 1;
    }
    let line_end = source[line_start..]
        .find('\n')
        .map_or(source.len(), |index| line_start + index);
    let line_text = &source[line_start..line_end];
    line_text
        .char_indices()
        .nth(column.saturating_sub(1) as usize)
        .map_or(line_end, |(index, _)| line_start + index)
}

/// 去掉 UTF-8 BOM。
///
/// BOM 是合法字节，但所有解析器都会把它当成内容的第一字符：JSON 解析失败、
/// CSS 选择器变成 `\u{feff}.a`、WXML 第一个标签认不出来。开发者工具不写 BOM，
/// 但 `Set-Content -Encoding utf8` / 部分编辑器会。
pub(crate) fn strip_bom(source: &str) -> &str {
    source.strip_prefix('\u{feff}').unwrap_or(source)
}
