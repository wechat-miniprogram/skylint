//! `skyline/fixed-offset-missing`：`position: fixed` 的某个轴没写偏移时，两端位置不同。
//!
//! Skyline 把 fixed 节点从原位换成 replacer、把节点本身 `AppendChild` 到页面根。
//! 没有写偏移的那一轴由布局层补：块容器取父级 padding-box 起点
//! （`float-pigment-layout/src/special_positioned.rs:349-353`），而此时的「父级」已经是
//! 页面根；flex 对齐那条分支（`:279-348`）问的也是根的 `justify-content` / `align-self`。
//!
//! WebView 用的是元素在**原容器**里的静态位置，所以只有原位置不等于根起点时才真的错位：
//!
//! - 垂直轴：块容器里元素前面还有流内兄弟（静态位置在兄弟之后，根起点在 0）；
//! - 任一轴：原容器是 flex 且该轴对齐不是起点（居中 / 末端 / space-*）。
//!
//! 水平轴在块容器里不报：块级盒子的静态位置就是 0，与根的起点相同。
//!
//! 已知漏报：flex 父级起点对齐、但父级自己的内容区不在页面原点（被上级居中、带
//! padding）时，两端也会错开一个父级偏移量。静态判父级是否在原点要往上追好几层布局，
//! 噪声比很高，暂不处理，这类靠 L3 截图兜。
//!
//! `absolute` 不走这条规则：它不被搬走，无偏移时的偏差来自 fp 未实现静态位置
//! （wechat-miniprogram/float-pigment#47），修在引擎侧。

use std::collections::HashSet;

use crate::input::wxml::{WxmlElement, WxmlNode};
use crate::matcher::Tree;
use crate::project::Project;
use crate::rules::wxml::{for_each_element, resolved_property_with};
use crate::rules::{Rule, RuleMeta, VisualCase, VisualExpect};
use crate::{Diagnostic, Severity};

const MESSAGE: &str = "`position: fixed` 在这个轴上没写偏移：Skyline 把节点提到页面根后按根的起点定位，WebView 用的是它在原容器里的位置，两端会错位";

const HELP: &str = "补上该轴的偏移——水平居中用 `left: 0; right: 0` 配 `justify-content: center`（或 `left: 50%; transform: translateX(-50%)`），垂直位置写确定的 `top` / `bottom`";

pub struct FixedOffsetMissing;

impl FixedOffsetMissing {
    pub const ID: &'static str = "skyline/fixed-offset-missing";
}

/// 视觉用例：fixed 元素缺垂直偏移，原容器里前面还有流内兄弟。
///
/// WebView 用元素在标题之后的静态位置；Skyline 提到页面根后按根起点定位，两端错开
/// 一个标题的高度。元素贴顶、与标题重叠，差异在第一屏就看得到。
static VISUAL_CASES: &[VisualCase] = &[VisualCase {
    wxml: r#"<view class="page">
  <view class="header">首页</view>
  <view class="bar" id="target">浮动提示</view>
</view>"#,
    wxss: ".page { display: block; padding: 12px; background: #f5f5f5; }\n.header { height: 60px; background: #333; color: #fff; }\n.bar { position: fixed; left: 20px; width: 160px; height: 36px; background: #e54d42; }\n",
    js: None,
    expect: VisualExpect::Differ,
    note: "fixed 缺垂直偏移：Skyline 提到页面根后落在顶部，WebView 留在标题之后的静态位置（相差约一个标题高度）",
    target: "#target",
}];

impl Rule for FixedOffsetMissing {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: FixedOffsetMissing::ID,
            severity: Severity::Warning,
            description: "`position: fixed` 元素没写某轴偏移：Skyline 下按页面根起点定位，WebView 下按它在原容器里的位置",
        }
    }

    fn visual_cases(&self) -> &'static [VisualCase] {
        VISUAL_CASES
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        let mut reported: HashSet<(String, u32, u32)> = HashSet::new();

        for_each_element(project, self.target(), |site| {
            let (doc, tree, element) = (site.doc, site.tree, site.element);
            if fixed(project, tree, element) != Some(true) {
                return;
            }
            if mispositioned(project, tree, element) != Some(true) {
                return;
            }
            let (line, column) = doc.position(element.tag_start);
            if !reported.insert((doc.path().to_string(), line, column)) {
                return;
            }
            out.push(
                Diagnostic::new(
                    Self::ID,
                    Severity::Warning,
                    MESSAGE,
                    doc.path(),
                    line,
                    column,
                )
                .help(site.reach.help(HELP)),
            );
        });
        out
    }
}

/// 元素是否 `position: fixed`。`None` = 判不了（模板定义体 / 选择器超出受限文法）。
fn fixed(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> Option<bool> {
    match resolved_property_with(project, tree, element, "position") {
        (_, true) => None,
        (Some(resolved), false) => Some(resolved.value.trim().eq_ignore_ascii_case("fixed")),
        (None, false) => Some(false),
    }
}

/// 该元素的两端位置会不会不同。
///
/// `None` = 判不了，调用方一律不报。
fn mispositioned(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> Option<bool> {
    if has_inset_shorthand(project, tree, element)? {
        return Some(false);
    }
    let horizontal = axis_unset(project, tree, element, "left", "right")?;
    let vertical = axis_unset(project, tree, element, "top", "bottom")?;
    if !horizontal && !vertical {
        return Some(false);
    }

    // 根元素没有「原容器」，提取前后都在页面根上，不报。
    let parent = tree.parent_of(element)?;
    let flex = flex_parent(project, tree, parent)?;
    let row = flex
        .then(|| flex_direction_row(project, tree, parent))
        .flatten();

    let differs = |axis: Axis| -> Option<bool> {
        let Some(row) = row else {
            // 块容器：静态位置沿垂直轴排在流内兄弟之后，水平轴就是内容区起点。
            return match axis {
                Axis::Vertical => has_preceding_in_flow_sibling(project, tree, parent, element),
                Axis::Horizontal => Some(false),
            };
        };
        let main = matches!(
            (axis, row),
            (Axis::Horizontal, true) | (Axis::Vertical, false)
        );
        Some(alignment(project, tree, parent, element, main)? == Alignment::Away)
    };

    if horizontal && differs(Axis::Horizontal)? {
        return Some(true);
    }
    if vertical && differs(Axis::Vertical)? {
        return Some(true);
    }
    Some(false)
}

/// 某个轴的起点 / 终点偏移都是 auto。`None` = 判不了。
fn axis_unset(
    project: &Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
    start: &str,
    end: &str,
) -> Option<bool> {
    Some(
        declaration_is_auto(project, tree, element, start)?
            && declaration_is_auto(project, tree, element, end)?,
    )
}

/// 没写等同于 `auto`；写了就看取值。
fn declaration_is_auto(
    project: &Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
    property: &str,
) -> Option<bool> {
    match resolved_property_with(project, tree, element, property) {
        (_, true) => None,
        (None, false) => Some(true),
        (Some(resolved), false) => Some(resolved.value.trim().eq_ignore_ascii_case("auto")),
    }
}

/// `inset` / `inset-inline` / `inset-block` 是偏移简写，取值拆解不在本工具判断范围内：
/// 写了任一条就跳过这个元素。
fn has_inset_shorthand(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> Option<bool> {
    for property in ["inset", "inset-inline", "inset-block"] {
        match resolved_property_with(project, tree, element, property) {
            (_, true) => return None,
            (Some(_), false) => return Some(true),
            (None, false) => {}
        }
    }
    Some(false)
}

/// 父级是不是 flex 容器。没写 `display` 时按渲染器的初始值：开
/// `defaultDisplayBlock` 是 block，否则 Skyline 是 flex。
fn flex_parent(project: &Project, tree: &Tree<'_>, parent: &WxmlElement) -> Option<bool> {
    match resolved_property_with(project, tree, parent, "display") {
        (_, true) => None,
        (Some(resolved), false) => Some(crate::rules::wxml::is_flex_value(&resolved.value)),
        (None, false) => Some(!default_block(project, tree)),
    }
}

fn default_block(project: &Project, tree: &Tree<'_>) -> bool {
    project.scope_of(tree.path()).map_or_else(
        || project.app_display_block(),
        |scope| project.renders_as_block(&scope),
    )
}

#[derive(Clone, Copy)]
enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Alignment {
    /// 起点（`normal` / `flex-start` / `start` / `stretch`，含 `space-between`：
    /// 只有一个脱流子项时也排在起点）。
    Start,
    /// 居中 / 末端 / `space-around` / `space-evenly`：静态位置不在根起点上。
    Away,
}

/// flex 容器在该轴上的对齐方式。
///
/// 主轴看 `justify-content`，交叉轴看元素自己的 `align-self`，没写才回落到父级的
/// `align-items`。
fn alignment(
    project: &Project,
    tree: &Tree<'_>,
    parent: &WxmlElement,
    element: &WxmlElement,
    main: bool,
) -> Option<Alignment> {
    let value = if main {
        declared_value(project, tree, parent, "justify-content")?
    } else {
        match declared_value(project, tree, element, "align-self")? {
            Some(value) if !value.eq_ignore_ascii_case("auto") => Some(value),
            _ => declared_value(project, tree, parent, "align-items")?,
        }
    };
    // 没写就用初始值（主轴 `flex-start`、交叉轴 `stretch`），都算起点。
    let Some(value) = value else {
        return Some(Alignment::Start);
    };
    Some(
        if matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "center" | "flex-end" | "end" | "space-around" | "space-evenly" | "right"
        ) {
            Alignment::Away
        } else {
            Alignment::Start
        },
    )
}

fn flex_direction_row(project: &Project, tree: &Tree<'_>, parent: &WxmlElement) -> Option<bool> {
    match resolved_property_with(project, tree, parent, "flex-direction") {
        (_, true) => None,
        (Some(resolved), false) => Some(matches!(
            resolved.value.trim().to_ascii_lowercase().as_str(),
            "row" | "row-reverse"
        )),
        // `defaultDisplayBlock` 开启时初始方向回到 row（`FlexDirection::ApplyInitial`）。
        (None, false) => Some(default_block(project, tree)),
    }
}

/// `Some(None)` = 没有声明；`Some(Some(值))` = 声明了；`None` = 判不了。
fn declared_value(
    project: &Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
    property: &str,
) -> Option<Option<String>> {
    match resolved_property_with(project, tree, element, property) {
        (_, true) => None,
        (Some(resolved), false) => Some(Some(resolved.value.trim().to_string())),
        (None, false) => Some(None),
    }
}

/// 同一个父级下，元素前面有没有占位的兄弟（`display: none` 与脱流兄弟不算）。
fn has_preceding_in_flow_sibling(
    project: &Project,
    tree: &Tree<'_>,
    parent: &WxmlElement,
    element: &WxmlElement,
) -> Option<bool> {
    for child in &parent.children {
        let WxmlNode::Element(sibling) = child else {
            continue;
        };
        if std::ptr::eq(sibling, element) {
            break;
        }
        match declared_value(project, tree, sibling, "display")? {
            Some(value) if value.trim().eq_ignore_ascii_case("none") => continue,
            _ => {}
        }
        if fixed_or_absolute(project, tree, sibling)? {
            continue;
        }
        return Some(true);
    }
    Some(false)
}

fn fixed_or_absolute(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> Option<bool> {
    match resolved_property_with(project, tree, element, "position") {
        (_, true) => None,
        (Some(resolved), false) => Some(matches!(
            resolved.value.trim().to_ascii_lowercase().as_str(),
            "fixed" | "absolute"
        )),
        (None, false) => Some(false),
    }
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testutil::Fixture;

    use super::FixedOffsetMissing;

    /// 视觉用例与单测共用同一份 WXML/WXSS：每条都必须被本规则命中。
    #[test]
    fn visual_cases_are_reported() {
        for case in super::VISUAL_CASES {
            let fixture =
                crate::rules::testutil::Fixture::page("fixed-offset-visual", case.wxml, case.wxss);
            let hits = crate::rules::testutil::check_rule(&fixture, &FixedOffsetMissing)
                .iter()
                .filter(|diagnostic| diagnostic.rule == FixedOffsetMissing::ID)
                .count();
            assert!(hits >= 1, "{}: {}", case.note, case.wxml);
        }
    }

    fn check(name: &str, wxml: &str, wxss: &str) -> usize {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"defaultDisplayBlock":true}}}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", wxml)
            .write("pages/index/index.wxss", wxss);
        let project = fixture.project();
        FixedOffsetMissing.check(&project).len()
    }

    /// 两轴都没写：WebView 的静态位置在前序兄弟之后，Skyline 落到根的起点。
    #[test]
    fn fixed_without_any_offset_after_a_block_sibling_is_reported() {
        let wxml = r#"<view class="page">
  <view class="header"/>
  <view class="bar"/>
</view>"#;
        let wxss = ".page { }\n.bar { position: fixed; }";
        assert_eq!(check("fixed-sibling", wxml, wxss), 1);
    }

    /// 块容器里只缺水平偏移不报：块级盒子的静态位置就是 0，与根起点一致。
    #[test]
    fn horizontal_axis_in_a_block_parent_is_skipped() {
        let wxml = r#"<view class="page">
  <view class="header"/>
  <view class="bar"/>
</view>"#;
        let wxss = ".page { }\n.bar { position: fixed; bottom: 116rpx; }";
        assert_eq!(check("fixed-block-horizontal", wxml, wxss), 0);
    }

    /// 它自己就是父级的第一个子项：静态位置就是父级内容区起点，与根起点一致。
    #[test]
    fn fixed_as_the_first_child_is_skipped() {
        let wxml = r#"<view class="page">
  <view class="bar"/>
</view>"#;
        let wxss = ".page { }\n.bar { position: fixed; }";
        assert_eq!(check("fixed-first-child", wxml, wxss), 0);
    }

    /// flex 父级的 `align-items: center` 在 WebView 里会把 fixed 元素水平居中；
    /// Skyline 提到页面根后按根的起点落在 0。
    #[test]
    fn flex_parent_alignment_is_reported() {
        let wxml = r#"<view class="page">
  <view class="bar"/>
</view>"#;
        let wxss = ".page { display: flex; flex-direction: column; align-items: center; }\n\
                    .bar { position: fixed; bottom: 116rpx; }";
        assert_eq!(check("fixed-flex-center", wxml, wxss), 1);
    }

    /// flex 主轴起点对齐时静态位置就在父级内容区起点，按「与页面原点相同」处理，不报。
    ///
    /// 这里漏批一种真实差异：父级自己的内容区不在页面原点（被上级居中、带 padding），
    /// 元素又起点对齐——两端会错开一个父级偏移量。静态判父级是否在原点要往上追好几层
    /// 布局，噪声比很高，暂不处理（`single-popup` 的 `.infoLayout` 就是这一种，靠显式
    /// 偏移修掉）。
    #[test]
    fn flex_main_axis_start_is_skipped() {
        let wxml = r#"<view class="page">
  <image class="bg"/>
  <view class="bar"/>
</view>"#;
        let wxss = ".page { display: flex; flex-direction: column; justify-content: flex-start; }\n\
                    .bg { height: 662rpx; width: 580rpx; }\n\
                    .bar { position: fixed; width: 580rpx; height: 662rpx; }";
        assert_eq!(check("fixed-flex-start", wxml, wxss), 0);
    }

    /// flex 列里它就是第一项、且主轴起点对齐：两端都在内容区起点，不报。
    #[test]
    fn flex_main_axis_start_as_first_item_is_skipped() {
        let wxml = r#"<view class="page">
  <view class="bar"/>
</view>"#;
        let wxss = ".page { display: flex; flex-direction: column; justify-content: flex-start; }\n\
                    .bar { position: fixed; }";
        assert_eq!(check("fixed-flex-start-first", wxml, wxss), 0);
    }

    /// 两轴都写死了偏移就不存在「静态位置」这回事。
    #[test]
    fn explicit_offsets_are_skipped() {
        let wxml = r#"<view class="page">
  <view class="header"/>
  <view class="bar"/>
</view>"#;
        let wxss = ".page { display: flex; flex-direction: column; align-items: center; }\n\
                    .bar { position: fixed; left: 0; right: 0; bottom: 0; }";
        assert_eq!(check("fixed-explicit", wxml, wxss), 0);
    }

    /// 垂直主轴上的居中同样来自 flex 对齐，不需要前序兄弟也会错位。
    #[test]
    fn flex_main_axis_centering_is_reported() {
        let wxml = r#"<view class="page">
  <view class="bar"/>
</view>"#;
        let wxss = ".page { display: flex; flex-direction: column; justify-content: center; }\n\
                    .bar { position: fixed; left: 0; }";
        assert_eq!(check("fixed-flex-main", wxml, wxss), 1);
    }

    /// 非 fixed 不归本规则管。
    #[test]
    fn absolute_and_static_are_skipped() {
        let wxml = r#"<view class="page">
  <view class="header"/>
  <view class="abs"/>
  <view class="plain"/>
</view>"#;
        let wxss = ".page { }\n.abs { position: absolute; bottom: 0; }\n.plain { }";
        assert_eq!(check("fixed-other-positions", wxml, wxss), 0);
    }
}
