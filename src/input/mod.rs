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

/// 去掉 UTF-8 BOM。
///
/// BOM 是合法字节，但所有解析器都会把它当成内容的第一字符：JSON 解析失败、
/// CSS 选择器变成 `\u{feff}.a`、WXML 第一个标签认不出来。开发者工具不写 BOM，
/// 但 `Set-Content -Encoding utf8` / 部分编辑器会。
pub(crate) fn strip_bom(source: &str) -> &str {
    source.strip_prefix('\u{feff}').unwrap_or(source)
}
