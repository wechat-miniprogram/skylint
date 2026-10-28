//! `skyline/fixed-layer`：`position: fixed` 元素的定位基准在 Skyline 下变成了页面根，
//! 祖先的包含块失效。
//!
//! 引擎把 fixed 节点从原位换成一个 replacer，把节点本身 append 到 root container
//! （`shadow/shadow_tree.cc:683-706` 收集、`widgets/widget_operation.cc:492-546`
//! 的 `UpdatePositionFixedWidgetOperation::Apply` 里 `InsertBefore(replacer)` +
//! `root_container_widget->AppendChild(widget)`）；每个页面只有一个 ShadowTree
//! （`shadow/context.cc:142`），组件共用它——组件里的 fixed 同样被提到**页面** root。
//!
//! WebView 里 fixed 相对视口定位，但祖先一旦形成包含块（`transform` / `perspective` /
//! `filter` / `backdrop-filter` / `contain` / `will-change: transform`），它改成相对那个
//! 祖先定位；Skyline 把节点提到页面根后不再受该祖先约束。两端位置不同，这才是本规则
//! 报的点。
//!
//! 单纯的「fixed + 没有 z-index」不是缺陷：非 fixed 内容永远盖不住 fixed，两个都没有
//! z-index 时页面级顺序与 WebView 一致（都是文档序），实测五个参考工程这类形态全部
//! 零差异。早期版本按「有 fixed 无整数 z-index 就报」，在真实工程上 5/5 是噪音。

use std::collections::HashSet;

use crate::input::wxml::WxmlElement;
use crate::matcher::Tree;
use crate::project::Project;
use crate::rules::Rule;
use crate::rules::RuleMeta;
use crate::rules::wxml::{ElementDoc, element_docs, resolved_property, walk_skyline};
use crate::rules::wxss::value_util::at_declaration;
use crate::{Diagnostic, Severity};

const MESSAGE: &str = "`position: fixed` 在 Skyline 下被提到页面根，祖先的包含块不再约束它；这里相对该祖先定位，WebView 与 Skyline 的位置会不同";

const HELP: &str = "去掉祖先的 `transform` / `filter` / `contain` 等包含块触发条件，改成把 fixed 元素提到组件根（脱离该祖先），或把定位方式改成 `absolute` 并调整布局";

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

        for ElementDoc { doc, reach } in element_docs(project) {
            let tree = Tree::new(&doc);
            walk_skyline(&doc, |element, ancestors| {
                let Some(position) = resolved_property(project, &tree, element, "position") else {
                    return;
                };
                if !clean_value(&position.value).eq_ignore_ascii_case("fixed") {
                    return;
                }
                // 只有祖先形成包含块时，WebView 与 Skyline 的定位基准才不同。
                if !has_containing_block(project, &tree, ancestors) {
                    return;
                }

                let diagnostic = match position.sheet.zip(position.declaration) {
                    Some((sheet, declaration)) => {
                        if !reported.insert((sheet.doc.path().to_string(), declaration.offset, 0)) {
                            return;
                        }
                        at_declaration(
                            Self::ID,
                            &sheet.doc,
                            declaration,
                            Severity::Warning,
                            MESSAGE,
                            reach.help(HELP),
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
                        .help(reach.help(HELP))
                    }
                };
                out.push(diagnostic);
            });
        }
        out
    }
}

/// 最近一个形成包含块的祖先。
///
/// WebView 里 fixed 会被这样的祖先「抓住」（相对它定位），Skyline 提到页面根后不受
/// 约束；没有这类祖先时两端都相对视口，行为一致。
///
/// `z-index` 不算：它只影响绘制顺序，不构成 fixed 的包含块。页面级叠放顺序两端都是
/// 文档序，也是为什么「fixed + 没有 z-index」本身不是缺陷。
fn has_containing_block(project: &Project, tree: &Tree<'_>, ancestors: &[&WxmlElement]) -> bool {
    let resolve = |element: &WxmlElement, property: &str| {
        resolved_property(project, tree, element, property)
            .map(|resolved| clean_value(&resolved.value).to_ascii_lowercase())
    };
    ancestors.iter().any(|ancestor| {
        let set = |property: &str| {
            resolve(ancestor, property)
                .is_some_and(|value| !value.is_empty() && value != "none" && value != "normal")
        };
        let animated = resolve(ancestor, "will-change").is_some_and(|value| {
            ["transform", "perspective", "filter"]
                .iter()
                .any(|trigger| value.contains(trigger))
        });
        set("transform")
            || set("perspective")
            || set("filter")
            || set("backdrop-filter")
            || set("contain")
            || animated
    })
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

    #[test]
    fn webview_only_component_is_reported_with_a_note() {
        // 组件只被 `renderer: "webview"` 的页面引用：元素级规则也要检查它，
        // 否则同一份文件的声明级规则报、元素级不报。
        let fixture = Fixture::new("fixed-webview-only");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index","pages/legacy/legacy"]}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", r#"<view/>"#)
            .write(
                "pages/legacy/legacy.json",
                r#"{"renderer":"webview","usingComponents":{"d":"/components/d/d"}}"#,
            )
            .write("pages/legacy/legacy.wxml", r#"<d/>"#)
            .write("components/d/d.json", "{}")
            .write(
                "components/d/d.wxml",
                r#"<view class="panel"><view class="bar"/></view>"#,
            )
            .write(
                "components/d/d.wxss",
                ".panel { transform: translate3d(0, 100%, 0); }\n.bar { position: fixed; bottom: 0; }"
            );
        let diagnostics = FixedLayer.check(&fixture.project());
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].path, "components/d/d.wxss");
        assert!(
            diagnostics[0]
                .help
                .as_deref()
                .is_some_and(|help| help.contains("只在 WebView 下渲染")),
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn containing_ancestor_makes_the_position_differ() {
        // WebView 里 fixed 相对这个 transform 祖先定位，Skyline 提到页面根后相对视口。
        let wxml = r#"<view class="panel">
  <view class="bar"/>
</view>"#;
        let diagnostics = check(
            "fixed-layer-transform",
            wxml,
            ".panel { transform: translate3d(0, 100%, 0); }
             .bar { position: fixed; bottom: 0; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("包含块"));
    }

    #[test]
    fn fixed_without_containing_ancestor_is_skipped() {
        // 两端都相对视口：没有 z-index 也不影响位置，只影响页面级叠放顺序，
        // 而那部分两端都是文档序。
        let wxml = r#"<view class="page">
  <view class="mask"/>
</view>"#;
        let diagnostics = check(
            "fixed-layer-plain",
            wxml,
            ".mask { position: fixed; top: 0; left: 0; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn z_index_ancestor_is_not_a_containing_block() {
        // `z-index` 只影响绘制顺序，不改变 fixed 的定位基准。
        let wxml = r#"<view class="panel">
  <view class="bar"/>
</view>"#;
        let diagnostics = check(
            "fixed-layer-z-ancestor",
            wxml,
            ".panel { position: relative; z-index: 5; }
             .bar { position: fixed; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn filter_and_contain_also_form_containing_blocks() {
        for (name, declaration) in [
            ("filter", "filter: blur(1px);"),
            ("backdrop", "backdrop-filter: blur(1px);"),
            ("contain", "contain: paint;"),
            ("perspective", "perspective: 100px;"),
        ] {
            let wxml = r#"<view class="panel">
  <view class="bar"/>
</view>"#;
            let diagnostics = check(
                &format!("fixed-layer-{name}"),
                wxml,
                &format!(".panel {{ {declaration} }} .bar {{ position: fixed; }}"),
            );
            assert_eq!(diagnostics.len(), 1, "{name}: {diagnostics:#?}");
        }
    }

    #[test]
    fn inline_style_is_reported_at_the_element() {
        let wxml = r#"<view style="transform: translateZ(0)">
  <view style="position: fixed"/>
</view>"#;
        let diagnostics = check("fixed-layer-inline", wxml, "");
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (2, 4));
        assert!(diagnostics[0].path.ends_with("pages/index/index.wxml"));
    }

    #[test]
    fn non_fixed_positions_are_not_reported() {
        let wxml = r#"<view class="panel">
  <view class="absolute"/>
  <view class="relative"/>
  <view class="sticky"/>
</view>"#;
        let diagnostics = check(
            "fixed-layer-other-positions",
            wxml,
            ".panel { transform: scale(1); }
             .absolute { position: absolute; }
             .relative { position: relative; }
             .sticky { position: sticky; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn every_fixed_element_is_reported_once() {
        // 同一文档里多个 fixed（含内联）不能被去重吞掉。
        let wxml = r#"<view class="panel">
  <view class="mask"/>
  <view class="toast" style="position: fixed"/>
</view>"#;
        let diagnostics = check(
            "fixed-layer-multiple",
            wxml,
            ".panel { transform: translateY(0); }
             .mask { position: fixed; }
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
    fn fixed_inside_a_component_is_reported_when_its_ancestor_contains_it() {
        // 组件共用页面的 ShadowTree，fixed 提到页面 root；祖先的 transform 在 WebView
        // 里会抓住它，两端位置不同。
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
            .write(
                "components/mask/mask.wxml",
                r#"<view class="wrap"><view class="mask"/></view>"#,
            )
            .write(
                "components/mask/mask.wxss",
                ".wrap { transform: translateZ(0); } .mask { position: fixed; }",
            );
        let project = fixture.project();
        let diagnostics = FixedLayer.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].path, "components/mask/mask.wxss");
    }
}
