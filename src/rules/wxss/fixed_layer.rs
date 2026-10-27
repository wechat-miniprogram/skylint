//! `skyline/fixed-layer`：`position: fixed` 元素在 Skyline 下被提升成独立层，
//! 没有有效 `z-index` 时它和页面内容的先后不可控。
//!
//! 引擎把 fixed 节点从原位换成一个 replacer，把节点本身 append 到 root container
//! （`shadow/shadow_tree.cc:683-706` 收集、`widgets/widget_operation.cc:492-546`
//! 的 `UpdatePositionFixedWidgetOperation::Apply` 里 `InsertBefore(replacer)` +
//! `root_container_widget->AppendChild(widget)`）。root container 内按子节点的
//! z-index 稳定排序（`package/skyline/lib/rendering/stacking.dart:44-60`），而整个
//! 页面只是它的**一个**子节点：fixed 节点与页面根平级，页面内部的层级关系整体压在
//! 页面根之下。它只有写了自己的 z-index 才能和页面根分出先后；两者都是 `auto` 时
//! 顺序由引擎的 append 时机决定，静态不可控——页面里 z-index 200 / 999 的浮层既可能
//! 盖住它，也可能被它盖住。这就是本规则报的点。
//!
//! 判据只有一条：解析后的 `position` 是 `fixed`，且解析不出有效的 `z-index`
//! （无声明 / `auto` / 非整数 / 变量或插值都算没有）。用户明确要求误报优于漏报，
//! 所以这里不做任何额外收窄。
//!
//! 为什么不再做层比较：旧实现试图找出「被这个 fixed 盖住的、z-index 更高的同父兄弟
//! 浮层」，为此要求 fixed 铺满视口（`top`/`left: 0` 且另一维撑满）。实测六个工程
//! 0 命中——真实工程里 fixed 很少写成这种严格形态（多用 `inset`、`100%`、安全区偏移），
//! 收窄后等于没有覆盖；跨文档比较同样不可靠（组件里的 fixed 提升到组件自己的 root
//! container，页面浮层仍在它之上，见历史 bug `8fc12686`）。

use std::collections::{HashMap, HashSet};

use crate::input::wxml::{WxmlDoc, WxmlElement};
use crate::matcher::Tree;
use crate::project::Project;
use crate::rules::Rule;
use crate::rules::RuleMeta;
use crate::rules::wxml::{load_doc, resolved_property, walk};
use crate::rules::wxss::value_util::at_declaration;
use crate::{Diagnostic, Severity};

const MESSAGE: &str = "`position: fixed` 在 Skyline 下被提升成独立层、与页面根平级；这里没有有效的 `z-index`，它和整页内容的先后不可控";

const HELP: &str = "需要它固定在最上层时显式给一个足够大的 `z-index`（例如 `z-index: 999`），否则它的层级取决于引擎把节点挂到 root container 的时机";

pub struct FixedLayer;

impl FixedLayer {
    pub const ID: &'static str = "skyline/fixed-layer";
}

impl Rule for FixedLayer {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: FixedLayer::ID,
            severity: Severity::Warning,
            description: "`position: fixed` 在 Skyline 下被提升成与页面根平级的独立层，没有有效 `z-index` 时先后不可控",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        // 同一个位置只报一次：样式表按声明偏移，内联 `style` 按元素起始位置。
        let mut reported: HashSet<(String, usize, usize)> = HashSet::new();
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

            for doc in loaded {
                let tree = Tree::new(doc);
                walk(doc, |element, _| {
                    let Some(position) = resolved_property(project, &tree, element, "position")
                    else {
                        return;
                    };
                    if !clean_value(&position.value).eq_ignore_ascii_case("fixed") {
                        return;
                    }
                    if has_z_index(project, &tree, element) {
                        return;
                    }

                    let diagnostic = match position.sheet.zip(position.declaration) {
                        Some((sheet, declaration)) => {
                            if !reported.insert((
                                sheet.doc.path().to_string(),
                                declaration.offset,
                                0,
                            )) {
                                return;
                            }
                            at_declaration(
                                Self::ID,
                                &sheet.doc,
                                declaration,
                                Severity::Warning,
                                MESSAGE,
                                HELP,
                            )
                        }
                        // 内联 `style`：没有声明可落，报在元素开始标签上。
                        None => {
                            let (line, column) = doc.position(element.tag_start);
                            if !reported.insert((
                                doc.path().to_string(),
                                line as usize,
                                column as usize,
                            )) {
                                return;
                            }
                            Diagnostic::new(
                                Self::ID,
                                Severity::Warning,
                                MESSAGE,
                                doc.path(),
                                line,
                                column,
                            )
                            .help(HELP)
                        }
                    };
                    out.push(diagnostic);
                });
            }
        }
        out
    }
}

/// 元素是否有一个能参与层叠的 `z-index`：解析出整数才算。
///
/// 无声明、`auto`、`calc(...)`、变量与插值一律视为没有。
fn has_z_index(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> bool {
    resolved_property(project, tree, element, "z-index")
        .is_some_and(|resolved| clean_value(&resolved.value).parse::<i64>().is_ok())
}

/// 去掉取值末尾的 `!important`。
///
/// 样式表解析器已在 `WxssDeclaration::value` 里剥掉它，内联 `style` 走的是纯文本
/// 切分（`inline_value`），所以这里统一再剥一次。
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

    use super::FixedLayer;

    fn check(name: &str, wxml: &str, wxss: &str) -> Vec<Diagnostic> {
        let fixture = Fixture::wxml(name, wxml);
        fixture.write("pages/index/index.wxss", wxss);
        let project = fixture.project();
        FixedLayer.check(&project)
    }

    const PAGE: &str = r#"<view class="page">
  <view class="mask"/>
</view>"#;

    #[test]
    fn fixed_without_z_index_is_reported() {
        let diagnostics = check(
            "fixed-layer-no-z",
            PAGE,
            ".mask { position: fixed; top: 0; left: 0; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (1, 9));
        assert!(diagnostics[0].message.contains("z-index"));
        assert!(diagnostics[0].help.as_deref().unwrap_or("").contains("999"));
    }

    #[test]
    fn fixed_with_numeric_z_index_is_not_reported() {
        let diagnostics = check(
            "fixed-layer-numeric-z",
            PAGE,
            ".mask { position: fixed; z-index: 999; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn auto_z_index_counts_as_missing() {
        let diagnostics = check(
            "fixed-layer-auto-z",
            PAGE,
            ".mask { position: fixed; z-index: auto; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    #[test]
    fn dynamic_z_index_counts_as_missing() {
        // WXSS 里 `{{}}` 是语法错误（整个样式表解析失败），动态值只可能出现在内联
        // `style` 里，见下面的用例。
        let diagnostics = check(
            "fixed-layer-dynamic-z",
            PAGE,
            ".mask { position: fixed; z-index: var(--layer); }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    #[test]
    fn inline_mustache_z_index_counts_as_missing() {
        // `{{}}` 出现在 WXSS 里是语法错误（整个样式表解析失败），出现在内联 `style`
        // 里时 WXML 解析器不提供字符串值；两种情况都只能当成「没有 z-index」。
        let wxml = r#"<view class="page">
  <view class="mask" style="z-index: {{layer}}"/>
</view>"#;
        let diagnostics = check(
            "fixed-layer-inline-mustache",
            wxml,
            ".mask { position: fixed; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (1, 9));
    }

    #[test]
    fn inline_fixed_style_is_reported_at_the_element() {
        let wxml = r#"<view class="page">
  <view style="position: fixed"/>
</view>"#;
        let diagnostics = check("fixed-layer-inline", wxml, "");
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (2, 4));
        assert!(diagnostics[0].path.ends_with("pages/index/index.wxml"));
    }

    #[test]
    fn inline_style_is_normalized_for_important() {
        // 两个元素：一个带有效 z-index，一个没有；`!important` 不能影响判定。
        let wxml = r#"<view class="page">
  <view style="position: fixed !important; z-index: 10 !important"/>
  <view style="position: fixed !important"/>
</view>"#;
        let diagnostics = check("fixed-layer-inline-important", wxml, "");
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (3, 4));
    }

    #[test]
    fn non_fixed_positions_are_not_reported() {
        let wxml = r#"<view class="page">
  <view class="absolute"/>
  <view class="relative"/>
  <view class="sticky"/>
  <view class="static"/>
</view>"#;
        let diagnostics = check(
            "fixed-layer-other-positions",
            wxml,
            ".absolute { position: absolute; }
             .relative { position: relative; }
             .sticky { position: sticky; }
             .static { z-index: 1; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn every_fixed_element_is_reported_once() {
        // 同一文档里多个 fixed（含内联）不能被去重吞掉。
        let wxml = r#"<view class="page">
  <view class="mask"/>
  <view class="toast" style="position: fixed"/>
</view>"#;
        let diagnostics = check(
            "fixed-layer-multiple",
            wxml,
            ".mask { position: fixed; }
             .toast { top: 0; }",
        );
        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
        let mut positions = diagnostics
            .iter()
            .map(|diagnostic| (diagnostic.path.clone(), diagnostic.line, diagnostic.column))
            .collect::<Vec<_>>();
        positions.sort();
        positions.dedup();
        assert_eq!(positions.len(), 2, "{diagnostics:#?}");
    }

    #[test]
    fn fixed_inside_a_component_is_still_reported() {
        // 组件里的 fixed 提升到的是组件自己的 root container，与组件根同样不可控；
        // 按「误报优于漏报」不做跨文档豁免。
        let fixture = Fixture::new("fixed-layer-component");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                r#"{"usingComponents":{"mask-layer":"/components/mask/mask"}}"#,
            )
            .write("pages/index/index.wxml", r#"<mask-layer/>"#)
            .write("components/mask/mask.json", r#"{"component":true}"#)
            .write("components/mask/mask.wxml", r#"<view class="mask"/>"#)
            .write("components/mask/mask.wxss", ".mask { position: fixed; }");
        let project = fixture.project();
        let diagnostics = FixedLayer.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].path, "components/mask/mask.wxss");
    }
}
