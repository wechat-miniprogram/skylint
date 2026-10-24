//! 背景 / 遮罩的多值数量限制。

use crate::project::Project;
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

use super::value_util::{at_value, is_dynamic, split_top_level};
use super::visit_declarations;

pub struct MultiValueLimit;

impl MultiValueLimit {
    pub const ID: &'static str = "skyline/multi-value-limit";
}

impl Rule for MultiValueLimit {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "background / mask 的多值数量超出 Skyline 支持范围",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit_declarations(project, |doc, decl, out| {
            let Some(max) = value_limit(&decl.name) else {
                return;
            };
            let value = decl.value.trim();
            if value.is_empty() || is_dynamic(&decl.tokens) {
                return;
            }
            let count = split_top_level(&decl.tokens).len();
            if count <= max {
                return;
            }
            let message = format!(
                "`{}` 在 Skyline 下最多支持 {max} 个值，当前有 {count} 个",
                decl.name
            );
            out.push(at_value(
                Self::ID,
                doc,
                decl,
                message,
                "将多值拆成多条声明，或只保留 Skyline 支持的组数",
            ));
        })
    }
}

fn value_limit(name: &str) -> Option<usize> {
    match name.to_ascii_lowercase().as_str() {
        // 每一层背景/遮罩各一份；引擎逐层应用，超过两层直接丢声明
        // （image_property_utils.h:390-403、:518-530、:655-666）。
        "background-image"
        | "mask-image"
        | "background-position"
        | "mask-position"
        | "background-position-x"
        | "mask-position-x"
        | "background-position-y"
        | "mask-position-y"
        | "background-repeat"
        | "mask-repeat"
        | "background-size"
        | "mask-size" => Some(2),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;

    use super::MultiValueLimit;
    use crate::rules::testutil::wxss::{messages, single};

    #[test]
    fn multi_value_reports_excess_values() {
        let (_f, project) = single(
            "multi-positive",
            r#"
            .a { background-image: url(a.png), url(b.png), url(c.png); }
            .b { background-position: 0 0, 10px 10px, 20px 20px; }
            .c { background-repeat: no-repeat, repeat, repeat-x; }
            .d { background-size: 10px 10px, 20px 20px, 30px 30px; }
            .e { mask-position: 0 0, 10px 10px, 20px 20px; }
            .f { mask-repeat: no-repeat, repeat, repeat-x; }
            .g { mask-size: 10px 10px, 20px 20px, 30px 30px; }
            .h { background-position-x: 0, 10px, 20px; }
            .i { mask-position-y: 0, 10px, 20px; }
            "#,
        );
        let diags = MultiValueLimit.check(&project);
        assert_eq!(diags.len(), 9);
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Warning));
        assert!(diags.iter().any(|d| d.message.contains("3 个")));
    }

    #[test]
    fn multi_value_accepts_two_values_and_commas_inside_parens() {
        let (_f, project) = single(
            "multi-negative",
            r#"
            .a { background-image: url(a.png), url(b.png); }
            .b { background-image: url("a,b.png"), url(c.png); }
            .c { background-position: 0 0, 10px 10px; }
            .d { background-repeat: no-repeat; }
            .e { background-size: 10px 10px; }
            .h { background-repeat: no-repeat, repeat; }
            .i { background-size: 10px 10px, 20px 20px; }
            .j { mask-position: 0 0, 10px 10px; }
            .k { mask-repeat: no-repeat, repeat; }
            .l { mask-size: 10px 10px, 20px 20px; }
            .m { background-position-x: 0, 10px; }
            .n { mask-position-y: 0, 10px; }
            .f { background-image: var(--imgs); }
            .g { background-image: linear-gradient(red, blue), url(a.png); }
            "#,
        );
        assert_eq!(messages(&MultiValueLimit, &project), Vec::<String>::new());
    }
}
