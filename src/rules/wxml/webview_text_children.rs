//! `<text>` 里的元素子节点在 WebView 下被整块丢弃。
//!
//! WebView 的 `wx-text` 用 `TextTemplateEngine` 挂一个「伪 slot」过滤子节点
//! （`wxapplib/src/wx-component/src/wx-text/wx-text.js:15-62` 的 `TextPseudoSlot.update`），
//! 只保留三类：文本节点、`__wxElement.is === 'wx-text'` 的子元素（即嵌套 `<text>`）、
//! 以及文档片段（递归进去）。其余元素——`<view>` / `<image>` / `<span>` / 任何自定义
//! 组件——既不渲染也不报错，直接消失。
//!
//! Skyline 没有这层过滤：`TextShadowNode` 把非文本子节点转成 WidgetSpan
//! （`skyline/shadow/shadow_node.cc:205-209,279-301`），元素照常显示。所以
//! 「`<text>` 包着组件」是只在 Skyline 成立的写法。
//!
//! 真实修复形态（`weapp-novel-plugin_1` 的 `feat-glass-easel-webview: c8944057`）：
//! 段评气泡与 `inline-content` 从 `<text>` 里挪出来，按 `renderer === 'skyline'` 分两支。

use crate::diag::{Diagnostic, Severity};
use crate::input::wxml::{WxmlElement, WxmlNode};
use crate::project::Project;
use crate::rules::{Rule, RuleMeta, Target};

use super::{element_diagnostic, visit, walk};

pub const ID: &str = "webview/text-children";

pub struct TextChildren;

impl Rule for TextChildren {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Error,
            description: "`<text>` 里的元素子节点在 WebView 下被丢弃（只保留文本与嵌套 text）",
        }
    }

    fn target(&self) -> Target {
        Target::Webview
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit(project, |doc, out| {
            walk(doc, |element, _| {
                if !element.tag.eq_ignore_ascii_case("text") || element.skyline_only() {
                    return;
                }
                let dropped = dropped_children(element);
                if dropped.is_empty() {
                    return;
                }
                out.push(
                    element_diagnostic(
                        ID,
                        doc,
                        element,
                        doc.path(),
                        Severity::Error,
                        format!(
                            "`<text>` 里的 {} 在 WebView 下会被丢弃（只保留文本与嵌套 `<text>`），Skyline 下才会渲染",
                            dropped.join("、")
                        ),
                    )
                    .help("把元素挪到 `<text>` 外，按 `wx:if=\"{{renderer === 'skyline'}}\"` 分成两支：Skyline 保留文本包装，WebView 直接用元素"),
                );
            });
        })
    }
}

/// `<text>` 下会被 WebView 丢掉的直接子元素（去重、保持出现顺序）。
///
/// 只看直接子节点：被丢的那棵子树整块消失，逐层报只是噪音。带 renderer 守卫的子节点
/// 只会在 Skyline 分支里出现，WebView 下本来就不渲染，不算缺陷。
fn dropped_children(element: &WxmlElement) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    for node in &element.children {
        let WxmlNode::Element(child) = node else {
            continue;
        };
        if child.tag.eq_ignore_ascii_case("text") || child.renderer_guarded() {
            continue;
        }
        let tag = format!("`<{}>`", child.tag);
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    tags
}

#[cfg(test)]
mod tests {
    use super::TextChildren;
    use crate::rules::testutil::wxml::{check_rule, project};

    #[test]
    fn custom_component_inside_text_is_reported() {
        let diagnostics = check_rule(
            &project(
                "webview/text-children",
                "<view><text id=\"t\"><inline-content/><seg-comment-badge/></text></view>",
            ),
            &TextChildren,
        );
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("<inline-content>"));
        assert!(diagnostics[0].message.contains("<seg-comment-badge>"));
    }

    #[test]
    fn nested_text_is_fine() {
        let diagnostics = check_rule(
            &project(
                "webview/text-children-nested",
                "<view><text>a<text class=\"b\">b</text></text></view>",
            ),
            &TextChildren,
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn plain_text_is_fine() {
        let diagnostics = check_rule(
            &project("webview/text-children-plain", "<view><text>a</text></view>"),
            &TextChildren,
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn renderer_guard_silences() {
        let diagnostics = check_rule(
            &project(
                "webview/text-children-guard",
                "<view><block wx:if=\"{{renderer === 'skyline'}}\"><text><inline-content/></text></block><block wx:else><inline-content/></block></view>",
            ),
            &TextChildren,
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn children_behind_wx_if_still_count() {
        let diagnostics = check_rule(
            &project(
                "webview/text-children-if",
                "<view><text><view wx:if=\"{{ok}}\"/></text></view>",
            ),
            &TextChildren,
        );
        assert_eq!(diagnostics.len(), 1);
    }
}
