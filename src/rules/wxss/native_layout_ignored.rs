//! `skyline/native-layout-ignored`：没有 CSS 盒的内置组件上，布局类声明全部不生效。
//!
//! 与 `native-padding` 同一条机制（共享 [`NATIVES`]）：这些组件在 Skyline 里由原生
//! widget 排布，`InflateNaked()` 连 yoga 节点都不建（`shadow/shadow_node.cc:269-277`），
//! 所以 `width` / `height` / `margin` / `display` / `flex-*` / `position` 都不会进布局。
//! list/grid 四件套还会在 style builder 的白名单里被挡掉，只剩背景色
//! （`style/style_builder.cc:155-166`、`:446-535`）；sticky / nested-scroll 系列连
//! computed style 都不建（`shadow/context.cc:867`）。
//!
//! `padding` 不在这里报：它的后果与改法都不同，由 `native-padding` 单独负责。
//!
//! WebView 里这些组件的内部是普通 DOM，布局属性照常生效，属于渲染器差异。

use crate::matcher::Tree;
use crate::project::Project;
use crate::rules::Rule;
use crate::rules::RuleMeta;
use crate::rules::wxml::{ElementDoc, Reach, element_diagnostic, element_docs, walk_skyline};
use crate::rules::wxss::natives::{NATIVES, Native, NativeKind};
use crate::{Diagnostic, Severity};

pub struct NativeLayoutIgnored;

impl NativeLayoutIgnored {
    pub const ID: &'static str = "skyline/native-layout-ignored";

    /// 会进布局的声明。`padding` 系列已经由 `native-padding` 负责，这里不重复报。
    const PROPERTIES: [&'static str; 24] = [
        "width",
        "height",
        "min-width",
        "min-height",
        "max-width",
        "max-height",
        "margin",
        "margin-top",
        "margin-right",
        "margin-bottom",
        "margin-left",
        "display",
        "position",
        "flex",
        "flex-direction",
        "flex-wrap",
        "flex-grow",
        "flex-shrink",
        "flex-basis",
        "justify-content",
        "align-items",
        "align-self",
        "gap",
        "overflow",
    ];

    fn message(native: &Native, properties: &[&str]) -> String {
        let list = properties
            .iter()
            .map(|property| format!("`{property}`"))
            .collect::<Vec<_>>()
            .join("、");
        format!(
            "`{}` 在 Skyline 下没有 CSS 盒（由原生 widget 排布）：{list} 不会进布局，WebView 下生效",
            native.tag
        )
    }

    fn help(native: &Native) -> &'static str {
        match native.kind {
            // 背景色是唯一还能画出来的样式。
            NativeKind::NoCssBox { background: true } => {
                "这层只负责滚动/布局容器：尺寸与间距交给组件自己的属性，或在里面包一层普通 `view` 承担布局；这个组件上只有 `background-color` 还会绘制"
            }
            NativeKind::NoCssBox { background: false } => {
                "这个组件不构建样式，所有 CSS 都不生效：布局与外观都放到内层普通 `view` 上"
            }
            NativeKind::BorderBoxTail => unreachable!("BorderBoxTail 不进这条规则"),
        }
    }
}

impl Rule for NativeLayoutIgnored {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "没有 CSS 盒的内置组件上，布局类声明在 Skyline 下不生效",
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
                let Some(native) = NATIVES
                    .iter()
                    .find(|native| native.tag == element.tag && native.kind.ignores_layout())
                else {
                    return;
                };
                let declared: Vec<&str> = Self::PROPERTIES
                    .iter()
                    .copied()
                    .filter(|property| {
                        crate::rules::wxml::resolved_property(project, &tree, element, property)
                            .is_some_and(|resolved| effective(property, &resolved.value))
                    })
                    .collect();
                if declared.is_empty() {
                    return;
                }
                out.push(
                    element_diagnostic(
                        Self::ID,
                        &doc,
                        element,
                        doc.path(),
                        Severity::Warning,
                        Self::message(native, &declared),
                    )
                    .help(reach.help(Self::help(native))),
                );
            });
        }
        out
    }
}

/// 这条声明是否真的想改变布局。
///
/// 判不了的（`{{…}}`、`var(…)`）与各属性的初始值都算「没写」，宁可漏报。
fn effective(property: &str, value: &str) -> bool {
    let value = value
        .trim()
        .trim_end_matches("!important")
        .trim()
        .to_ascii_lowercase();
    if value.is_empty() || value.contains("var(") || value.contains("{{") {
        return false;
    }
    let zero = |value: &str| {
        value.split_whitespace().all(|token| {
            token.trim_end_matches(|c: char| c.is_ascii_alphabetic() || c == '%') == "0"
        })
    };
    !match property {
        "width" | "height" | "flex-basis" => value == "auto",
        "min-width" | "min-height" => value == "auto" || zero(&value),
        "max-width" | "max-height" => value == "none",
        "margin" | "margin-top" | "margin-right" | "margin-bottom" | "margin-left" | "gap" => {
            zero(&value)
        }
        "position" => value == "static",
        "flex-direction" => value == "row",
        "flex-wrap" => value == "nowrap",
        "flex-grow" => value == "0",
        "flex-shrink" => value == "1",
        "align-items" => matches!(value.as_str(), "normal" | "stretch"),
        "align-self" => matches!(value.as_str(), "auto" | "normal" | "stretch"),
        "overflow" => value == "visible",
        // `display` / `flex` / `justify-content`：初始值取决于 defaultDisplayBlock 与
        // 父级类型，静态判不准，写了就报。
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use crate::Diagnostic;
    use crate::rules::Rule;
    use crate::rules::testutil::wxss::Fixture;

    use super::NativeLayoutIgnored;

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
        NativeLayoutIgnored.check(&project)
    }

    #[test]
    fn layout_properties_on_list_view_are_reported() {
        let diagnostics = check(
            "layout-list-view",
            r#"<list-view class="list"><view class="row"/></list-view>"#,
            r#".list { display: flex; margin: 8px; background-color: #fff; }"#,
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("`list-view`"));
        assert!(diagnostics[0].message.contains("`display`"));
        assert!(diagnostics[0].message.contains("`margin`"));
    }

    #[test]
    fn only_no_css_box_components_are_reported() {
        // scroll-view 有 yoga 节点，width/height 是有效的；swiper-item 的子节点更是普通布局。
        let diagnostics = check(
            "layout-scroll-view",
            r#"<scroll-view class="list"><view class="row"/></scroll-view><swiper><swiper-item class="page"/></swiper>"#,
            r#".list { width: 100%; height: 300px; } .page { display: flex; }"#,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn defaults_and_dynamic_values_are_ignored() {
        let diagnostics = check(
            "layout-defaults",
            r#"<sticky-section class="s"><view class="row"/></sticky-section>"#,
            r#".s { width: auto; margin: 0; display: none; max-width: var(--w); }"#,
        );
        // `display: none` 不是初始值，会报；`width: auto` / `margin: 0` / `var()` 不报。
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("`display`"));
        assert!(!diagnostics[0].message.contains("`width`"));
    }

    #[test]
    fn webview_page_is_not_reported() {
        let fixture = Fixture::new("layout-webview");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", r#"{"renderer":"webview"}"#)
            .write(
                "pages/index/index.wxml",
                r#"<list-view class="list"><view class="row"/></list-view>"#,
            )
            .write("pages/index/index.wxss", r#".list { display: flex; }"#);
        let project = fixture.project();
        assert!(NativeLayoutIgnored.check(&project).is_empty());
    }
}
