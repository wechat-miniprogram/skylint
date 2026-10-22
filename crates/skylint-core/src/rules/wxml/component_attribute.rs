//! Skyline 组件属性值白名单：写了但会被静默忽略的取值。
//!
//! 数据文件 `data/component_attributes.txt` 是唯一依据，Rust 侧只做查表。
//! 只判静态属性：`{{}}` 插值（`WxmlAttribute::is_static == false`）与无值属性
//! 一律跳过——静态阶段无法确定它们的实际取值。

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use crate::diag::{Diagnostic, Severity};
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};

use super::{load_doc, wxml_files};

const DATA: &str = include_str!("../data/component_attributes.txt");

#[derive(Debug, Clone)]
enum Constraint {
    /// 合法值白名单；值以 `*` 结尾表示前缀匹配。
    OneOf(Vec<&'static str>),
    /// 只在 WebView 下生效的值：命中即报。非命中值交给同 key 的 `OneOf`，
    /// 这样裁剪模式才能拿到专门的 help。
    WebviewOnly(Vec<&'static str>),
    /// 属性本身不支持：出现即报。
    Unsupported,
    /// 闭区间 `min..max`。
    Between(f64, f64),
    /// 正整数。
    PositiveInteger,
}

impl Constraint {
    fn rejects(&self, value: &str) -> bool {
        match self {
            Self::OneOf(values) => !contains(values, value),
            Self::WebviewOnly(values) => contains(values, value),
            Self::Unsupported => true,
            Self::Between(min, max) => !value
                .parse::<f64>()
                .is_ok_and(|number| number >= *min && number <= *max),
            Self::PositiveInteger => !value
                .parse::<f64>()
                .is_ok_and(|number| number >= 1.0 && number.fract() == 0.0),
        }
    }
}

fn contains(values: &[&'static str], value: &str) -> bool {
    values.iter().any(|&item| match item.strip_suffix('*') {
        Some(prefix) => value
            .get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix)),
        None => item.eq_ignore_ascii_case(value),
    })
}

#[derive(Debug)]
struct Entry {
    constraint: Constraint,
    help: Option<&'static str>,
}

#[derive(Debug, Default)]
pub(crate) struct Table {
    entries: HashMap<&'static str, HashMap<&'static str, Vec<Entry>>>,
}

impl Table {
    fn lookup(&self, tag: &str, attr: &str) -> Option<&[Entry]> {
        self.entries.get(tag)?.get(attr).map(Vec::as_slice)
    }
}

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| parse(DATA).expect("组件属性白名单格式错误"))
}

#[derive(Clone, Copy)]
enum Section {
    WebviewOnly,
    Enum,
    Unsupported,
    Number,
}

impl Section {
    fn name(self) -> &'static str {
        match self {
            Self::WebviewOnly => "webview-only",
            Self::Enum => "enum",
            Self::Unsupported => "unsupported",
            Self::Number => "number",
        }
    }
}

/// 解析数据文件。格式错误一律返回带行号的说明，便于同步时定位。
fn parse(source: &'static str) -> Result<Table, String> {
    let mut table = Table::default();
    let mut seen = HashSet::new();
    let mut section = None;

    for (index, raw) in source.lines().enumerate() {
        let line_no = index + 1;
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            section = Some(match name.trim() {
                "webview-only" => Section::WebviewOnly,
                "enum" => Section::Enum,
                "unsupported" => Section::Unsupported,
                "number" => Section::Number,
                other => return Err(format!("第 {line_no} 行：未知分区 [{other}]")),
            });
            continue;
        }
        let section = section.ok_or_else(|| format!("第 {line_no} 行：数据出现在分区之前"))?;
        let fields: Vec<&'static str> = line.split('|').map(str::trim).collect();
        if fields.len() != 4 {
            return Err(format!(
                "第 {line_no} 行：需要 `组件 | 属性 | 约束 | 说明` 四列，实际 {} 列",
                fields.len()
            ));
        }
        let (tag, attrs, constraint, help) = (fields[0], fields[1], fields[2], fields[3]);
        if tag.is_empty() || !is_lowercase(tag) {
            return Err(format!("第 {line_no} 行：组件名必须非空且小写：{tag}"));
        }
        let constraint = match section {
            Section::WebviewOnly => Constraint::WebviewOnly(parse_values(constraint, line_no)?),
            Section::Enum => Constraint::OneOf(parse_values(constraint, line_no)?),
            Section::Unsupported => {
                if constraint != "-" {
                    return Err(format!("第 {line_no} 行：[unsupported] 的约束列固定为 `-`"));
                }
                Constraint::Unsupported
            }
            Section::Number => parse_number(constraint, line_no)?,
        };
        let help = (help != "-").then_some(help);
        for attr in attrs.split(',').map(str::trim) {
            if attr.is_empty() || !is_lowercase(attr) {
                return Err(format!("第 {line_no} 行：属性名必须非空且小写：{attr}"));
            }
            if !seen.insert((tag, attr, section.name())) {
                return Err(format!(
                    "第 {line_no} 行：[{}] 重复的条目 {tag} {attr}",
                    section.name()
                ));
            }
            table
                .entries
                .entry(tag)
                .or_default()
                .entry(attr)
                .or_default()
                .push(Entry {
                    constraint: constraint.clone(),
                    help,
                });
        }
    }

    Ok(table)
}

fn parse_values(text: &'static str, line_no: usize) -> Result<Vec<&'static str>, String> {
    let mut values = Vec::new();
    for value in text.split(',').map(str::trim) {
        if value.is_empty() {
            return Err(format!("第 {line_no} 行：合法值不能为空"));
        }
        if values
            .iter()
            .any(|seen: &&str| seen.eq_ignore_ascii_case(value))
        {
            return Err(format!("第 {line_no} 行：重复的合法值 {value}"));
        }
        values.push(value);
    }
    Ok(values)
}

fn parse_number(text: &str, line_no: usize) -> Result<Constraint, String> {
    if text == "positive" {
        return Ok(Constraint::PositiveInteger);
    }
    let Some((min, max)) = text.split_once("..") else {
        return Err(format!(
            "第 {line_no} 行：数值约束需要 `positive` 或 `min..max`"
        ));
    };
    let (min, max) = (parse_bound(min, line_no)?, parse_bound(max, line_no)?);
    if min > max {
        return Err(format!("第 {line_no} 行：数值区间下界大于上界"));
    }
    Ok(Constraint::Between(min, max))
}

fn parse_bound(text: &str, line_no: usize) -> Result<f64, String> {
    text.trim()
        .parse()
        .map_err(|_| format!("第 {line_no} 行：数值边界非法：{text}"))
}

fn is_lowercase(text: &str) -> bool {
    !text.bytes().any(|byte| byte.is_ascii_uppercase())
}

pub struct ComponentAttribute;

impl ComponentAttribute {
    pub const ID: &'static str = "skyline/component-attribute";
}

impl Rule for ComponentAttribute {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "Skyline 会静默忽略的组件属性取值（枚举白名单 / 不支持的属性 / 数值范围）",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let table = table();
        let mut diagnostics = Vec::new();
        for path in wxml_files(project) {
            let Some(doc) = load_doc(project, path) else {
                continue;
            };
            for element in doc.elements() {
                for attr in &element.attributes {
                    if !attr.is_static {
                        continue;
                    }
                    let Some(value) = attr.value.as_deref().map(str::trim) else {
                        // 无值属性在 WXML 里表示布尔 true，不判定取值。
                        continue;
                    };
                    if value.is_empty() {
                        continue;
                    }
                    let name = attr.name.to_ascii_lowercase();
                    let Some(entries) = table.lookup(&element.tag, &name) else {
                        continue;
                    };
                    let Some(entry) = entries.iter().find(|entry| entry.constraint.rejects(value))
                    else {
                        continue;
                    };
                    let (line, column) = doc.position(attr.start);
                    let (end_line, end_column) = doc.position(attr.end);
                    diagnostics.push(
                        Diagnostic::new(
                            Self::ID,
                            Severity::Warning,
                            message(&element.tag, &name, value, &entry.constraint),
                            path,
                            line,
                            column,
                        )
                        .end(end_line, end_column)
                        .help(help(entry)),
                    );
                }
            }
        }
        diagnostics
    }
}

fn message(tag: &str, attr: &str, value: &str, constraint: &Constraint) -> String {
    match constraint {
        Constraint::Unsupported => {
            format!("`<{tag}>` 的 `{attr}` 属性在 Skyline 下不支持，会被静默忽略")
        }
        Constraint::PositiveInteger => format!("`<{tag} {attr}=\"{value}\">` 必须是正整数"),
        Constraint::Between(min, max) => {
            format!("`<{tag} {attr}=\"{value}\">` 必须是 {min}~{max} 的数值")
        }
        Constraint::OneOf(_) | Constraint::WebviewOnly(_) => {
            format!("`<{tag} {attr}=\"{value}\">` 不是 Skyline 支持的值，会被忽略")
        }
    }
}

fn help(entry: &Entry) -> String {
    if let Some(help) = entry.help {
        return help.to_string();
    }
    match &entry.constraint {
        Constraint::OneOf(values) => format!(
            "合法值：{}",
            values
                .iter()
                .map(|value| value.trim_end_matches('*'))
                .collect::<Vec<_>>()
                .join("、")
        ),
        Constraint::WebviewOnly(_) => "改用 aspectFill 配合 CSS 定位".to_string(),
        Constraint::Unsupported => "Skyline 不支持该属性，建议删除".to_string(),
        Constraint::Between(min, max) => format!("合法值：{min}~{max}"),
        Constraint::PositiveInteger => "合法值：正整数".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::testutil::{Fixture, check_rule, project};
    use super::{ComponentAttribute, Constraint, DATA, parse};

    #[test]
    fn enum_violations_are_warnings_on_the_attribute() {
        let diagnostics = check_rule(
            &project(
                "attr-enum",
                r#"<view><image mode="cover"/><text overflow="hidden"/></view>"#,
            ),
            &ComponentAttribute,
        );
        assert_eq!(diagnostics.len(), 2);
        assert!(
            diagnostics
                .iter()
                .all(|d| d.severity == crate::Severity::Warning)
        );
        let image = diagnostics
            .iter()
            .find(|d| d.message.contains("image"))
            .unwrap();
        assert_eq!((image.line, image.column), (1, 14));
        assert_eq!(
            image.help.as_deref().unwrap(),
            "合法值：scaleToFill、aspectFit、aspectFill、widthFix、heightFix"
        );
    }

    #[test]
    fn valid_enum_values_are_allowed() {
        let diagnostics = check_rule(
            &project(
                "attr-enum-ok",
                r#"<image mode="aspectfill"/>
<swiper layout-type="transformer" indicator-type="wormThin" direction="positive"/>
<share-element rect-tween-type="cubic-bezier(0.4, 0, 0.2, 1)" shuttle-on-push="from"/>
<scroll-view scroll-into-view-alignment="CENTER" associative-container="nested-scroll-view"/>
<input type="nickname" confirm-type="done"/>
<snapshot mode="picture"/>
<text overflow="ellipsis"/>"#,
            ),
            &ComponentAttribute,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn dynamic_and_valueless_attributes_are_skipped() {
        let diagnostics = check_rule(
            &project(
                "attr-dynamic",
                r#"<image mode="{{m}}"/><image mode="a{{m}}"/><text space/><share-element shuttle-on-pop/><picker-view indicator-class mask-style/><text max-lines="{{n}}"/>"#,
            ),
            &ComponentAttribute,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn webview_only_crop_modes_get_specific_help() {
        let diagnostics = check_rule(
            &project(
                "attr-crop",
                r#"<image mode="top left"/><image mode="center"/>"#,
            ),
            &ComponentAttribute,
        );
        assert_eq!(diagnostics.len(), 2);
        for diagnostic in &diagnostics {
            assert!(diagnostic.message.contains("不是 Skyline 支持的值"));
            assert!(diagnostic.help.as_deref().unwrap().contains("aspectFill"));
        }
        assert!(diagnostics[0].message.contains("top left"));
    }

    #[test]
    fn unsupported_attributes_are_reported() {
        let diagnostics = check_rule(
            &project(
                "attr-unsupported",
                r#"<text space="nbsp"/><text decode="true"/><picker-view indicator-class="a" mask-style="b"/>"#,
            ),
            &ComponentAttribute,
        );
        assert_eq!(diagnostics.len(), 4);
        assert!(
            diagnostics
                .iter()
                .all(|d| d.message.contains("不支持") && d.severity == crate::Severity::Warning)
        );
        assert!(
            diagnostics
                .iter()
                .any(|d| d.help.as_deref().unwrap().contains("white-space"))
        );
    }

    #[test]
    fn unsupported_attributes_are_scoped_to_their_component() {
        let diagnostics = check_rule(
            &project(
                "attr-scope",
                r#"<view space="nbsp" decode="true"/><text mode="cover"/><image overflow="hidden"/>"#,
            ),
            &ComponentAttribute,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn number_constraints_are_reported() {
        let diagnostics = check_rule(
            &project(
                "attr-number",
                r#"<text max-lines="0"/><text max-lines="2.5"/><text max-lines="abc"/>
<draggable-sheet initial-child-size="1.5" min-child-size="-0.1" max-child-size="2"/>"#,
            ),
            &ComponentAttribute,
        );
        assert_eq!(diagnostics.len(), 6);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("正整数") && d.message.contains("max-lines"))
        );
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("0~1 的数值") && d.message.contains("min-child-size"))
        );
    }

    #[test]
    fn number_constraints_accept_bounds_and_skip_dynamic() {
        let diagnostics = check_rule(
            &project(
                "attr-number-ok",
                r#"<text max-lines="1"/><text max-lines="1.0"/><text max-lines="{{n}}"/>
<draggable-sheet initial-child-size="0" min-child-size="0.25" max-child-size="1"/>"#,
            ),
            &ComponentAttribute,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// `"renderer": "webview"` 的页面不迁 Skyline，整页跳过。
    #[test]
    fn opted_out_page_is_skipped() {
        let fixture = Fixture::new("attr-opt-out");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index","pages/webview/webview"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", "<view/>")
            .write("pages/webview/webview.json", r#"{"renderer":"webview"}"#)
            .write(
                "pages/webview/webview.wxml",
                r#"<image mode="cover"/><text space="nbsp"/>"#,
            );
        let diagnostics = check_rule(&fixture, &ComponentAttribute);
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn data_file_parses_every_section() {
        let table = parse(DATA).unwrap();

        let image_mode = table.lookup("image", "mode").unwrap();
        assert!(matches!(
            image_mode[0].constraint,
            Constraint::WebviewOnly(_)
        ));
        let Constraint::OneOf(values) = &image_mode[1].constraint else {
            panic!("image mode 缺少枚举分区");
        };
        assert!(values.contains(&"aspectFill"));

        assert!(matches!(
            table.lookup("text", "space").unwrap()[0].constraint,
            Constraint::Unsupported
        ));
        assert!(matches!(
            table.lookup("picker-view", "mask-style").unwrap()[0].constraint,
            Constraint::Unsupported
        ));
        assert!(matches!(
            table.lookup("text", "max-lines").unwrap()[0].constraint,
            Constraint::PositiveInteger
        ));
        assert!(matches!(
            table.lookup("draggable-sheet", "initial-child-size").unwrap()[0].constraint,
            Constraint::Between(min, max) if min == 0.0 && max == 1.0
        ));
        assert!(table.lookup("share-element", "shuttle-on-pop").is_some());
        assert!(table.lookup("view", "space").is_none());
    }

    #[test]
    fn every_data_row_cites_a_source() {
        for (index, raw) in DATA.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
                continue;
            }
            assert!(
                line.contains('#') && line.contains(".md:"),
                "第 {} 行缺少文档来源：{line}",
                index + 1
            );
        }
    }

    #[test]
    fn rejects_malformed_rows() {
        let err = parse("[enum]\ntext | overflow | visible\n").unwrap_err();
        assert!(err.contains("四列"), "{err}");

        let err = parse("[nope]\n").unwrap_err();
        assert!(err.contains("未知分区"), "{err}");

        let err = parse("[number]\ntext | max-lines | lots | -  # x\n").unwrap_err();
        assert!(err.contains("positive"), "{err}");

        let err = parse("[enum]\ntext | overflow | visible,visible | -  # x\n").unwrap_err();
        assert!(err.contains("重复的合法值"), "{err}");

        let err =
            parse("[enum]\ntext | overflow | visible | -  # x\ntext | overflow | fade | -  # x\n")
                .unwrap_err();
        assert!(err.contains("重复的条目"), "{err}");
    }
}
