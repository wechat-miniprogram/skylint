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

/// Skyline 组件支持表。
///
/// 来源：/Volumes/2tb-ssd/nebula/Work/skyline-skills/skills/skyline-overview/references/introduction/component-support.md
/// 同步方式：按「完全支持的组件」「基本支持的组件」「Skyline 新增组件」更新本表，
/// 按「暂不支持的组件」更新 `UNSUPPORTED_COMPONENTS`；替代方案为空时填「该组件在 Skyline 下不可用」。
/// 标签一律小写。
const SUPPORTED_COMPONENTS: &[&str] = &[
    "view",
    "cover-view",
    "button",
    "scroll-view",
    "swiper",
    "swiper-item",
    "input",
    "textarea",
    "navigator",
    "map",
    "canvas",
    "radio",
    "radio-group",
    "label",
    "checkbox",
    "checkbox-group",
    "picker",
    "camera",
    "root-portal",
    "form",
    "ad",
    "official-account",
    "live-player",
    "live-pusher",
    "voip-room",
    "icon",
    "slider",
    "switch",
    "share-element",
    "page-container",
    "text",
    "image",
    "cover-image",
    "video",
    "picker-view",
    "rich-text",
    "page-meta",
    "span",
    "sticky-header",
    "sticky-section",
    "list-view",
    "grid-view",
    "nested-scroll-header",
    "nested-scroll-body",
    "draggable-sheet",
    "snapshot",
    "tap-gesture-handler",
    "double-tap-gesture-handler",
    "long-press-gesture-handler",
    "pan-gesture-handler",
    "scale-gesture-handler",
    "horizontal-drag-gesture-handler",
    "vertical-drag-gesture-handler",
    "force-press-gesture-handler",
];

/// Skyline 下不可用的组件及替代方案。来源同 `SUPPORTED_COMPONENTS`。
const UNSUPPORTED_COMPONENTS: &[(&str, &str)] = &[
    ("web-view", r#"该页面配置 `"renderer": "webview"`"#),
    ("movable-area", "手势 + worklet 动画方案"),
    ("movable-view", "手势 + worklet 动画方案"),
    ("editor", "该组件在 Skyline 下不可用"),
    ("progress", "该组件在 Skyline 下不可用"),
    ("match-media", "该组件在 Skyline 下不可用"),
    // 知识库 component-support.md:63 把该组件拼成 `keyboard-accessary`（官方名为
    // keyboard-accessory，developers.weixin.qq.com/miniprogram/dev/component/keyboard-accessory.html），
    // 我们按官方名收录，否则真实使用永远命中不了。
    (
        "keyboard-accessory",
        "input 的 worklet:onkeyboardheightchange 回调",
    ),
    ("navigation-bar", "Skyline 只能用自定义导航"),
    ("xr-frame", "该组件在 Skyline 下不可用"),
];

/// 组件是否属于内置组件（用于区分自定义组件）。
pub(crate) fn is_builtin(tag: &str) -> bool {
    SUPPORTED_COMPONENTS.contains(&tag)
}

/// 黑名单命中才返回替代方案；白名单与未知自定义组件都不报。
pub(crate) fn unsupported_replacement(tag: &str) -> Option<&'static str> {
    if is_builtin(tag) {
        return None;
    }
    UNSUPPORTED_COMPONENTS
        .iter()
        .find(|(name, _)| *name == tag)
        .map(|(_, replacement)| *replacement)
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

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{SUPPORTED_COMPONENTS, UNSUPPORTED_COMPONENTS};

    #[test]
    fn component_tables_have_unique_keys() {
        let mut supported = HashSet::new();
        for tag in SUPPORTED_COMPONENTS {
            assert!(supported.insert(*tag), "重复的受支持组件：{tag}");
        }
        let mut unsupported = HashSet::new();
        for (tag, _) in UNSUPPORTED_COMPONENTS {
            assert!(unsupported.insert(*tag), "重复的不支持组件：{tag}");
            assert!(!supported.contains(tag), "组件同时出现在两张表：{tag}");
        }
    }
}
