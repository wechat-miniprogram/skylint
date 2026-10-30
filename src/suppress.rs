//! 行内抑制注释。
//!
//! 用法（注释紧贴在被抑制行的上一行）：
//!
//! ```text
//! <!-- skylint-disable-next-line -->
//! <view>{{a}}<image/></view>
//! ```
//!
//! 也可以只抑制指定规则：`<!-- skylint-disable-next-line skyline/no-pseudo -->`。
//! 两种注释外壳都接受（`<!-- -->` 与 `/* */`），因为没有哪种 WXML/WXSS 文件
//! 同时容得下两种——统一识别比按扩展名分支更省事，也不会误伤。

use std::collections::HashMap;

use crate::Diagnostic;
use crate::project::Project;

const DIRECTIVE: &str = "skylint-disable-next-line";

/// 规则 id 形如 `skyline/<name>`；只认这个前缀，避免把注释里的普通单词当成规则。
fn is_rule_id(token: &str) -> bool {
    token.starts_with("skyline/") && token.len() > "skyline/".len()
}

/// 丢弃被抑制的诊断。读取失败的文件原样保留，宁可多报也不静默丢。
pub fn apply(project: &Project, diagnostics: Vec<Diagnostic>) -> Vec<Diagnostic> {
    let mut cache: HashMap<String, Vec<Disable>> = HashMap::new();
    let mut kept = Vec::with_capacity(diagnostics.len());

    for diag in diagnostics {
        let disables = cache
            .entry(diag.path.clone())
            .or_insert_with(|| read_disables(project, &diag.path));
        let suppressed = disables.iter().any(|d| {
            d.line + 1 == diag.line
                && (d.rules.is_empty() || d.rules.iter().any(|r| r == &diag.rule))
        });
        if !suppressed {
            kept.push(diag);
        }
    }
    kept
}

struct Disable {
    /// 注释所在行（1-based）。
    line: u32,
    /// 空表示抑制全部规则。
    rules: Vec<String>,
}

fn read_disables(project: &Project, path: &str) -> Vec<Disable> {
    let Ok(source) = std::fs::read_to_string(project.abs(path)) else {
        return Vec::new();
    };
    source
        .lines()
        .enumerate()
        .filter_map(|(idx, text)| {
            let (before, rest) = text.split_once(DIRECTIVE)?;
            // 必须出现在注释里，否则 JS 字符串里出现这串字就会误抑制。
            if !before.contains("<!--") && !before.contains("/*") && !before.contains("//") {
                return None;
            }
            // 指令之后是规则列表；外壳与正文可能粘连，按空白/逗号切。
            let tokens: Vec<&str> = rest
                .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
                .map(|token| token.trim_end_matches(['-', '>', '*', '/']))
                .filter(|token| !token.is_empty())
                .collect();
            // 写了规则名但一个都认不出来（例如漏了 `skyline/` 前缀）：不抑制任何规则。
            // 否则空列表会被当成「抑制全部」，反而变成误报的橡皮擦。
            if !tokens.is_empty() && !tokens.iter().any(|token| is_rule_id(token)) {
                return None;
            }
            let rules = tokens
                .iter()
                .filter(|token| is_rule_id(token))
                .map(|token| token.to_string())
                .collect();
            Some(Disable {
                line: idx as u32 + 1,
                rules,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::apply;
    use crate::{Diagnostic, Project, Severity};

    struct Fixture(PathBuf);

    impl Fixture {
        fn new(name: &str) -> Self {
            let root = crate::fixture_root("skylint-sup", name);
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(root.join("pages/a")).unwrap();
            fs::write(
                root.join("project.config.json"),
                r#"{"miniprogramRoot":""}"#,
            )
            .unwrap();
            fs::write(
                root.join("app.json"),
                r#"{"pages":["pages/a/a"],"renderer":"skyline"}"#,
            )
            .unwrap();
            fs::write(root.join("pages/a/a.json"), "{}").unwrap();
            Self(root)
        }

        fn wxml(&self, body: &str) -> Project {
            fs::write(self.0.join("pages/a/a.wxml"), body).unwrap();
            Project::load(&self.0).unwrap()
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn diag(rule: &str, line: u32) -> Diagnostic {
        Diagnostic::new(rule, Severity::Warning, "x", "pages/a/a.wxml", line, 1)
    }

    #[test]
    fn disables_the_next_line() {
        let f = Fixture::new("next");
        let project = f.wxml(
            "<!-- skylint-disable-next-line -->\n<view>a<image/></view>\n<view>b<image/></view>",
        );
        let kept = apply(
            &project,
            vec![
                diag("skyline/no-inline-text", 2),
                diag("skyline/no-inline-text", 3),
            ],
        );
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].line, 3);
    }

    #[test]
    fn rule_list_limits_scope() {
        let f = Fixture::new("scoped");
        let project =
            f.wxml("<!-- skylint-disable-next-line skyline/no-pseudo -->\n<view>a<image/></view>");
        let kept = apply(
            &project,
            vec![
                diag("skyline/no-pseudo", 2),
                diag("skyline/no-inline-text", 2),
            ],
        );
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].rule, "skyline/no-inline-text");
    }

    #[test]
    fn css_style_comment_works_too() {
        let f = Fixture::new("css");
        let project = f.wxml("/* skylint-disable-next-line */\n<view>a<image/></view>");
        assert!(apply(&project, vec![diag("skyline/no-inline-text", 2)]).is_empty());
    }

    #[test]
    fn only_the_adjacent_line_is_affected() {
        let f = Fixture::new("far");
        let project = f.wxml("<!-- skylint-disable-next-line -->\n\n<view>a<image/></view>");
        let kept = apply(&project, vec![diag("skyline/no-inline-text", 3)]);
        assert_eq!(kept.len(), 1, "中间隔了空行就不该生效");
    }

    /// 漏写 `skyline/` 前缀的规则名不能被当成「抑制全部」。
    #[test]
    fn bare_rule_name_does_not_silence_everything() {
        let f = Fixture::new("bare");
        let project =
            f.wxml("<!-- skylint-disable-next-line no-pseudo -->\n<view>a<image/></view>");
        let kept = apply(&project, vec![diag("skyline/no-pseudo", 2)]);
        assert_eq!(kept.len(), 1, "认不出的规则名应当不抑制任何规则");
    }

    /// 指令必须出现在注释里：字符串字面量里出现同样的文本不算。
    #[test]
    fn directive_inside_a_string_literal_is_ignored() {
        let f = Fixture::new("literal");
        let project = f.wxml("<!-- ok -->\n<view>a<image/></view>");
        fs::write(
            f.path().join("pages/a/a.js"),
            "const s = 'skylint-disable-next-line'\nPage({})",
        )
        .unwrap();
        let kept = apply(
            &project,
            vec![Diagnostic::new(
                "skyline/no-pseudo",
                Severity::Warning,
                "x",
                "pages/a/a.js",
                1,
                1,
            )],
        );
        assert_eq!(kept.len(), 1);
    }

    #[test]
    fn unreadable_file_keeps_diagnostics() {
        let f = Fixture::new("missing");
        let project = f.wxml("<view/>");
        let kept = apply(
            &project,
            vec![Diagnostic::new(
                "skyline/no-pseudo",
                Severity::Warning,
                "x",
                "pages/none.wxss",
                1,
                1,
            )],
        );
        assert_eq!(kept.len(), 1);
        assert!(Path::new(f.path()).exists());
    }
}
