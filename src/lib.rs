//! Skyline 迁移检查核心。
//!
//! 职责分界：本 crate 自己提供「事实」（文件在哪、配置是什么、组件闭包）与「判断」
//! （Skyline 支持什么、语义差在哪、怎么报），不依赖编译器实现。
//!
//! 唯一的对外口径是 [`Registry::builtin().run`]：给它一个 [`Project`]，拿回一组
//! [`Diagnostic`]。库可以直接被别的工具链消费（`src/main.rs` 只是其中一层薄 CLI）。

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
