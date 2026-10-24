//! 配置类规则：`project.config.json` / `app.json` / 页面 json。
//!
//! 这组规则的立足点是一处不对称：页面 json 有 schema 校验，`minidev build` 会对
//! 页面里写错的 `rendererOptions`（例如 `defaultDisplayBlock: "yes"`）直接报错；
//! 而同一份配置写进 `app.json`、`libVersion` 写进 `project.config.json` 时，
//! 构建**完全静默**（实测 app.json 的 `renderer: 123`、未知顶层键、
//! `tagNameStyleIsolation: "bogus"` 都能通过）。运行时又只按自己认的键和类型取值，
//! 取不到就回落默认值——于是配置写错不会崩，只会静默失效。
//!
//! 这组规则把这个「静默」变成诊断，且只做静态可判定的事：不做版本协商、不做
//! 运行时求值，也不去补 minidev 缺失的 app.json schema（那会引入大量与本工具
//! 无关的过时告警）。

pub mod config_ignored;
pub mod default_layout_drift;
pub mod lib_version;
pub mod renderer_options;

pub use config_ignored::ConfigIgnored;
pub use default_layout_drift::DefaultLayoutDrift;
pub use lib_version::LibVersion;
pub use renderer_options::RendererOptions;
