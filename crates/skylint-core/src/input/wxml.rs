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

use glass_easel_template_compiler::parse::tag::{
    Attribute, ClassAttribute, Element, ElementKind, EventBinding, Ident, Node, NormalAttribute,
    NormalAttributePrefix, StaticAttribute, StyleAttribute, Value,
};
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
    /// 整个元素（含子节点）的范围。
    pub start: Position,
    pub end: Position,
    /// 开始标签里标签名的范围，用于把诊断落在标签名上。
    pub tag_start: Position,
    pub tag_end: Position,
    pub attributes: Vec<WxmlAttribute>,
    pub children: Vec<WxmlNode>,
}

/// 元素上的一个属性。
///
/// `value` 是解析后的静态字符串（含实体解码）；动态绑定（`{{}}`，或
/// `a{{b}}c` 这类混排）不提供字符串值，`is_static` 为 `false`，规则据此跳过。
/// 没有值的属性（如 `disabled`）`value` 为 `None`，但 `is_static` 仍为 `true`。
#[derive(Debug, Clone)]
pub struct WxmlAttribute {
    /// 属性原名，保留连字符与冒号前缀：`aria-role`、`bind:tap`、`catch:tap`。
    pub name: String,
    pub value: Option<String>,
    pub is_static: bool,
    /// 属性名（含前缀）到值结尾的范围。
    pub start: Position,
    pub end: Position,
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
            tag_start: tag_name.location.start,
            tag_end: tag_name.location.end,
            attributes: attributes(element, source),
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
            tag_start: element.tag_location.start.0.start,
            tag_end: element.tag_location.start.1.end,
            attributes: attributes(element, source),
            children: Vec::new(),
        })],
        _ => Vec::new(),
    }
}

/// 元素属性。只收集「用户写在开始标签里」的属性：`wx:for`/`wx:if` 被解析成
/// 包装节点，不在其内；`<block>` 摊平后属性也不会保留。
fn attributes(element: &Element, source: &str) -> Vec<WxmlAttribute> {
    let mut out = Vec::new();
    match &element.kind {
        ElementKind::Normal {
            attributes: normal,
            class,
            style,
            change_attributes,
            worklet_attributes,
            generics,
            extra_attr,
            let_vars,
            common,
            ..
        } => {
            out.extend(normal.iter().map(|attr| normal_attribute(attr, source)));
            out.extend(class_attributes(class, source));
            out.extend(style_attributes(style, source));
            out.extend(
                change_attributes
                    .iter()
                    .map(|attr| prefixed_attribute(attr, source)),
            );
            out.extend(
                worklet_attributes
                    .iter()
                    .map(|attr| static_attribute(attr, source)),
            );
            out.extend(generics.iter().map(|attr| static_attribute(attr, source)));
            out.extend(extra_attr.iter().map(|attr| static_attribute(attr, source)));
            out.extend(let_vars.iter().map(|attr| prefixed_attribute(attr, source)));
            if let Some((range, value)) = &common.id {
                out.push(value_attribute("id".to_string(), range, Some(value)));
            }
            if let Some((range, value)) = &common.slot {
                out.push(value_attribute("slot".to_string(), range, Some(value)));
            }
            out.extend(
                common
                    .slot_value_refs
                    .iter()
                    .map(|attr| static_attribute(attr, source)),
            );
            out.extend(
                common
                    .data
                    .iter()
                    .map(|attr| prefixed_attribute(attr, source)),
            );
            out.extend(
                common
                    .marks
                    .iter()
                    .map(|attr| prefixed_attribute(attr, source)),
            );
            out.extend(
                common
                    .event_bindings
                    .iter()
                    .map(|binding| event_attribute(binding, source)),
            );
        }
        ElementKind::Slot { values, common, .. } => {
            out.extend(values.iter().map(|attr| prefixed_attribute(attr, source)));
            if let Some((range, value)) = &common.id {
                out.push(value_attribute("id".to_string(), range, Some(value)));
            }
            out.extend(
                common
                    .event_bindings
                    .iter()
                    .map(|binding| event_attribute(binding, source)),
            );
        }
        _ => {}
    }
    // 按源码顺序排列，便于规则和调试时阅读。
    out.sort_by_key(|attr| (attr.start.line, attr.start.utf16_col));
    out
}

fn value_attribute(name: String, range: &Range<Position>, value: Option<&Value>) -> WxmlAttribute {
    match value {
        Some(Value::Static {
            value, location, ..
        }) => WxmlAttribute {
            name,
            value: Some(value.to_string()),
            is_static: true,
            start: range.start,
            end: location.end,
        },
        Some(dynamic @ Value::Dynamic { .. }) => WxmlAttribute {
            name,
            value: None,
            is_static: false,
            start: range.start,
            end: dynamic.location().end,
        },
        // 解析器新增的值形态按「动态」处理，规则不会把它当成静态字符串。
        Some(_) => WxmlAttribute {
            name,
            value: None,
            is_static: false,
            start: range.start,
            end: range.end,
        },
        None => WxmlAttribute {
            name,
            value: None,
            is_static: true,
            start: range.start,
            end: range.end,
        },
    }
}

fn normal_attribute(attr: &NormalAttribute, source: &str) -> WxmlAttribute {
    let prefix = match &attr.prefix {
        NormalAttributePrefix::None => None,
        NormalAttributePrefix::Model(range) => Some(range),
    };
    let name = full_name(prefix, &attr.name, source);
    let start = prefix.map_or(attr.name.location.start, |range| range.start);
    value_attribute(name, &(start..attr.name.location.end), attr.value.as_ref())
}

fn prefixed_attribute(attr: &Attribute, source: &str) -> WxmlAttribute {
    let prefix = attr.prefix_location.as_ref();
    let name = full_name(prefix, &attr.name, source);
    let start = prefix.map_or(attr.name.location.start, |range| range.start);
    value_attribute(name, &(start..attr.name.location.end), attr.value.as_ref())
}

fn static_attribute(attr: &StaticAttribute, source: &str) -> WxmlAttribute {
    let prefix = attr.prefix_location.as_ref();
    let name = full_name(prefix, &attr.name, source);
    let start = prefix.map_or(attr.name.location.start, |range| range.start);
    WxmlAttribute {
        name,
        value: Some(attr.value.name.to_string()),
        is_static: true,
        start,
        end: attr.value.location.end,
    }
}

fn event_attribute(binding: &EventBinding, source: &str) -> WxmlAttribute {
    let name = full_name(Some(&binding.prefix_location), &binding.name, source);
    value_attribute(
        name,
        &(binding.prefix_location.start..binding.name.location.end),
        binding.value.as_ref(),
    )
}

fn class_attributes(class: &ClassAttribute, source: &str) -> Vec<WxmlAttribute> {
    match class {
        ClassAttribute::None => Vec::new(),
        ClassAttribute::String(range, value) => {
            vec![value_attribute("class".to_string(), range, Some(value))]
        }
        ClassAttribute::Multiple(items) => items
            .iter()
            .map(|(range, name, value)| {
                let full = full_name(Some(range), name, source);
                let range = range.start..name.location.end;
                value_attribute(full, &range, value.as_ref())
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn style_attributes(style: &StyleAttribute, source: &str) -> Vec<WxmlAttribute> {
    match style {
        StyleAttribute::None => Vec::new(),
        StyleAttribute::String(range, value) => {
            vec![value_attribute("style".to_string(), range, Some(value))]
        }
        StyleAttribute::Multiple(items) => items
            .iter()
            .map(|(range, name, value)| {
                let full = full_name(Some(range), name, source);
                let range = range.start..name.location.end;
                value_attribute(full, &range, Some(value))
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// 带前缀的属性名（`bind:tap`、`model:value`、`mark:foo`）从原文切出，
/// 前缀与名字之间的冒号因此得以保留。
fn full_name(prefix: Option<&Range<Position>>, name: &Ident, source: &str) -> String {
    match prefix {
        Some(prefix) => slice(source, prefix.start..name.location.end),
        None => name.name.to_string(),
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

    fn attributes(doc: &WxmlDoc) -> Vec<&WxmlAttribute> {
        doc.elements().next().unwrap().attributes.iter().collect()
    }

    #[test]
    fn attribute_names_keep_hyphens_and_colons() {
        let doc = WxmlDoc::parse(
            "t.wxml",
            r#"<view aria-role="button" bind:tap="onTap" catch:tap="onCatch" type="list"/>"#,
        )
        .unwrap();
        let names: Vec<&str> = attributes(&doc).iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"aria-role"), "{names:?}");
        assert!(names.contains(&"bind:tap"), "{names:?}");
        assert!(names.contains(&"catch:tap"), "{names:?}");

        let aria = attributes(&doc)
            .into_iter()
            .find(|a| a.name == "aria-role")
            .unwrap();
        assert_eq!(aria.value.as_deref(), Some("button"));
        assert!(aria.is_static);
    }

    #[test]
    fn dynamic_attribute_value_is_not_a_static_string() {
        let doc = WxmlDoc::parse("t.wxml", r#"<view type="{{t}}"/>"#).unwrap();
        let attr = &attributes(&doc)[0];
        assert_eq!(attr.name, "type");
        assert!(!attr.is_static);
        assert_eq!(attr.value, None);
    }

    #[test]
    fn mixed_interpolation_attribute_value_is_not_static() {
        let doc = WxmlDoc::parse("t.wxml", r#"<view type="a{{t}}"/>"#).unwrap();
        assert!(!attributes(&doc)[0].is_static);
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
