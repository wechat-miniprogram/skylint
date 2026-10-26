//! `skyline/media-query`：媒体查询条件被引擎丢弃，块内声明无条件生效。
//!
//! 引擎解析规则时只把 `media_index` 拿去做颜色主题（`css/css_parser_impl.h:350-373`
//! 的 `ParseColorScheme` 只看 `MediaExpression::Tag::Theme`），生成
//! `CSSStyleRule` 时并不带任何媒体条件（同文件 `:443-460`）。于是
//! `@media (max-device-height: 558px) { … }` 里的声明在 Skyline 下对所有设备生效，
//! 而 WebView 只在条件成立时生效——这类块通常写的是小屏适配或弹窗高度覆盖，
//! 两端结果会明显不同。
//!
//! 唯一被识别的是 `prefers-color-scheme`，这类块跳过。

use std::collections::HashSet;

use crate::project::Project;
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

pub struct MediaQuery;

impl MediaQuery {
    pub const ID: &'static str = "skyline/media-query";
}

impl Rule for MediaQuery {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: MediaQuery::ID,
            severity: Severity::Warning,
            description: "媒体查询条件在 Skyline 下被丢弃，块内声明对全部设备生效",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        let mut reported: HashSet<(String, usize)> = HashSet::new();
        for sheet in &project.styles {
            for rule in &sheet.doc.rules {
                for at in &rule.at_rules {
                    if at.name != "media" || is_theme_query(&at.prelude) {
                        continue;
                    }
                    // 同一个 @media 块里的每条规则都带同一个 prelude，只报一次。
                    if !reported.insert((sheet.doc.path().to_string(), at.offset)) {
                        continue;
                    }
                    let (line, column) = sheet.doc.position(at.offset);
                    let (end_line, end_column) = sheet.doc.position(at.offset + at.prelude.len());
                    out.push(
                        Diagnostic::new(
                            Self::ID,
                            Severity::Warning,
                            format!(
                                "`@media {}` 的条件在 Skyline 下被丢弃，块内声明对所有设备生效",
                                at.prelude
                            ),
                            sheet.doc.path(),
                            line,
                            column,
                        )
                        .end(end_line, end_column)
                        .help(
                            "把条件里的样式改成 class（按 renderer 或尺寸切换）或改用页面布局手段；\
                             Skyline 只识别 prefers-color-scheme",
                        ),
                    );
                }
            }
        }
        out
    }
}

/// 引擎只认主题表达式（`css_parser_impl.h:355-373`）。
fn is_theme_query(prelude: &str) -> bool {
    prelude
        .to_ascii_lowercase()
        .contains("prefers-color-scheme")
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testutil::wxss::single;

    use super::MediaQuery;

    #[test]
    fn reports_media_blocks_with_ignored_conditions() {
        let (_f, project) = single(
            "media-query",
            r#"
@media (max-device-height: 558px) {
  .dialog { height: calc(100% - 16px); }
}
@media (prefers-color-scheme: dark) {
  .page { background: #000; }
}
.page { color: red; }
"#,
        );
        let diagnostics = MediaQuery.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("max-device-height"));
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (2, 8));
    }

    #[test]
    fn reports_each_block_once() {
        let (_f, project) = single(
            "media-query-once",
            r#"
@media screen and (min-width: 600px) {
  .a { color: red; }
  .b { color: blue; }
}
"#,
        );
        assert_eq!(MediaQuery.check(&project).len(), 1);
    }
}
