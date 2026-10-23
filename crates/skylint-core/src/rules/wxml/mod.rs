//! WXML 规则。
//!
//! 一条规则一个文件：逐元素的局部判定（内联文本、组件白名单、scroll-view type）
//! 与父子结构约束（scroll-view 家族、enable-flex）各占一个文件。它们共用同一套
//! 「页面 + 组件 WXML 遍历」与解析辅助。

pub mod branch;
pub mod component_attribute;
pub mod component_children;
pub mod horizontal_scroll_enable_flex;
pub mod no_inline_text;
pub mod scroll_view_family;
pub mod scroll_view_type;
pub mod share_element_key;
pub mod unsupported_component;

pub use component_attribute::ComponentAttribute;
pub use component_children::ComponentChildren;
pub use horizontal_scroll_enable_flex::HorizontalScrollEnableFlex;
pub use no_inline_text::NoInlineText;
pub use scroll_view_family::ScrollViewFamily;
pub use scroll_view_type::ScrollViewType;
pub use share_element_key::ShareElementKey;
pub use unsupported_component::UnsupportedComponent;

use std::collections::HashSet;
use std::fs;

use glass_easel_template_compiler::parse::Position;

use crate::diag::{Diagnostic, Severity};
use crate::input::wxml::{self, WxmlDoc, WxmlElement};
use crate::project::Project;

/// 页面与组件的 WXML + 被 import / include 的模板，按路径去重。
///
/// 页面 json 显式声明 `"renderer": "webview"` 的整页跳过：那是「这页不迁
/// Skyline」的明确决定（典型是含 `<web-view>` 的页面），对它讲 Skyline 的
/// 限制只是误报。app 级 webview / 页面未表态不跳——那只是还没迁，正是要检查的。
/// 组件的 wxml 没有 renderer 可言，一律检查；模板不属于任何单元，也一律检查。
fn wxml_files(project: &Project) -> impl Iterator<Item = &str> {
    let mut seen = HashSet::new();
    project
        .pages
        .iter()
        .filter(|page| !page.opted_out)
        .filter_map(|page| page.files.wxml.as_deref())
        .chain(
            project
                .components
                .iter()
                .filter_map(|component| component.files.wxml.as_deref()),
        )
        .chain(project.template_files.iter().map(String::as_str))
        .filter(move |path| seen.insert(*path))
}

pub(crate) fn load_doc(project: &Project, path: &str) -> Option<WxmlDoc> {
    let source = fs::read_to_string(project.abs(path)).ok()?;
    wxml::parse(path, &source).ok()
}

/// 组件支持表。数据文件是唯一依据，Rust 侧只做查表。
const COMPONENTS_DATA: &str = include_str!("../data/components.txt");

/// 组件是否属于内置组件（用于区分自定义组件）。
pub(crate) fn is_builtin(tag: &str) -> bool {
    COMPONENTS_DATA
        .lines()
        .skip_while(|line| line.trim() != "[supported]")
        .skip(1)
        .take_while(|line| line.trim() != "[unsupported]")
        .map(str::trim)
        .any(|line| !line.is_empty() && !line.starts_with('#') && line == tag)
}

/// 定位在 WXML 源文本任意一段区间上的诊断。
pub(crate) fn span_diagnostic(
    id: &'static str,
    doc: &WxmlDoc,
    start: Position,
    end: Position,
    path: &str,
    severity: Severity,
    message: impl Into<String>,
) -> Diagnostic {
    let (line, column) = doc.position(start);
    let (end_line, end_column) = doc.position(end);
    Diagnostic::new(id, severity, message, path, line, column).end(end_line, end_column)
}

/// 定位在元素标签起止位置上的诊断。
fn element_diagnostic(
    id: &'static str,
    doc: &WxmlDoc,
    element: &WxmlElement,
    path: &str,
    severity: Severity,
    message: impl Into<String>,
) -> Diagnostic {
    span_diagnostic(
        id,
        doc,
        element.tag_start,
        element.tag_end,
        path,
        severity,
        message,
    )
}

