//! `skyline/image-text-align`：`<image>` 在 Skyline 是块级盒，父级的 `text-align`
//! 不再居中它；WebView 下 `<image>` 是行内元素，会被父级 `text-align` 居中。
//!
//! 实测（`account/account-ext/pages/confirm-phone`，同代码两端各跑一次）：`.box` 写
//! `text-align: center`、头像 `.avatar { width: 150rpx; height: 150rpx }` 没有任何
//! margin——WebView 里头像水平居中，Skyline 里紧贴左边界。Skyline 的 `<image>` 独占
//! 一行、`text-align` 对它无效。
//!
//! 不报的情况：
//!
//! - 父级不是 flex 容器且 `text-align` 是 `left` / `start`：两端都靠左；
//! - 自己写了水平 `auto` 外边距（`margin: 0 auto`）：Skyline 的块级盒靠 auto 居中；
//! - 自身显式 `display: block` / `inline-block` / `flex` 等：WebView 也不再按行内居中；
//! - 作为 flex 子项：两端都按 flex item 排，父级 `text-align` 两边都不参与；
//! - 脱流（`position: absolute | fixed`）：`text-align` 本来就不影响；
//! - RTL / 竖排：`text-align` 到物理方向的映射两端没实测，不判；
//! - 模板定义体与 `<include>` 片段：祖先链不全，判不出父级对齐。

use crate::diag::{Diagnostic, Severity};
use crate::input::wxml::WxmlElement;
use crate::matcher::Tree;
use crate::project::Project;
use crate::rules::wxml::style::{
    has_exotic_flow, inherited_text_align, inline_box_in_skyline, is_initial_value,
};
use crate::rules::wxml::{
    ElementSite, element_diagnostic, flex_container, for_each_element, is_out_of_flow,
    resolved_property_with,
};
use crate::rules::{Rule, RuleMeta};

pub const ID: &str = "skyline/image-text-align";

const HELP: &str =
    "给图片加 `margin: 0 auto`、或把外层容器改成 flex + `justify-content: center`；两端都会居中";

pub struct ImageTextAlign;

impl Rule for ImageTextAlign {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Warning,
            description: "父级靠 `text-align` 居中 `<image>`：WebView 里图片是行内元素会被居中，Skyline 里是块级盒、居中失效",
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
    if element.tag != "image" || doc.fragment {
        return;
    }
    if !inline_box_in_skyline(project, tree, element) || is_out_of_flow(project, tree, element) {
        return;
    }
    if let Some(parent) = tree.parent_of(element)
        && flex_container(project, tree, parent)
    {
        return;
    }
    if has_exotic_flow(project, tree, element) {
        return;
    }
    let (auto_left, auto_right) = auto_margins(project, tree, element);
    if auto_left && auto_right {
        return;
    }
    let align = inherited_text_align(project, tree, element);
    let message = if matches!(align.as_str(), "center" | "right" | "end") {
        format!(
            "`<image>` 靠父级 `text-align: {align}` 居中：Skyline 把图片当块级盒，这条对图片不再生效"
        )
    } else if auto_left || auto_right {
        "`<image>` 写了单侧 `auto` 外边距：WebView 里图片是行内元素、`auto` 不计，Skyline 里是块级盒会把图片推走".to_string()
    } else {
        return;
    };

    out.push(
        element_diagnostic(ID, doc, element, doc.path(), Severity::Warning, message).help(HELP),
    );
}

/// 图片自身的水平外边距写成 `auto` 的情况：两侧都是 `auto` 时两端都居中，不报；
/// 只有单侧是 `auto` 时两端位置不同（WebView 行内元素不计 `auto`）。
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

/// 简写 `margin` 里是否有 `auto`（`0 auto` / `10rpx auto`）。
fn has_auto_component(value: &str) -> bool {
    value.split_whitespace().any(|part| part == "auto")
}

#[cfg(test)]
mod tests {
    use super::{ID, ImageTextAlign};
    use crate::rules::testutil::wxml::{Fixture, check_rule};

    fn count(wxml: &str, wxss: &str) -> usize {
        let fixture = Fixture::page("image-align", wxml, wxss);
        check_rule(&fixture, &ImageTextAlign)
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
    fn text_centered_child_does_not_shadow_image() {
        assert_eq!(
            count(
                r#"<view class="box"><text class="t">说明</text><image class="avatar" src="/a.png"/></view>"#,
                ".box { text-align: center; } .t { text-align: center; }"
            ),
            1
        );
    }
}
