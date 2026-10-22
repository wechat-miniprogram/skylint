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

/// 测试夹具目录。并发跑测试时同一进程里多个测试会同时建夹具，`name` 相同时会互相
/// 删目录（曾让两条选择器规则的单测随机失败），所以带上测试线程名——Rust 的测试
/// 线程名就是完整用例路径，天然唯一。
#[cfg(test)]
pub(crate) fn fixture_root(prefix: &str, name: &str) -> std::path::PathBuf {
    let thread = std::thread::current();
    let test = thread.name().unwrap_or("shared").replace("::", "-");
    std::env::temp_dir().join(format!("{prefix}-{name}-{test}-{}", std::process::id()))
}
