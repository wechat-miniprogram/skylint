//! Skyline 小程序迁移检查工具（集成层）。
//!
//! 引擎在 [`skylint_core`]：工程加载、模板/样式/脚本解析、选择器匹配、规则与诊断。
//! 本 crate 只做两件事：
//!
//! - 把引擎的公开面原样再导出，保证 `use skylint::…` 的老写法继续可用；
//! - 提供 CLI 需要的终端渲染（[`render`]）与 `skylint` 二进制。
//!
//! 所以「只想要一个 linter」的用法不受影响，而迁移/MCP 这类新集成只依赖
//! `skylint-core`，不会把 clap / miette 带进去。

pub use skylint_core::{
    ComponentUnit, Diagnostic, PageUnit, Project, Registry, Report, Rule, RuleMeta, Severity,
    Target,
};
pub use skylint_core::{
    diag, input, inspect, matcher, migrate, project, report, rules, suppress, wxml_graph,
};

pub mod render;
