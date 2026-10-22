//! 属性 / 值支持表（数据驱动）。
//!
//! 两条规则共用 `data/wxss_support.txt`：一条查「属性完全不生效」，一条查
//! 「属性可用但某个整值不支持」。只做整值比较，不展开简写、不解析函数、不做级联。

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::project::Project;
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

use super::visit;

/// 数据文件是唯一依据，Rust 侧只做查表。
const SUPPORT_DATA: &str = include_str!("../data/wxss_support.txt");

#[derive(Debug, Default)]
struct Table {
    properties: HashMap<&'static str, &'static str>,
    /// 属性 → (整值 → 建议)。
    values: HashMap<&'static str, HashMap<&'static str, &'static str>>,
}

impl Table {
    fn property(&self, name: &str) -> Option<&'static str> {
        self.properties.get(name).copied()
    }

    fn value(&self, name: &str, value: &str) -> Option<&'static str> {
        self.values.get(name)?.get(value).copied()
    }
}

fn table() -> &'static Table {
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
fn normalize_value(value: &str) -> Option<String> {
    let lower = value.trim().to_ascii_lowercase();
    if lower.is_empty() || lower.contains("var(") || lower.contains("env(") || lower.contains("{{")
    {
        return None;
    }
    Some(lower)
}

/// Skyline 下完全不生效的属性。
pub struct UnsupportedProperty;

impl UnsupportedProperty {
    pub const ID: &'static str = "skyline/unsupported-property";
}

impl Rule for UnsupportedProperty {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Error,
            description: "声明了 Skyline 完全不支持的属性，该属性不会生效",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let table = table();
        visit(project, |doc, out| {
            for rule in &doc.rules {
                for decl in &rule.declarations {
                    let name = decl.name.to_ascii_lowercase();
                    let Some(advice) = table.property(&name) else {
                        continue;
                    };
                    let (line, column) = doc.position(decl.offset);
                    let (end_line, end_column) = doc.position(decl.offset + decl.name.len());
                    out.push(
                        Diagnostic::new(
                            Self::ID,
                            Severity::Error,
                            format!("`{name}` 属性在 Skyline 下不生效，该声明会被忽略"),
                            doc.path(),
                            line,
                            column,
                        )
                        .end(end_line, end_column)
                        .help(advice),
                    );
                }
            }
        })
    }
}

/// 属性可用，但该**整值**在 Skyline 下不支持，声明会被静默忽略。
pub struct UnsupportedValue;

impl UnsupportedValue {
    pub const ID: &'static str = "skyline/unsupported-value";
}

impl Rule for UnsupportedValue {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Error,
            description: "属性在 Skyline 下可用，但该值不支持，声明会被静默忽略",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let table = table();
        visit(project, |doc, out| {
            for rule in &doc.rules {
                for decl in &rule.declarations {
                    let name = decl.name.to_ascii_lowercase();
                    let Some(value) = normalize_value(&decl.value) else {
                        continue;
                    };
                    let Some(advice) = table.value(&name, &value) else {
                        continue;
                    };
                    let (line, column) = doc.position(decl.offset);
                    let (end_line, end_column) = doc.position(decl.value_end());
                    out.push(
                        Diagnostic::new(
                            Self::ID,
                            Severity::Error,
                            format!(
                                "`{name}: {}` 在 Skyline 下不受支持，该值会被静默忽略",
                                decl.value.trim()
                            ),
                            doc.path(),
                            line,
                            column,
                        )
                        .end(end_line, end_column)
                        .help(advice),
                    );
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("skylint-dict-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            Self { root }
        }

        fn write(&self, rel: &str, content: &str) -> &Self {
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

    #[test]
    fn property_reports_unsupported_properties() {
        let (_f, project) = single(
            "property-positive",
            r#"
            .a { float: left; }
            .b { writing-mode: vertical-rl; }
            .c { text-indent: 2em; }
            .d { mask-clip: border-box; }
            .e { justify-items: center; }
            .f { list-style-type: none; }
            .g { FLOAT: left; }
            "#,
        );
        let diags = UnsupportedProperty.check(&project);
        assert_eq!(diags.len(), 7);
        assert!(diags.iter().all(|d| d.severity == Severity::Error));
        assert!(diags.iter().any(|d| d.message.contains("`float`")));
        assert!(diags.iter().any(|d| d.message.contains("`writing-mode`")));
        assert!(diags.iter().any(|d| d.message.contains("`text-indent`")));
        assert!(diags.iter().any(|d| d.message.contains("`mask-clip`")));
        assert!(diags.iter().any(|d| d.message.contains("`justify-items`")));
        assert_eq!(diags[0].help.as_deref(), Some("使用 flex 布局"));
    }

    #[test]
    fn property_ignores_custom_properties_values_and_prefixed_names() {
        let (_f, project) = single(
            "property-negative",
            r#"
            .a { --float: left; }
            .b { -wx-contain: size; }
            .c { content: "float"; }
            .d { transition-property: float; }
            .e { -webkit-mask-clip: border-box; }
            .f { color: red; }
            .g { --writing-mode: vertical-rl; }
            .h { border-left: 1px solid red; }
            "#,
        );
        assert_eq!(
            messages(&UnsupportedProperty, &project),
            Vec::<String>::new()
        );
    }

    #[test]
    fn property_names_are_matched_exactly_not_by_substring() {
        let (_f, project) = single(
            "property-attribution",
            r#"
            .a { background-clip: padding-box; }
            "#,
        );
        let diags = UnsupportedProperty.check(&project);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("`background-clip`"));
        assert!(!diags[0].message.contains("mask-clip"));
    }

    #[test]
    fn value_reports_unsupported_values() {
        let (_f, project) = single(
            "value-positive",
            r#"
            .a { display: grid; }
            .b { position: sticky; }
            .c { overflow: auto; }
            .d { visibility: collapse; }
            .e { white-space: pre-wrap; }
            .f { font-weight: bolder; }
            .g { border-top-style: double; }
            .h { background-repeat: space; }
            .i { will-change: scroll-position; }
            .j { DISPLAY: GRID !important; }
            "#,
        );
        let diags = UnsupportedValue.check(&project);
        assert_eq!(diags.len(), 10);
        assert!(diags.iter().all(|d| d.severity == Severity::Error));
        assert!(diags.iter().any(|d| d.message.contains("display: grid")));
        assert!(diags.iter().any(|d| d.message.contains("position: sticky")));
        assert!(diags.iter().any(|d| d.message.contains("overflow: auto")));
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("border-top-style: double"))
        );
        assert_eq!(diags[0].help.as_deref(), Some("改用 flex 布局替代 grid"));
    }

    #[test]
    fn value_ignores_supported_values_vars_and_partial_matches() {
        let (_f, project) = single(
            "value-negative",
            r#"
            .a { display: flex; }
            .b { position: relative; }
            .c { overflow: hidden; }
            .d { display: var(--d); }
            .e { margin: 0 auto; }
            .f { background: url(grid.png); }
            .g { overflow: auto hidden; }
            .h { content: "pre-wrap"; }
            .i { display: env(display); }
            .j { width: calc(100% - 1px); }
            .k { text-decoration-line: underline line-through; }
            .l { --grid: 1; }
            "#,
        );
        assert_eq!(messages(&UnsupportedValue, &project), Vec::<String>::new());
    }
}
