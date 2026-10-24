//! `box-sizing` 在 Skyline 下只接受 `content-box` / `border-box`。
//!
//! 与 WebView 的差异是**默认值**：Skyline 默认 `border-box`，WebView 默认
//! `content-box`。声明不受支持的值时样式直接失效，所以报 error。

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
            description: "box-sizing 的值在 Skyline 下不受支持，声明会被忽略",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit_declarations(project, |doc, decl, out| {
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
            out.push(at_value_with(
                Self::ID,
                doc,
                decl,
                Severity::Error,
                format!("`box-sizing: {value}` 在 Skyline 下不受支持，该声明会被忽略"),
                "Skyline 只支持 content-box / border-box，且默认值为 border-box（WebView 为 content-box）；请显式声明并复核元素尺寸",
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
    use crate::rules::testutil::wxss::{messages, single};

    #[test]
    fn box_sizing_reports_unsupported_values() {
        let (_f, project) = single(
            "box-sizing-positive",
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
        assert!(diags[0].help.as_deref().unwrap().contains("border-box"));
    }

    #[test]
    fn box_sizing_accepts_supported_values_and_keywords() {
        let (_f, project) = single(
            "box-sizing-negative",
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
        let (_f, project) = single(
            "box-sizing-initial",
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

    /// 预处理器变量不是「不受支持的值」，必须全静默（SCSS / Less 工程上 error 级误报）。
    #[test]
    fn preprocessor_variables_are_not_reported() {
        let (_f, project) = single(
            "box-sizing-dynamic",
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
