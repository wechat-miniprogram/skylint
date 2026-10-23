//! Skyline 不支持的伪类 / 伪元素。
//!
//! 白名单依据不是文档而是引擎：`css/css_parser_impl.h:271-329` 只把
//! `fp::PseudoClassesType` 的 7 个值映射成 `PseudoClassType`（`empty` / `host` /
//! `only-child` / `first-child` / `last-child` / `not()` / `nth-child()`），其余值走
//! `default` 分支报 `unsupported pseudo-class type`；`selector_matcher.cc:168-232`
//! 逐个实现匹配。伪元素只实现了 `::before` / `::after`（changelog 1.0.0）。
//! 所以 `:hover` / `:active` / `:focus` 这类确实是负例，而 `:first-child` 等必须放过。
//!
//! 两处特殊：`fp` 把单冒号的 `:before` / `:after` 解析成伪元素并给一条
//! `InvalidPseudoElement` 警告但不拒绝（`selector.rs:345-375`），样式仍然生效；
//! `:nth-of-type()` 能被 `fp` 解析，但 `PseudoClassesType::Tag` 里没有它，
//! 落到 `default` 分支报错——所以它是不支持的，而 `:nth-last-child()` 在 `fp` 层就拒了。

use crate::project::Project;
use crate::{Diagnostic, Severity};

use super::visit;
use crate::rules::{Rule, RuleMeta};

pub struct NoPseudo;

impl NoPseudo {
    pub const ID: &'static str = "skyline/no-pseudo";
}

impl Rule for NoPseudo {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "Skyline 不支持该伪类/伪元素，选择器不会命中",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit(project, |doc, out| {
            for rule in &doc.rules {
                for selector in &rule.selectors {
                    let Some(hit) = first_unsupported_pseudo(&selector.text) else {
                        continue;
                    };
                    let start = selector.offset + hit.offset;
                    let (line, column) = doc.position(start);
                    let (end_line, end_column) = doc.position(start + hit.text.len());
                    out.push(
                        Diagnostic::new(
                            Self::ID,
                            Severity::Warning,
                            format!(
                                "选择器 `{}` 中的伪{} `{}` 在 Skyline 下不受支持",
                                selector.text,
                                if hit.element { "元素" } else { "类" },
                                hit.text
                            ),
                            doc.path(),
                            line,
                            column,
                        )
                        .end(end_line, end_column)
                        .help(
                            "Skyline 只支持 ::before/::after 与 :not()/:only-child()/:empty()/:host 等有限伪类；请改用 class 或调整 WXML 结构",
                        ),
                    );
                }
            }
        })
    }
}

struct PseudoHit {
    /// 相对选择器文本的字节偏移。
    offset: usize,
    /// 含前导冒号的伪类/伪元素原文，如 `::before`。
    text: String,
    element: bool,
}

/// 找出选择器中第一个不受支持的伪类/伪元素。字符串与属性选择器内的冒号不算。
fn first_unsupported_pseudo(selector: &str) -> Option<PseudoHit> {
    let chars: Vec<(usize, char)> = selector.char_indices().collect();
    let mut i = 0;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut brackets = 0i32;

    while i < chars.len() {
        let (offset, ch) = chars[i];
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match ch {
            // 转义的下一个字符不是语法：`.hover\:bg-red` 里的冒号是 class 名的一部分。
            '\\' => i += 2,
            '"' | '\'' => {
                quote = Some(ch);
                i += 1;
            }
            '[' => {
                brackets += 1;
                i += 1;
            }
            ']' => {
                brackets -= 1;
                i += 1;
            }
            ':' if brackets <= 0 => {
                let element = chars.get(i + 1).is_some_and(|(_, c)| *c == ':');
                let name_start = i + if element { 2 } else { 1 };
                let mut j = name_start;
                while j < chars.len() {
                    let c = chars[j].1;
                    if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                        j += 1;
                    } else {
                        break;
                    }
                }
                if j == name_start {
                    i += 1;
                    continue;
                }
                let name: String = chars[name_start..j].iter().map(|(_, c)| *c).collect();
                let text = format!("{}{}", if element { "::" } else { ":" }, name);
                if !is_supported_pseudo(element, &name.to_ascii_lowercase()) {
                    return Some(PseudoHit {
                        offset,
                        text,
                        element,
                    });
                }
                i = j;
            }
            _ => i += 1,
        }
    }
    None
}

fn is_supported_pseudo(element: bool, name: &str) -> bool {
    if element {
        matches!(name, "before" | "after")
    } else {
        matches!(
            name,
            // 单冒号的 `:before` / `:after` 是伪元素的旧写法，引擎接受并生效。
            "before"
                | "after"
                | "empty"
                | "host"
                | "only-child"
                | "first-child"
                | "last-child"
                | "not"
                | "nth-child"
        )
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use crate::rules::Rule;

    use super::super::testutil::{Fixture, messages, single};
    use super::NoPseudo;

    #[test]
    fn no_pseudo_reports_unsupported_pseudos() {
        let (_f, project) = single(
            "no-pseudo-positive",
            r#"
            .a:hover { color: red; }
            .b::placeholder { color: red; }
            .c:nth-of-type(2) { color: red; }
            .d, .e:focus { color: red; }
            "#,
        );
        let diags = NoPseudo.check(&project);
        assert_eq!(diags.len(), 4);
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Warning));
        assert!(diags.iter().any(|d| d.message.contains(":hover")));
        assert!(diags.iter().any(|d| d.message.contains("::placeholder")));
        assert!(diags.iter().any(|d| d.message.contains(":nth-of-type")));
        // 多选择器规则只报带伪类的那个。
        assert!(diags.iter().any(|d| d.message.contains(".e")));
        assert!(!diags.iter().any(|d| d.message.contains(".d")));
    }

    /// 引擎实现了 7 个伪类：`css_parser_impl.h:271-329` 的映射 + `selector_matcher.cc:168-232`。
    /// 它们不能报——旧白名单漏了三个，会把正常写法判成不支持。
    /// 单冒号的 `:before` / `:after` 是旧写法：引擎解析成伪元素并生效，不能报。
    #[test]
    fn single_colon_pseudo_elements_are_accepted() {
        let (_f, project) = single(
            "no-pseudo-legacy-elements",
            r#"
            .a:before { content: "x"; }
            .b:after { content: "x"; }
            .c::before { content: "x"; }
            "#,
        );
        assert_eq!(messages(&NoPseudo, &project), Vec::<String>::new());
    }

    #[test]
    fn child_position_pseudos_are_supported() {
        let (_f, project) = single(
            "no-pseudo-child",
            r#"
            .a:first-child { color: red; }
            .b:last-child { color: red; }
            .c:nth-child(2n+1) { color: red; }
            .d:nth-child(3) { color: red; }
            "#,
        );
        assert_eq!(messages(&NoPseudo, &project), Vec::<String>::new());
    }

    #[test]
    fn no_pseudo_accepts_plain_selectors_strings_and_supported_pseudos() {
        let (_f, project) = single(
            "no-pseudo-negative",
            r#"
            .a { color: red; }
            [data-x] { color: red; }
            [data-label="::after"] { color: red; }
            @keyframes fade { 0% { opacity: 0; } 100% { opacity: 1; } }
            .b::before { content: "x"; }
            .c:not(.d) { color: red; }
            .e { content: "::after"; }
            .f:only-child, .g:empty, :host { color: red; }
            "#,
        );
        assert_eq!(messages(&NoPseudo, &project), Vec::<String>::new());
    }

    /// Tailwind 风格转义冒号：`.hover\:bg-red` 里的冒号是类名的一部分，不是伪类。
    #[test]
    fn escaped_colon_in_a_class_name_is_not_a_pseudo() {
        let (_f, project) = single(
            "no-pseudo-escaped",
            r#"
            .hover\:bg-red { color: red; }
            .md\:flex { display: flex; }
            .hover\:bg-red:hover { color: blue; }
            "#,
        );
        let messages = messages(&NoPseudo, &project);
        // 只有第三条的真实 `:hover` 命中。
        assert_eq!(messages.len(), 1, "{messages:?}");
        assert!(messages[0].contains(":hover"));
    }

    #[test]
    fn broken_or_foreign_sources_do_not_stop_the_scan() {
        let f = Fixture::new("isolation");
        f.write(
            "project.config.json",
            r#"{"miniprogramRoot":"","setting":{"useCompilerPlugins":["less"]}}"#,
        )
        .write(
            "app.json",
                r#"{"pages":["pages/a/a","pages/b/b","pages/c/c","pages/legacy/legacy"],"renderer":"skyline","usingComponents":{"nav":"/components/nav/nav"}}"#,
        )
        // 正常 wxss：应报。
        .write("pages/a/a.json", "{}")
        .write("pages/a/a.wxml", "<view/>")
        .write("pages/a/a.wxss", ".a:hover { color: red; }")
        // 非法 UTF-8：读失败，跳过。
        .write("pages/b/b.json", "{}")
        .write("pages/b/b.wxml", "<view/>")
        .write_bytes("pages/b/b.wxss", &[0xff, 0xfe, 0x00])
        // less 源：预处理器语法同样解析（嵌套摊平），要查。
        .write("pages/c/c.json", "{}")
        .write("pages/c/c.wxml", "<view/>")
        .write("pages/c/c.less", ".c:hover { color: red; }")
        // 未迁到 Skyline 的页面同样检查（迁移工具，目标用户就是 WebView 工程）。
        .write("pages/legacy/legacy.json", r#"{"renderer":"webview"}"#)
        .write("pages/legacy/legacy.wxml", "<view/>")
        .write("pages/legacy/legacy.wxss", ".l:hover { color: red; }")
        // 组件：全部检查。
        .write("components/nav/nav.json", r#"{"component":true}"#)
        .write("components/nav/nav.wxml", "<view/>")
        .write("components/nav/nav.wxss", ".n:focus { color: red; }");

        let project = f.project();
        let diags = NoPseudo.check(&project);
        assert_eq!(diags.len(), 4);
        let paths: HashSet<_> = diags.iter().map(|d| d.path.as_str()).collect();
        assert_eq!(
            paths,
            HashSet::from([
                "pages/a/a.wxss",
                "pages/c/c.less",
                "pages/legacy/legacy.wxss",
                "components/nav/nav.wxss",
            ])
        );
    }
}
