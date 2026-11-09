//! Skyline 迁移检查引擎。
//!
//! 职责分界：本 crate 自己提供「事实」（文件在哪、配置是什么、组件闭包、元素的计算值）
//! 与「判断」（Skyline 支持什么、语义差在哪、怎么报），不依赖编译器实现，也不做终端
//! 渲染——`skylint` 的 CLI 与后续的迁移/MCP 工具都只是它的集成。
//!
//! 唯一的对外口径是 [`Registry::builtin().run`]：给它一个 [`Project`]，拿回一组
//! [`Diagnostic`]。需要单个元素的计算值时走 [`rules::wxml`] 里的取值函数，不必自己
//! 重算级联。

pub mod config;
pub mod diag;
pub mod input;
pub mod inspect;
pub mod matcher;
pub mod migrate;
pub mod project;
pub mod report;
pub mod rules;
pub mod suppress;
pub mod wxml_graph;

pub use config::Excludes;
pub use diag::{Diagnostic, Severity};
pub use inspect::{
    AncestorView, Ancestors, CascadeView, Facts, HandlerFunction, HandlerView, LayoutFacts,
    LoadedDoc, Renderer, UnitFacts, UnitKind, UsageView,
};
pub use migrate::{Anchor, Baseline, Finding, Job, Ledger, Plan, PlanUnit, Status, VerifyReport};
pub use project::{ComponentUnit, PageUnit, Project};
pub use report::Report;
pub use rules::{Registry, Rule, RuleMeta, Target};

/// 测试夹具目录。并发跑测试时同一进程里多个测试会同时建夹具，`name` 相同时会互相
/// 删目录（曾让两条选择器规则的单测随机失败），所以带上测试线程名——Rust 的测试
/// 线程名就是完整用例路径，天然唯一。
#[cfg(test)]
pub(crate) fn fixture_root(prefix: &str, name: &str) -> std::path::PathBuf {
    let thread = std::thread::current();
    let test = thread.name().unwrap_or("shared").replace("::", "-");
    std::env::temp_dir().join(format!("{prefix}-{name}-{test}-{}", std::process::id()))
}
