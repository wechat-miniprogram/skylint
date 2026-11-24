//! 渲染树 → WXML 文本 + 字节偏移映射。
//!
//! 输出是紧凑格式（子节点之间不换行）：原生 WXML 与 Taro 产物都不含
//! 「格式化空白」，合成树里也不该凭空多出文本节点——`inline-run`
//! 这类规则会把文本节点算进去。
//!
//! 映射只记「合成文件里的字节偏移 → 源文件里的字节偏移」，由 [`crate::remap`]
//! 在诊断产出后换算行列。WXML 解析与规则判定全部走 skylint 现成实现。

use crate::jsx::{AttrValue, Child, Element, SourceFile};

#[derive(Debug, Default)]
pub struct FileMap {
    /// 合成出来的 WXML。
    pub text: String,
    /// `(合成文件字节偏移, 源文件下标, 源文件字节偏移)`，按合成偏移升序。
    ///
    /// 一个合成单元里可能混着多个源文件：页面自己 + 被内联的组件。
    pub spans: Vec<(u32, u32, u32)>,
    /// 下标到源文件的映射。
    pub sources: Vec<SourceFile>,
}

pub fn emit(roots: &[Element], sources: Vec<SourceFile>) -> FileMap {
    let mut text = String::new();
    let mut spans = Vec::new();
    for (index, root) in roots.iter().enumerate() {
        if index > 0 {
            text.push('\n');
        }
        emit_element(root, &mut text, &mut spans);
    }
    FileMap {
        text,
        spans,
        sources,
    }
}

fn emit_element(el: &Element, out: &mut String, spans: &mut Vec<(u32, u32, u32)>) {
    spans.push((out.len() as u32, el.file, el.span));
    out.push('<');
    out.push_str(&el.tag);

    for attr in &el.attrs {
        out.push(' ');
        spans.push((out.len() as u32, attr.file, attr.span));
        out.push_str(&attr.name);
        match &attr.value {
            AttrValue::Bool => {}
            AttrValue::Static(value) => {
                out.push_str("=\"");
                escape_attr(value, out);
                out.push('"');
            }
            AttrValue::Dynamic => out.push_str("=\"{{_}}\""),
        }
    }

    if el.children.is_empty() {
        out.push_str(" />");
        return;
    }
    out.push('>');
    for run in merge_runs(&el.children) {
        match run {
            Run::Element(child) => emit_element(child, out, spans),
            Run::Text {
                text,
                file,
                span,
                dynamic,
            } => {
                spans.push((out.len() as u32, file, span));
                if dynamic {
                    out.push_str("{{_}}");
                } else {
                    escape_text(&text, out);
                }
            }
        }
    }
    out.push_str("</");
    out.push_str(&el.tag);
    out.push('>');
}

enum Run<'a> {
    Element(&'a Element),
    Text {
        text: String,
        file: u32,
        span: u32,
        dynamic: bool,
    },
}

/// 相邻的文本 / 插值合并成一个文本节点。
///
/// JSX 的 `{a}{b}` 在 WXML 里等价于 `{{a}}{{b}}`——一个文本节点里的两个插值；
/// 拆成两个文本节点会让 `inline-run` 把同一段文字当成并列元素。
fn merge_runs(children: &[Child]) -> Vec<Run<'_>> {
    let mut runs: Vec<Run> = Vec::new();
    for child in children {
        match child {
            Child::Element(el) => runs.push(Run::Element(el)),
            Child::Text { text, file, span } => match runs.last_mut() {
                Some(Run::Text {
                    text: last,
                    dynamic,
                    ..
                }) => {
                    last.push_str(text);
                    let _ = dynamic;
                }
                _ => runs.push(Run::Text {
                    text: text.clone(),
                    file: *file,
                    span: *span,
                    dynamic: false,
                }),
            },
            Child::Dynamic { file, span } => match runs.last_mut() {
                Some(Run::Text { dynamic: true, .. }) => {}
                Some(Run::Text { dynamic, .. }) => {
                    *dynamic = true;
                }
                _ => runs.push(Run::Text {
                    text: String::new(),
                    file: *file,
                    span: *span,
                    dynamic: true,
                }),
            },
            Child::Slot => runs.push(Run::Text {
                text: String::new(),
                file: 0,
                span: 0,
                dynamic: true,
            }),
        }
    }
    runs
}

fn escape_attr(value: &str, out: &mut String) {
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
}

fn escape_text(value: &str, out: &mut String) {
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
}
