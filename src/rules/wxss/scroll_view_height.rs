//! `skyline/scroll-view-height`：`flex: 1` 的 `scroll-view` 靠父容器撑高，
//! 而父容器是「主轴尺寸不定」的列容器——Skyline 下 scroll-view 没有内容高度兜底，
//! 实测会塌成 0。
//!
//! 为什么这是 Skyline 差异：WebView 的 scroll-view 是原生滚动容器，内容多高它都能滚；
//! Skyline 的 scroll-view 只有在 `enableScrollViewAutoSize` 开启时才成为 measurable
//! 节点（`shadow/components/scroll_view_shadow_node.cc:1098-1103`），否则它就是一个
//! 普通布局节点，高度完全由 CSS 决定——`flex: 1` 在不定主轴的列容器里拿不到剩余空间
//! （fp 的 `Percent` 在不定父轴下 `resolve` 成 None，`float-pigment-layout/src/types.rs:224`，
//! 于是退化成内容求和），内容又是懒加载的，最终就是 0。
//!
//! 判据（全部满足才报，方向是宁可漏报）：
//!
//! 1. 元素是 `<scroll-view>`；
//! 2. 它的 `flex-grow > 0`（含 `flex: 1` / `flex: auto` 简写）；
//! 3. 它自己没有确定的 `height`（写了任何非 `auto` 的值都算确定，含 `%`——百分比
//!    写法在真实工程里基本都配着确定的父高，按确定处理可少报）；
//! 4. 父元素是 flex 容器，且主轴是纵向（`flex-direction: column`，Skyline 默认值）；
//! 5. 父元素同样没有确定的 `height`。
//!
//! 工程级 `rendererOptions.skyline.enableScrollViewAutoSize` 开启时整条规则跳过：
//! 那时 scroll-view 变 measurable，尺寸来自内容而不是 flex 分配。

use std::collections::HashMap;

use crate::input::wxml::{WxmlDoc, WxmlElement};
use crate::matcher::Tree;
use crate::project::Project;
use crate::rules::Rule;
use crate::rules::RuleMeta;
use crate::rules::wxml::structure::find_attr;
use crate::rules::wxml::{
    Resolved, element_diagnostic, load_doc, page_wxml_groups, resolved_property, walk,
};
use crate::rules::wxss::value_util::at_declaration;
use crate::{Diagnostic, Severity};

pub struct ScrollViewHeight;

impl ScrollViewHeight {
    pub const ID: &'static str = "skyline/scroll-view-height";
}

impl Rule for ScrollViewHeight {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "`flex: 1` 的 scroll-view 在主轴尺寸不定的列容器里没有高度可撑，Skyline 下会塌成 0",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        if project.scroll_view_auto_size() {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut docs: HashMap<String, Option<WxmlDoc>> = HashMap::new();

        for (_page_wxml, paths) in page_wxml_groups(project) {
            for path in &paths {
                if !docs.contains_key(path) {
                    docs.insert(path.clone(), load_doc(project, path));
                }
            }
            for path in &paths {
                let Some(doc) = docs.get(path).and_then(Option::as_ref) else {
                    continue;
                };
                let tree = Tree::new(doc);
                let default_block = project.scope_of(doc.path()).map_or_else(
                    || project.app_display_block(),
                    |scope| project.renders_as_block(&scope),
                );
                walk(doc, |element, ancestors| {
                    if element.tag != "scroll-view" {
                        return;
                    }
                    let flex = resolved_property(project, &tree, element, "flex");
                    let grow = resolved_property(project, &tree, element, "flex-grow");
                    if !grows(grow.as_ref(), flex.as_ref()) {
                        return;
                    }
                    if determinate(project, &tree, element) {
                        return;
                    }
                    let Some(parent) = ancestors.last() else {
                        return;
                    };
                    if !is_column_flex(project, &tree, parent, default_block) {
                        return;
                    }
                    if determinate_height(
                        project,
                        &tree,
                        parent,
                        &ancestors[..ancestors.len() - 1],
                        default_block,
                    ) {
                        return;
                    }

                    let message = "`scroll-view` 用 `flex` 撑高，但所在列容器没有确定高度：Skyline 下 scroll-view 没有内容高度兜底，flex 分不到高度时会塌成 0";
                    let help = "给父容器一个确定高度（如 `height: 100vh`），或给这个 scroll-view 补 `height: 0` 让它按 flex 分配";
                    out.push(
                        match flex
                            .or(grow)
                            .and_then(|resolved| Some((resolved.sheet?, resolved.declaration?)))
                        {
                            Some((sheet, declaration)) => at_declaration(
                                Self::ID,
                                &sheet.doc,
                                declaration,
                                Severity::Warning,
                                message,
                                help,
                            ),
                            None => element_diagnostic(
                                Self::ID,
                                doc,
                                element,
                                doc.path(),
                                Severity::Warning,
                                message,
                            )
                            .help(help),
                        },
                    );
                });
            }
        }
        out
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

/// 属性的取值是否为「确定尺寸」：写了 `auto` / `inherit` 之外的值就算。
///
/// 不对百分比追父链：真实工程里 `height: 100%` 基本都配着确定父高，按确定处理可少报。
fn determinate(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> bool {
    if dynamic_inline_style(element) {
        return true;
    }
    let Some(resolved) = resolved_property(project, tree, element, "height") else {
        return false;
    };
    let value = resolved.value.trim().to_ascii_lowercase();
    !matches!(
        value.as_str(),
        "" | "auto" | "inherit" | "initial" | "unset"
    )
}

/// 容器的高度是否确定：自己写了 `height`，或者作为 flex 子项被上一级列容器分配。
///
/// 标准「整页固定 + 中间滚动」是 `.outer{height:100vh} > .mid{flex:1} > scroll-view`，
/// `.mid` 自己没写高度，只看一层会把它当不定高。
fn determinate_height(
    project: &Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
    ancestors: &[&WxmlElement],
    default_block: bool,
) -> bool {
    if determinate(project, tree, element) {
        return true;
    }
    let flex = resolved_property(project, tree, element, "flex");
    let grow = resolved_property(project, tree, element, "flex-grow");
    if !grows(grow.as_ref(), flex.as_ref()) {
        return false;
    }
    let Some(parent) = ancestors.last() else {
        return false;
    };
    is_column_flex(project, tree, parent, default_block)
        && determinate_height(
            project,
            tree,
            parent,
            &ancestors[..ancestors.len() - 1],
            default_block,
        )
}

/// 带 `{{…}}` 或纯动态的内联 `style`：运行时的尺寸静态判不出，按确定处理（宁可漏报）。
fn dynamic_inline_style(element: &WxmlElement) -> bool {
    find_attr(element, "style").is_some_and(|attr| {
        attr.value
            .as_deref()
            .is_none_or(|value| value.contains("{{"))
    })
}

/// 元素是否为「主轴纵向的 flex 容器」。
///
/// Skyline 默认 `display: flex` + `flex-direction: column`；开了 `defaultDisplayBlock`
/// 之后默认变成 `block` + `row`（`renderer_options` 的整工程对齐开关）。
fn is_column_flex(
    project: &Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
    default_block: bool,
) -> bool {
    let display = resolved_property(project, tree, element, "display")
        .map(|resolved| resolved.value.trim().to_ascii_lowercase())
        .unwrap_or_else(|| {
            if default_block {
                "block".to_string()
            } else {
                "flex".to_string()
            }
        });
    if display != "flex" && display != "inline-flex" {
        return false;
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
    direction == "column" || direction == "column-reverse"
}

/// 诊断落在元素开始标签上（没有可落的声明时）。
#[cfg(test)]
mod tests {
    use crate::Diagnostic;
    use crate::rules::Rule;
    use crate::rules::testutil::wxss::Fixture;

    use super::ScrollViewHeight;

    /// 页面：`<view class="panel"><scroll-view class="list"/></view>`。
    fn check(name: &str, app_json: &str, wxss: &str) -> Vec<Diagnostic> {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write("app.json", app_json)
            .write("pages/index/index.json", "{}")
            .write(
                "pages/index/index.wxml",
                r#"<view class="panel"><scroll-view class="list"/></view>"#,
            )
            .write("pages/index/index.wxss", wxss);
        let project = fixture.project();
        ScrollViewHeight.check(&project)
    }

    const SKYLINE_APP: &str = r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#;

    #[test]
    fn flex_one_in_unbounded_column_container_is_reported() {
        let diagnostics = check(
            "svh-report",
            SKYLINE_APP,
            ".panel { display: flex; flex-direction: column; }\n.list { flex: 1; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("塌成 0"));
        assert_eq!(diagnostics[0].line, 2);
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
        let diagnostics = check(
            "svh-own-height",
            SKYLINE_APP,
            ".panel { display: flex; flex-direction: column; }\n.list { flex: 1; height: 0; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn row_direction_is_not_reported() {
        // 主轴横向时 flex 分配的是宽度，高度与内容无关。
        let diagnostics = check(
            "svh-row",
            SKYLINE_APP,
            ".panel { display: flex; flex-direction: row; }\n.list { flex: 1; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn block_parent_is_not_reported() {
        let diagnostics = check(
            "svh-block",
            SKYLINE_APP,
            ".panel { display: block; }\n.list { flex: 1; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn flex_none_is_not_reported() {
        let diagnostics = check(
            "svh-none",
            SKYLINE_APP,
            ".panel { display: flex; flex-direction: column; }\n.list { flex: none; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
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
