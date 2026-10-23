//! 数据表「读行骨架」。
//!
//! 只负责把 `include_str!` 进来的表拆成「行号 + 分区 + 列」：
//! 跳过空行与 `#` 注释（含行尾）、维护 `[section]`、按调用方给的切分方式切列并校验列数。
//! 各表自己的语义（Kind / Relation / Constraint 解析、匹配口径、错误文案）仍留在各自的
//! 规则文件里——这里不做规则 DSL，也不认识任何具体列。

use std::collections::HashSet;
use std::hash::Hash;

/// 一条数据行。`fields` 已按调用方的切分方式切好并去掉两端空白。
pub(crate) struct Row<'a> {
    /// 源文件里的 1-based 行号。
    pub(crate) line_no: usize,
    /// 当前分区名；表里没有分区头时为 `None`。
    pub(crate) section: Option<&'a str>,
    pub(crate) fields: Vec<&'a str>,
}

/// 读表结果：分区头与数据行都保留源行号，调用方据此校验分区名、给出定位。
pub(crate) struct Table<'a> {
    /// 出现过的分区头 `(行号, 名称)`，按出现顺序。
    pub(crate) sections: Vec<(usize, &'a str)>,
    pub(crate) rows: Vec<Row<'a>>,
}

/// 期望列数及错误文案里的列说明，例如 `` `键 | 值 | 建议` 三列 ``。
pub(crate) struct Columns {
    pub(crate) count: usize,
    pub(crate) expected: &'static str,
}

impl Columns {
    pub(crate) const fn new(count: usize, expected: &'static str) -> Self {
        Self { count, expected }
    }
}

/// 按固定分隔符切列，并去掉每列两端空白。
pub(crate) fn split_on(delim: char) -> impl for<'a> Fn(Option<&'a str>, &'a str) -> Vec<&'a str> {
    move |_, line| line.split(delim).map(str::trim).collect()
}

/// 整行作为唯一一列（单列表格用）。
pub(crate) fn whole_line<'a>(_: Option<&'a str>, line: &'a str) -> Vec<&'a str> {
    vec![line]
}

/// 读取骨架：跳过空行与 `#` 注释（含行尾），维护 `[section]`，按行切列。
///
/// `split` 拿到当前分区与去掉注释后的整行；`columns` 按分区给出期望列数。列数不符时
/// 返回带行号的错误，其余语义校验由调用方继续做。
pub(crate) fn read<'a>(
    source: &'a str,
    split: impl Fn(Option<&'a str>, &'a str) -> Vec<&'a str>,
    columns: impl Fn(Option<&'a str>) -> Columns,
) -> Result<Table<'a>, String> {
    let mut table = Table {
        sections: Vec::new(),
        rows: Vec::new(),
    };
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
            let name = name.trim();
            section = Some(name);
            table.sections.push((line_no, name));
            continue;
        }
        let expected = columns(section);
        let fields = split(section, line);
        if fields.len() != expected.count {
            return Err(format!(
                "第 {line_no} 行：需要 {}，实际 {} 列",
                expected.expected,
                fields.len()
            ));
        }
        table.rows.push(Row {
            line_no,
            section,
            fields,
        });
    }

    Ok(table)
}

/// 去重辅助：`key` 已出现过时返回带行号的错误。
pub(crate) fn unique<T: Hash + Eq>(
    seen: &mut HashSet<T>,
    key: T,
    line_no: usize,
    message: impl FnOnce() -> String,
) -> Result<(), String> {
    if seen.insert(key) {
        Ok(())
    } else {
        Err(format!("第 {line_no} 行：{}", message()))
    }
}
