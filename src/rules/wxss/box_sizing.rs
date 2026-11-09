//! `box-sizing` 在 Skyline 下只接受 `content-box` / `border-box`。
//!
//! `padding-box` / `margin-box` 在 WebView 同样非法（MDN formal syntax 只有
//! `content-box | border-box`），两端都丢弃该声明，差异只在**回落值**：未开
//! `defaultContentBox` 时 Skyline 回落 `border-box`（`skyline/css/css_properties/box_sizing.cc`
//! 的 ApplyInitial），WebView 回落 `content-box`；开了开关后两端都回落
//! `content-box`，这类非法值不再产生差异。

use crate::project::Project;
use crate::{Diagnostic, Severity};

use super::value_util::{at_value_with, is_dynamic};
use super::visit_declarations;
use crate::rules::{Rule, RuleMeta};

pub struct BoxSizing;

impl BoxSizing {
    pub const ID: &'static str = "skyline/box-sizing";
}

impl Rule for BoxSizing {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Error,
            description: "box-sizing 的值在两端都非法，Skyline 与 WebView 的回落值不同",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit_declarations(project, |sheet, _rule, decl, out| {
            let doc = &sheet.doc;
            if !decl.name.eq_ignore_ascii_case("box-sizing") {
                return;
            }
            let value = decl.value.trim();
            if value.is_empty() {
                return;
            }
            let lower = value.to_ascii_lowercase();
            // 变量 / 插值无法静态判定，交给运行时（$var、@var、#{} 同样算）。
            if is_dynamic(&decl.tokens) {
                return;
            }
            if is_box_sizing_value(&lower) {
                // `initial` / `unset` / `revert` 落到 ApplyInitial：只有开启
                // defaultContentBox 才是 content-box（`box_sizing.cc:33-41`）。
                if is_global_value(&lower) && !project.app_content_box() {
                    out.push(at_value_with(
                        Self::ID,
                        doc,
                        decl,
                        Severity::Warning,
                        format!("`box-sizing: {value}` 在 Skyline 下会解析成 border-box（WebView 是 content-box）"),
                        "显式写 content-box 或 border-box：未开启 defaultContentBox 时 Skyline 的初始值是 border-box",
                    ));
                }
                return;
            }
            // 非法值两端都会丢弃，差异只在回落值；开了 defaultContentBox 后两端都
            // 回落 content-box，没有可见差异，不报。
            if project.app_content_box() {
                return;
            }
            out.push(at_value_with(
                Self::ID,
                doc,
                decl,
                Severity::Error,
                format!("`box-sizing: {value}` 在 Skyline 与 WebView 下都不是合法值，该声明会被忽略；Skyline 回落 border-box，WebView 回落 content-box"),
                "Skyline 只接受 content-box / border-box；未开启 defaultContentBox 时它的回落值是 border-box（WebView 是 content-box），请显式声明并复核元素尺寸",
            ));
        })
    }
}

/// 全局关键字：它们的天空值是 `ApplyInitial` 的结果。
fn is_global_value(value: &str) -> bool {
    matches!(value, "initial" | "unset" | "revert" | "revert-layer")
}

fn is_box_sizing_value(value: &str) -> bool {
    matches!(
        value,
        "content-box" | "border-box" | "inherit" | "initial" | "unset" | "revert" | "revert-layer"
    )
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;

    use super::BoxSizing;
    use crate::rules::testutil::wxss::{Fixture, messages, page};

    #[test]
    fn box_sizing_reports_unsupported_values() {
        let (_f, project) = page(
            "box-sizing-positive",
            r#"<view class="a"/><view class="b"/><view class="c"/>"#,
            r#"
            .a { box-sizing: padding-box; }
            .b { box-sizing: padding-box !important; }
            .c { box-sizing: margin-box; }
            "#,
        );
        let diags = BoxSizing.check(&project);
        assert_eq!(diags.len(), 3);
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Error));
        assert!(diags[0].message.contains("padding-box"));
        assert!(diags[0].message.contains("content-box"));
        assert!(diags[0].help.as_deref().unwrap().contains("border-box"));
    }

    #[test]
    fn box_sizing_accepts_supported_values_and_keywords() {
        let (_f, project) = page(
            "box-sizing-negative",
            r#"<view class="a"/><view class="b"/><view class="c"/><view class="d"/><view class="f"/>"#,
            r#"
            .a { box-sizing: content-box; }
            .b { box-sizing: border-box; }
            .c { box-sizing: inherit; }
            .d { box-sizing: var(--box); }
            .f { padding-box: 1px; }
            "#,
        );
        assert_eq!(messages(&BoxSizing, &project), Vec::<String>::new());
    }

    /// 未开启 defaultContentBox 时 `initial` / `unset` 解析成 border-box，与 WebView 不同。
    #[test]
    fn initial_resolves_to_border_box_without_the_flag() {
        let (_f, project) = page(
            "box-sizing-initial",
            r#"<view class="a"/><view class="b"/>"#,
            r#"
            .a { box-sizing: initial; }
            .b { box-sizing: unset; }
            "#,
        );
        let diags = BoxSizing.check(&project);
        assert_eq!(diags.len(), 2, "{diags:#?}");
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Warning));
        assert!(diags[0].message.contains("border-box"));
    }

    /// 非法值两端都会丢弃，差异只在回落值：开了 defaultContentBox 后两端都回落
    /// content-box，没有可见差异，不报。
    #[test]
    fn invalid_values_are_silent_with_default_content_box() {
        let f = Fixture::new("box-sizing-content-box");
        f.write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"defaultContentBox":true}}}"#,
            )
            .write("pages/index/index.json", "{}")
            .write(
                "pages/index/index.wxml",
                r#"<view class="a"/><view class="b"/>"#,
            )
            .write(
                "pages/index/index.wxss",
                r#"
                .a { box-sizing: padding-box; }
                .b { box-sizing: margin-box; }
                "#,
            );
        assert_eq!(messages(&BoxSizing, &f.project()), Vec::<String>::new());
    }

    /// 预处理器变量不是「不受支持的值」，必须全静默（SCSS / Less 工程上 error 级误报）。
    #[test]
    fn preprocessor_variables_are_not_reported() {
        let (_f, project) = page(
            "box-sizing-dynamic",
            r#"<view class="a"/><view class="b"/><view class="c"/><view class="d"/>"#,
            r#"
            .a { box-sizing: $box; }
            .b { box-sizing: @box; }
            .c { box-sizing: #{$box}; }
            .d { box-sizing: calc(var(--x)); }
            "#,
        );
        assert_eq!(messages(&BoxSizing, &project), Vec::<String>::new());
    }
}
