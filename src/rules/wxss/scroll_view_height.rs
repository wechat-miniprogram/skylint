//! `skyline/scroll-view-height`：`scroll-view` 没有确定高度，Skyline 下会塌成 0。
//!
//! 为什么这是 Skyline 差异：WebView 的 scroll-view 是原生滚动容器，内容多高它都能滚；
//! Skyline 只有 `enableScrollViewAutoSize` 打开时才成为 measurable 节点
//! （`shadow/components/scroll_view_shadow_node.cc:1096-1103`），否则：
//!
//! - 它自己的 yoga 节点在 fp 布局树里是**叶子**：子元素各自 `SetAsRoot()` 独立布局
//!   （`shadow/shadow_node.cc:195-226`，`YGConnector` 只在没有 concrete widget 时创建），
//!   `ScrollView::AppendChild` 只挂 Widget 树、不碰 fp（`widgets/scroll_view.cc:346-353`）；
//! - 没有任何 measure 兜底：`SetLayoutNode` 只用于失效通知（`scroll_view.cc:318-320`），
//!   尺寸一律由 `FPNodeCalculateLayout` 从自身 CSS + 父约束解出（`widgets/yoga.cc:473-554`）；
//! - Flutter 侧拿到的是 `BoxConstraints.loose`（`yoga.h:365-370` + `yoga.dart:575-584`），
//!   内容只能填满盒子，撑不大它。
//!
//! 所以高度只能来自 CSS：[`definite_height`] 逐条覆盖「自身 `height`、百分比沿父链、
//! 绝对定位上下拉伸、flex 主轴 grow、交叉轴 stretch」，全都不成立就是 0。
//!
//! 工程级 `rendererOptions.skyline.enableScrollViewAutoSize` 开启时整条规则跳过：
//! 那时 scroll-view 变 measurable，尺寸来自内容（`ScrollView::MeasureContent`）。

use crate::input::wxml::WxmlElement;
use crate::matcher::Tree;
use crate::project::Project;
use crate::rules::Rule;
use crate::rules::RuleMeta;
use crate::rules::wxml::structure::find_attr;
use crate::rules::wxml::{
    ElementDoc, Resolved, element_diagnostic, element_docs, resolved_property, walk_skyline,
};
use crate::{Diagnostic, Severity};

pub struct ScrollViewHeight;

impl ScrollViewHeight {
    pub const ID: &'static str = "skyline/scroll-view-height";

    const MESSAGE: &'static str = "`scroll-view` 没有确定高度：Skyline 默认不开 `enableScrollViewAutoSize`，高度只来自 CSS，内容再多也撑不开，会塌成 0";

    const HELP: &'static str = "给它确定高度：`height` / 定高父级下的 `height: 100%` / 定高容器里的 `flex: 1`；只写 `max-height` 不产生高度（WebView 靠内容撑开，Skyline 不会），要配合 `height` 或打开 `app.json` 的 `rendererOptions.skyline.enableScrollViewAutoSize`";
}

impl Rule for ScrollViewHeight {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "scroll-view 没有确定高度，Skyline 下内容不会撑开它，会塌成 0",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        if project.scroll_view_auto_size() {
            return Vec::new();
        }
        let mut out = Vec::new();

        for ElementDoc { doc, reach } in element_docs(project) {
            let tree = Tree::new(&doc);
            let default_block = project.scope_of(doc.path()).map_or_else(
                || project.app_display_block(),
                |scope| project.renders_as_block(&scope),
            );
            walk_skyline(&doc, |element, _| {
                if element.tag != "scroll-view"
                    || definite_height(project, &tree, element, default_block, 0)
                {
                    return;
                }
                out.push(
                    element_diagnostic(
                        Self::ID,
                        &doc,
                        element,
                        doc.path(),
                        Severity::Warning,
                        Self::MESSAGE,
                    )
                    .help(reach.help(Self::HELP)),
                );
            });
        }
        out
    }
}

/// 元素在 Skyline 下拿到的高度是否确定（= 不会塌成 0）。
///
/// 判不了的情况（动态内联 `style`、递归过深）一律当确定，宁可漏报。
fn definite_height(
    project: &Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
    default_block: bool,
    depth: u8,
) -> bool {
    if depth > 16 || dynamic_inline_style(element) {
        return true;
    }
    // 框架自己定尺寸的盒子：`swiper-item` 撑满 swiper，`nested-scroll-body` /
    // `nested-scroll-header` 由滚动视口分配。它们的分支自己的高度问题会在宿主元素上报，
    // 这里按确定处理，避免把百分比链一律断在这里（宁可漏报）。
    if matches!(
        element.tag.as_str(),
        "swiper-item" | "nested-scroll-body" | "nested-scroll-header"
    ) {
        return true;
    }
    // `min-height` 是地板：定长的它就把盒子顶到那个高度。
    if length_value(project, tree, element, "min-height").is_some_and(|min| {
        !min.contains('%') || parent_definite(project, tree, element, default_block, depth)
    }) {
        return true;
    }
    if let Some(height) = length_value(project, tree, element, "height") {
        // 百分比要沿父链解到确定值：父高不定时它退化成 `auto`。
        return !height.contains('%')
            || parent_definite(project, tree, element, default_block, depth);
    }
    // `aspect-ratio` + 宽度确定：高度由比例反算（`style_builder.cc:814`、
    // `flex_box.rs:343-370` / `flow.rs:440-460`）。
    if aspect_ratio(project, tree, element).is_some()
        && length_value(project, tree, element, "width").is_some_and(|width| {
            !width.contains('%') || parent_definite(project, tree, element, default_block, depth)
        })
    {
        return true;
    }
    // 绝对定位 + 上下都有值：高度由定位决定。
    if is_out_of_flow(project, tree, element)
        && length_value(project, tree, element, "top").is_some()
        && length_value(project, tree, element, "bottom").is_some()
    {
        return true;
    }
    let Some(parent) = tree.parent_of(element) else {
        // 顶层元素挂在页面根上：`shadow_tree.cc:30` 给根节点 `height:100%`，
        // 未开 `defaultDisplayBlock` 时根是 `flex-direction: column`，`flex:1` 能分到整屏。
        let flex = resolved_property(project, tree, element, "flex");
        let grow = resolved_property(project, tree, element, "flex-grow");
        return !default_block && grows(grow.as_ref(), flex.as_ref());
    };
    match flex_axis(project, tree, parent, default_block) {
        // 主轴是高度：`flex-basis` 定长，或 grow 从确定高度的父级分到剩余空间。
        Some(false) => {
            if length_value(project, tree, element, "flex-basis")
                .is_some_and(|basis| !basis.contains('%'))
            {
                return true;
            }
            let flex = resolved_property(project, tree, element, "flex");
            let grow = resolved_property(project, tree, element, "flex-grow");
            grows(grow.as_ref(), flex.as_ref())
                && parent_definite(project, tree, element, default_block, depth)
        }
        // 主轴是宽度：高度走交叉轴，`stretch`（默认）时等于父级高度。
        Some(true) => {
            stretches(project, tree, element, parent)
                && parent_definite(project, tree, element, default_block, depth)
        }
        None => false,
    }
}

/// 父级的高度是否确定；父级就是页面时按视口高度算确定。
fn parent_definite(
    project: &Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
    default_block: bool,
    depth: u8,
) -> bool {
    match tree.parent_of(element) {
        Some(parent) => definite_height(project, tree, parent, default_block, depth + 1),
        None => true,
    }
}

/// 属性的取值，`auto` / `em` 一类空值返回 `None`（等同没写）。
/// `aspect-ratio` 的宽高比（`auto 16/9` 只取比例部分）。0 或无法解析时返回 `None`。
fn aspect_ratio(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> Option<f64> {
    let value = resolved_property(project, tree, element, "aspect-ratio")?
        .value
        .trim()
        .to_ascii_lowercase();
    let ratio = value.strip_prefix("auto").unwrap_or(&value).trim();
    let (width, height) = match ratio.split_once('/') {
        Some((width, height)) => (width.trim(), height.trim()),
        None => (ratio, "1"),
    };
    let width: f64 = width.parse().ok()?;
    let height: f64 = height.parse().ok()?;
    (width > 0.0 && height > 0.0).then_some(width / height)
}

fn length_value(
    project: &Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
    property: &str,
) -> Option<String> {
    let value = resolved_property(project, tree, element, property)?
        .value
        .trim()
        .to_ascii_lowercase();
    match value.as_str() {
        "" | "auto" | "inherit" | "initial" | "unset" | "none" => None,
        _ => Some(value),
    }
}

/// `flex-grow` 是否大于 0，考虑 `flex` 简写与两条声明的先后（后者覆盖前者）。
///
/// `flex: <number>` 与 `flex: auto` 都让 grow 为正；`flex: none` / `flex: 0 …` / `initial` 为 0。
fn grows(grow: Option<&Resolved<'_>>, flex: Option<&Resolved<'_>>) -> bool {
    // 内联 `style` 没有声明位置，优先级最高。
    let offset =
        |resolved: &Resolved<'_>| resolved.declaration.map_or(u64::MAX, |d| d.offset as u64);
    let flex_wins = match (grow, flex) {
        (Some(grow), Some(flex)) => offset(flex) > offset(grow),
        (None, Some(_)) => true,
        _ => false,
    };
    if flex_wins {
        return flex.is_some_and(flex_grows);
    }
    match grow.and_then(|grow| grow.value.trim().parse::<f32>().ok()) {
        Some(value) => value > 0.0,
        None => flex.is_some_and(flex_grows),
    }
}

fn flex_grows(flex: &Resolved<'_>) -> bool {
    let text = flex.value.trim().to_ascii_lowercase();
    match text.split_ascii_whitespace().next() {
        Some("auto") => true,
        Some("none") | Some("initial") | None => false,
        Some(first) => first.parse::<f32>().is_ok_and(|value| value > 0.0),
    }
}

/// 交叉轴是否被拉伸（`align-self` 缺省 `auto` 时看父级 `align-items`，再缺省就是 `stretch`）。
fn stretches(
    project: &Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
    parent: &WxmlElement,
) -> bool {
    let align = resolved_property(project, tree, element, "align-self")
        .map(|resolved| resolved.value.trim().to_ascii_lowercase())
        .filter(|value| value != "auto")
        .or_else(|| {
            resolved_property(project, tree, parent, "align-items")
                .map(|resolved| resolved.value.trim().to_ascii_lowercase())
        });
    match align.as_deref() {
        Some("flex-start" | "start" | "flex-end" | "end" | "center" | "baseline") => false,
        // 缺省、`stretch`、`normal` 都是拉伸。
        _ => true,
    }
}

/// 元素是 flex 容器时返回主轴方向（`true` = 横向），否则 `None`。
///
/// Skyline 默认 `display: flex` + `flex-direction: column`；开了 `defaultDisplayBlock`
/// 之后默认变成 `block` + `row`（`renderer_options` 的整工程对齐开关）。
fn flex_axis(
    project: &Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
    default_block: bool,
) -> Option<bool> {
    let display = resolved_property(project, tree, element, "display")
        .map(|resolved| resolved.value.trim().to_ascii_lowercase())
        .unwrap_or_else(|| {
            if default_block {
                "block".to_string()
            } else {
                "flex".to_string()
            }
        });
    if !matches!(display.as_str(), "flex" | "inline-flex") {
        return None;
    }
    let direction = resolved_property(project, tree, element, "flex-direction")
        .map(|resolved| resolved.value.trim().to_ascii_lowercase())
        .unwrap_or_else(|| {
            if default_block {
                "row".to_string()
            } else {
                "column".to_string()
            }
        });
    Some(direction.starts_with("row"))
}

fn is_out_of_flow(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> bool {
    resolved_property(project, tree, element, "position").is_some_and(|resolved| {
        matches!(
            resolved.value.trim().to_ascii_lowercase().as_str(),
            "absolute" | "fixed"
        )
    })
}

/// 带 `{{…}}` 或纯动态的内联 `style`：运行时的尺寸静态判不出，按确定处理（宁可漏报）。
fn dynamic_inline_style(element: &WxmlElement) -> bool {
    find_attr(element, "style").is_some_and(|attr| {
        attr.value
            .as_deref()
            .is_none_or(|value| value.contains("{{"))
    })
}

#[cfg(test)]
mod tests {
    use crate::Diagnostic;
    use crate::rules::Rule;
    use crate::rules::testutil::wxss::Fixture;

    use super::ScrollViewHeight;

    /// 页面：`<view class="panel"><scroll-view class="list"/></view>`。
    fn check(name: &str, app_json: &str, wxss: &str) -> Vec<Diagnostic> {
        check_wxml(
            name,
            app_json,
            wxss,
            r#"<view class="panel"><scroll-view class="list"/></view>"#,
        )
    }

    fn check_wxml(name: &str, app_json: &str, wxss: &str, wxml: &str) -> Vec<Diagnostic> {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write("app.json", app_json)
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", wxml)
            .write("pages/index/index.wxss", wxss);
        let project = fixture.project();
        ScrollViewHeight.check(&project)
    }

    const SKYLINE_APP: &str = r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#;

    #[test]
    fn aspect_ratio_with_definite_width_is_a_height() {
        let diagnostics = check_wxml(
            "svh-aspect",
            SKYLINE_APP,
            ".list { width: 100%; aspect-ratio: 16 / 9; }",
            r#"<scroll-view class="list"/>"#,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn top_level_flex_one_takes_the_page_height() {
        let diagnostics = check_wxml(
            "svh-root-flex",
            SKYLINE_APP,
            ".list { flex: 1; }",
            r#"<scroll-view class="list"/>"#,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    /// 开了 `defaultDisplayBlock` 时页面根是 block，`flex: 1` 不再分到高度。
    #[test]
    fn top_level_flex_one_is_not_enough_with_default_block() {
        let app = r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"defaultDisplayBlock":true}}}"#;
        let diagnostics = check_wxml(
            "svh-root-flex-block",
            app,
            ".list { flex: 1; }",
            r#"<scroll-view class="list"/>"#,
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    }

    #[test]
    fn scroll_view_without_height_is_reported() {
        let diagnostics = check(
            "svh-report",
            SKYLINE_APP,
            ".panel { display: flex; flex-direction: column; }\n.list { flex: 1; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("塌成 0"));
        assert_eq!(diagnostics[0].line, 1);
    }

    #[test]
    fn bounded_parent_height_is_not_reported() {
        let diagnostics = check(
            "svh-bounded",
            SKYLINE_APP,
            ".panel { display: flex; flex-direction: column; height: 100vh; }\n.list { flex: 1; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn own_height_is_not_reported() {
        for (name, css) in [
            ("svh-own-height", ".list { flex: 1; height: 0; }"),
            ("svh-own-vh", ".list { height: 100vh; }"),
            ("svh-min-height", ".list { min-height: 200px; }"),
            (
                "svh-absolute",
                ".list { position: absolute; top: 0; bottom: 0; }",
            ),
        ] {
            let diagnostics = check(name, SKYLINE_APP, css);
            assert!(diagnostics.is_empty(), "{name}: {diagnostics:#?}");
        }
    }

    /// 行向容器的交叉轴是高度：父级定高时 `stretch` 撑满，父级不定高时跟着塌。
    #[test]
    fn row_container_stretches_only_under_a_bounded_parent() {
        let bounded = check(
            "svh-row-bounded",
            SKYLINE_APP,
            ".panel { display: flex; flex-direction: row; height: 100vh; }\n.list { flex: 1; }",
        );
        assert!(bounded.is_empty(), "{bounded:#?}");

        let unbounded = check(
            "svh-row-unbounded",
            SKYLINE_APP,
            ".panel { display: flex; flex-direction: row; }\n.list { flex: 1; }",
        );
        assert_eq!(unbounded.len(), 1, "{unbounded:#?}");
    }

    /// 交叉轴上不是 `stretch` 就不吃父级高度。
    #[test]
    fn non_stretch_alignment_is_reported() {
        let diagnostics = check(
            "svh-align-start",
            SKYLINE_APP,
            ".panel { display: flex; flex-direction: row; height: 100vh; }\n.list { align-self: flex-start; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    /// 百分比要沿父链解：父级自动高度时它退化成 `auto`。
    #[test]
    fn percentage_height_needs_a_bounded_parent() {
        let broken = check(
            "svh-percent-unbounded",
            SKYLINE_APP,
            ".panel { display: block; }\n.list { height: 100%; }",
        );
        assert_eq!(broken.len(), 1, "{broken:#?}");

        let nested = check(
            "svh-percent-two-levels",
            SKYLINE_APP,
            ".outer { height: 100vh; }\n.panel { display: block; height: 100%; }\n.list { height: 100%; }",
        );
        assert!(nested.is_empty(), "百分比链解到确定值时不报：{nested:#?}");
    }

    /// 块级父容器、`flex: none`、甚至直接挂在页面下，都没有高度来源。
    #[test]
    fn no_layout_source_is_reported() {
        for (name, css) in [
            (
                "svh-block",
                ".panel { display: block; }\n.list { flex: 1; }",
            ),
            (
                "svh-none",
                ".panel { display: flex; flex-direction: column; }\n.list { flex: none; }",
            ),
            ("svh-plain", ".panel { margin: 10px; }"),
        ] {
            let diagnostics = check(name, SKYLINE_APP, css);
            assert_eq!(diagnostics.len(), 1, "{name}: {diagnostics:#?}");
        }

        let top_level = check_wxml(
            "svh-top-level",
            SKYLINE_APP,
            ".list { margin: 10px; }",
            r#"<scroll-view class="list"/>"#,
        );
        assert_eq!(top_level.len(), 1, "{top_level:#?}");
    }

    /// `max-height` 是上限不是尺寸：WebView 下内容把它顶到 500px，Skyline 下内容
    /// 不贡献高度，盒子仍是 0——所以只有 `max-height` 照报。
    #[test]
    fn max_height_alone_is_not_a_height() {
        for (name, css) in [
            ("svh-max-height", ".list { max-height: 500px; }"),
            (
                "svh-max-height-calc",
                ".list { max-height: calc(75vh - 60px); }",
            ),
        ] {
            let diagnostics = check(name, SKYLINE_APP, css);
            assert_eq!(diagnostics.len(), 1, "{name}: {diagnostics:#?}");
        }
        let bounded = check(
            "svh-max-height-with-height",
            SKYLINE_APP,
            ".list { height: 300px; max-height: 500px; }",
        );
        assert!(bounded.is_empty(), "{bounded:#?}");
    }

    #[test]
    fn auto_size_flag_skips_the_rule() {
        let diagnostics = check(
            "svh-flag",
            r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"enableScrollViewAutoSize":true}}}"#,
            ".panel { display: flex; flex-direction: column; }\n.list { flex: 1; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }
}
