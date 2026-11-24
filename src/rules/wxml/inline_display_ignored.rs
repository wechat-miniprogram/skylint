//! `skyline/inline-display-ignored`：非文本元素上的 `display: inline` 在 Skyline 不生效。
//!
//! 实测（同一探针页两端各跑一次）：
//!
//! - WebView：`<view style="display: inline">` 真的变成行内盒——盒子收缩成只有文字宽的一行；
//!   兄弟 `<view>` 是块级，仍然另起一行；
//! - Skyline：`display: inline` 被忽略，元素仍是块级盒——宽度撑满父级、后面的兄弟另起一行。
//!
//! 也就是说「靠 `display: inline` 让若干块级元素并排」的写法迁到 Skyline 后会全部竖排。
//! 并排要换成 flex 布局；需要行内文字用 `<text>` 承载。

use crate::diag::{Diagnostic, Severity};
use crate::input::wxml::WxmlElement;
use crate::matcher::Tree;
use crate::project::Project;
use crate::rules::wxml::{
    ElementSite, element_diagnostic, flex_container, for_each_element, is_out_of_flow,
    resolved_property_with,
};
use crate::rules::{Rule, RuleMeta, VisualCase, VisualExpect};

pub const ID: &str = "skyline/inline-display-ignored";

const MESSAGE: &str = "`display: inline` 在 Skyline 下对非文本元素不生效：它仍是块级盒（独占一行、宽度撑满），WebView 会按行内盒排";

const HELP: &str = "并排改 flex 布局（`display: flex` + `align-items`）；行内文字用 `<text>` 承载；`display: inline-block` 两端都按块级盒处理，也不要用";

/// 文本节点容器：`<text>` / `<span>` 不是本规则的对象。
const TEXT_CONTAINERS: [&str; 2] = ["text", "span"];

pub struct InlineDisplayIgnored;

/// 视觉用例：同一段 WXML/WXSS 两端各渲一次，看 `#target` 的几何与像素。
///
/// 两个 `display: inline` 的 `<view>`：WebView 都收缩成文字宽并排在同一行；
/// Skyline 忽略这条声明，两个都是撑满父宽的块级盒、各占一行。
static VISUAL_CASES: &[VisualCase] = &[VisualCase {
    wxml: r#"<view class="row"><view class="a" id="target">甲</view><view class="b">乙</view></view>"#,
    wxss: ".row { padding: 10px; }\n.a { display: inline; background: #cde; }\n.b { display: inline; background: #edc; }\n",
    js: None,
    expect: VisualExpect::Differ,
    note: "WebView 的 `display: inline` 让 `.a`、`.b` 都收缩成文字宽并排在同一行；Skyline 忽略它，两个都撑满父宽、各占一行。规则对 `.a`、`.b` 各报一条，属预期。整页模式，证据强度弱于元素模式",
    target: "",
}];

impl Rule for InlineDisplayIgnored {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Warning,
            description: "非文本元素上的 `display: inline` 在 Skyline 被忽略，元素仍按块级盒布局",
        }
    }

    fn visual_cases(&self) -> &'static [VisualCase] {
        VISUAL_CASES
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
    if TEXT_CONTAINERS.contains(&element.tag.as_str()) {
        return;
    }
    if !declares_inline(project, tree, element) {
        return;
    }
    // 脱流元素与 flex 子项在两端都被块化（`inline` 本来就无效），不构成差异。
    if is_out_of_flow(project, tree, element) {
        return;
    }
    if let Some(parent) = tree.parent_of(element)
        && flex_container(project, tree, parent)
    {
        return;
    }
    out.push(
        element_diagnostic(ID, doc, element, doc.path(), Severity::Warning, MESSAGE).help(HELP),
    );
}

/// 元素自己是否声明了 `display: inline`。
fn declares_inline(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> bool {
    match resolved_property_with(project, tree, element, "display") {
        (Some(resolved), false) => resolved.value.trim().eq_ignore_ascii_case("inline"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{ID, InlineDisplayIgnored, VISUAL_CASES};
    use crate::rules::testutil::wxml::{Fixture, check_rule};

    fn count(wxml: &str, wxss: &str) -> usize {
        let fixture = Fixture::page("inline-display", wxml, wxss);
        check_rule(&fixture, &InlineDisplayIgnored)
            .iter()
            .filter(|diagnostic| diagnostic.rule == ID)
            .count()
    }

    #[test]
    fn inline_view_is_reported() {
        assert_eq!(
            count(
                r#"<view class="row"><view class="a">甲</view><view class="b">乙</view></view>"#,
                ".a { display: inline; }"
            ),
            1
        );
    }

    #[test]
    fn text_is_skipped() {
        assert_eq!(
            count(
                r#"<view class="row"><text class="a">甲</text></view>"#,
                ".a { display: inline; }"
            ),
            0
        );
    }

    #[test]
    fn flex_child_is_skipped() {
        assert_eq!(
            count(
                r#"<view class="row"><view class="a">甲</view></view>"#,
                ".row { display: flex; } .a { display: inline; }"
            ),
            0
        );
    }

    #[test]
    fn inline_block_is_skipped() {
        assert_eq!(
            count(
                r#"<view class="row"><view class="a">甲</view></view>"#,
                ".a { display: inline-block; }"
            ),
            0
        );
    }

    #[test]
    fn absolute_is_skipped() {
        assert_eq!(
            count(
                r#"<view class="row"><view class="a">甲</view></view>"#,
                ".a { display: inline; position: absolute; }"
            ),
            0
        );
    }

    /// 视觉用例的同一份 WXML/WXSS 必须真的能触发本规则。
    #[test]
    fn visual_case_triggers_rule() {
        let case = &VISUAL_CASES[0];
        assert!(count(case.wxml, case.wxss) >= 1, "视觉用例未触发规则");
    }
}
