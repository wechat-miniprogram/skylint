//! `skyline/native-padding`：内置组件上的 CSS `padding` / `border-width` / `gap`
//! 不会内缩它的子节点。
//!
//! 判据只有一条：这个组件在 Skyline 里**有没有自己的 CSS 盒**。子节点一旦成为独立布局
//! 根，父节点的 padding（连同 border / gap）就与它无关：
//!
//! - 父 widget 链尾不是 yoga 连接器就 `SetAsRoot()`；
//! - `CreateConcreteWidget()` 返回空才建 yoga 连接器；
//! - 全仓只有一处把子 yoga 节点挂进父树；
//! - concrete widget 拿到的约束是 border box，padding 没有扣减机会。
//!
//! 两档后果不同，所以文案分开：
//!
//! - [`CssBox::None`]：`InflateNaked()` / `SetSkipStyleBuild()`，连布局节点都不建，
//!   CSS 布局属性一律无效。`list-view` / `grid-view` /
//!   `list-builder` / `grid-builder` 还会在 style builder 的白名单里被挡掉，只剩背景色。
//! - [`CssBox::BorderBoxTail`]：有布局节点、padding 也写进去了，
//!   但 concrete widget 是 widget 链尾，子节点仍是独立根，所以内容不内缩。
//!
//! 只收有证据的属性：`border-width` 与 `gap` 由同一条「子节点独立根」机制覆盖；
//! `border` / `border-*` 简写与 `border-style` 的组合尚无实测用例，先不报。
//!
//! WebView 里这些组件的内部是普通 DOM（`wx-swiper > .wx-swiper-wrapper{width:100%}` 之类），
//! padding 照常内缩，属于渲染器差异。
//!
//! 排除 `swiper-item`：它 `CreateConcreteWidget()` 返回空 → 走 yoga 连接器，子节点进它
//! 自己的布局树，padding 正常生效。

use crate::project::Project;
use crate::rules::wxml::tags::{Capability, CssBox, PaddingAttr, capability};
use crate::rules::wxml::{Reach, element_diagnostic, for_each_element};
use crate::rules::wxss::natives;
use crate::rules::{Rule, RuleMeta, VisualCase, VisualExpect};
use crate::{Diagnostic, Severity};

pub struct NativePadding;

const PADDING_ATTR: &str = "把内边距写成组件属性 `padding=\"{{[top,right,bottom,left]}}\"`（与 CSS padding 是两条通路；组件 padding 是 sliver padding，滚动方向上也占位），或在里面包一层普通 `view` 写 padding";
const NO_PADDING_ATTR: &str =
    "这个组件没有 `padding` 属性：在里面包一层普通 `view`，把内边距写在那层 `view` 上";
const INNER_VIEW: &str = "组件自带的 `padding` 属性管不到边框与间距：在里面包一层普通 `view`，把内边距 / 边框 / 间距写在那层 `view` 上";

impl NativePadding {
    pub const ID: &'static str = "skyline/native-padding";

    /// 声明了非零 padding / border-width / gap 的属性名（含简写）。
    const PROPERTIES: [&'static str; 13] = [
        "padding",
        "padding-top",
        "padding-right",
        "padding-bottom",
        "padding-left",
        "border-width",
        "border-top-width",
        "border-right-width",
        "border-bottom-width",
        "border-left-width",
        "gap",
        "row-gap",
        "column-gap",
    ];

    fn message(capability: &Capability, properties: &[&str]) -> String {
        let list = properties
            .iter()
            .map(|property| format!("`{property}`"))
            .collect::<Vec<_>>()
            .join("、");
        match capability.css_box {
            CssBox::None => format!(
                "`{}` 在 Skyline 下由原生 widget 排布子节点、自身不参与 CSS 布局，{list} 不会内缩子节点（WebView 下生效）",
                capability.tag
            ),
            CssBox::BorderBoxTail => format!(
                "`{}` 的 {list} 不会内缩子节点：子节点是独立布局根，容器按 border box 铺满（WebView 下生效）",
                capability.tag
            ),
            CssBox::Full => unreachable!("CssBox::Full 不进这条规则"),
        }
    }
}

/// 视觉用例：`swiper` 的 CSS padding 在 WebView 内缩子节点，Skyline 的子节点仍是独立布局根。
static VISUAL_CASES: &[VisualCase] = &[VisualCase {
    wxml: r#"<swiper class="slides">
  <swiper-item class="page">
    <view class="inner" id="target">页 1</view>
  </swiper-item>
</swiper>"#,
    wxss: ".slides { width: 300px; height: 160px; padding: 20px; background: #dcdcdc; }\n.page { height: 120px; }\n.inner { width: 120px; height: 60px; background: #f56c6c; }\n",
    js: None,
    expect: VisualExpect::Differ,
    note: "`swiper` 的 `padding: 20px` 在 WebView 里把内容内缩 20px，Skyline 的子节点是独立布局根，红块贴左上角",
    target: "#target",
}];

impl Rule for NativePadding {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "内置组件上的 CSS padding / border-width / gap 在 Skyline 下不内缩子节点",
        }
    }

    fn visual_cases(&self) -> &'static [VisualCase] {
        VISUAL_CASES
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut out = Vec::new();

        for_each_element(project, self.target(), |site| {
            if site.reach != Reach::Skyline {
                return;
            }
            let (doc, tree, element) = (site.doc, site.tree, site.element);
            let reach = site.reach;
            let Some(capability) = capability(&element.tag) else {
                return;
            };
            if capability.css_box == CssBox::Full {
                return;
            }
            // 判不了的选择器可能改写结果：只要出现就不报（宁可漏报）。
            let Some(declared) =
                natives::declared(project, tree, element, &Self::PROPERTIES, |_, value| {
                    nonzero_inset(value)
                })
            else {
                return;
            };
            if declared.is_empty() {
                return;
            }
            // 只有命中 `padding*` 时才能把修法挂到组件的 `padding` 属性上；
            // `border-width` / `gap` 一律要包一层内层 `view`。
            let padding_hit = declared
                .iter()
                .any(|property| property.starts_with("padding"));
            let help = match (capability.padding_attr, padding_hit) {
                (PaddingAttr::Attribute, true) => PADDING_ATTR,
                (PaddingAttr::Absent, true) => NO_PADDING_ATTR,
                _ => INNER_VIEW,
            };
            out.push(
                element_diagnostic(
                    Self::ID,
                    doc,
                    element,
                    doc.path(),
                    Severity::Warning,
                    Self::message(capability, &declared),
                )
                .help(reach.help(help)),
            );
        });
        out
    }
}

/// 值里有任意一边不为 0 就是「真的想留内缩」。
///
/// `border-width` 的 `medium`（初值）不算，`thin` / `thick` 算；
/// 判不了的（`{{…}}`、`var(…)` 一类运行时值）按 0 处理，宁可漏报。
fn nonzero_inset(value: &str) -> bool {
    value.split_whitespace().any(|token| {
        let token = token.to_ascii_lowercase();
        if token.contains("var(") {
            return false;
        }
        if matches!(token.as_str(), "thin" | "thick") {
            return true;
        }
        if matches!(token.as_str(), "medium" | "normal") {
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
    use crate::rules::testutil::wxss::{self, Fixture};

    use super::NativePadding;

    const SKYLINE_APP: &str = r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#;

    fn check(name: &str, wxml: &str, wxss: &str) -> Vec<Diagnostic> {
        wxss::check(&NativePadding, name, wxml, wxss)
    }

    const SWIPER: &str = r#"<swiper class="slides"><swiper-item class="page"><view class="inner"/></swiper-item></swiper>"#;
    const SCROLL: &str = r#"<scroll-view class="list"><view class="item"/></scroll-view>"#;

    #[test]
    fn visual_case_swiper_padding_is_reported() {
        let diagnostics = check(
            "native-pad-visual",
            r#"<swiper class="slides"><swiper-item class="page"><view class="inner" id="target">页 1</view></swiper-item></swiper>"#,
            ".slides { width: 300px; height: 160px; padding: 20px; background: #dcdcdc; }\n.page { height: 120px; }\n.inner { width: 120px; height: 60px; background: #f56c6c; }",
        );
        assert!(!diagnostics.is_empty(), "{diagnostics:#?}");
    }

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
    fn border_width_is_reported() {
        let diagnostics = check(
            "native-pad-border",
            SCROLL,
            ".list { border-width: 2px; height: 200px; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("`border-width`"));
        assert!(
            diagnostics[0]
                .help
                .as_deref()
                .is_some_and(|help| help.contains("包一层普通 `view`"))
        );
    }

    #[test]
    fn gap_is_reported() {
        let diagnostics = check(
            "native-pad-gap",
            SWIPER,
            ".slides { gap: 16px; height: 200px; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("`gap`"));
    }

    #[test]
    fn default_or_zero_border_and_gap_are_ignored() {
        for (name, wxss) in [
            (
                "native-pad-border-default",
                ".list { border-width: medium; height: 200px; }",
            ),
            (
                "native-pad-border-zero",
                ".list { border-width: 0; height: 200px; }",
            ),
            (
                "native-pad-gap-normal",
                ".list { gap: normal; height: 200px; }",
            ),
            ("native-pad-gap-zero", ".list { gap: 0; height: 200px; }"),
        ] {
            let diagnostics = check(name, SCROLL, wxss);
            assert!(diagnostics.is_empty(), "{name}: {diagnostics:#?}");
        }
    }

    /// `Full` 档有正常的 CSS 盒：子节点进自己的布局树，padding / border / gap 都生效。
    #[test]
    fn full_css_box_component_is_not_reported() {
        let diagnostics = check(
            "native-pad-full",
            SWIPER,
            ".slides { height: 200px; }\n.page { padding: 8px; border-width: 2px; gap: 8px; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// 能力表里没有证据的标签保持不报：宁可漏报，不按「应该有」扩表。
    #[test]
    fn unconfirmed_components_are_not_reported() {
        let diagnostics = check(
            "native-pad-unconfirmed",
            r#"<picker-view class="p"><view/></picker-view><rich-text class="r"/><cover-view class="c"/><page-container class="pc"><view/></page-container>"#,
            ".p { padding: 8px; border-width: 2px; gap: 4px; }\n.r { padding: 8px; }\n.c { padding: 8px; }\n.pc { padding: 8px; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

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

    /// 片段里 `padding` 判不了（祖先未知 / 选择器超出受限文法）：不能当成没写。
    #[test]
    fn undecidable_fragment_padding_is_not_reported() {
        let fixture = Fixture::new("native-pad-undecidable");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write("app.json", SKYLINE_APP)
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", r#"<include src="./part"/>"#)
            .write(
                "pages/index/part.wxml",
                r#"<scroll-view class="list"><view/></scroll-view>"#,
            )
            .write(
                "pages/index/index.wxss",
                ".list { padding: 8px; height: 100px; }\n.ghost + .list { padding: 0; }",
            );
        let diagnostics = NativePadding.check(&fixture.project());
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// `<import>` 进来的模板定义体也在检查范围：定义体在 `<template is>` 调用处渲染。
    #[test]
    fn imported_template_body_is_reported() {
        let fixture = Fixture::new("native-pad-imported-template");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write("app.json", SKYLINE_APP)
            .write("pages/index/index.json", "{}")
            .write(
                "pages/index/index.wxml",
                r#"<import src="../../templates/t.wxml"/><template is="card"/>"#,
            )
            .write(
                "templates/t.wxml",
                r#"<template name="card"><scroll-view class="list"><view/></scroll-view></template>"#,
            )
            .write(
                "pages/index/index.wxss",
                ".list { padding: 8px; height: 100px; }",
            );
        let diagnostics = NativePadding.check(&fixture.project());
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].path, "templates/t.wxml");
    }
}
