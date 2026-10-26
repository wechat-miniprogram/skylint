//! filter / backdrop-filter 的函数限制。

use crate::project::Project;
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

use super::value_util::{at_value, is_dynamic, top_level_calls};
use super::visit_declarations;

/// `visual.md:109-110` 的 `<filter-function>` BNF。
const FUNCTIONS: [&str; 9] = [
    "blur",
    "brightness",
    "contrast",
    "grayscale",
    "hue-rotate",
    "invert",
    "opacity",
    "saturate",
    "sepia",
];

pub struct FilterLimit;

impl FilterLimit {
    pub const ID: &'static str = "skyline/filter-limit";
}

impl Rule for FilterLimit {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "filter / backdrop-filter 使用了不支持的函数或组合",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit_declarations(project, |doc, _, decl, out| {
            if !decl.name.eq_ignore_ascii_case("filter")
                && !decl.name.eq_ignore_ascii_case("backdrop-filter")
            {
                return;
            }
            let value = decl.value.trim();
            if value.is_empty() || is_dynamic(&decl.tokens) {
                return;
            }
            let calls = top_level_calls(&decl.tokens);
            let mut reasons = Vec::new();
            if calls.iter().any(|c| c.name == "url") {
                reasons.push("不支持 url()".to_string());
            }
            if calls.iter().any(|c| c.name == "drop-shadow") {
                reasons.push("不支持 drop-shadow()，可用 box-shadow 替代".to_string());
            }
            // 白名单之外的函数名（含 url / drop-shadow 以外的未知函数）。
            let unknown: Vec<&str> = calls
                .iter()
                .map(|c| c.name.as_str())
                .filter(|name| *name != "url" && *name != "drop-shadow")
                .filter(|name| !FUNCTIONS.contains(name))
                .collect();
            if !unknown.is_empty() {
                let list = unknown
                    .iter()
                    .map(|name| format!("`{name}()`"))
                    .collect::<Vec<_>>()
                    .join("、");
                reasons.push(format!("不支持 {list}"));
            }
            if calls.len() > 1 {
                reasons.push("不支持多个函数组合".to_string());
            }
            if reasons.is_empty() {
                return;
            }
            out.push(at_value(
                        Self::ID,
                        doc,
                        decl,
                        format!("`{}` 在 Skyline 下{}", decl.name, reasons.join("；")),
                        "filter / backdrop-filter 只支持单个 blur/brightness/contrast/grayscale/hue-rotate/invert/opacity/saturate/sepia 函数",
                    ));
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use crate::rules::Rule;

    use super::FilterLimit;
    use crate::rules::testutil::wxss::{Fixture, messages, single};

    #[test]
    fn filter_limit_reports_url_drop_shadow_and_combinations() {
        let (_f, project) = single(
            "filter-positive",
            r#"
            .a { filter: url(#f); }
            .b { filter: drop-shadow(0 0 2px #000); }
            .c { backdrop-filter: blur(2px) brightness(1.2); }
            "#,
        );
        let diags = FilterLimit.check(&project);
        assert_eq!(diags.len(), 3);
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Warning));
        assert!(diags.iter().any(|d| d.message.contains("url()")));
        assert!(diags.iter().any(|d| d.message.contains("drop-shadow()")));
        assert!(diags.iter().any(|d| d.message.contains("多个函数")));
    }

    #[test]
    fn filter_limit_accepts_single_functions_and_vars() {
        let (_f, project) = single(
            "filter-negative",
            r#"
            .a { filter: blur(2px); }
            .b { backdrop-filter: grayscale(1); }
            .c { filter: none; }
            .d { filter: var(--f); }
            .e { filter: sepia(0.5); }
            .f { filter: hue-rotate(90deg); }
            "#,
        );
        assert_eq!(messages(&FilterLimit, &project), Vec::<String>::new());
    }

    /// 白名单之外的函数名要报（BNF 只列了 9 个）。
    #[test]
    fn filter_limit_reports_unknown_functions() {
        let (_f, project) = single(
            "filter-unknown",
            r#"
            .a { filter: custom(2px); }
            .b { backdrop-filter: blur(2px) opacity(0.5) nobody(1); }
            "#,
        );
        let diags = FilterLimit.check(&project);
        assert_eq!(diags.len(), 2, "{diags:#?}");
        assert!(diags[0].message.contains("`custom()`"));
        assert!(diags[1].message.contains("`nobody()`"));
    }

    #[test]
    fn broken_sources_do_not_stop_the_scan() {
        let f = Fixture::new("shape-isolation");
        f.write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/a/a","pages/b/b"],"renderer":"skyline"}"#,
            )
            .write("pages/a/a.json", "{}")
            .write("pages/a/a.wxml", "<view/>")
            .write("pages/a/a.wxss", ".a { filter: url(#f); }")
            // 非法 UTF-8：读失败，跳过。
            .write("pages/b/b.json", "{}")
            .write("pages/b/b.wxml", "<view/>")
            .write_bytes("pages/b/b.wxss", &[0xff, 0xfe, 0x00]);

        let project = f.project();
        let diags = FilterLimit.check(&project);
        assert_eq!(diags.len(), 1);
        let paths: HashSet<_> = diags.iter().map(|d| d.path.as_str()).collect();
        assert_eq!(paths, HashSet::from(["pages/a/a.wxss"]));
    }
}
