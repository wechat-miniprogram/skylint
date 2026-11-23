//! `skyline/media-query`：媒体查询条件被引擎丢弃，块内声明无条件生效。
//!
//! 引擎解析规则时只把 `media_index` 拿去做颜色主题（
//! 的 `ParseColorScheme` 只看 `MediaExpression::Tag::Theme`），生成
//! `CSSStyleRule` 时并不带任何媒体条件（同文件 `:443-460`）。于是
//! `@media (max-device-height: 558px) { … }` 里的声明在 Skyline 下对所有设备生效，
//! 而 WebView 只在条件成立时生效——这类块通常写的是小屏适配或弹窗高度覆盖，
//! 两端结果会明显不同。
//!
//! 唯一被识别的是 `prefers-color-scheme`，这类块跳过。

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::input::wxml::WxmlDoc;
use crate::input::wxss::WxssSelector;
use crate::matcher::Tri;
use crate::project::Project;
use crate::rules::{Rule, RuleMeta, VisualCase, VisualExpect};
use crate::{Diagnostic, Severity};

pub struct MediaQuery;

impl MediaQuery {
    pub const ID: &'static str = "skyline/media-query";
}

/// 视觉用例：恒不成立的媒体条件在 Skyline 下被丢弃，块内声明无条件生效。
///
/// 375px 视口下 `min-width: 5000px` 必然不成立，WebView 只用基础声明；
/// Skyline 不解析条件，后写的块内声明生效。
static VISUAL_CASES: &[VisualCase] = &[VisualCase {
    wxml: r#"<view class="panel" id="target">面板</view>"#,
    wxss: ".panel { height: 40px; line-height: 40px; text-align: center; background: #007aff; color: #fff; }\n@media (min-width: 5000px) { .panel { height: 160px; line-height: 160px; background: #ff3b30; } }\n",
    js: None,
    expect: VisualExpect::Differ,
    note: "`@media (min-width: 5000px)` 恒不成立：WebView 保持 40px 蓝底，Skyline 丢弃条件后块内声明无条件生效，变成 160px 红底",
    target: "#target",
}];

impl Rule for MediaQuery {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: MediaQuery::ID,
            severity: Severity::Warning,
            description: "媒体查询条件在 Skyline 下被丢弃，块内声明对全部设备生效",
        }
    }

    fn visual_cases(&self) -> &'static [VisualCase] {
        VISUAL_CASES
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        let mut reported: HashSet<(String, usize)> = HashSet::new();
        let mut docs: HashMap<String, Option<Rc<WxmlDoc>>> = HashMap::new();
        for sheet in &project.styles {
            // 块内选择器在作用域里一个节点都命不中时，条件丢失不产生可见差异。
            let matched = |at_offset: usize, docs: &mut HashMap<String, Option<Rc<WxmlDoc>>>| {
                let selectors: Vec<WxssSelector> = sheet
                    .doc
                    .rules
                    .iter()
                    .filter(|rule| rule.at_rules.iter().any(|at| at.offset == at_offset))
                    .flat_map(|rule| rule.selectors.iter().cloned())
                    .collect();
                super::any_selector_matches(project, sheet, &selectors, docs)
            };
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
                    match matched(at.offset, &mut docs) {
                        // 块内选择器一个节点都对不上：条件丢失不产生可见差异。
                        Tri::No => continue,
                        // 命中取决于运行时 class（动态 class / 动态属性）：静态判不出，
                        // 但也不能静默——这类块恰恰是最容易在迁移后被漏掉的一类。
                        Tri::Unknown => {
                            let (end_line, end_column) =
                                sheet.doc.position(at.offset + at.prelude.len());
                            out.push(
                                Diagnostic::new(
                                    Self::ID,
                                    Severity::Info,
                                    format!(
                                        "`@media {}` 的条件在 Skyline 下被丢弃；块内选择器命中的节点取决于运行时数据，静态判不出是否生效",
                                        at.prelude
                                    ),
                                    sheet.doc.path(),
                                    line,
                                    column,
                                )
                                .end(end_line, end_column)
                                .help(
                                    "确认块内声明的节点在运行时确实存在；Skyline 只识别 prefers-color-scheme",
                                ),
                            );
                            continue;
                        }
                        Tri::Yes => {}
                    }
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

/// 引擎只认主题表达式。
fn is_theme_query(prelude: &str) -> bool {
    prelude
        .to_ascii_lowercase()
        .contains("prefers-color-scheme")
}

#[cfg(test)]
mod tests {
    use crate::rules::testutil::wxss;

    use super::MediaQuery;

    fn check(name: &str, wxml: &str, wxss: &str) -> Vec<crate::Diagnostic> {
        wxss::check(&MediaQuery, name, wxml, wxss)
    }

    /// 视觉用例与单测共用同一份 WXML/WXSS：每条都必须被本规则命中。
    #[test]
    fn visual_cases_are_reported() {
        for case in super::VISUAL_CASES {
            let fixture =
                crate::rules::testutil::Fixture::page("media-query-visual", case.wxml, case.wxss);
            let hits = crate::rules::testutil::check_rule(&fixture, &MediaQuery)
                .iter()
                .filter(|diagnostic| diagnostic.rule == MediaQuery::ID)
                .count();
            assert!(hits >= 1, "{}: {}", case.note, case.wxml);
        }
    }

    #[test]
    fn reports_media_blocks_with_ignored_conditions() {
        let diagnostics = check(
            "media-query",
            r#"<view class="dialog"/><view class="page"/>"#,
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
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("max-device-height"));
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (2, 8));
    }

    #[test]
    fn blocks_whose_selectors_match_nothing_are_skipped() {
        // 条件被丢弃但块内选择器在本作用域的 WXML 上一个节点都命不中：没有可见差异。
        let diagnostics = check(
            "media-query-dead-block",
            r#"<view class="real"/>"#,
            r#"
@media (max-device-height: 558px) {
  .container { height: 100%; }
}
.real { color: red; }
"#,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_each_block_once() {
        let diagnostics = check(
            "media-query-once",
            r#"<view class="a"/><view class="b"/>"#,
            r#"
@media screen and (min-width: 600px) {
  .a { color: red; }
  .b { color: blue; }
}
"#,
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }
}
