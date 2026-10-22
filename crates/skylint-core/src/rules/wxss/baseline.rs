//! 需要手写判定的 WXSS 规则。
//!
//! 三条 P0 规则都只依赖「单条规则内的原始声明」，不做级联、不解析值、不匹配选择器。

use crate::project::Project;
use crate::{Diagnostic, Severity};

use super::visit;
use crate::rules::{Rule, RuleMeta};

/// Skyline 不支持的伪类 / 伪元素。
///
/// 知识库明确列出 Skyline 支持 `::before` / `::after`（changelog 1.0.0）、
/// `:not()` / `:only-child()` / `:empty()`（1.3.0）、`:host`（1.4.13），因此这里
/// 用「白名单」而不是「一刀切」。其余伪类/伪元素按不支持处理。
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
        matches!(name, "not" | "only-child" | "empty" | "host")
    }
}

/// `box-sizing` 在 Skyline 下只接受 `content-box` / `border-box`。
///
/// 与 WebView 的差异是**默认值**：Skyline 默认 `border-box`，WebView 默认
/// `content-box`。声明不受支持的值时样式直接失效，所以报 error。
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
        visit(project, |doc, out| {
            for rule in &doc.rules {
                for decl in &rule.declarations {
                    if !decl.name.eq_ignore_ascii_case("box-sizing") {
                        continue;
                    }
                    let value = decl.value.trim();
                    if value.is_empty() {
                        continue;
                    }
                    let lower = value.to_ascii_lowercase();
                    // 变量/插值无法静态判定，交给运行时。
                    if lower.contains("var(") || lower.contains("env(") {
                        continue;
                    }
                    if is_box_sizing_value(&lower) {
                        continue;
                    }
                    let (line, column) = doc.position(decl.value_offset);
                    let (end_line, end_column) = doc.position(decl.value_end());
                    out.push(
                        Diagnostic::new(
                            Self::ID,
                            Severity::Error,
                            format!("`box-sizing: {value}` 在 Skyline 下不受支持，该声明会被忽略"),
                            doc.path(),
                            line,
                            column,
                        )
                        .end(end_line, end_column)
                        .help(
                            "Skyline 只支持 content-box / border-box，且默认值为 border-box（WebView 为 content-box）；请显式声明并复核元素尺寸",
                        ),
                    );
                }
            }
        })
    }
}

fn is_box_sizing_value(value: &str) -> bool {
    matches!(
        value,
        "content-box" | "border-box" | "inherit" | "initial" | "unset" | "revert" | "revert-layer"
    )
}

/// `display: flex` 在 Skyline 下默认 `flex-direction: column`，WebView 为 `row`。
///
/// 同一条规则内没有 `flex-direction` 时，迁移后布局方向会翻转，报 warning。
pub struct FlexDirection;

impl FlexDirection {
    pub const ID: &'static str = "skyline/flex-direction";
}

impl Rule for FlexDirection {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "display: flex 未声明 flex-direction，迁移后默认方向会从 row 变为 column",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit(project, |doc, out| {
            for rule in &doc.rules {
                if rule.declaration("flex-direction").is_some() {
                    continue;
                }
                let Some(display) = rule.declarations.iter().find(|decl| {
                    decl.name.eq_ignore_ascii_case("display") && is_flex_value(&decl.value)
                }) else {
                    continue;
                };
                let (line, column) = doc.position(display.offset);
                let (end_line, end_column) = doc.position(display.value_end());
                out.push(
                    Diagnostic::new(
                        Self::ID,
                        Severity::Warning,
                        "display: flex 未显式声明 flex-direction；Skyline 默认 column，WebView 默认 row，迁移后布局方向会翻转",
                        doc.path(),
                        line,
                        column,
                    )
                    .end(end_line, end_column)
                    .help("为这条规则补上 `flex-direction: row`，或按 column 重新设计布局"),
                );
            }
        })
    }
}

fn is_flex_value(value: &str) -> bool {
    let lower = value.trim().to_ascii_lowercase();
    if lower.contains("var(") || lower.contains("env(") || lower.contains("{{") {
        return false;
    }
    matches!(lower.as_str(), "flex" | "inline-flex")
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::fs;
    use std::path::PathBuf;

    use super::*;

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("skylint-wxss-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            Self { root }
        }

        fn write(&self, rel: &str, content: &str) -> &Self {
            self.write_bytes(rel, content.as_bytes())
        }

        fn write_bytes(&self, rel: &str, content: &[u8]) -> &Self {
            let path = self.root.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
            self
        }

        fn project(&self) -> Project {
            Project::load(&self.root).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    /// 单页 Skyline 工程，页面 wxss 即 `wxss`。
    fn single(name: &str, wxss: &str) -> (Fixture, Project) {
        let f = Fixture::new(name);
        f.write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", "<view/>")
            .write("pages/index/index.wxss", wxss);
        let project = f.project();
        (f, project)
    }

    fn messages(rule: &dyn Rule, project: &Project) -> Vec<String> {
        rule.check(project).into_iter().map(|d| d.message).collect()
    }

    #[test]
    fn no_pseudo_reports_unsupported_pseudos() {
        let (_f, project) = single(
            "no-pseudo-positive",
            r#"
            .a:hover { color: red; }
            .b::placeholder { color: red; }
            .c:nth-child(2) { color: red; }
            .d, .e:focus { color: red; }
            "#,
        );
        let diags = NoPseudo.check(&project);
        assert_eq!(diags.len(), 4);
        assert!(diags.iter().all(|d| d.severity == Severity::Warning));
        assert!(diags.iter().any(|d| d.message.contains(":hover")));
        assert!(diags.iter().any(|d| d.message.contains("::placeholder")));
        assert!(diags.iter().any(|d| d.message.contains(":nth-child")));
        // 多选择器规则只报带伪类的那个。
        assert!(diags.iter().any(|d| d.message.contains(".e")));
        assert!(!diags.iter().any(|d| d.message.contains(".d")));
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
        assert!(diags.iter().all(|d| d.severity == Severity::Error));
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
            .e { box-sizing: initial; }
            .f { padding-box: 1px; }
            "#,
        );
        assert_eq!(messages(&BoxSizing, &project), Vec::<String>::new());
    }

    #[test]
    fn flex_direction_reports_flex_without_direction() {
        let (_f, project) = single(
            "flex-positive",
            r#"
            .a { display: flex; }
            .b { display: inline-flex; color: red; }
            @media (min-width: 10px) { .c { display: flex; } }
            .d { display: FLEX; }
            "#,
        );
        let diags = FlexDirection.check(&project);
        assert_eq!(diags.len(), 4);
        assert!(diags.iter().all(|d| d.severity == Severity::Warning));
        // 嵌套在 @media 里的规则同样要报。
        assert!(diags.iter().any(|d| d.message.contains("flex-direction")));
    }

    #[test]
    fn flex_direction_ignores_declared_direction_block_and_vars() {
        let (_f, project) = single(
            "flex-negative",
            r#"
            .a { display: flex; flex-direction: column; }
            .b { display: block; }
            .c { flex-direction: row; }
            .d { display: var(--display); }
            .f { display: flex; flex-direction: var(--dir); }
            "#,
        );
        assert_eq!(messages(&FlexDirection, &project), Vec::<String>::new());
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
        // less 源：P0 只检查 .wxss，跳过。
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
        assert_eq!(diags.len(), 3);
        let paths: HashSet<_> = diags.iter().map(|d| d.path.as_str()).collect();
        assert_eq!(
            paths,
            HashSet::from([
                "pages/a/a.wxss",
                "pages/legacy/legacy.wxss",
                "components/nav/nav.wxss",
            ])
        );
    }
}
