//! `<text overflow="ellipsis" max-lines="N">` 里的硬换行末行不画省略号。
//!
//! 省略号落在「末行」时，是否在**硬换行**（`\n`）结束的那一行也画省略号由开关
//! `show_hard_line_break_ellipsis` 决定，默认关闭
//! （`common/feature_toggle.cc:20-22` 静态 false，`feature_toggle.h:17` 登记，
//! `widgets/text.cc:435-437` 只在开启时 `SetShowHardLineBreakEllipsis(true)`）。
//! 关闭时 WebView 会给省略号、Skyline 不给，用户看到的就是「截断了但没省略号」。
//!
//! 只判静态文本里的 `\n`：换行落在截断末行之外时没有差异，而落在哪里取决于宽度、
//! 字体与 `max-lines`，静态判不出来——所以这条是提示，不是错误。

use crate::diag::{Diagnostic, Severity};
use crate::input::wxml::{WxmlDoc, WxmlElement, WxmlNode};
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};

use super::structure::find_attr;
use super::{element_diagnostic, visit, walk_skyline};

pub const ID: &str = "skyline/text-hard-break-ellipsis";

pub struct TextHardBreakEllipsis;

impl Rule for TextHardBreakEllipsis {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Info,
            description: "多行省略里含硬换行时，Skyline 默认不给末行省略号",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit(project, |doc, out| {
            walk_skyline(doc, |element, _ancestors| {
                check_element(doc, element, out);
            });
        })
    }
}

fn check_element(doc: &WxmlDoc, element: &WxmlElement, out: &mut Vec<Diagnostic>) {
    if !element.tag.eq_ignore_ascii_case("text") {
        return;
    }
    // 触发条件是这对属性组合：只有 overflow 是 ellipsis 且有行数上限时才画省略号。
    if find_attr(element, "overflow")
        .and_then(|attr| attr.value.as_deref())
        .is_none_or(|value| !value.eq_ignore_ascii_case("ellipsis"))
    {
        return;
    }
    let Some(max_lines) = find_attr(element, "max-lines") else {
        return;
    };
    // 动态上限视为已给（与 `component-attribute` 的口径一致）；静态 `0` 表示不限制。
    if max_lines.is_static
        && max_lines
            .value
            .as_deref()
            .is_some_and(|value| value.trim() == "0")
    {
        return;
    }
    if !has_hard_break(&element.children) {
        return;
    }
    out.push(
        element_diagnostic(
            ID,
            doc,
            element,
            doc.path(),
            Severity::Info,
            "这段文本含硬换行：Skyline 默认不绘制硬换行末行的省略号，看不到截断提示",
        )
        .help("把 `\n` 换成独立的行（每行一个 `<text>`），或改用 CSS 的 `-webkit-line-clamp` 链路并实测末行"),
    );
}

/// 子树里是否有硬换行。嵌套 `<text>` 并入父级同一段排版，所以要递归到底。
fn has_hard_break(nodes: &[WxmlNode]) -> bool {
    nodes.iter().any(|node| match node {
        WxmlNode::Text(text) => text.text.contains('\n'),
        WxmlNode::Element(element) => has_hard_break(&element.children),
        WxmlNode::Comment => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::testutil::wxml;

    fn hits(source: &str) -> Vec<String> {
        let fixture = wxml::project("text-hard-break", source);
        wxml::check_rule(&fixture, &TextHardBreakEllipsis)
            .into_iter()
            .map(|diagnostic| diagnostic.message)
            .collect()
    }

    #[test]
    fn hard_break_inside_ellipsis_is_reported() {
        let source = "<text overflow=\"ellipsis\" max-lines=\"2\">第一行\n第二行</text>";
        assert_eq!(hits(source).len(), 1);
    }

    #[test]
    fn single_line_text_is_fine() {
        let source = "<text overflow=\"ellipsis\" max-lines=\"2\">让文字换行的是宽度</text>";
        assert!(hits(source).is_empty());
    }

    #[test]
    fn without_ellipsis_there_is_no_missing_ellipsis() {
        let source = "<text max-lines=\"2\">第一行\n第二行</text>";
        assert!(hits(source).is_empty());
    }
}
