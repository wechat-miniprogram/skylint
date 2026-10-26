//! JS / worklet 规则。
//!
//! 一条规则一个文件，全部消费 [`crate::project::Project::scripts`] 里已解析的
//! [`crate::input::js::JsDoc`]：规则侧不再读盘、不再做语法分析。每条规则的证据
//! 都写在文件头，数据驱动的部分（不可用 API 表）放在 `rules/data/` 下。

pub mod callee;
pub mod draggable_sheet_scroll_to;
pub mod route_type;
pub mod scroll_context_ref;
pub mod unsupported_api;
pub mod worklet_api_scope;
pub mod worklet_data_freeze;
pub mod worklet_directive;
pub mod worklet_es5;

pub(crate) use callee::Callee;
pub use draggable_sheet_scroll_to::DraggableSheetScrollTo;
pub use route_type::RouteType;
pub use scroll_context_ref::ScrollContextRef;
pub use unsupported_api::UnsupportedApi;
pub use worklet_api_scope::WorkletApiScope;
pub use worklet_data_freeze::WorkletDataFreeze;
pub use worklet_directive::WorkletDirective;
pub use worklet_es5::WorkletCompileSetting;

use std::collections::HashSet;

use serde_json::Value;

use crate::project::{JsFile, Project, Scope};
use crate::{Diagnostic, Severity};

/// 插件工程：`compileType` 优先；缺省时按 minidev 的目录约定（没有 `miniprogramRoot`
/// 但有 `pluginRoot`）判断。两者都读不到时不判。
pub(crate) fn is_plugin(project: &Project) -> bool {
    match project
        .project_config
        .get("compileType")
        .and_then(Value::as_str)
    {
        Some(kind) => kind == "plugin",
        None => {
            !project.project_config.contains_key("miniprogramRoot")
                && project.project_config.contains_key("pluginRoot")
        }
    }
}

/// 这个脚本是否属于插件代码——插件限制只对插件代码成立。
///
/// `compileType: plugin` 的工程里 `miniprogramRoot` 通常是宿主示例小程序
/// （`weapp-novel-plugin_1` 是 `dist/miniprogram` + `dist/plugin`），宿主代码
/// 照常能用 `wx.router`。只有落在 `pluginRoot` 下的文件才受插件限制；
/// 纯插件仓库（没有 `pluginRoot`、代码就在根目录）按 `compileType` 整体算。
pub(crate) fn in_plugin(project: &Project, path: &str) -> bool {
    if project.plugin_view {
        return true;
    }
    match project.plugin_root.as_deref() {
        Some(root) => path == root || path.starts_with(&format!("{root}/")),
        None => is_plugin(project),
    }
}

/// 脚本所属单元是否真的按 Skyline 渲染。
///
/// 「Skyline 不支持某个 API」只对会走 Skyline 的代码成立：页面看 `renderer`，
/// 组件看有没有 Skyline 页面用到它。`app.js`、`utils/` 这类没有单元归属的脚本
/// 静态判断不了，按「不是」处理（宁可漏报）。
pub(crate) fn renders_in_skyline(project: &Project, scope: &Scope) -> bool {
    let page_is_skyline = |route: &str| {
        project
            .pages
            .iter()
            .any(|page| page.route == route && page.is_skyline && !page.opted_out)
    };
    match scope {
        Scope::Page(route) => page_is_skyline(route),
        Scope::Component(path) => project
            .component_pages(path)
            .iter()
            .any(|route| page_is_skyline(route)),
        Scope::Global | Scope::Loose(_) => false,
    }
}

/// 单元对应的脚本。`Project.scripts` 里一个文件只归一个单元，所以按 scope 查即可。
///
/// [`Scope::Loose`] 不是单元：显式返回 `None`，避免把 `utils/` 里的文件当成页面脚本。
pub(crate) fn scope_script<'a>(project: &'a Project, scope: &Scope) -> Option<&'a JsFile> {
    if matches!(scope, Scope::Loose(_)) {
        return None;
    }
    project.scripts.iter().find(|file| &file.scope == scope)
}

/// 页面与组件的 WXML 及所属单元，按路径去重（同一文件被多次引用只留一份）。
///
/// 页面显式声明 `renderer: webview` 的整页跳过，与 wxml 规则同口径：那是「这页不迁
/// Skyline」的决定，对它讲 Skyline 的限制只是误报。
pub(crate) fn unit_wxml(project: &Project) -> Vec<(&str, Scope)> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for page in project.pages.iter().filter(|page| !page.opted_out) {
        if let Some(path) = page.files.wxml.as_deref()
            && seen.insert(path)
        {
            out.push((path, Scope::Page(page.route.clone())));
        }
    }
    for component in &project.components {
        if let Some(path) = component.files.wxml.as_deref()
            && seen.insert(path)
        {
            out.push((path, Scope::Component(component.path.clone())));
        }
    }
    out
}

/// 把诊断落到脚本文件的一段字节区间上。区间是 `[start, end)`，与解析器的 span 一致。
pub(crate) fn at_span(
    rule: &'static str,
    severity: Severity,
    file: &JsFile,
    start: u32,
    end: u32,
    message: impl Into<String>,
) -> Diagnostic {
    let (line, column) = file.doc.position(start as usize);
    let (end_line, end_column) = file.doc.position(end as usize);
    Diagnostic::new(rule, severity, message, &file.path, line, column).end(end_line, end_column)
}

#[cfg(test)]
mod tests {
    use crate::rules::testutil::js::Fixture;

    /// 页面 / 组件 / app.js 全部进入 `Project.scripts`，同一组件被多个页面引用只留一份。
    #[test]
    fn scripts_cover_app_pages_and_components_once() {
        use crate::project::Scope;

        let f = Fixture::new("scripts");
        f.write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write("app.json", r#"{"pages":["pages/a/a","pages/b/b"]}"#)
            .write("app.js", "App({});")
            .write(
                "pages/a/a.json",
                r#"{"usingComponents":{"c":"/components/c/c"}}"#,
            )
            .write("pages/a/a.js", "Page({});")
            .write(
                "pages/b/b.json",
                r#"{"usingComponents":{"c":"/components/c/c"}}"#,
            )
            .write("pages/b/b.js", "Page({});")
            .write("components/c/c.json", r#"{"component":true}"#)
            .write("components/c/c.js", "Component({});");
        let project = f.project();
        let scripts: Vec<(&str, &Scope)> = project
            .scripts
            .iter()
            .map(|script| (script.path.as_str(), &script.scope))
            .collect();
        assert_eq!(
            scripts,
            [
                ("app.js", &Scope::Global),
                (
                    "components/c/c.js",
                    &Scope::Component("components/c/c".into())
                ),
                ("pages/a/a.js", &Scope::Page("pages/a/a".into())),
                ("pages/b/b.js", &Scope::Page("pages/b/b".into())),
            ]
        );
    }
}
