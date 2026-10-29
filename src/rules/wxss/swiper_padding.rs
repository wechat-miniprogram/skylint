//! `skyline/swiper-padding`：`swiper` 上的 CSS `padding` 不会收缩轮播视口。
//!
//! WebView 的 `<swiper>` 编译成 `wx-swiper > .wx-swiper-wrapper(width/height:100%) > …`
//! （`wxapplib/src/wx-component/src/wx-swiper/wx-swiper.less:9-33`）：wrapper 在正常流里，
//! WebView 是 content-box，所以 `width: 100%` 取的是**内容盒**，swiper 上的 padding
//! 会把所有 page 一起内缩。
//!
//! Skyline 的 pager 直接吃 border box 约束（`skyline/widgets/yoga.cc:616-624` →
//! `package/skyline/lib/widgets/yoga.dart:575-584`），`Swiper` widget 创建时也没有
//! padding 参数（`shadow/components/swiper_shadow_node.cc:148-191`），padding 完全不生效。

use crate::matcher::Tree;
use crate::project::Project;
use crate::rules::Rule;
use crate::rules::RuleMeta;
use crate::rules::wxml::{
    ElementDoc, Reach, element_diagnostic, element_docs, resolved_property, walk_skyline,
};
use crate::{Diagnostic, Severity};

pub struct SwiperPadding;

impl SwiperPadding {
    pub const ID: &'static str = "skyline/swiper-padding";

    const MESSAGE: &'static str = "`swiper` 的 CSS `padding` 不会内缩轮播内容：WebView 靠内层 `width: 100%` 的 wrapper 落在内容盒里生效，Skyline 的 pager 直接铺满 border box";

    const HELP: &'static str = "要内缩 page，把 padding 从 `swiper` 挪到一层普通 `view` 上（外层包一层写 padding，或在内层 page 里写），`swiper` 自己只保留尺寸";

    /// 声明了非零 padding 的属性名（含 `padding` 简写）。
    const PROPERTIES: [&'static str; 5] = [
        "padding",
        "padding-top",
        "padding-right",
        "padding-bottom",
        "padding-left",
    ];
}

impl Rule for SwiperPadding {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "swiper 上的 CSS padding 在 Skyline 下不生效，内容不会内缩",
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
                if element.tag != "swiper" {
                    return;
                }
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
                        Self::MESSAGE,
                    )
                    .help(reach.help(Self::HELP)),
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
        let number = token.trim_end_matches(|c: char| c.is_ascii_alphabetic() || c == '%');
        number.parse::<f64>().is_ok_and(|number| number != 0.0) && !token.contains("var(")
    })
}

#[cfg(test)]
mod tests {
    use crate::Diagnostic;
    use crate::rules::Rule;
    use crate::rules::testutil::wxss::Fixture;

    use super::SwiperPadding;

    const SKYLINE_APP: &str = r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#;

    fn check(name: &str, wxss: &str) -> Vec<Diagnostic> {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write("app.json", SKYLINE_APP)
            .write("pages/index/index.json", "{}")
            .write(
                "pages/index/index.wxml",
                r#"<swiper class="slides"><swiper-item class="page"><view class="inner"/></swiper-item></swiper>"#,
            )
            .write("pages/index/index.wxss", wxss);
        let project = fixture.project();
        SwiperPadding.check(&project)
    }

    #[test]
    fn swiper_padding_is_reported() {
        let diagnostics = check(
            "swiper-padding-report",
            ".slides { margin: 0 -16px; padding: 0 16px; height: 200px; }\n.page { padding: 0 16px; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("border box"));
        assert_eq!(diagnostics[0].line, 1);
    }

    #[test]
    fn longhand_padding_is_reported() {
        let diagnostics = check(
            "swiper-padding-longhand",
            ".slides { padding-left: 16px; height: 200px; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    #[test]
    fn zero_or_dynamic_padding_is_not_reported() {
        let padding_zero = check(
            "swiper-padding-zero",
            ".slides { padding: 0; height: 200px; }",
        );
        assert!(padding_zero.is_empty(), "{padding_zero:#?}");

        let padding_var = check(
            "swiper-padding-var",
            ".slides { padding: var(--inset); height: 200px; }",
        );
        assert!(padding_var.is_empty(), "{padding_var:#?}");
    }

    /// 真实工程的样式源常常是 `.less` / `.scss`，且带嵌套：解不开就等于这条规则从未生效。
    #[test]
    fn nested_less_source_is_reported() {
        let fixture = Fixture::new("swiper-padding-less");
        fixture
            .write(
                "project.config.json",
                r#"{"miniprogramRoot":"","setting":{"useCompilerPlugins":["less"]}}"#,
            )
            .write("app.json", SKYLINE_APP)
            .write("pages/index/index.json", "{}")
            .write(
                "pages/index/index.wxml",
                r#"<view class="panel"><swiper class="slides"><swiper-item class="page"/></swiper></view>"#,
            )
            .write(
                "pages/index/index.less",
                ".panel {\n  .slides {\n    margin: 0 -16px;\n    padding: 0 16px;\n    height: 200px;\n  }\n}",
            );
        let project = fixture.project();
        let diagnostics = SwiperPadding.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        // 诊断挂在元素上（与其他元素级样式规则一致），路径是 WXML。
        assert_eq!(diagnostics[0].path, "pages/index/index.wxml");
    }

    /// padding 写在 page 内的普通 `view` 上才是 WebView 里的等价写法。
    #[test]
    fn padding_on_inner_view_is_not_reported() {
        let diagnostics = check(
            "swiper-padding-inner",
            ".slides { height: 200px; }\n.page { padding: 0; }\n.inner { padding: 0 16px; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// WebView 页面不归这条规则管。
    #[test]
    fn webview_page_is_not_reported() {
        let fixture = Fixture::new("swiper-padding-webview");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", r#"{"renderer":"webview"}"#)
            .write(
                "pages/index/index.wxml",
                r#"<swiper class="slides"><swiper-item class="page"/></swiper>"#,
            )
            .write(
                "pages/index/index.wxss",
                ".slides { padding: 0 16px; height: 200px; }",
            );
        let project = fixture.project();
        assert!(SwiperPadding.check(&project).is_empty());
    }
}
