//! 渐变函数在 Skyline 下不受支持的部分：`repeating-*` 与 mask 的 `conic-gradient`。
//!
//! - `repeating-*-gradient()`：float-pigment 的渐变解析只处理 linear / radial / conic
//!   （`float-pigment-css/src/parser/property_value/gradient.rs:376-438`），整条声明不会生效。
//! - mask 的 `conic-gradient()`：引擎的 SetMaskImage 只分派
//!   linear / radial，conic 静默不设置。
//!
//! mask 的 linear / radial 渐变是支持的，不再报。

use crate::input::wxss::ValueToken;
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

use super::value_util::{Call, all_calls, at_value, is_dynamic};
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

/// 标准渐变的第一个参数不能是裸方向词：`left top` / `top` 属旧 WebKit 语法，
/// 标准写法要带 `to`（`to left top`），否则整条声明在两个引擎里都被丢弃。
/// 角度（`45deg`）、形状（`circle`）、直接给颜色停止的都是合法开头。
fn parses_as_standard_gradient(call: &Call<'_>) -> bool {
    match call.args.first() {
        Some(ValueToken::Ident(word)) => !matches!(
            word.to_ascii_lowercase().as_str(),
            "left" | "right" | "top" | "bottom"
        ),
        _ => true,
    }
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
            // 标准渐变兜底时，两端渲染一致，不必报。但「看起来像标准」不等于能解析：
            // `linear-gradient(left top, …)` 缺 `to`，是旧 WebKit 语法，标准语法里非法，
            // 两个引擎都会丢掉这条声明——这种不能当兜底，否则真缺陷会被消掉。
            let has_standard_fallback = rule.declarations.iter().any(|other| {
                other.name.eq_ignore_ascii_case(&name)
                    && all_calls(&other.tokens).iter().any(|call| {
                        STANDARD_GRADIENTS.contains(&call.name.as_str())
                            && parses_as_standard_gradient(call)
                    })
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

    /// `linear-gradient(left top, …)` 缺 `to`，是旧 WebKit 语法，标准语法里非法，
    /// 两个引擎都会丢掉它。它不算「标准兜底」，否则同一条规则里的前缀写法会被消掉，
    /// 而实际渲染是「根本没有背景」——`openplatform-verify-bulk` 就是这么丢黄底的。
    #[test]
    fn a_legacy_direction_standard_gradient_is_not_a_fallback() {
        let (_f, project) = page(
            "gradient-legacy-direction",
            r#"<view class="a"/><view class="b"/><view class="c"/><view class="d"/>"#,
            r#"
            .a { background-image: linear-gradient(left top, #ffd000, #ffbd00);
                 background-image: -webkit-linear-gradient(left top, #ffd000, #ffbd00); }
            .b { background-image: linear-gradient(to left top, #ffd000, #ffbd00);
                 background-image: -webkit-linear-gradient(left top, #ffd000, #ffbd00); }
            .c { background-image: linear-gradient(45deg, #ffd000, #ffbd00);
                 background-image: -webkit-linear-gradient(left top, #ffd000, #ffbd00); }
            .d { background-image: linear-gradient(#ffd000, #ffbd00);
                 background-image: -webkit-linear-gradient(left top, #ffd000, #ffbd00); }
            "#,
        );
        let diags = GradientUnsupported.check(&project);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert!(diags[0].message.contains("-webkit-linear-gradient"));
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
