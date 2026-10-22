//! `mask-image` 不支持渐变，只支持 `url()`。

use crate::project::Project;
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

use super::value_util::{all_calls, at_value, is_dynamic};
use super::visit;

pub struct MaskImageGradient;

impl MaskImageGradient {
    pub const ID: &'static str = "skyline/mask-image-gradient";
}

impl Rule for MaskImageGradient {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "mask-image 不支持 linear-gradient / radial-gradient，仅 url() 可用",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit(project, |doc, out| {
            for rule in &doc.rules {
                for decl in &rule.declarations {
                    if !decl.name.eq_ignore_ascii_case("mask-image") {
                        continue;
                    }
                    let value = decl.value.trim();
                    if value.is_empty() || is_dynamic(value) {
                        continue;
                    }
                    let calls = all_calls(value);
                    let Some(gradient) = calls
                        .iter()
                        .find(|c| c.name == "linear-gradient" || c.name == "radial-gradient")
                    else {
                        continue;
                    };
                    out.push(at_value(
                        Self::ID,
                        doc,
                        decl,
                        format!(
                            "`mask-image` 不支持 `{}()`，Skyline 仅支持 url() 形式",
                            gradient.name
                        ),
                        "mask-image 仅支持 url()，请改用 url() 或 PNG 遮罩图",
                    ));
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;

    use super::super::testutil::{messages, single};
    use super::MaskImageGradient;

    #[test]
    fn mask_image_gradient_reports_gradients() {
        let (_f, project) = single(
            "mask-positive",
            r#"
            .a { mask-image: linear-gradient(red, blue); }
            .b { mask-image: radial-gradient(circle, red 0%, blue 100%); }
            .c { mask-image: url(a.png), linear-gradient(red, blue); }
            "#,
        );
        let diags = MaskImageGradient.check(&project);
        assert_eq!(diags.len(), 3);
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Warning));
        assert!(diags.iter().any(|d| d.message.contains("linear-gradient")));
        assert!(diags.iter().any(|d| d.message.contains("radial-gradient")));
    }

    #[test]
    fn mask_image_gradient_accepts_url_none_and_vars() {
        let (_f, project) = single(
            "mask-negative",
            r#"
            .a { mask-image: url(a.png); }
            .b { mask-image: none; }
            .c { mask-image: var(--mask); }
            .d { mask-image: conic-gradient(red, blue); }
            "#,
        );
        assert_eq!(messages(&MaskImageGradient, &project), Vec::<String>::new());
    }
}
