//! 渐变函数在 Skyline 下不受支持的部分：`repeating-*` 与 mask 的 `conic-gradient`。
//!
//! - `repeating-*-gradient()`：float-pigment 的渐变解析只处理 linear / radial / conic
//!   （`float-pigment-css/src/parser/property_value/gradient.rs:376-438`），整条声明不会生效。
//! - mask 的 `conic-gradient()`：`widgets/decorated_box.cc:596-612` 的 SetMaskImage 只分派
//!   linear / radial，conic 静默不设置。
//!
//! mask 的 linear / radial 渐变是支持的，不再报。

use crate::project::Project;
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

use super::value_util::{all_calls, at_value, is_dynamic};
use super::visit_declarations;

pub struct GradientUnsupported;

/// 只存在于 WebView 的旧式渐变函数名（float-pigment 的 `gradient_repr` 只认三个标准名）。
const PREFIXED_GRADIENTS: [&str; 4] = [
    "-webkit-linear-gradient",
    "-webkit-radial-gradient",
    "-webkit-repeating-linear-gradient",
    "-webkit-gradient",
];

const STANDARD_GRADIENTS: [&str; 3] = ["linear-gradient", "radial-gradient", "conic-gradient"];

impl GradientUnsupported {
    pub const ID: &'static str = "skyline/gradient-unsupported";
}

impl Rule for GradientUnsupported {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "repeating / 厂商前缀渐变不被 Skyline 解析，mask 也不支持 conic 渐变",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit_declarations(project, |sheet, rule, decl, out| {
            let doc = &sheet.doc;
            let name = decl.name.to_ascii_lowercase();
            let is_mask = name == "mask-image" || name == "mask";
            if !is_mask && name != "background-image" && name != "background" {
                return;
            }
            if decl.value.trim().is_empty() || is_dynamic(&decl.tokens) {
                return;
            }
            let mut problems: Vec<(String, &'static str)> = Vec::new();
            // 前缀写法常和标准写法成对出现（autoprefixer 产物）：同一条规则里已经有
            // 标准渐变兜底时，两端渲染一致，不必报。
            let has_standard_fallback = rule.declarations.iter().any(|other| {
                other.name.eq_ignore_ascii_case(&name)
                    && all_calls(&other.tokens)
                        .iter()
                        .any(|call| STANDARD_GRADIENTS.contains(&call.name.as_str()))
            });
            for call in all_calls(&decl.tokens) {
                let repeating =
                    call.name.starts_with("repeating-") && call.name.ends_with("-gradient");
                if repeating {
                    problems.push((
                        format!(
                            "`{name}` 使用了 `{}()`；Skyline 不解析 repeating 渐变，整条声明不会生效",
                            call.name
                        ),
                        "改用非 repeating 的 linear-gradient / radial-gradient，或用图片实现重复图案",
                    ));
                } else if PREFIXED_GRADIENTS.iter().any(|prefix| call.name == *prefix) {
                    if !has_standard_fallback {
                        problems.push((
                            format!(
                                "`{name}` 使用了 `{}()`；Skyline 只认标准渐变函数名，整条声明不会生效",
                                call.name
                            ),
                            "改用标准语法 linear-gradient / radial-gradient / conic-gradient",
                        ));
                    }
                } else if is_mask && call.name == "conic-gradient" {
                    problems.push((
                        format!(
                            "`{name}` 不支持 `conic-gradient()`；mask 仅支持 url() / linear-gradient / radial-gradient"
                        ),
                        "改用 linear-gradient / radial-gradient，或用 url() 遮罩图",
                    ));
                }
            }
            let Some((message, help)) = problems.into_iter().next() else {
                return;
            };
            out.push(at_value(Self::ID, doc, decl, message, help));
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;

    use super::GradientUnsupported;
    use crate::rules::testutil::wxss::{messages, page};

    #[test]
    fn prefixed_gradient_is_reported_unless_a_standard_fallback_exists() {
        let (_f, project) = page(
            "gradient-prefixed",
            r#"<view class="a"/><view class="b"/>"#,
            r#"
            .a { background-image: -webkit-linear-gradient(bottom, #fff, transparent); }
            .b { background-image: -webkit-linear-gradient(bottom, #fff, transparent);
                 background-image: linear-gradient(to top, #fff, transparent); }
            "#,
        );
        let diags = GradientUnsupported.check(&project);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert!(diags[0].message.contains("-webkit-linear-gradient"));
    }

    #[test]
    fn repeating_and_mask_conic_gradients_are_reported() {
        let (_f, project) = page(
            "gradient-unsupported-positive",
            r#"<view class="a"/><view class="b"/><view class="c"/><view class="d"/>"#,
            r#"
            .a { background-image: repeating-linear-gradient(red 0 10px, blue 10px 20px); }
            .b { background: repeating-radial-gradient(circle, red 0 10px, blue 10px 20px); }
            .c { mask-image: conic-gradient(red, blue); }
            .d { mask: conic-gradient(red, blue); }
            "#,
        );
        let diags = GradientUnsupported.check(&project);
        assert_eq!(diags.len(), 4, "{diags:#?}");
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Warning));
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("repeating-linear-gradient"))
        );
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("repeating-radial-gradient"))
        );
        assert!(diags.iter().any(|d| d.message.contains("conic-gradient")));
    }

    #[test]
    fn mask_linear_radial_and_normal_background_gradients_are_accepted() {
        let (_f, project) = page(
            "gradient-unsupported-negative",
            r#"<view class="a"/><view class="b"/><view class="c"/><view class="d"/><view class="e"/><view class="f"/><view class="g"/><view class="h"/>"#,
            r#"
            .a { mask-image: linear-gradient(red, blue); }
            .b { mask-image: radial-gradient(circle, red 0%, blue 100%); }
            .c { mask: linear-gradient(red, blue) center / cover no-repeat; }
            .d { mask-image: url(a.png); }
            .e { background-image: linear-gradient(red, blue); }
            .f { background-image: radial-gradient(circle, red 0%, blue 100%); }
            .g { background-image: var(--gradient); }
            .h { mask: conic-gradient(var(--from), blue); }
            "#,
        );
        assert_eq!(
            messages(&GradientUnsupported, &project),
            Vec::<String>::new()
        );
    }
}
