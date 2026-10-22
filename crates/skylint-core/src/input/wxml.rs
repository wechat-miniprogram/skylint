//! WXML 视图。
//!
//! 只做「解析 + 定位」：把 glass-easel 的模板 AST 摊平成规则能直接遍历的树，
//! 并把它的 `(line, utf16_col)` 位置换算成诊断使用的「1-based 行 + 按 Unicode
//! 字符计的列」。
//!
//! 结构包装节点（`<block>`、`wx:for`、`wx:if`）在渲染上不产生自己的盒子，因此
//! 这里把它们直接摊平进父节点的子节点列表——规则看到的父子关系与渲染树一致，
//! `<block wx:for>` 里的裸文本会落在真正包含它的元素之下。

use std::ops::Range;

use glass_easel_template_compiler::parse::tag::{Element, ElementKind, Node, Value};
use glass_easel_template_compiler::parse::{self, ParseErrorLevel, Position, TemplateStructure};

/// 解析一个 WXML 文件。
pub fn parse(path: &str, source: &str) -> Result<WxmlDoc, String> {
    WxmlDoc::parse(path, source)
}

/// 一个 WXML 文件的解析结果。
#[derive(Debug)]
pub struct WxmlDoc {
    source: String,
    pub nodes: Vec<WxmlNode>,
}

/// 摊平后的节点。`Comment` 不携带内容：规则不需要，保留它是为了不把注释误当元素。
#[derive(Debug, Clone)]
pub enum WxmlNode {
    Element(WxmlElement),
    Text(WxmlText),
    Comment,
}

#[derive(Debug, Clone)]
pub struct WxmlElement {
    /// 标签名，小写（与 WXML 解析器一致）。
    pub tag: String,
    pub start: Position,
    pub end: Position,
    pub children: Vec<WxmlNode>,
}

#[derive(Debug, Clone)]
pub struct WxmlText {
    /// 文本内容；静态文本是解码后的值，插值是原文（含 `{{}}`）。
    pub text: String,
    pub start: Position,
    pub end: Position,
}

impl WxmlText {
    /// 空白文本不算「有意义的文本」。
    pub fn is_meaningful(&self) -> bool {
        !self.text.trim().is_empty()
    }
}

impl WxmlDoc {
    /// `path` 只用于错误消息，不需要是磁盘上的真实路径。
    pub fn parse(path: &str, source: &str) -> Result<Self, String> {
        let (template, state) = parse::parse(path, source);
        if let Some(err) = state
            .warnings()
            .find(|e| e.level() >= ParseErrorLevel::Fatal)
        {
            let start = err.location.start;
            return Err(format!(
                "{}:{}:{}: {}",
                path,
                start.line + 1,
                column_at(source, start),
                err.kind
            ));
        }
        Ok(Self {
            source: source.to_string(),
            nodes: flatten(&template.content, source),
        })
    }

    /// `parse::Position`（0-based 行、UTF-16 列）→ 诊断位置（1-based 行、字符列）。
    pub fn position(&self, pos: Position) -> (u32, u32) {
        (pos.line + 1, column_at(&self.source, pos))
    }

    /// 深度优先遍历全部元素节点。
    pub fn elements(&self) -> impl Iterator<Item = &WxmlElement> {
        let mut stack: Vec<&WxmlNode> = self.nodes.iter().rev().collect();
        std::iter::from_fn(move || {
            while let Some(node) = stack.pop() {
                if let WxmlNode::Element(element) = node {
                    stack.extend(element.children.iter().rev());
                    return Some(element);
                }
            }
            None
        })
    }
}

/// 1-based 字符列。`utf16_col` 落在代理对中间时按该字符计入，不会越界。
fn column_at(source: &str, pos: Position) -> u32 {
    let line = source.split('\n').nth(pos.line as usize).unwrap_or("");
    let mut utf16 = 0u32;
    let mut chars = 0u32;
    for ch in line.chars() {
        if utf16 >= pos.utf16_col {
            break;
        }
        utf16 += ch.len_utf16() as u32;
        chars += 1;
    }
    chars + 1
}

/// 位置 → 字节偏移，用于从原文切出插值文本。
fn byte_offset(source: &str, pos: Position) -> Option<usize> {
    let mut offset = 0;
    for (index, line) in source.split('\n').enumerate() {
        if index as u32 == pos.line {
            let mut utf16 = 0u32;
            for (bytes, ch) in line.char_indices() {
                if utf16 >= pos.utf16_col {
                    return Some(offset + bytes);
                }
                utf16 += ch.len_utf16() as u32;
            }
            return Some(offset + line.len());
        }
        offset += line.len() + 1;
    }
    None
}

fn text_node(value: &Value, source: &str) -> WxmlText {
    let range = value.location();
    if let Value::Static { value: raw, .. } = value {
        // 解析器把文本前的空白也包进了 Value，这里裁掉并把起点前移，
        // 让诊断落在真正的第一个字符上。
        let trimmed = raw.trim();
        let lead = &raw[..raw.len() - raw.trim_start().len()];
        let start = advance(range.start, lead);
        return WxmlText {
            text: trimmed.to_string(),
            start,
            end: advance(start, trimmed),
        };
    }
    WxmlText {
        text: slice(source, range.clone()),
        start: range.start,
        end: range.end,
    }
}

fn advance(mut pos: Position, text: &str) -> Position {
    for ch in text.chars() {
        if ch == '\n' {
            pos.line += 1;
            pos.utf16_col = 0;
        } else {
            pos.utf16_col += ch.len_utf16() as u32;
        }
    }
    pos
}

fn slice(source: &str, range: Range<Position>) -> String {
    let (Some(start), Some(end)) = (
        byte_offset(source, range.start),
        byte_offset(source, range.end),
    ) else {
        return String::new();
    };
    source.get(start..end).unwrap_or("").to_string()
}

/// 包装节点摊平；元素保留。
fn flatten(nodes: &[Node], source: &str) -> Vec<WxmlNode> {
    nodes
        .iter()
        .flat_map(|node| node_nodes(node, source))
        .collect()
}

fn node_nodes(node: &Node, source: &str) -> Vec<WxmlNode> {
    match node {
        Node::Text(value) => vec![WxmlNode::Text(text_node(value, source))],
        Node::Element(element) => element_nodes(element, source),
        Node::Comment(_) => vec![WxmlNode::Comment],
        _ => Vec::new(),
    }
}

fn element_nodes(element: &Element, source: &str) -> Vec<WxmlNode> {
    match &element.kind {
        ElementKind::Normal {
            tag_name, children, ..
        } => vec![WxmlNode::Element(WxmlElement {
            tag: tag_name.name.to_string(),
            start: element.location().start,
            end: element.location().end,
            children: flatten(children, source),
        })],
        ElementKind::Pure { children, .. } | ElementKind::For { children, .. } => {
            flatten(children, source)
        }
        ElementKind::If {
            branches,
            else_branch,
            ..
        } => branches
            .iter()
            .flat_map(|(_, _, children)| flatten(children, source))
            .chain(
                else_branch
                    .iter()
                    .flat_map(|(_, children)| flatten(children, source)),
            )
            .collect(),
        ElementKind::Slot { .. } => vec![WxmlNode::Element(WxmlElement {
            tag: "slot".to_string(),
            start: element.location().start,
            end: element.location().end,
            children: Vec::new(),
        })],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_nodes(doc: &WxmlDoc) -> Vec<&WxmlText> {
        fn walk<'a>(nodes: &'a [WxmlNode], out: &mut Vec<&'a WxmlText>) {
            for node in nodes {
                match node {
                    WxmlNode::Text(text) => out.push(text),
                    WxmlNode::Element(element) => walk(&element.children, out),
                    WxmlNode::Comment => {}
                }
            }
        }
        let mut out = Vec::new();
        walk(&doc.nodes, &mut out);
        out
    }

    #[test]
    fn ascii_columns_are_one_based() {
        let doc = WxmlDoc::parse("t.wxml", "<view>hi</view>").unwrap();
        let text = text_nodes(&doc)[0];
        assert_eq!(doc.position(text.start), (1, 7));
        assert_eq!(doc.position(text.end), (1, 9));
    }

    #[test]
    fn emoji_columns_count_characters_not_utf16_units() {
        // `<view>` = 6 个 UTF-16 单元；`😀` 占 2 个单元但只算 1 列。
        let doc = WxmlDoc::parse("t.wxml", "<view>😀hi</view>").unwrap();
        let text = text_nodes(&doc)[0];
        assert_eq!(doc.position(text.start), (1, 7));
        assert_eq!(
            doc.position(Position {
                line: 0,
                utf16_col: 8
            }),
            (1, 8)
        );
        assert_eq!(doc.position(text.end), (1, 10));
    }

    #[test]
    fn chinese_columns_are_character_based() {
        let doc = WxmlDoc::parse("t.wxml", "<view>中文</view>").unwrap();
        let text = text_nodes(&doc)[0];
        assert_eq!(doc.position(text.start), (1, 7));
        assert_eq!(doc.position(text.end), (1, 9));
    }

    #[test]
    fn multiline_positions() {
        let doc = WxmlDoc::parse("t.wxml", "<view>\n  <text>a</text>\n</view>").unwrap();
        let text = text_nodes(&doc)[0];
        assert_eq!(doc.position(text.start), (2, 9));
    }

    #[test]
    fn interpolation_keeps_source_text() {
        let doc = WxmlDoc::parse("t.wxml", "<view>{{ a }}</view>").unwrap();
        let text = text_nodes(&doc)[0];
        assert_eq!(text.text, "{{ a }}");
        assert!(text.is_meaningful());
    }

    #[test]
    fn fatal_parse_error_is_reported_with_position() {
        let err = WxmlDoc::parse("t.wxml", "<view>{{ a</view>").unwrap_err();
        assert!(err.starts_with("t.wxml:1:"), "{err}");
    }

    #[test]
    fn non_fatal_warnings_do_not_fail() {
        // 缺少结束标签只是 Warn。
        assert!(WxmlDoc::parse("t.wxml", "<view>").is_ok());
    }

    #[test]
    fn structural_wrappers_are_transparent() {
        let doc = WxmlDoc::parse(
            "t.wxml",
            "<block wx:for=\"{{list}}\"><view>{{item}}</view></block>",
        )
        .unwrap();
        let tags: Vec<&str> = doc.elements().map(|e| e.tag.as_str()).collect();
        assert_eq!(tags, ["view"]);
        assert_eq!(text_nodes(&doc)[0].text, "{{item}}");
    }
}
