//! WXML `style` 属性里的内联声明。
//!
//! 内联样式不在任何 `.wxss` 里，属性 / 取值类检查只有走 [`visit`] 才读得到它。
//! 解析复用 `input::wxss`：把属性值包成 `.x{…}` 交给同一份解析器，属性名、取值与
//! 词元跟样式表里完全同源，规则侧不需要第二套切分逻辑。
//!
//! 定位要换回 WXML。合成文档里的行列相对 `.x{` 起算，[`Host`] 把它们换算到属性值上：
//! 诊断因此落在 `style="…"` 的那一段文本里（可以精确到单条声明），而不是文件开头。

use crate::Diagnostic;
use crate::input::wxml::{WxmlAttribute, WxmlDoc};
use crate::input::wxss::{self, WxssDoc, WxssRule};
use crate::project::{Project, Scope, StyleSheet};

/// 合成文档里包住声明块的选择器。长度参与定位换算。
const WRAPPER: &str = ".x{";

/// 把一段声明文本解析成一条**没有选择器**的规则。
///
/// 清空选择器是刻意的：内联声明不经过选择器匹配，靠「这条声明落在哪些节点上」做的
/// 豁免（`float`、`fit-content`、`text-overflow`）在这里没有依据，规则会按「不豁免」
/// 处理——内联样式本来就只落在一个元素上，是保守的一侧。
///
/// 解析结果不是「恰好一条带声明的规则」时返回 `None`：取值里出现 `}`、或整段不是
/// 声明块（内联的 HTML 片段）时判不了，直接跳过。
pub(crate) fn parse_block(path: &str, text: &str) -> Option<WxssDoc> {
    let mut doc = wxss::parse(path, &format!("{WRAPPER}{text}}}")).ok()?;
    if doc.rules.len() != 1 || doc.rules[0].declarations.is_empty() {
        return None;
    }
    doc.rules[0].selectors.clear();
    Some(doc)
}

/// 遍历全部 WXML `style` 属性里的声明。
///
/// 文档集合与元素级 WXML 规则一致（显式 `renderer: "webview"` 的页面整页跳过）；
/// 只在 WebView 分支里渲染的元素也跳过——迁到 Skyline 后它根本不存在。
pub(crate) fn visit(
    project: &Project,
    mut f: impl FnMut(&StyleSheet, &WxssRule, &mut Vec<Diagnostic>),
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for path in crate::rules::wxml::wxml_files(project) {
        let Some(doc) = crate::rules::wxml::load_doc(project, path) else {
            continue;
        };
        let scopes = scopes_of(project, path);
        for element in doc.elements() {
            if element.webview_only() {
                continue;
            }
            for attribute in &element.attributes {
                let (true, Some(value_start)) = (attribute.is_static, attribute.value_start) else {
                    continue;
                };
                if !attribute.name.eq_ignore_ascii_case("style") {
                    continue;
                }
                let text = attribute_text(&doc, attribute);
                let Some(block) = parse_block(path, text) else {
                    continue;
                };
                let sheet = StyleSheet {
                    doc: block,
                    scopes: scopes.clone(),
                };
                let emitted = out.len();
                f(&sheet, &sheet.doc.rules[0], &mut out);
                let host = Host {
                    start: doc.position(value_start),
                    value: text,
                };
                for diagnostic in &mut out[emitted..] {
                    host.remap(&sheet.doc, diagnostic);
                }
            }
        }
    }
    out
}

/// 样式表作用域：内联样式跟着它所在的单元走，模板片段取引用它的单元。
fn scopes_of(project: &Project, path: &str) -> Vec<Scope> {
    project.scope_of(path).map_or_else(
        || {
            project
                .template_scopes
                .get(path)
                .cloned()
                .unwrap_or_default()
        },
        |scope| vec![scope],
    )
}

/// 属性值的原文（未做实体解码）：定位换算用的是源码字符，与原文保持一致。
///
/// 解析器给的 `end` 不含结尾引号；比解码后的取值长时防御性地剥一次引号，
/// 避免不同解析器版本把引号一起算进区间。
fn attribute_text<'a>(doc: &'a WxmlDoc, attribute: &WxmlAttribute) -> &'a str {
    let text = doc.slice(
        attribute.value_start.expect("调用方已确认有值起点"),
        attribute.end,
    );
    match attribute.value.as_deref() {
        Some(value) if text.len() > value.len() => text.trim_end_matches(['"', '\'']),
        _ => text,
    }
}

/// 内联样式在宿主文件里的起点，以及属性值原文。
struct Host<'a> {
    /// 值起点（1-based 行、按字符计的列）。
    start: (u32, u32),
    value: &'a str,
}

impl Host<'_> {
    /// 把诊断从合成文档的行列换算回宿主文件。
    fn remap(&self, doc: &WxssDoc, diagnostic: &mut Diagnostic) {
        let (line, column) = self.position(doc, diagnostic.line, diagnostic.column);
        let (end_line, end_column) = self.position(doc, diagnostic.end_line, diagnostic.end_column);
        diagnostic.line = line;
        diagnostic.column = column;
        diagnostic.end_line = end_line;
        diagnostic.end_column = end_column;
        diagnostic.path = doc.path().to_string();
    }

    fn position(&self, doc: &WxssDoc, line: u32, column: u32) -> (u32, u32) {
        let limit = WRAPPER.len() + self.value.len();
        let offset = doc
            .offset(line, column)
            .min(limit)
            .saturating_sub(WRAPPER.len());
        self.value
            .get(..offset)
            .map_or(self.start, |prefix| advance(self.start, prefix))
    }
}

/// 从 `start` 起走完 `text` 后的行列（`\n` 归位到下一行第 1 列）。
fn advance(mut at: (u32, u32), text: &str) -> (u32, u32) {
    for c in text.chars() {
        if c == '\n' {
            at = (at.0 + 1, 1);
        } else {
            at.1 += 1;
        }
    }
    at
}

#[cfg(test)]
mod tests {
    use super::{Host, advance, parse_block};
    use crate::rules::Rule;
    use crate::rules::testutil::check_rule;
    use crate::rules::wxss::{
        CalcMinMax, GradientSupport, ShorthandPartial, UnsupportedProperty, UnsupportedValue,
        ValueDegraded, VendorPrefix,
    };

    /// 属性 / 取值类规则都从同一个入口读到 WXML `style` 属性里的声明。
    #[test]
    fn declaration_rules_read_the_style_attribute() {
        let fixture = crate::rules::testutil::Fixture::page(
            "inline-style-entry",
            r##"<view class="a" style="float: left; display: grid; word-break: keep-all; background: url(x.png) fixed; -webkit-animation: fade .3s; background: -webkit-linear-gradient(#fff, #000); width: min(100%, 300px)"></view>"##,
            ".a { color: red }\n",
        );
        let rules: [&dyn Rule; 7] = [
            &UnsupportedProperty,
            &UnsupportedValue,
            &ValueDegraded,
            &ShorthandPartial,
            &VendorPrefix,
            &GradientSupport,
            &CalcMinMax,
        ];
        for rule in rules {
            let diagnostics = check_rule(&fixture, rule);
            let hits: Vec<_> = diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.path == "pages/index/index.wxml")
                .collect();
            assert!(
                !hits.is_empty(),
                "{} 没读到 style 属性：{diagnostics:#?}",
                rule.meta().id
            );
            // 都落在 `style` 的值里（`<view class="a" style="` 之后）。
            assert!(
                hits.iter()
                    .all(|diagnostic| diagnostic.line == 1 && diagnostic.column > 22),
                "{} 的位置没落到属性值上：{hits:#?}",
                rule.meta().id
            );
        }
    }

    #[test]
    fn a_declaration_block_is_parsed_without_selectors() {
        let doc = parse_block("a.wxml", "display: grid; color: red").unwrap();
        assert_eq!(doc.rules.len(), 1);
        assert!(doc.rules[0].selectors.is_empty());
        let names: Vec<&str> = doc.rules[0]
            .declarations
            .iter()
            .map(|declaration| declaration.name.as_str())
            .collect();
        assert_eq!(names, ["display", "color"]);
    }

    /// 内联的 HTML 片段 / 拼接出来的半截不是声明块，不能硬解析成声明。
    #[test]
    fn non_declaration_text_is_rejected() {
        assert!(parse_block("a.wxml", "网络异常，请稍后重试").is_none());
        assert!(parse_block("a.wxml", "<span style='color:red'>").is_none());
        assert!(parse_block("a.wxml", "").is_none());
    }

    /// 合成文档以 `.x{` 起头：第一行的列要减掉前缀，后续行与宿主原文对齐。
    #[test]
    fn positions_map_back_to_the_attribute_value() {
        let value = "display: grid;\n  color: red";
        let doc = parse_block("a.wxml", value).unwrap();
        let host = Host {
            start: (3, 20),
            value,
        };
        // 第一条声明的属性名起点。
        let (line, column) = doc.position(doc.rules[0].declarations[0].offset);
        assert_eq!(host.position(&doc, line, column), (3, 20));
        // 第二条声明在第二行，列号从 1 起，与宿主那一行一致。
        let (line, column) = doc.position(doc.rules[0].declarations[1].offset);
        assert_eq!(host.position(&doc, line, column), (4, 3));
    }

    #[test]
    fn advance_counts_characters_not_bytes() {
        assert_eq!(advance((1, 1), "中文"), (1, 3));
        assert_eq!(advance((1, 5), "\n"), (2, 1));
    }
}
