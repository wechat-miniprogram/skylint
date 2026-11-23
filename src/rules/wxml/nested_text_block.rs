//! `skyline/nested-text-block`：嵌套在 `<text>` 里的 `<text>` 写 `display: block` 不生效。
//!
//! 实测（同一探针页两端各跑一次）：
//!
//! - WebView：`<text>` 内部的 inline 上下文里，子 `<text>` 声明 `display: block` 会真的
//!   断成独立一行（`外层A<text display:block>内层</text>外层C` 渲染成三行）；
//! - Skyline：文本节点内部只按 inline 流排版，`display` 被忽略，仍然是一行。
//!
//! 想让一段文字单独占一行时，WebView 下写 `display: block` 是可行的老写法，迁到 Skyline
//! 后必须改成同级 `<text>`（或换行符 + `white-space: pre-line`），否则会挤成一行。

use crate::diag::{Diagnostic, Severity};
use crate::project::Project;
use crate::rules::wxml::{
    ElementSite, element_diagnostic, for_each_element, resolved_property_with,
};
use crate::rules::{Rule, RuleMeta};

pub const ID: &str = "skyline/nested-text-block";

const MESSAGE: &str = "嵌套在 `<text>` 里的 `<text>` 写了 `display: block`：WebView 会断成独立一行，Skyline 忽略这条声明";

const HELP: &str =
    "把这段改成与外层 `<text>` 同级的 `<text>`；或在父级用 `white-space: pre-line` 配合换行符";

/// 文本节点容器：它们内部的 `<text>` 两端都是 inline。
const TEXT_CONTAINERS: [&str; 2] = ["text", "span"];

pub struct NestedTextBlock;

impl Rule for NestedTextBlock {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Warning,
            description: "嵌套 `<text>` 上的 `display: block` 只在 WebView 生效：Skyline 的文本内部只走 inline 流",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        for_each_element(project, self.target(), |site| {
            check_element(site, &mut out);
        });
        out
    }
}

fn check_element(site: &ElementSite<'_, '_>, out: &mut Vec<Diagnostic>) {
    let (project, doc, tree, element) = (site.project, site.doc, site.tree, site.element);
    if !TEXT_CONTAINERS.contains(&element.tag.as_str()) {
        return;
    }
    if !site
        .ancestors
        .iter()
        .any(|ancestor| TEXT_CONTAINERS.contains(&ancestor.tag.as_str()))
    {
        return;
    }
    match resolved_property_with(project, tree, element, "display") {
        (_, true) => {}
        (Some(resolved), false) => {
            if resolved.value.trim().eq_ignore_ascii_case("block") {
                out.push(
                    element_diagnostic(ID, doc, element, doc.path(), Severity::Warning, MESSAGE)
                        .help(HELP),
                );
            }
        }
        (None, false) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{ID, NestedTextBlock};
    use crate::rules::testutil::wxml::{Fixture, check_rule};

    fn count(wxml: &str, wxss: &str) -> usize {
        let fixture = Fixture::page("nested-text-block", wxml, wxss);
        check_rule(&fixture, &NestedTextBlock)
            .iter()
            .filter(|diagnostic| diagnostic.rule == ID)
            .count()
    }

    #[test]
    fn nested_block_is_reported() {
        assert_eq!(
            count(
                r#"<view><text class="outer">上层<text class="inner">下层</text></text></view>"#,
                ".inner { display: block; }"
            ),
            1
        );
    }

    #[test]
    fn nested_inline_is_skipped() {
        assert_eq!(
            count(
                r#"<view><text class="outer">上层<text class="inner">下层</text></text></view>"#,
                ".inner { display: inline; }"
            ),
            0
        );
    }

    #[test]
    fn top_level_block_is_skipped() {
        assert_eq!(
            count(
                r#"<view><text class="t">文字</text></view>"#,
                ".t { display: block; }"
            ),
            0
        );
    }
}
