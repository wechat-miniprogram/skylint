//! `skyline/fixed-z-index-inversion`：`position: fixed` 被提到页面根后，
//! 祖先的层叠上下文失效，两个 fixed 元素的上下顺序可能与 WebView 相反。
//!
//! 引擎把 fixed 节点提到 root container（`shadow/shadow_tree.cc:683-706` 收集、
//! `widgets/widget_operation.cc:492-546` 的 `UpdatePositionFixedWidgetOperation::Apply`
//! 里 `root_container_widget->AppendChild(widget)`），而 Skyline 的 `z-index` 只在
//! 同一父节点的兄弟之间排序（`package/skyline/lib/rendering/stacking.dart:19-56`），
//! 没有 CSS 的层叠上下文概念。
//!
//! 于是同一份 CSS 在两端的绘制顺序不同：
//!
//! - WebView：`.wrap { position: relative; z-index: 0 }` 把内部 fixed 关在 0 这一层里，
//!   外面的 `z-index: 4000` 元素盖在它上面；
//! - Skyline：内部 fixed 被提到页面根，直接拿自己的 `z-index: 5000` 与 4000 比，
//!   反过来盖住外面的元素。
//!
//! 判据：取每个 fixed 元素的两个键——WebView 用「最外层层叠上下文祖先的 z-index
//! （没有则用自身 z）」，Skyline 用「自身 z-index」；同一渲染树里两个元素的相对顺序
//! 在两个键下翻转就报。
//!
//! 只比较同一渲染树（页面 + 它递归用到的组件文档）内的元素；跨页面不构成竞争。

use std::collections::HashSet;

use crate::matcher::Tree;
use crate::project::Project;
use crate::rules::Rule;
use crate::rules::RuleMeta;
use crate::rules::wxml::{ElementDoc, element_docs, resolved_property, walk_skyline};
use crate::wxml_graph::WxmlGraph;
use crate::{Diagnostic, Severity};

pub struct FixedZIndexInversion;

/// 一个 fixed 元素在两种渲染器下参与排序的键。
struct Layer {
    path: String,
    line: u32,
    column: u32,
    /// 自身 `z-index`：Skyline 的排序键。
    own: i64,
    /// WebView 的排序键：最外层层叠上下文祖先的 `z-index`，没有则等于 `own`。
    flat: i64,
}

impl Rule for FixedZIndexInversion {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "`position: fixed` 在 Skyline 下跳出祖先的层叠上下文，上下顺序可能与 WebView 相反",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let graph = WxmlGraph::new(project);
        let layers = collect_layers(project, &graph);
        if layers.len() < 2 {
            return Vec::new();
        }

        // 页面 → 它渲染到的全部文档，同一渲染树内的 fixed 元素才互相竞争。
        let trees: Vec<HashSet<&str>> = project
            .pages
            .iter()
            .filter_map(|page| page.files.wxml.as_deref())
            .map(|path| graph.render_docs(path).into_iter().collect::<HashSet<_>>())
            .collect();

        // 按「遮挡方」聚合：同一个遮罩压住多个上层时只报一条，避免同因噪声。
        let mut groups: Vec<(usize, Vec<usize>)> = Vec::new();
        for (index, first) in layers.iter().enumerate() {
            for (offset, second) in layers[index + 1..].iter().enumerate() {
                let second_index = index + 1 + offset;
                if !shares_tree(&trees, &first.path, &second.path) {
                    continue;
                }
                // 任一端打平：先后由文档序 / 提到 root 的顺序决定，静态不可判，放过。
                if first.own == second.own || first.flat == second.flat {
                    continue;
                }
                let webview_first_on_top =
                    layer_key(first.flat, first) > layer_key(second.flat, second);
                let skyline_first_on_top =
                    layer_key(first.own, first) > layer_key(second.own, second);
                if webview_first_on_top == skyline_first_on_top {
                    continue;
                }
                // 在 WebView 里压在上面、在 Skyline 里被反超的那个才是被盖住的元素。
                let (loser, winner) = if webview_first_on_top {
                    (index, second_index)
                } else {
                    (second_index, index)
                };
                match groups.iter_mut().find(|(group, _)| *group == winner) {
                    Some((_, losers)) => losers.push(loser),
                    None => groups.push((winner, vec![loser])),
                }
            }
        }

        groups
            .into_iter()
            .map(|(winner, losers)| {
                let winner = &layers[winner];
                let losers: Vec<&Layer> = losers.into_iter().map(|index| &layers[index]).collect();
                Diagnostic::new(
                    Self::ID,
                    Severity::Warning,
                    Self::message(winner, &losers),
                    &winner.path,
                    winner.line,
                    winner.column,
                )
                .help(Self::HELP)
            })
            .collect()
    }
}

impl FixedZIndexInversion {
    pub const ID: &'static str = "skyline/fixed-z-index-inversion";

    const HELP: &'static str = "Skyline 会把这个 fixed 元素提到页面根：把它自身的 `z-index` 调到高于被它覆盖的元素，或把两者放进同一个组件、不要依赖祖先的 `z-index` 分层";

    fn message(winner: &Layer, losers: &[&Layer]) -> String {
        let shown = losers
            .iter()
            .take(4)
            .map(|loser| format!("`{}:{}`（{}）", loser.path, loser.line, loser.own))
            .collect::<Vec<_>>()
            .join("、");
        let more = match losers.len().checked_sub(4) {
            Some(more) if more > 0 => format!(" 等 {} 个", losers.len()),
            _ => String::new(),
        };
        format!(
            "`z-index` 在 Skyline 下只在同一父节点内排序，fixed 又被提到页面根：这一层（{}）会盖住 WebView 里原本在它上面的 {} 个 fixed 层 —— {shown}{more}",
            winner.own,
            losers.len()
        )
    }
}

/// 比较键：先层叠位置，再文档序（同一层里后出现的在上）。
fn layer_key(z: i64, layer: &Layer) -> (i64, &str, u32, u32) {
    (z, layer.path.as_str(), layer.line, layer.column)
}

fn shares_tree(trees: &[HashSet<&str>], a: &str, b: &str) -> bool {
    trees
        .iter()
        .any(|docs| docs.contains(a) && docs.contains(b))
}

/// 收集所有 `position: fixed` 且有整数 `z-index` 的元素。
fn collect_layers(project: &Project, graph: &WxmlGraph) -> Vec<Layer> {
    let mut out = Vec::new();
    for ElementDoc { doc, .. } in element_docs(project) {
        let tree = Tree::new(&doc);
        walk_skyline(&doc, |element, ancestors| {
            let Some(position) = resolved_property(project, &tree, element, "position") else {
                return;
            };
            if !clean_value(&position.value).eq_ignore_ascii_case("fixed") {
                return;
            }
            let Some(own) = integer(project, &tree, element, "z-index") else {
                return;
            };
            // 最外层层叠上下文祖先：WebView 里它整层参与兄弟排序，内部 z-index 被关在里面。
            let flat = ancestors
                .iter()
                .rev()
                .find_map(|ancestor| {
                    let position = resolved_property(project, &tree, ancestor, "position")?;
                    if clean_value(&position.value).eq_ignore_ascii_case("static") {
                        return None;
                    }
                    integer(project, &tree, ancestor, "z-index")
                })
                .unwrap_or(own);
            let (line, column) = doc.position(element.tag_start);
            out.push(Layer {
                path: doc.path().to_string(),
                line,
                column,
                own,
                flat,
            });
        });
    }
    // `graph` 现阶段只用于确认文档在图里，避免把模板片段当成渲染树。
    out.retain(|layer| graph.contains(&layer.path));
    out
}

fn integer(
    project: &Project,
    tree: &Tree<'_>,
    element: &crate::input::wxml::WxmlElement,
    property: &str,
) -> Option<i64> {
    let resolved = resolved_property(project, tree, element, property)?;
    clean_value(&resolved.value).parse().ok()
}

/// 去掉 `!important` 与首尾空白。
fn clean_value(value: &str) -> &str {
    let value = value.trim();
    let lower = value.to_ascii_lowercase();
    match lower.strip_suffix("!important") {
        Some(head) => value[..head.len()].trim_end(),
        None => value,
    }
}

#[cfg(test)]
mod tests {
    use crate::Diagnostic;
    use crate::rules::Rule;
    use crate::rules::testutil::Fixture;

    use super::FixedZIndexInversion;

    fn check(
        name: &str,
        page: &str,
        page_style: &str,
        component: &str,
        component_style: &str,
    ) -> Vec<Diagnostic> {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                r#"{"usingComponents":{"panel":"/components/panel/panel"}}"#,
            )
            .write("pages/index/index.wxml", page)
            .write("pages/index/index.wxss", page_style)
            .write("pages/index/index.js", "Page({});")
            .write("components/panel/panel.json", r#"{"component":true}"#)
            .write("components/panel/panel.js", "Component({});")
            .write("components/panel/panel.wxml", component)
            .write("components/panel/panel.wxss", component_style);
        let project = fixture.project();
        FixedZIndexInversion.check(&project)
    }

    /// 评论输入框的形态：组件内 fixed 面板被祖先 `z-index: 0` 关住，使用方的 fixed 层更高。
    #[test]
    fn component_layer_inside_stacking_context_is_reported() {
        let diagnostics = check(
            "zindex-inversion",
            r#"<panel/><view class="input"/>"#,
            ".input { position: fixed; z-index: 4000; }",
            r#"<view class="wrap"><view class="panel"/></view>"#,
            ".wrap { position: relative; z-index: 0; }\n.panel { position: fixed; z-index: 5000; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        // 诊断落在「遮挡方」上：它是被提升到根、压住别人那一层。
        assert_eq!(diagnostics[0].path, "components/panel/panel.wxml");
        assert!(diagnostics[0].message.contains("5000"));
        // 被压住的层列在消息里，带它自己的位置与键，方便跳过去改。
        assert!(diagnostics[0].message.contains("pages/index/index.wxml:1"));
        assert!(diagnostics[0].message.contains("4000"));
    }

    /// 同一个遮挡方压住多个上层时只报一条（同因噪声收口）。
    #[test]
    fn one_occluder_is_reported_once() {
        let diagnostics = check(
            "zindex-aggregate",
            r#"<panel/><view class="input"/><view class="tag"/>"#,
            ".input { position: fixed; z-index: 4000; }\n.tag { position: fixed; z-index: 100; }",
            r#"<view class="wrap"><view class="panel"/></view>"#,
            ".wrap { position: relative; z-index: 0; }\n.panel { position: fixed; z-index: 5000; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        let message = &diagnostics[0].message;
        assert!(message.contains("2 个"), "{message}");
        assert!(message.contains("`pages/index/index.wxml:1`"), "{message}");
        assert!(
            message.contains("`pages/index/index.wxml:1`（4000）") && message.contains("（100）"),
            "{message}"
        );
    }

    /// 顺序与 WebView 一致时不报：内部层级本来就低于外面的 fixed。
    #[test]
    fn order_preserved_is_not_reported() {
        let diagnostics = check(
            "zindex-inversion-ok",
            r#"<panel/><view class="input"/>"#,
            ".input { position: fixed; z-index: 4000; }",
            r#"<view class="wrap"><view class="panel"/></view>"#,
            ".wrap { position: relative; z-index: 0; }\n.panel { position: fixed; z-index: 3000; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// 没有层叠上下文祖先时，两端的键相同，顺序不会翻转。
    #[test]
    fn no_stacking_ancestor_is_not_reported() {
        let diagnostics = check(
            "zindex-inversion-flat",
            r#"<panel/><view class="input"/>"#,
            ".input { position: fixed; z-index: 4000; }",
            r#"<view class="panel"/>"#,
            ".panel { position: fixed; z-index: 5000; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// 不同页面渲染树里的元素不构成竞争。
    #[test]
    fn different_trees_are_not_compared() {
        let fixture = Fixture::new("zindex-inversion-pages");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/a/a","pages/b/b"],"renderer":"skyline"}"#,
            )
            .write("pages/a/a.json", "{}")
            .write("pages/a/a.wxml", r#"<view class="low"/>"#)
            .write("pages/a/a.wxss", ".low { position: fixed; z-index: 4000; }")
            .write("pages/b/b.json", "{}")
            .write("pages/b/b.wxml", r#"<view class="wrap"><view class="panel"/></view>"#)
            .write(
                "pages/b/b.wxss",
                ".wrap { position: relative; z-index: 0; }\n.panel { position: fixed; z-index: 5000; }",
            );
        let project = fixture.project();
        assert!(FixedZIndexInversion.check(&project).is_empty());
    }
}
