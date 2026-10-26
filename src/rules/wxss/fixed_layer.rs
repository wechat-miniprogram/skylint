//! `skyline/fixed-layer`：`position: fixed` 元素在 Skyline 下被提升成独立一层，
//! 盖住 z-index 更高的普通定位元素。
//!
//! 引擎把 fixed 节点从原位换成一个 replacer，把节点本身 append 到 root container
//! （`shadow/shadow_tree.cc:683-706` 收集、`widgets/widget_operation.cc:492-546`
//! 的 `UpdatePositionFixedWidgetOperation::Apply` 里 `InsertBefore(replacer)` +
//! `root_container_widget->AppendChild(widget)`）。root container 的绘制顺序按子节点
//! z-index 稳定排序（`package/skyline/lib/rendering/stacking.dart:44-60`），而整个页面
//! 是它的**一个**子节点——fixed 节点的 z-index 因此只和「页面根」这一层比较，页面内部
//! 那些 z-index 200 / 999 的浮层全在它之下。
//!
//! WebView 里两者在同一个 stacking context 里按 z-index 比较，所以「fixed 盖住
//! z-index 更高的浮层」只在 Skyline 发生。真实适配里四处踩过这个坑，修法都是把遮罩
//! 从 fixed 改成 absolute（见 weapp-novel-plugin 的
//! `skylinenovel/index.less:960,2132,2882` 三处注释与「返回按钮点不动」）。
//!
//! 误报边界（两条都要满足才报）：
//! - 只判铺满视口的 fixed 遮罩（`top: 0` + `left: 0` 且另一维撑满）。固定边条、底栏、
//!   离屏挂载的画布/快照同样会被提升，但是否与更高 z-index 的浮层重叠静态判不了，
//!   报出来全是噪音——真实工程里这几类占绝大多数。
//! - 只判同一个父节点下的兄弟浮层。跨文档（页面 ↔ 组件）不判：组件里的 fixed 提升到
//!   的是组件自己的 root container，页面浮层仍在它之上（历史 bug `8fc12686` 正是
//!   「fixed 在组件、被盖的 z-250 在页面」，当时要靠抬高 z-index 才盖住）。

use std::collections::{HashMap, HashSet};

use crate::input::wxml::{WxmlDoc, WxmlElement};
use crate::matcher::Tree;
use crate::project::Project;
use crate::rules::Rule;
use crate::rules::RuleMeta;
use crate::rules::wxml::{load_doc, resolved_property, walk};
use crate::rules::wxss::value_util::at_declaration;
use crate::{Diagnostic, Severity};

pub struct FixedLayer;

impl FixedLayer {
    pub const ID: &'static str = "skyline/fixed-layer";
}

/// 参与比较的一层：定位元素、它的 z-index、它的祖先链（判断层关系用）。
struct Layer<'a> {
    doc: &'a WxmlDoc,
    element: &'a WxmlElement,
    parent: Option<*const WxmlElement>,
    z: i64,
    fixed: bool,
    /// fixed 层的 `position: fixed` 声明（内联 `style` 时为空，报在元素上）。
    origin: Option<(
        &'a crate::project::StyleSheet,
        &'a crate::input::wxss::WxssDeclaration,
    )>,
}

impl Rule for FixedLayer {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: FixedLayer::ID,
            severity: Severity::Warning,
            description: "`position: fixed` 在 Skyline 下被提升成独立层，会盖住 z-index 更高的浮层",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        let mut reported: HashSet<(String, usize)> = HashSet::new();
        let mut docs: HashMap<String, Option<WxmlDoc>> = HashMap::new();

        for page in &project.pages {
            if page.opted_out {
                continue;
            }
            let Some(page_wxml) = page.files.wxml.as_deref() else {
                continue;
            };
            let mut paths = vec![page_wxml.to_string()];
            paths.extend(project.page_components(&page.route).into_iter().filter_map(
                |component| {
                    project
                        .components
                        .iter()
                        .find(|unit| unit.path == component)
                        .and_then(|unit| unit.files.wxml.clone())
                },
            ));

            for path in &paths {
                if !docs.contains_key(path) {
                    docs.insert(path.clone(), load_doc(project, path));
                }
            }
            let loaded: Vec<&WxmlDoc> = paths
                .iter()
                .filter_map(|path| docs.get(path).and_then(Option::as_ref))
                .collect();

            let mut layers: Vec<Layer<'_>> = Vec::new();
            for doc in loaded.iter().copied() {
                let tree = Tree::new(doc);
                walk(doc, |element, ancestors| {
                    let Some(position) = resolved_property(project, &tree, element, "position")
                        .map(|resolved| resolved.value.trim().to_ascii_lowercase())
                    else {
                        return;
                    };
                    let fixed = position == "fixed";
                    // `static` 上的 z-index 在两种渲染器下都不参与层叠。
                    if !fixed && !matches!(position.as_str(), "absolute" | "relative" | "sticky") {
                        return;
                    }
                    let resolved = resolved_property(project, &tree, element, "z-index");
                    let z = resolved
                        .as_ref()
                        .and_then(|resolved| resolved.value.trim().parse().ok())
                        .unwrap_or(0);
                    if fixed && !covers_viewport(project, &tree, element) {
                        return;
                    }
                    layers.push(Layer {
                        doc,
                        element,
                        parent: ancestors.last().map(|a| *a as *const _),
                        z,
                        fixed,
                        origin: fixed
                            .then(|| {
                                resolved_property(project, &tree, element, "position").and_then(
                                    |resolved| Some((resolved.sheet?, resolved.declaration?)),
                                )
                            })
                            .flatten(),
                    });
                });
            }

            for (index, fixed) in layers.iter().enumerate().filter(|(_, l)| l.fixed) {
                let Some(above) = layers
                    .iter()
                    .enumerate()
                    .filter(|(other, layer)| *other != index && !layer.fixed && layer.z > fixed.z)
                    .filter(|(_, layer)| !related(layer, fixed))
                    .max_by_key(|(_, layer)| layer.z)
                    .map(|(_, layer)| layer)
                else {
                    continue;
                };
                let message = format!(
                    "`position: fixed` 在 Skyline 下被提升成独立层，会盖住 z-index 为 {} 的 {}；WebView 下两者在同一层按 z-index 比较，是它盖住这个 fixed 元素",
                    above.z,
                    describe(above.element)
                );
                let help = "遮罩 / 弹层顶层改用 `position: absolute` 放进页面的 relative 容器，才能与其它浮层比较 z-index；必须 fixed 时把它和浮层放在同一父节点下并给最高 z-index";
                let diagnostic = match fixed.origin {
                    Some((sheet, declaration)) => {
                        if !reported.insert((sheet.doc.path().to_string(), declaration.offset)) {
                            continue;
                        }
                        at_declaration(
                            Self::ID,
                            &sheet.doc,
                            declaration,
                            Severity::Warning,
                            message,
                            help,
                        )
                    }
                    None => {
                        if !reported.insert((fixed.doc.path().to_string(), 0)) {
                            continue;
                        }
                        Diagnostic::new(
                            Self::ID,
                            Severity::Warning,
                            message,
                            fixed.doc.path(),
                            fixed.element.tag_start.line,
                            fixed.element.tag_start.utf16_col,
                        )
                        .help(help)
                    }
                };
                out.push(diagnostic);
            }
        }
        out
    }
}

/// fixed 元素是否铺满视口（遮罩层）。
///
/// 只有遮罩层真正依赖 z-index 去和别的浮层分先后；装饰性的 fixed 元素
/// （离屏挂载、固定边条）被提升一层也不会改变可见结果，报出来是噪音。
fn covers_viewport(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> bool {
    let value = |property: &str| {
        resolved_property(project, tree, element, property)
            .map(|resolved| resolved.value.trim().to_ascii_lowercase())
            .unwrap_or_default()
    };
    let zero = |text: &str| matches!(text, "0" | "0px" | "0rpx" | "0%");
    let full_width = zero(&value("left"))
        && (zero(&value("right")) || value("width") == "100vw" || value("width") == "100%");
    let full_height = zero(&value("top"))
        && (zero(&value("bottom")) || value("height") == "100vh" || value("height") == "100%");
    full_width && full_height
}

/// 只有同一个父节点下的兄弟浮层才按同一个 stacking context 比较。
///
/// 跨文档（页面 ↔ 组件）不判：组件里的 fixed 节点提升到的是组件自己的 root
/// container，页面里的浮层仍然在它之上——历史 bug `8fc12686` 就是「fixed 在组件、
/// 被盖的 z-250 在页面」，当时必须靠抬高 z-index 才盖住，说明跨文档并不发生提升。
fn related(layer: &Layer<'_>, fixed: &Layer<'_>) -> bool {
    !std::ptr::eq(layer.doc, fixed.doc) || layer.parent != fixed.parent
}

/// 诊断里指代一个元素：优先给类名，其次标签。
fn describe(element: &WxmlElement) -> String {
    let class = element
        .attributes
        .iter()
        .find(|attr| attr.name == "class")
        .and_then(|attr| attr.value.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.contains("{{"));
    match class {
        Some(class) => format!("<{} class=\"{class}\">", element.tag),
        None => format!("<{}>", element.tag),
    }
}

#[cfg(test)]
mod tests {
    use crate::Diagnostic;
    use crate::project::Project;
    use crate::rules::Rule;
    use crate::rules::testutil::Fixture;

    use super::FixedLayer;

    /// 页面结构：一个容器里放遮罩与一个更高 z-index 的浮层。
    const PAGE: &str = r#"<view class="page">
  <view class="mask"/>
  <view class="panel"/>
</view>"#;

    fn check(name: &str, wxss: &str) -> (Fixture, Project, Vec<Diagnostic>) {
        let fixture = Fixture::wxml(name, PAGE);
        fixture.write("pages/index/index.wxss", wxss);
        let project = fixture.project();
        let diagnostics = FixedLayer.check(&project);
        (fixture, project, diagnostics)
    }

    #[test]
    fn fixed_mask_covering_higher_panel_is_reported() {
        let (_f, _p, diagnostics) = check(
            "fixed-layer-mask",
            ".mask { position: fixed; top: 0; left: 0; width: 100vw; height: 100vh; z-index: 1; }
             .panel { position: absolute; bottom: 0; z-index: 2; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("z-index 为 2"));
        assert!(diagnostics[0].message.contains("panel"));
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (1, 9));
    }

    #[test]
    fn fixed_mask_above_everything_is_not_reported() {
        let (_f, _p, diagnostics) = check(
            "fixed-layer-highest",
            ".mask { position: fixed; top: 0; left: 0; width: 100vw; height: 100vh; z-index: 9; }
             .panel { position: absolute; bottom: 0; z-index: 2; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn non_fullscreen_fixed_element_is_skipped() {
        // 离屏挂载 / 固定边条：同样被提升，但静态判不出是否与浮层重叠。
        let (_f, _p, diagnostics) = check(
            "fixed-layer-offscreen",
            ".mask { position: fixed; left: -9999px; top: -9999px; width: 500px; height: 400px; }
             .panel { position: absolute; bottom: 0; z-index: 2; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn static_z_index_does_not_create_a_layer() {
        // `static` 元素上的 z-index 在两种渲染器下都不参与层叠。
        let (_f, _p, diagnostics) = check(
            "fixed-layer-static",
            ".mask { position: fixed; top: 0; left: 0; width: 100vw; height: 100vh; z-index: 1; }
             .panel { z-index: 2; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn fixed_inside_the_panel_is_not_reported() {
        let fixture = Fixture::wxml(
            "fixed-layer-nested",
            r#"<view class="panel"><view class="mask"/></view><view class="other" style="z-index: 5"/>"#,
        );
        fixture.write(
            "pages/index/index.wxss",
            ".mask { position: fixed; top: 0; left: 0; width: 100vw; height: 100vh; z-index: 1; }
             .panel { position: relative; z-index: 2; }",
        );
        let project = fixture.project();
        // 遮罩在 panel 的子树里，两种渲染器都由父级决定层叠；页面上没有别的同父兄弟。
        assert!(FixedLayer.check(&project).is_empty());
    }

    #[test]
    fn fixed_in_component_is_not_compared_with_page_layers() {
        // 组件里的 fixed 提升到的是组件自己的 root container，页面浮层仍在它之上。
        let fixture = Fixture::new("fixed-layer-component");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", r#"{"usingComponents":{"mask-layer":"/components/mask/mask"}}"#)
            .write(
                "pages/index/index.wxml",
                r#"<mask-layer/><view class="panel"/>"#,
            )
            .write(
                "pages/index/index.wxss",
                ".panel { position: absolute; z-index: 999; }",
            )
            .write("components/mask/mask.json", r#"{"component":true}"#)
            .write("components/mask/mask.wxml", r#"<view class="mask"/>"#)
            .write(
                "components/mask/mask.wxss",
                ".mask { position: fixed; top: 0; left: 0; width: 100vw; height: 100vh; z-index: 1; }",
            );
        let project = fixture.project();
        assert!(FixedLayer.check(&project).is_empty());
    }
}
