//! 主体是 `*`（或只剩属性选择器）的规则在 Skyline 下整条不生效。
//!
//! Skyline 的规则集按**主体 compound** 的 id / 标签 / 类分桶
//! （`skyline/style/rule_set.cc:114-159` 的 `ExtractRuleBySelector`）：三者皆空
//! 且不是 `:host` 时直接落空返回，规则根本不进规则集。这不是「等价于 universal」，
//! 而是所有声明都不参与级联——`* { box-sizing: border-box }` 这类全局重置在
//! Skyline 下完全失效，`WebView` 下却照常生效。
//!
//! 属性选择器同理：C++ 侧在转 AST 时就丢掉属性条件
//! （`css/css_parser_impl.h:191-220`），`[data-x]` 退化成 `*`，一起落空。
//! 但那一种写法由 `skyline/attribute-selector` 负责（它按两种方言逐元素比对，
//! 只在真有可观测差异时报）——本规则只管 `*`。
//! 组合选择器的**祖先**部分不受影响（`.a *` 的主体仍是 `*`，整条规则死）。

use crate::matcher::subject;
use crate::project::Project;
use crate::rules::{Rule, RuleMeta, Target};
use crate::{Diagnostic, Severity};

use super::visit_matching_rules;

pub struct UniversalSelector;

impl UniversalSelector {
    pub const ID: &'static str = "skyline/universal-selector";
}

impl Rule for UniversalSelector {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "主体是 `*`（或只剩属性选择器）的规则在 Skyline 下不进规则集，声明全部不生效",
        }
    }

    fn target(&self) -> Target {
        Target::Skyline
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit_matching_rules(project, |sheet, rule, out| {
            for selector in &rule.selectors {
                let Some(subject) = subject(selector.text.trim()) else {
                    continue;
                };
                // 主体只要带了 id / 标签 / 类 / `:host` 就有桶，规则会进规则集。
                if subject.has_tag || subject.has_class || subject.has_id || subject.has_host {
                    continue;
                }
                // 只有属性选择器的形态归 `skyline/attribute-selector`（它按两种方言
                // 逐元素比对，只在真有可观测差异时报），这里只管 `*`。
                if !subject.universal {
                    continue;
                }
                let message = "主体是 `*`：Skyline 的规则集只按 id / 标签 / 类分桶，没有桶的规则不会进入规则集，声明全部不生效";
                let (line, column) = sheet.doc.position(selector.offset);
                out.push(
                    Diagnostic::new(
                        Self::ID,
                        Severity::Warning,
                        message,
                        sheet.doc.path(),
                        line,
                        column,
                    )
                    .end(line, column + selector.text.chars().count() as u32)
                    .help(HELP),
                );
            }
        })
    }
}

const HELP: &str = "换成具体标签（如 `view`）或类名；确实需要全局重置时逐个标签写，不要用 `*`";

#[cfg(test)]
mod tests {
    use crate::rules::testutil::wxss::{messages, page};

    use super::UniversalSelector;

    fn report(name: &str, wxss: &str) -> Vec<String> {
        let (_fixture, project) = page(name, r#"<view class="a"><view/></view>"#, wxss);
        messages(&UniversalSelector, &project)
    }

    #[test]
    fn universal_subject_is_reported() {
        let out = report("universal-subject", "* { box-sizing: border-box; }");
        assert_eq!(out.len(), 1, "{out:?}");
        assert!(out[0].contains("`*`"));
    }

    #[test]
    fn universal_after_a_class_is_still_reported() {
        // 主体是 `*`，`.a` 只是祖先约束，规则同样进不了规则集。
        let out = report("universal-after-class", ".a * { margin: 0; }");
        assert_eq!(out.len(), 1, "{out:?}");
    }

    #[test]
    fn tag_class_and_host_subjects_are_fine() {
        for (name, source) in [
            ("ok-tag", "view { box-sizing: border-box; }"),
            ("ok-class", ".a { color: red; }"),
            ("ok-id", "#app { color: red; }"),
            ("ok-host", ":host { display: block; }"),
            ("ok-descendant", ".a view { color: red; }"),
        ] {
            let out = report(name, source);
            assert!(out.is_empty(), "{source} 不该报：{out:?}");
        }
    }
}
