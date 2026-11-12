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

pub mod autoprefix_build;
pub mod component_framework;
pub mod config_ignored;
pub mod default_layout_drift;
pub mod lazy_code_loading;
pub mod lib_version;
pub mod navigation_style;
pub mod renderer_options;
pub mod webview_tabbar_renderer;

pub use autoprefix_build::AutoprefixBuild;
pub use component_framework::ComponentFramework;
pub use config_ignored::ConfigIgnored;
pub use default_layout_drift::DefaultLayoutDrift;
pub use lazy_code_loading::LazyCodeLoading;
pub use lib_version::LibVersion;
pub use navigation_style::NavigationStyle;
pub use renderer_options::RendererOptions;
pub use webview_tabbar_renderer::TabBarRenderer;

use crate::diag::{Diagnostic, JsonAnchor, Severity};
use crate::project::Project;

/// 配置类规则的适用面：工程级配置只有一份，单文件视图没有工程可读；插件视图
/// 只检查它自己的 `plugin.json` 与插件页 json，工程级事实（`libVersion`）
/// 仍由宿主视图报一次。
pub(crate) fn applicable(project: &Project) -> bool {
    !project.single_file
}

/// 工程级事实（`project.config.json`）的规则在插件视图里跳过，避免同一份配置报两遍。
pub(crate) fn applicable_project_level(project: &Project) -> bool {
    applicable(project) && !project.plugin_view
}

/// RFC 6901 指针：`~` → `~0`、`/` → `~1`，每段前加 `/`。
pub(crate) fn pointer(segments: &[&str]) -> String {
    let mut out = String::new();
    for segment in segments {
        out.push('/');
        out.push_str(&segment.replace('~', "~0").replace('/', "~1"));
    }
    out
}

/// 在已有指针下追加一段（同样做 RFC 6901 转义）。
pub(crate) fn child(pointer: &str, segment: &str) -> String {
    format!(
        "{pointer}/{}",
        segment.replace('~', "~0").replace('/', "~1")
    )
}

/// 结构锚点：指针 + 触发值的紧凑 JSON 文本（`None` 表示键缺失）。
pub(crate) fn anchor(segments: &[&str], value: Option<&serde_json::Value>) -> JsonAnchor {
    JsonAnchor::new(pointer(segments), value.map(serde_json::Value::to_string))
}

/// 配置类诊断：给出 `at` 就落在该位置，否则落在文件首字符。
///
/// 参数是诊断字段的直接投影（位置三项 + 内容两项 + 严重级），拆成结构体只会在
/// 调用点再造一层面具；真正的入口是下面两个函数。
#[allow(clippy::too_many_arguments)]
fn emit(
    out: &mut Vec<Diagnostic>,
    id: &'static str,
    path: &str,
    at: Option<(u32, u32)>,
    anchor: JsonAnchor,
    message: String,
    help: &str,
    severity: Severity,
) {
    let (line, column) = at.unwrap_or((1, 1));
    out.push(
        Diagnostic::new(id, severity, message, path, line, column)
            .help(help)
            .json(anchor),
    );
}

pub(crate) fn warn(
    out: &mut Vec<Diagnostic>,
    id: &'static str,
    path: &str,
    at: Option<(u32, u32)>,
    anchor: JsonAnchor,
    message: String,
    help: &str,
) {
    emit(out, id, path, at, anchor, message, help, Severity::Warning);
}

pub(crate) fn error(
    out: &mut Vec<Diagnostic>,
    id: &'static str,
    path: &str,
    at: Option<(u32, u32)>,
    anchor: JsonAnchor,
    message: String,
    help: &str,
) {
    emit(out, id, path, at, anchor, message, help, Severity::Error);
}
