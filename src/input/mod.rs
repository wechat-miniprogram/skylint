//! 输入视图：把源文件解析成规则可消费的结构。
//!
//! 每个视图只做「解析 + 定位」，不含任何 Skyline 语义判断——判断属于 `rules`。
//! 解析失败返回 `Err(String)`，由调用方决定是否降级（单文件失败不得终止全扫描）。

pub mod js;
pub mod json;
pub mod wxml;
pub mod wxss;
