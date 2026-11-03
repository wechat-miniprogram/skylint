//! `skyline/native-padding`：内置组件上的 CSS `padding` 不会内缩它的子节点。
//!
//! 判据只有一条：这个组件在 Skyline 里**有没有自己的 CSS 盒**。子节点一旦成为独立布局
//! 根，父节点的 padding（连同 border / gap）就与它无关：
//!
//! - `shadow/shadow_node.cc:195-203`：父 widget 链尾不是 `YGConnector` 就 `SetAsRoot()`；
//! - `shadow/shadow_node.cc:211-226`：`CreateConcreteWidget()` 返回空才建 `YGConnector`；
//! - `widgets/yoga_connector.cc:95-107`：全仓唯一把子 yoga 节点挂进父树的入口；
//! - `widgets/yoga.cc:616-624` + `package/skyline/lib/widgets/yoga.dart:575-584`：concrete
//!   widget 拿到的约束是 border box，padding 没有扣减机会。
//!
//! 两档后果不同，所以文案分开：
//!
//! - [`NativeKind::NoCssBox`]：`InflateNaked()` / `SetSkipStyleBuild()`，连 yoga 节点都不建，
//!   CSS 布局属性一律无效（`shadow_node.cc:269-277`）。`list-view` / `grid-view` /
//!   `list-builder` / `grid-builder` 还会在 style builder 的白名单里被挡掉，只剩背景色
//!   （`style/style_builder.cc:155-166`、`:446-535`）。
//! - [`NativeKind::BorderBoxTail`]：有 yoga 节点、padding 也写进去了（`style_builder.cc:648-670`），
//!   但 concrete widget 是 widget 链尾，子节点仍是独立根，所以内容不内缩。
//!
//! WebView 里这些组件的内部是普通 DOM（`wx-swiper > .wx-swiper-wrapper{width:100%}` 之类），
//! padding 照常内缩，属于渲染器差异。
//!
//! 排除 `swiper-item`：它 `CreateConcreteWidget()` 返回空 → 走 `YGConnector`，子节点进它
//! 自己的 yoga 树，padding 正常生效（`shadow/components/swiper_item_shadow_node.cc:30-32`）。

use crate::matcher::Tree;
use crate::project::Project;
use crate::rules::Rule;
use crate::rules::RuleMeta;
use crate::rules::wxml::{
    ElementDoc, Reach, element_diagnostic, element_docs, resolved_property, walk_skyline,
};
use crate::rules::wxss::natives::{NATIVES, Native, NativeKind};
use crate::{Diagnostic, Severity};

pub struct NativePadding;

const PADDING_ATTR: &str = "把内边距写成组件属性 `padding=\"{{[top,right,bottom,left]}}\"`（与 CSS padding 是两条通路；组件 padding 是 sliver padding，滚动方向上也占位），或在里面包一层普通 `view` 写 padding";
const INNER_VIEW: &str =
    "这个组件没有 `padding` 属性：在里面包一层普通 `view`，把内边距写在那层 `view` 上";

impl NativePadding {
    pub const ID: &'static str = "skyline/native-padding";

    /// 声明了非零 padding 的属性名（含 `padding` 简写）。
    const PROPERTIES: [&'static str; 5] = [
        "padding",
        "padding-top",
        "padding-right",
        "padding-bottom",
        "padding-left",
    ];

    fn message(native: &Native) -> String {
        match native.kind {
            NativeKind::NoCssBox { .. } => format!(
                "`{}` 在 Skyline 下由原生 widget 排布子节点、自身不参与 CSS 布局，CSS `padding` 不会内缩子节点（WebView 下生效）",
                native.tag
            ),
            NativeKind::BorderBoxTail => format!(
                "`{}` 的 CSS `padding` 不会内缩子节点：子节点是独立布局根，容器按 border box 铺满（WebView 下生效）",
                native.tag
            ),
        }
    }
}

impl Rule for NativePadding {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "内置组件上的 CSS padding 在 Skyline 下不内缩子节点",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut out = Vec::new();

        for ElementDoc { doc, reach } in element_docs(project) {
            if reach != Reach::Skyline {
                continue;
            }
            let tree = Tree::new(&doc);
            walk_skyline(&doc, |element, _| {
                let Some(native) = NATIVES.iter().find(|native| native.tag == element.tag) else {
                    return;
                };
                let padded = Self::PROPERTIES.iter().any(|property| {
                    resolved_property(project, &tree, element, property)
                        .is_some_and(|resolved| nonzero_padding(&resolved.value))
                });
                if !padded {
                    return;
                }
                out.push(
                    element_diagnostic(
                        Self::ID,
                        &doc,
                        element,
                        doc.path(),
                        Severity::Warning,
                        Self::message(native),
                    )
                    .help(reach.help(if native.padding_attr {
                        PADDING_ATTR
                    } else {
                        INNER_VIEW
                    })),
                );
            });
        }
        out
    }
}

/// 值里有任意一边不为 0 就是「真的想留内边距」。
///
/// 判不了的（`{{…}}`、`var(…)` 一类运行时值）按 0 处理，宁可漏报。
fn nonzero_padding(value: &str) -> bool {
    value.split_whitespace().any(|token| {
        if token.contains("var(") {
            return false;
        }
        let number = token.trim_end_matches(|c: char| c.is_ascii_alphabetic() || c == '%');
        number.parse::<f64>().is_ok_and(|number| number != 0.0)
    })
}

#[cfg(test)]
mod tests {
    use crate::Diagnostic;
    use crate::rules::Rule;
    use crate::rules::testutil::wxss::Fixture;

    use super::NativePadding;

    const SKYLINE_APP: &str = r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#;

    fn check(name: &str, wxml: &str, wxss: &str) -> Vec<Diagnostic> {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write("app.json", SKYLINE_APP)
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", wxml)
            .write("pages/index/index.wxss", wxss);
        let project = fixture.project();
        NativePadding.check(&project)
    }

    const SWIPER: &str = r#"<swiper class="slides"><swiper-item class="page"><view class="inner"/></swiper-item></swiper>"#;
    const SCROLL: &str = r#"<scroll-view class="list"><view class="item"/></scroll-view>"#;

    #[test]
    fn border_box_tail_component_is_reported() {
        let diagnostics = check(
            "native-pad-swiper",
            SWIPER,
            ".slides { margin: 0 -16px; padding: 0 16px; height: 200px; }\n.page { padding: 0 16px; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("独立布局根"));
        assert!(diagnostics[0].message.contains("`swiper`"));
        assert_eq!(diagnostics[0].line, 1);
    }

    #[test]
    fn ignored_css_component_is_reported_with_padding_attribute_help() {
        for (name, tag) in [
            ("native-pad-scroll", "scroll-view"),
            ("native-pad-listview", "list-view"),
            ("native-pad-sticky", "sticky-section"),
        ] {
            let diagnostics = check(
                name,
                &format!("<{tag} class=\"box\"><view class=\"item\"/></{tag}>"),
                ".box { padding: 0 16px; }",
            );
            assert_eq!(diagnostics.len(), 1, "{name}: {diagnostics:#?}");
            assert!(diagnostics[0].message.contains(&format!("`{tag}`")));
            assert!(
                diagnostics[0]
                    .help
                    .as_deref()
                    .is_some_and(|help| help.contains("padding=\"{{["))
            );
        }
    }

    /// `nested-scroll-header` / `nested-scroll-body` 没有 `padding` 属性，建议只能是内层包一层。
    #[test]
    fn component_without_padding_attribute_suggests_inner_view() {
        let diagnostics = check(
            "native-pad-nested",
            r#"<scroll-view type="nested"><nested-scroll-body class="body"><view class="item"/></nested-scroll-body></scroll-view>"#,
            ".body { padding: 16px; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(
            diagnostics[0]
                .help
                .as_deref()
                .is_some_and(|help| help.contains("没有 `padding` 属性"))
        );
    }

    #[test]
    fn longhand_padding_is_reported() {
        let diagnostics = check(
            "native-pad-longhand",
            SCROLL,
            ".list { padding-left: 16px; height: 200px; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    #[test]
    fn zero_or_dynamic_padding_is_not_reported() {
        for (name, wxss) in [
            ("native-pad-zero", ".list { padding: 0; height: 200px; }"),
            (
                "native-pad-var",
                ".list { padding: var(--inset); height: 200px; }",
            ),
        ] {
            let diagnostics = check(name, SCROLL, wxss);
            assert!(diagnostics.is_empty(), "{name}: {diagnostics:#?}");
        }
    }

    /// 内边距写在内部普通节点上才是等价写法；`swiper-item` 自己有 yoga 树，padding 生效。
    #[test]
    fn padding_on_inner_view_or_swiper_item_is_not_reported() {
        let diagnostics = check(
            "native-pad-inner",
            SWIPER,
            ".slides { height: 200px; }\n.page { padding: 0 16px; }\n.inner { padding: 0 16px; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// 真实工程的样式源常常是 `.less` / `.scss`，且带嵌套：解不开就等于这条规则从未生效。
    #[test]
    fn nested_less_source_is_reported() {
        let fixture = Fixture::new("native-pad-less");
        fixture
            .write(
                "project.config.json",
                r#"{"miniprogramRoot":"","setting":{"useCompilerPlugins":["less"]}}"#,
            )
            .write("app.json", SKYLINE_APP)
            .write("pages/index/index.json", "{}")
            .write(
                "pages/index/index.wxml",
                r#"<view class="panel"><scroll-view class="list"><view class="item"/></scroll-view></view>"#,
            )
            .write(
                "pages/index/index.less",
                ".panel {\n  .list {\n    padding: 0 16px;\n    height: 200px;\n  }\n}",
            );
        let diagnostics = NativePadding.check(&fixture.project());
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].path, "pages/index/index.wxml");
    }

    /// WebView 页面不归这条规则管。
    #[test]
    fn webview_page_is_not_reported() {
        let fixture = Fixture::new("native-pad-webview");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write("app.json", SKYLINE_APP)
            .write("pages/index/index.json", r#"{"renderer":"webview"}"#)
            .write("pages/index/index.wxml", SCROLL)
            .write(
                "pages/index/index.wxss",
                ".list { padding: 0 16px; height: 200px; }",
            );
        assert!(NativePadding.check(&fixture.project()).is_empty());
    }
}
