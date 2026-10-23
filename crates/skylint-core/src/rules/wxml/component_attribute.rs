//! Skyline 组件属性值白名单：写了但会被静默忽略的取值，以及跨属性的生效条件。
//!
//! 数据文件 `data/component_attributes.txt` 是唯一依据，Rust 侧只做查表。
//! 只判静态属性：`{{}}` 插值（`WxmlAttribute::is_static == false`）一律跳过，
//! 静态阶段无法确定它们的实际取值。`[requires]` 除外：无值属性视为布尔 true，
//! 这足以确认 `circular=*` 这类触发条件。

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use crate::diag::{Diagnostic, Severity};
use crate::input::wxml::{WxmlAttribute, WxmlElement};
use crate::project::Project;
use crate::rules::data_table::{self, Columns};
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

/// 跨属性生效条件：触发属性命中时，要求属性必须满足约束。
#[derive(Debug)]
struct Requires {
    trigger: Trigger,
    requirement: Requirement,
    help: Option<&'static str>,
}

#[derive(Debug)]
enum Trigger {
    /// `attr=*`：属性存在且取值非空；无值属性（布尔 true）同样触发。
    Present(&'static str),
    Equals(&'static str, &'static str),
    NotAny(&'static str, Vec<&'static str>),
}

#[derive(Debug)]
enum Requirement {
    /// `attr=value`：属性必须存在且等于该值。
    Equals(&'static str, &'static str),
    /// `attr=!(a|b)`：属性不得等于其中任一值。
    NotAny(&'static str, Vec<&'static str>),
}

impl Trigger {
    fn attr(&self) -> &'static str {
        match self {
            Self::Present(attr) | Self::Equals(attr, _) | Self::NotAny(attr, _) => attr,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct Table {
    entries: HashMap<&'static str, HashMap<&'static str, Vec<Entry>>>,
    requires: HashMap<&'static str, Vec<Requires>>,
}

impl Table {
    fn lookup(&self, tag: &str, attr: &str) -> Option<&[Entry]> {
        let attrs = self.entries.get(tag)?;
        if let Some(entries) = attrs.get(attr) {
            return Some(entries);
        }
        // 属性名以 `*` 结尾表示前缀匹配（如 safe-password-*）；精确名优先。
        attrs
            .iter()
            .filter_map(|(name, entries)| {
                let prefix = name.strip_suffix('*')?;
                attr.starts_with(prefix).then_some((prefix.len(), entries))
            })
            .max_by_key(|(len, _)| *len)
            .map(|(_, entries)| entries.as_slice())
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
    Requires,
}

impl Section {
    fn name(self) -> &'static str {
        match self {
            Self::WebviewOnly => "webview-only",
            Self::Enum => "enum",
            Self::Unsupported => "unsupported",
            Self::Number => "number",
            Self::Requires => "requires",
        }
    }
}

/// 解析数据文件。格式错误一律返回带行号的说明，便于同步时定位。
fn parse(source: &'static str) -> Result<Table, String> {
    let parsed = data_table::read(source, split_fields, |_| {
        Columns::new(4, "`组件 | 属性 | 约束 | 说明` 四列")
    })?;
    for (line_no, name) in &parsed.sections {
        if !matches!(
            *name,
            "webview-only" | "enum" | "unsupported" | "number" | "requires"
        ) {
            return Err(format!("第 {line_no} 行：未知分区 [{name}]"));
        }
    }

    let mut table = Table::default();
    let mut seen = HashSet::new();
    for row in parsed.rows {
        let section = match row.section {
            Some("webview-only") => Section::WebviewOnly,
            Some("enum") => Section::Enum,
            Some("unsupported") => Section::Unsupported,
            Some("number") => Section::Number,
            Some("requires") => Section::Requires,
            Some(other) => return Err(format!("第 {} 行：未知分区 [{other}]", row.line_no)),
            None => return Err(format!("第 {} 行：数据出现在分区之前", row.line_no)),
        };
        let (tags, attrs, constraint, help) =
            (row.fields[0], row.fields[1], row.fields[2], row.fields[3]);
        if tags.is_empty() || !is_lowercase(tags) {
            return Err(format!(
                "第 {} 行：组件名必须非空且小写：{tags}",
                row.line_no
            ));
        }
        let help = (help != "-").then_some(help);
        if matches!(section, Section::Requires) {
            if tags.contains(',') {
                return Err(format!(
                    "第 {} 行：[requires] 每行只能写一个组件",
                    row.line_no
                ));
            }
            data_table::unique(
                &mut seen,
                (tags, attrs, section.name()),
                row.line_no,
                || format!("[requires] 重复的条目 {tags} {attrs}"),
            )?;
            table.requires.entry(tags).or_default().push(parse_requires(
                attrs,
                constraint,
                help,
                row.line_no,
            )?);
            continue;
        }
        let constraint = match section {
            Section::WebviewOnly => Constraint::WebviewOnly(parse_values(constraint, row.line_no)?),
            Section::Enum => Constraint::OneOf(parse_values(constraint, row.line_no)?),
            Section::Unsupported => {
                if constraint != "-" {
                    return Err(format!(
                        "第 {} 行：[unsupported] 的约束列固定为 `-`",
                        row.line_no
                    ));
                }
                Constraint::Unsupported
            }
            Section::Number => parse_number(constraint, row.line_no)?,
            Section::Requires => unreachable!("[requires] 已在前面处理"),
        };
        for tag in tags.split(',').map(str::trim) {
            if tag.is_empty() {
                return Err(format!("第 {} 行：组件名不能为空", row.line_no));
            }
            for attr in attrs.split(',').map(str::trim) {
                if attr.is_empty() || !is_lowercase(attr) {
                    return Err(format!(
                        "第 {} 行：属性名必须非空且小写：{attr}",
                        row.line_no
                    ));
                }
                data_table::unique(&mut seen, (tag, attr, section.name()), row.line_no, || {
                    format!("[{}] 重复的条目 {tag} {attr}", section.name())
                })?;
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
    }

    Ok(table)
}

/// 按 `|` 分列，但括弧内的 `|` 属于 `!(a|b)`，不能当分隔符。
fn split_fields<'a>(_: Option<&'a str>, line: &'a str) -> Vec<&'a str> {
    let mut fields = Vec::new();
    let mut depth = 0_u32;
    let mut start = 0;
    for (index, byte) in line.bytes().enumerate() {
        match byte {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b'|' if depth == 0 => {
                fields.push(line[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    fields.push(line[start..].trim());
    fields
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

/// `[requires]` 的一行：`触发 | 要求`。
fn parse_requires(
    trigger_text: &'static str,
    requirement_text: &'static str,
    help: Option<&'static str>,
    line_no: usize,
) -> Result<Requires, String> {
    let (trigger_attr, trigger_expr) = parse_condition(trigger_text, line_no)?;
    let trigger = match trigger_expr {
        Expr::Any => Trigger::Present(trigger_attr),
        Expr::Equals(value) => Trigger::Equals(trigger_attr, value),
        Expr::NotAny(values) => Trigger::NotAny(trigger_attr, values),
    };
    let (requirement_attr, requirement_expr) = parse_condition(requirement_text, line_no)?;
    let requirement = match requirement_expr {
        Expr::Any => {
            return Err(format!("第 {line_no} 行：[requires] 的要求列不支持 `*`"));
        }
        Expr::Equals(value) => Requirement::Equals(requirement_attr, value),
        Expr::NotAny(values) => Requirement::NotAny(requirement_attr, values),
    };
    Ok(Requires {
        trigger,
        requirement,
        help,
    })
}

#[derive(Debug)]
enum Expr {
    /// `*`：存在且非空。
    Any,
    Equals(&'static str),
    NotAny(Vec<&'static str>),
}

fn parse_condition(text: &'static str, line_no: usize) -> Result<(&'static str, Expr), String> {
    let Some((attr, expr)) = text.split_once('=') else {
        return Err(format!("第 {line_no} 行：需要 `属性=取值`，实际 `{text}`"));
    };
    let (attr, expr) = (attr.trim(), expr.trim());
    if attr.is_empty() || !is_lowercase(attr) {
        return Err(format!("第 {line_no} 行：属性名必须非空且小写：{attr}"));
    }
    Ok((attr, parse_expr(expr, line_no)?))
}

fn parse_expr(text: &'static str, line_no: usize) -> Result<Expr, String> {
    if text == "*" {
        return Ok(Expr::Any);
    }
    if let Some(inner) = text.strip_prefix("!(") {
        let inner = inner
            .strip_suffix(')')
            .ok_or_else(|| format!("第 {line_no} 行：`!(` 缺少右括号：{text}"))?;
        let values: Vec<&'static str> = inner.split('|').map(str::trim).collect();
        if values.iter().any(|value| value.is_empty()) {
            return Err(format!("第 {line_no} 行：`!(...)` 里有空值：{text}"));
        }
        return Ok(Expr::NotAny(values));
    }
    if text.is_empty() {
        return Err(format!("第 {line_no} 行：取值不能为空"));
    }
    Ok(Expr::Equals(text))
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
            description: "Skyline 会静默忽略的组件属性取值与组合（枚举白名单 / 不支持的属性 / 数值范围 / 跨属性条件）",
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
                    let name = attr.name.to_ascii_lowercase();
                    let Some(entries) = table.lookup(&element.tag, &name) else {
                        continue;
                    };
                    // 无值属性是布尔 true：`[unsupported]` 只看属性是否存在，
                    // 取值类约束（枚举 / 区间）无法判定，跳过。
                    let value = attr.value.as_deref().map(str::trim).unwrap_or("");
                    let Some(entry) = entries.iter().find(|entry| match entry.constraint {
                        Constraint::Unsupported => true,
                        _ => !value.is_empty() && entry.constraint.rejects(value),
                    }) else {
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
                let Some(rules) = table.requires.get(element.tag.as_str()) else {
                    continue;
                };
                for rule in rules {
                    let Some(attr) = find_attribute(element, rule.trigger.attr()) else {
                        continue;
                    };
                    if !attr.is_static {
                        continue;
                    }
                    let Some(value) = trigger_value(attr, &rule.trigger) else {
                        continue;
                    };
                    let Verdict::Violated(current) = judge(element, &rule.requirement) else {
                        continue;
                    };
                    let (line, column) = doc.position(attr.start);
                    let (end_line, end_column) = doc.position(attr.end);
                    diagnostics.push(
                        Diagnostic::new(
                            Self::ID,
                            Severity::Warning,
                            requires_message(&element.tag, rule, &value, current.as_deref()),
                            path,
                            line,
                            column,
                        )
                        .end(end_line, end_column)
                        .help(
                            rule.help
                                .unwrap_or("调整这两个属性的取值，避免组合被忽略")
                                .to_string(),
                        ),
                    );
                }
            }
        }
        diagnostics
    }
}

fn find_attribute<'a>(element: &'a WxmlElement, name: &str) -> Option<&'a WxmlAttribute> {
    element
        .attributes
        .iter()
        .find(|attr| attr.name.eq_ignore_ascii_case(name))
}

/// 触发条件命中时返回用于展示的取值，无值属性返回空串。
fn trigger_value(attr: &WxmlAttribute, trigger: &Trigger) -> Option<String> {
    let value = attr.value.as_deref().map(str::trim).unwrap_or_default();
    let hit = match trigger {
        // 无值属性（`<swiper circular>`）等于布尔 true，必须能触发。
        Trigger::Present(_) => attr.value.is_none() || !value.is_empty(),
        Trigger::Equals(_, expected) => !value.is_empty() && value.eq_ignore_ascii_case(expected),
        Trigger::NotAny(_, banned) => !value.is_empty() && !contains(banned, value),
    };
    hit.then(|| value.to_string())
}

enum Verdict {
    /// 要求成立，或要求属性是动态值、静态阶段无法判断。
    Ok,
    /// 要求不成立；`None` 表示要求属性缺失。
    Violated(Option<String>),
}

fn judge(element: &WxmlElement, requirement: &Requirement) -> Verdict {
    match requirement {
        Requirement::Equals(name, expected) => {
            let Some(attr) = find_attribute(element, name) else {
                // 缺失即走默认值，按「不存在」处理：要求 `=v` 不成立。
                return Verdict::Violated(None);
            };
            if !attr.is_static {
                return Verdict::Ok;
            }
            let value = attr.value.as_deref().map(str::trim).unwrap_or_default();
            if value.eq_ignore_ascii_case(expected) {
                Verdict::Ok
            } else {
                Verdict::Violated(Some(value.to_string()))
            }
        }
        Requirement::NotAny(name, banned) => {
            let Some(attr) = find_attribute(element, name) else {
                // 缺失同样按「不存在」处理：不等于任一被禁值（如 layout-type 缺省 normal），
                // 所以不报。
                return Verdict::Ok;
            };
            if !attr.is_static {
                return Verdict::Ok;
            }
            let value = attr.value.as_deref().map(str::trim).unwrap_or_default();
            if contains(banned, value) {
                Verdict::Violated(Some(value.to_string()))
            } else {
                Verdict::Ok
            }
        }
    }
}

fn requires_message(tag: &str, rule: &Requires, value: &str, current: Option<&str>) -> String {
    let trigger = match value {
        "" => rule.trigger.attr().to_string(),
        value => format!("{}=\"{value}\"", rule.trigger.attr()),
    };
    match &rule.requirement {
        Requirement::Equals(name, expected) => {
            let current = match current {
                Some(value) => format!("{name}=\"{value}\""),
                None => format!("未设置 {name}"),
            };
            format!(
                "`<{tag} {trigger}>` 的 `{}` 需要 `{name}=\"{expected}\"`，当前 {current}",
                rule.trigger.attr()
            )
        }
        Requirement::NotAny(name, _) => format!(
            "`<{tag} {trigger}>` 的 `{}` 在 `{name}=\"{}\"` 时不生效，会被忽略",
            rule.trigger.attr(),
            current.unwrap_or_default()
        ),
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
    use super::{ComponentAttribute, Constraint, DATA, Requirement, Trigger, parse};

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
    fn dynamic_and_valueless_enum_attributes_are_skipped() {
        let diagnostics = check_rule(
            &project(
                "attr-dynamic",
                r#"<image mode="{{m}}"/><image mode="a{{m}}"/><share-element shuttle-on-pop/><text max-lines="{{n}}"/>"#,
            ),
            &ComponentAttribute,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// 无值属性是布尔 true：`[unsupported]` 的属性出现即无效，不能因为没写值就跳过。
    #[test]
    fn unsupported_attribute_without_value_is_reported() {
        let diagnostics = check_rule(
            &project(
                "attr-bare",
                r#"<text space/><picker-view indicator-class/>"#,
            ),
            &ComponentAttribute,
        );
        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
        assert!(diagnostics.iter().all(|d| d.message.contains("不支持")));
        assert!(diagnostics[0].message.contains("space"));
        assert!(diagnostics[1].message.contains("indicator-class"));
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

        let Constraint::OneOf(values) = &table
            .lookup("horizontal-drag-gesture-handler", "native-view")
            .unwrap()[0]
            .constraint
        else {
            panic!("native-view 缺少枚举分区");
        };
        assert_eq!(values, &["scroll-view", "swiper"]);

        // `[unsupported]` 的属性名以 `*` 结尾时按前缀匹配。
        assert!(matches!(
            table.lookup("input", "placeholder-class").unwrap()[0].constraint,
            Constraint::Unsupported
        ));
        assert!(matches!(
            table.lookup("input", "safe-password-nonce").unwrap()[0].constraint,
            Constraint::Unsupported
        ));
        assert!(table.lookup("input", "safe-password").is_none());
        assert!(table.lookup("view", "placeholder-class").is_none());

        let swiper = table.requires.get("swiper").unwrap();
        assert_eq!(swiper.len(), 3);
        assert!(matches!(
            &swiper[0].trigger,
            Trigger::Present("transformer-type")
        ));
        assert!(matches!(
            &swiper[1].requirement,
            Requirement::NotAny("layout-type", _)
        ));
    }

    #[test]
    fn requires_transformer_type_needs_transformer_layout() {
        let diagnostics = check_rule(
            &project(
                "attr-requires-transformer",
                r#"<swiper layout-type="normal" transformer-type="threeD"/>
<swiper transformer-type="scaleAndFade"/>
<swiper layout-type="transformer" transformer-type="threeD"/>
<swiper layout-type="transformer"/>"#,
            ),
            &ComponentAttribute,
        );
        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("transformer-type"));
        assert!(
            diagnostics[0]
                .message
                .contains(r#"layout-type="transformer""#)
        );
        assert!(
            diagnostics[0]
                .message
                .contains(r#"当前 layout-type="normal""#)
        );
        // 无 layout-type 即默认 normal，同样不满足要求。
        assert!(diagnostics[1].message.contains("未设置 layout-type"));
        assert_eq!((diagnostics[1].line, diagnostics[1].column), (2, 9));
        assert!(
            diagnostics[0]
                .help
                .as_deref()
                .unwrap()
                .contains("layout-type")
        );
    }

    #[test]
    fn requires_indicator_type_needs_normal_under_stack_layouts() {
        let diagnostics = check_rule(
            &project(
                "attr-requires-indicator",
                r#"<swiper layout-type="stackLeft" indicator-type="worm"/>
<swiper layout-type="stackRight" indicator-type="slide"/>
<swiper layout-type="tinder" indicator-type="scale"/>
<swiper layout-type="stackLeft" indicator-type="normal"/>
<swiper layout-type="normal" indicator-type="worm"/>
<swiper layout-type="transformer" indicator-type="slide"/>"#,
            ),
            &ComponentAttribute,
        );
        let hits: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.message.contains("`indicator-type`"))
            .collect();
        assert_eq!(hits.len(), 3, "{diagnostics:#?}");
        assert!(hits[0].message.contains(r#"layout-type="stackLeft""#));
        assert!(hits[1].message.contains(r#"layout-type="stackRight""#));
        assert!(hits[2].message.contains(r#"layout-type="tinder""#));
        assert_eq!((hits[2].line, hits[2].column), (3, 30));
    }

    #[test]
    fn requires_circular_conflicts_with_indicator_animations() {
        let diagnostics = check_rule(
            &project(
                "attr-requires-circular",
                r#"<swiper indicator-type="swap" circular/>
<swiper indicator-type="swap" circular="true"/>
<swiper indicator-type="scrollFixedCenter" circular="{{true}}"/>
<swiper indicator-type="swapYRotation" circular="{{circular}}"/>
<swiper indicator-type="normal" circular/>
<swiper indicator-type="swap"/>
<swiper circular/>"#,
            ),
            &ComponentAttribute,
        );
        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
        // 无值属性等于布尔 true，同样要触发。
        assert!(diagnostics[0].message.contains("<swiper circular>"));
        assert!(diagnostics[0].message.contains(r#"indicator-type="swap""#));
        assert!(diagnostics[1].message.contains(r#"circular="true""#));
        assert!(diagnostics[0].help.as_deref().unwrap().contains("circular"));
    }

    #[test]
    fn requires_is_skipped_for_dynamic_values() {
        let diagnostics = check_rule(
            &project(
                "attr-requires-dynamic",
                r#"<swiper indicator-type="{{type}}" circular/>
<swiper layout-type="{{layout}}" transformer-type="threeD"/>
<swiper layout-type="{{layout}}" indicator-type="worm"/>"#,
            ),
            &ComponentAttribute,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn native_view_only_accepts_scroll_view_and_swiper() {
        let diagnostics = check_rule(
            &project(
                "attr-native-view",
                r#"<vertical-drag-gesture-handler native-view="view"/>
<vertical-drag-gesture-handler native-view="scroll-view"/>
<vertical-drag-gesture-handler native-view="SWIPER"/>
<horizontal-drag-gesture-handler native-view="swiper"/>
<pan-gesture-handler native-view="view"/>
<view native-view="view"/>"#,
            ),
            &ComponentAttribute,
        );
        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("native-view"));
        assert!(diagnostics[1].message.contains("pan-gesture-handler"));
        assert!(
            diagnostics[0]
                .help
                .as_deref()
                .unwrap()
                .contains("scroll-view")
        );
    }

    #[test]
    fn input_webview_only_attributes_are_unsupported() {
        let diagnostics = check_rule(
            &project(
                "attr-input-webview",
                r#"<input type="text"/><input placeholder-class="ph"/><input safe-password-nonce="n"/><input safe-password-length="6"/>"#,
            ),
            &ComponentAttribute,
        );
        assert_eq!(diagnostics.len(), 3, "{diagnostics:#?}");
        assert!(diagnostics.iter().all(|d| d.message.contains("不支持")));
        assert!(diagnostics[0].message.contains("placeholder-class"));
        assert!(diagnostics[1].message.contains("safe-password-nonce"));
        assert!(diagnostics[2].message.contains("safe-password-length"));
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

        let err =
            parse("[requires]\nswiper | transformer-type | layout-type=transformer | -  # x\n")
                .unwrap_err();
        assert!(err.contains("属性=取值"), "{err}");

        let err =
            parse("[requires]\nswiper | circular=* | indicator-type=* | -  # x\n").unwrap_err();
        assert!(err.contains("要求列不支持"), "{err}");
    }
}
