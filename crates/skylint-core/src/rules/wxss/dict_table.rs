//! 属性 / 值支持表解析（两条 dict 规则共用）。
//!
//! 数据文件是唯一依据，Rust 侧只做查表。

use std::collections::HashMap;
use std::sync::OnceLock;

const SUPPORT_DATA: &str = include_str!("../data/wxss_support.txt");

#[derive(Debug, Default)]
pub(crate) struct Table {
    properties: HashMap<&'static str, &'static str>,
    /// 属性 → (整值 → 建议)。
    values: HashMap<&'static str, HashMap<&'static str, &'static str>>,
}

impl Table {
    pub(crate) fn property(&self, name: &str) -> Option<&'static str> {
        self.properties.get(name).copied()
    }

    pub(crate) fn value(&self, name: &str, value: &str) -> Option<&'static str> {
        self.values.get(name)?.get(value).copied()
    }
}

pub(crate) fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| parse(SUPPORT_DATA).expect("WXSS 支持表格式错误"))
}

#[derive(Clone, Copy)]
enum Section {
    Property,
    Value,
}

/// 解析数据文件。格式错误一律返回带行号的说明，便于同步时定位。
fn parse(source: &'static str) -> Result<Table, String> {
    let mut table = Table::default();
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
                "unsupported-property" => Section::Property,
                "unsupported-value" => Section::Value,
                other => return Err(format!("第 {line_no} 行：未知分区 [{other}]")),
            });
            continue;
        }
        let section = section.ok_or_else(|| format!("第 {line_no} 行：数据出现在分区之前"))?;
        let fields: Vec<&'static str> = line.split('|').map(str::trim).collect();
        if fields.len() != 3 {
            return Err(format!(
                "第 {line_no} 行：需要 `键 | 值 | 建议` 三列，实际 {} 列",
                fields.len()
            ));
        }
        let (key, value, advice) = (fields[0], fields[1], fields[2]);
        if key.is_empty() || advice.is_empty() {
            return Err(format!("第 {line_no} 行：键与建议不能为空"));
        }
        if !is_lowercase(key) {
            return Err(format!("第 {line_no} 行：键必须小写：{key}"));
        }
        match section {
            Section::Property => {
                if value != "-" {
                    return Err(format!("第 {line_no} 行：属性分区的值列固定为 `-`"));
                }
                if table.properties.insert(key, advice).is_some() {
                    return Err(format!("第 {line_no} 行：重复的属性 {key}"));
                }
            }
            Section::Value => {
                if value.is_empty() || !is_lowercase(value) {
                    return Err(format!("第 {line_no} 行：值必须非空且小写：{value}"));
                }
                for name in key.split(',') {
                    let name = name.trim();
                    if name.is_empty() || !is_lowercase(name) {
                        return Err(format!("第 {line_no} 行：属性名非法：{name}"));
                    }
                    let by_value = table.values.entry(name).or_default();
                    if by_value.insert(value, advice).is_some() {
                        return Err(format!("第 {line_no} 行：重复的条目 {name}: {value}"));
                    }
                }
            }
        }
    }

    Ok(table)
}

fn is_lowercase(text: &str) -> bool {
    !text.bytes().any(|b| b.is_ascii_uppercase())
}

/// 值归一化：`trim` + 小写；含运行时变量/插值时返回 `None`（无法静态判定）。
pub(crate) fn normalize_value(value: &str) -> Option<String> {
    let lower = value.trim().to_ascii_lowercase();
    if lower.is_empty() || lower.contains("var(") || lower.contains("env(") || lower.contains("{{")
    {
        return None;
    }
    Some(lower)
}

#[cfg(test)]
mod tests {
    use super::{SUPPORT_DATA, parse};

    #[test]
    fn data_file_parses_and_contains_known_entries() {
        let table = parse(SUPPORT_DATA).unwrap();
        assert_eq!(table.property("float"), Some("使用 flex 布局"));
        assert!(table.property("mask-clip").is_some());
        assert_eq!(
            table.value("display", "grid"),
            Some("改用 flex 布局替代 grid")
        );
        assert!(table.value("border-top-style", "double").is_some());
        assert!(table.value("border-left-style", "outset").is_some());
        assert!(table.value("display", "flex").is_none());
    }

    #[test]
    fn rejects_lines_with_missing_columns() {
        let err = parse("[unsupported-property]\nfloat | 使用 flex 布局\n").unwrap_err();
        assert!(err.contains("三列"), "{err}");

        let err = parse("[unsupported-value]\ndisplay | grid\n").unwrap_err();
        assert!(err.contains("三列"), "{err}");
    }

    #[test]
    fn rejects_unknown_section_and_bad_property_row() {
        let err = parse("[nope]\nfloat | - | x\n").unwrap_err();
        assert!(err.contains("未知分区"), "{err}");

        let err = parse("[unsupported-property]\nfloat | left | x\n").unwrap_err();
        assert!(err.contains("值列固定为"), "{err}");

        let err = parse("[unsupported-value]\nDisplay | grid | x\n").unwrap_err();
        assert!(err.contains("必须小写"), "{err}");
    }
}
