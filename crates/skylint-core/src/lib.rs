//! Skyline 迁移检查核心。
//!
//! 职责分界：minidev 提供「事实」（文件在哪、配置是什么、组件闭包），本 crate
//! 提供「判断」（Skyline 支持什么、语义差在哪、怎么报）。因此这里不出现任何
//! 编译流水线概念，也不复用 minidev 的规则与诊断文案。
//!
//! 与 minidev 的全部接触点收敛在 [`project`] 里，其余模块只消费本 crate 自己的
//! 类型——依赖换源或将来完全脱离 minidev 时只改那一处。

pub mod diag;
pub mod input;
pub mod project;
pub mod report;
pub mod rules;
pub mod suppress;

pub use diag::{Diagnostic, Severity};
pub use project::{ComponentUnit, PageUnit, Project};
pub use report::Report;
pub use rules::{Registry, Rule, RuleMeta};
