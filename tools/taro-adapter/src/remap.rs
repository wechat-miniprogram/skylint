//! 诊断重定位：合成文件的行列 → 源文件（TSX / dist 配置文件）的行列。
//!
//! 一个合成单元可能混着多个源文件——页面自己 + 被内联进来的组件，
//! 所以每个锚点都带着自己的文件下标。

use skylint::Diagnostic;

use crate::jsx::SourceFile;
use crate::synth::Synth;

pub fn remap(mut diagnostics: Vec<Diagnostic>, synth: &Synth) -> Vec<Diagnostic> {
    for diag in &mut diagnostics {
        if let Some(map) = synth.maps.get(&diag.path) {
            let start = line_col_to_offset(&map.text, diag.line, diag.column);
            let Some(start) = anchor(&map.spans, &map.sources, start) else {
                continue;
            };
            let (line, column) = offset_to_line_col(start.text, start.offset);
            let end = line_col_to_offset(&map.text, diag.end_line, diag.end_column);
            let end_offset = match anchor(&map.spans, &map.sources, end) {
                Some(end) if end.file == start.file => end.offset.max(start.offset),
                _ => start.offset,
            };
            let (end_line, end_column) = offset_to_line_col(start.text, end_offset);
            diag.path = start.path.to_string();
            diag.line = line;
            diag.column = column;
            diag.end_line = end_line.max(line);
            diag.end_column = end_column;
        } else if let Some(original) = synth.copied.get(&diag.path) {
            diag.path = original.clone();
        }
    }
    diagnostics.sort_by(|a, b| {
        (&a.path, a.line, a.column, &a.rule).cmp(&(&b.path, b.line, b.column, &b.rule))
    });
    diagnostics
}

struct Position<'a> {
    file: u32,
    offset: u32,
    path: &'a str,
    text: &'a str,
}

/// 最近的锚点（元素 `<`、属性名、文本起点）。诊断只会落在这些位置上，
/// 因此不做「锚点 + 偏移」的推算——转义与多字节字符会让差值失真。
fn anchor<'a>(spans: &[(u32, u32, u32)], sources: &'a [SourceFile], offset: u32) -> Option<Position<'a>> {
    let index = spans.partition_point(|(synth, ..)| *synth <= offset).max(1) - 1;
    let (_, file, source) = *spans.get(index)?;
    let entry = sources.get(file as usize)?;
    Some(Position {
        file,
        offset: source,
        path: &entry.path,
        text: &entry.text,
    })
}

fn line_col_to_offset(text: &str, line: u32, column: u32) -> u32 {
    let (mut cur_line, mut cur_col) = (1u32, 1u32);
    if line <= 1 && column <= 1 {
        return 0;
    }
    for (offset, ch) in text.char_indices() {
        if cur_line == line && cur_col == column {
            return offset as u32;
        }
        if ch == '\n' {
            cur_line += 1;
            cur_col = 1;
        } else {
            cur_col += 1;
        }
    }
    text.len() as u32
}

fn offset_to_line_col(text: &str, target: u32) -> (u32, u32) {
    let (mut line, mut column) = (1u32, 1u32);
    for (offset, ch) in text.char_indices() {
        if offset as u32 >= target {
            break;
        }
        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line, column)
}
