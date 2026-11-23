//! `skyline/image-block-layout`：`<image>` 在 Skyline 是**块级盒**、在 WebView 是
//! **inline-block**，于是所有「行内级」才成立的写法在 Skyline 静默失效。
//!
//! 实测（页面 A，同代码两端各跑一次）：父级 `.box`
//! 写 `text-align: center`、头像 `.avatar { width: 150rpx; height: 150rpx }` 没有
//! 任何 margin——WebView 里头像水平居中，Skyline 里紧贴左边界。同页 `<text>` 的
//! 探针实测给出同一结论：Skyline 把这类宿主的 `display: inline` / `inline-block`
//! 一律按块级处理（`display: inline` 被忽略的情形见 `skyline/inline-display-ignored`）。
//!
//! 本规则收四类「因块级化而失效」的声明：
//!
//! 1. 父级 `text-align: center | right | end`：WebView 靠行内对齐居中图片，Skyline 不居中；
//! 2. 单侧 `auto` 水平外边距：WebView 里行内盒子的 `auto` 算成 0，Skyline 里块级盒被推走；
//! 3. `vertical-align`：只在行内级盒子之间有参照，Skyline 里图片不参与行盒，声明无效；
//! 4. 祖先的 `text-indent`：只缩进块容器的首行，Skyline 里图片自成一行、不被缩进。
//!
//! 不在这里重复「并列图片各自换行」——那是 [`super::inline_run`] 的判据。
//!
//! 不报的情况：
//!
//! - 两侧都 `auto` 外边距（两端都居中）；
//! - 父级是 flex 容器：两端都按 flex item 排，行内对齐不参与；
//! - 脱流（`position: absolute | fixed`）：行内对齐本来就不影响；
//! - `display` 是两端都成块的值（`block` / `flex` / `grid` / …）。注意
//!   `inline-block` **不在**这个名单里：Skyline 也把它按块级处理，写它并不能恢复行内；
//! - RTL / 竖排上下文下的对齐判定（映射未实测）；
//! - 模板定义体与 `<include>` 片段：祖先链不全。

use crate::diag::{Diagnostic, Severity};
use crate::input::wxml::WxmlElement;
use crate::matcher::Tree;
use crate::project::Project;
use crate::rules::wxml::style::{has_exotic_flow, inherited_text_align, is_initial_value, is_zero};
use crate::rules::wxml::{
    ElementSite, element_diagnostic, flex_container, for_each_element, is_out_of_flow,
    resolved_property_with,
};
use crate::rules::{Rule, RuleMeta, VisualCase, VisualExpect};

pub const ID: &str = "skyline/image-block-layout";

const HELP: &str = "把图片放进 flex 容器（`display: flex` + `justify-content` / `align-items`），或显式 `margin: 0 auto`；需要和文字同排时给外层 `display: flex` 并用 `align-items` 对齐";

/// 两端都成块的 `display` 取值（`inline-block` 不算：Skyline 也按块级处理）。
const BLOCK_IN_BOTH: [&str; 7] = [
    "block",
    "flex",
    "inline-flex",
    "grid",
    "inline-grid",
    "flow-root",
    "none",
];

pub struct ImageBlockLayout;

/// 视觉用例：同一段 WXML/WXSS 两端各渲一次，看 `#target` 的几何与像素。
///
/// 父级 `text-align: center`：WebView 靠行内对齐把 inline-block 的 `<image>` 居中，
/// Skyline 的 `<image>` 是块级盒，行内对齐不参与，紧贴左边。
static VISUAL_CASES: &[VisualCase] = &[VisualCase {
    wxml: r#"<view class="box"><image class="avatar" id="target"/></view>"#,
    wxss: ".box { padding: 10px; text-align: center; }\n.avatar { width: 75px; height: 75px; background: #cde; }\n",
    js: None,
    expect: VisualExpect::Differ,
    note: "父级 text-align:center 时 WebView 把 <image> 行内居中；Skyline 的 <image> 是块级盒，紧贴左边",
    target: "",
}];

impl Rule for ImageBlockLayout {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Warning,
            description: "`<image>` 在 Skyline 是块级盒、WebView 是 inline-block：靠行内对齐居中、单侧 auto 外边距、`vertical-align`、祖先的 `text-indent` 在 Skyline 下都不再生效",
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
    if element.tag != "image" || doc.fragment {
        return;
    }
    if block_in_both(project, tree, element) || is_out_of_flow(project, tree, element) {
        return;
    }
    let in_flex = tree
        .parent_of(element)
        .is_some_and(|parent| flex_container(project, tree, parent));
    let (auto_left, auto_right) = auto_margins(project, tree, element);

    let mut hits: Vec<String> = Vec::new();
    if !in_flex && !(auto_left && auto_right) {
        let align = inherited_text_align(project, tree, element);
        if !has_exotic_flow(project, tree, element)
            && matches!(align.as_str(), "center" | "right" | "end")
        {
            hits.push(format!("父级 `text-align: {align}`"));
        } else if auto_left || auto_right {
            hits.push("单侧 `auto` 外边距".to_string());
        }
    }
    if !in_flex && let Some(value) = meaningful_vertical_align(project, tree, element) {
        hits.push(format!("`vertical-align: {value}`"));
    }
    if !in_flex && let Some(value) = inherited_text_indent(project, tree, element) {
        hits.push(format!("祖先 `text-indent: {value}`"));
    }
    if hits.is_empty() {
        return;
    }

    out.push(
        element_diagnostic(
            ID,
            doc,
            element,
            doc.path(),
            Severity::Warning,
            "`<image>` 在 Skyline 是块级盒（WebView 是 inline-block），这些写法只对行内级盒子生效，Skyline 下不会再生效",
        )
        .help(format!("这里见到的是 {}。{HELP}", hits.join("、"))),
    );
}

/// `display` 是两端都成块的取值。`inline-block` 不算：Skyline 同样按块级处理。
fn block_in_both(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> bool {
    match resolved_property_with(project, tree, element, "display") {
        (_, true) => true,
        (Some(resolved), false) => {
            BLOCK_IN_BOTH.contains(&resolved.value.trim().to_ascii_lowercase().as_str())
        }
        (None, false) => false,
    }
}

/// 图片自身的水平外边距写成 `auto` 的情况。
fn auto_margins(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> (bool, bool) {
    let side = |property: &str| match resolved_property_with(project, tree, element, property) {
        (Some(resolved), false) => {
            !is_initial_value(&resolved.value) && has_auto_component(&resolved.value)
        }
        _ => false,
    };
    let shorthand: Vec<String> = resolved_property_with(project, tree, element, "margin")
        .0
        .filter(|resolved| !is_initial_value(&resolved.value))
        .map(|resolved| {
            resolved
                .value
                .split_whitespace()
                .map(|part| part.to_ascii_lowercase())
                .collect()
        })
        .unwrap_or_default();
    // CSS 简写展开：1 值四边、2 值上下/左右、3 值上/左右/下、4 值上/右/下/左。
    let picked = |pick: usize| shorthand.get(pick).is_some_and(|part| part == "auto");
    let (shorthand_left, shorthand_right) = match shorthand.len() {
        1 => (picked(0), picked(0)),
        2 | 3 => (picked(1), picked(1)),
        _ => (picked(3), picked(1)),
    };
    (
        side("margin-left") || shorthand_left,
        side("margin-right") || shorthand_right,
    )
}

fn has_auto_component(value: &str) -> bool {
    value
        .split_whitespace()
        .any(|part| part.eq_ignore_ascii_case("auto"))
}

/// `vertical-align` 写了非初始值：`baseline` 之外的值都指望「与同一行的内容对齐」，
/// Skyline 里图片不参与行盒，声明无效。
fn meaningful_vertical_align(
    project: &Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
) -> Option<String> {
    let (resolved, undecidable) = resolved_property_with(project, tree, element, "vertical-align");
    if undecidable {
        return None;
    }
    let value = resolved?.value.trim().to_ascii_lowercase();
    let inert = is_initial_value(&value) || value == "baseline" || is_zero(&value);
    (!inert).then_some(value)
}

/// 最近的祖先 `text-indent`（可继承）：WebView 里它缩进父级首行、把行内的图片一起带走，
/// Skyline 里图片自成一行，缩进不落在它身上。
fn inherited_text_indent(
    project: &Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
) -> Option<String> {
    let mut current = tree.parent_of(element);
    while let Some(node) = current {
        match resolved_property_with(project, tree, node, "text-indent") {
            (Some(resolved), false) => {
                let value = resolved.value.trim().to_ascii_lowercase();
                return (!is_initial_value(&value) && !is_zero(&value)).then_some(value);
            }
            (_, true) => return None,
            (None, false) => current = tree.parent_of(node),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{ID, ImageBlockLayout, VISUAL_CASES};
    use crate::rules::testutil::wxml::{Fixture, check_rule};

    fn count(wxml: &str, wxss: &str) -> usize {
        let fixture = Fixture::page("image-block", wxml, wxss);
        check_rule(&fixture, &ImageBlockLayout)
            .iter()
            .filter(|diagnostic| diagnostic.rule == ID)
            .count()
    }

    #[test]
    fn image_centered_by_parent_text_align_is_reported() {
        assert_eq!(
            count(
                r#"<view class="box"><image class="avatar" src="/a.png"/></view>"#,
                ".box { text-align: center; } .avatar { width: 150rpx; height: 150rpx; }"
            ),
            1
        );
    }

    #[test]
    fn left_aligned_parent_is_skipped() {
        assert_eq!(
            count(
                r#"<view class="box"><image class="avatar" src="/a.png"/></view>"#,
                ".box { text-align: left; } .avatar { width: 150rpx; }"
            ),
            0
        );
    }

    #[test]
    fn auto_margin_is_skipped() {
        assert_eq!(
            count(
                r#"<view class="box"><image class="avatar" src="/a.png"/></view>"#,
                ".box { text-align: center; } .avatar { margin: 0 auto; }"
            ),
            0
        );
    }

    #[test]
    fn single_side_auto_margin_is_reported() {
        assert_eq!(
            count(
                r#"<view class="box"><image class="avatar" src="/a.png"/></view>"#,
                ".box { text-align: center; } .avatar { margin-left: auto; }"
            ),
            1
        );
    }

    #[test]
    fn flex_parent_is_skipped() {
        assert_eq!(
            count(
                r#"<view class="box"><image class="avatar" src="/a.png"/></view>"#,
                ".box { text-align: center; display: flex; justify-content: center; }"
            ),
            0
        );
    }

    #[test]
    fn explicit_block_is_skipped() {
        assert_eq!(
            count(
                r#"<view class="box"><image class="avatar" src="/a.png"/></view>"#,
                ".box { text-align: center; } .avatar { display: block; }"
            ),
            0
        );
    }

    #[test]
    fn inline_block_display_is_still_reported() {
        // Skyline 把 inline-block 也按块级处理，写它并不能恢复行内。
        assert_eq!(
            count(
                r#"<view class="box"><image class="avatar" src="/a.png"/></view>"#,
                ".box { text-align: center; } .avatar { display: inline-block; }"
            ),
            1
        );
    }

    #[test]
    fn out_of_flow_image_is_skipped() {
        assert_eq!(
            count(
                r#"<view class="box"><image class="avatar" src="/a.png"/></view>"#,
                ".box { text-align: center; } .avatar { position: absolute; left: 0; }"
            ),
            0
        );
    }

    #[test]
    fn rtl_context_is_skipped() {
        assert_eq!(
            count(
                r#"<view class="box"><image class="avatar" src="/a.png"/></view>"#,
                ".box { text-align: center; direction: rtl; }"
            ),
            0
        );
    }

    #[test]
    fn right_aligned_parent_is_reported() {
        assert_eq!(
            count(
                r#"<view class="box"><image class="avatar" src="/a.png"/></view>"#,
                ".box { text-align: right; }"
            ),
            1
        );
    }

    #[test]
    fn non_image_tag_is_skipped() {
        assert_eq!(
            count(
                r#"<view class="box"><view class="avatar"/></view>"#,
                ".box { text-align: center; }"
            ),
            0
        );
    }

    #[test]
    fn vertical_align_middle_is_reported() {
        assert_eq!(
            count(
                r#"<view class="row"><image class="icon" src="/a.png"/></view>"#,
                ".icon { vertical-align: middle; }"
            ),
            1
        );
    }

    #[test]
    fn vertical_align_baseline_is_skipped() {
        assert_eq!(
            count(
                r#"<view class="row"><image class="icon" src="/a.png"/></view>"#,
                ".icon { vertical-align: baseline; }"
            ),
            0
        );
    }

    #[test]
    fn inherited_text_indent_is_reported() {
        assert_eq!(
            count(
                r#"<view class="para"><image class="icon" src="/a.png"/></view>"#,
                ".para { text-indent: 2em; }"
            ),
            1
        );
    }

    #[test]
    fn zero_text_indent_is_skipped() {
        assert_eq!(
            count(
                r#"<view class="para"><image class="icon" src="/a.png"/></view>"#,
                ".para { text-indent: 0; }"
            ),
            0
        );
    }

    #[test]
    fn text_centered_child_does_not_shadow_image() {
        assert_eq!(
            count(
                r#"<view class="box"><text class="t">说明</text><image class="avatar" src="/a.png"/></view>"#,
                ".box { text-align: center; } .t { text-align: center; }"
            ),
            1
        );
    }

    /// 视觉用例的同一份 WXML/WXSS 必须真的能触发本规则。
    #[test]
    fn visual_case_triggers_rule() {
        let case = &VISUAL_CASES[0];
        assert!(count(case.wxml, case.wxss) >= 1, "视觉用例未触发规则");
    }
}
