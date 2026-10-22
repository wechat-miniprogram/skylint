//! WXML 结构类规则。
//!
//! `baseline` 是逐元素的局部判定，`structure` 是父子结构约束
//! （scroll-view 家族、list-view/grid-view 的挂载位置等）。两者共用同一套
//! 「页面 + 组件 WXML 遍历」与解析辅助。

pub mod baseline;
pub mod structure;

pub use baseline::{NoInlineText, ScrollViewType, UnsupportedComponent};
pub use structure::{HorizontalScrollEnableFlex, ScrollViewFamily};

use std::collections::HashSet;
use std::fs;

use crate::input::wxml::{self, WxmlDoc};
use crate::project::Project;

/// 页面与组件的 WXML，按路径去重（同一文件可能被多次引用）。
fn wxml_files(project: &Project) -> impl Iterator<Item = &str> {
    let mut seen = HashSet::new();
    project
        .pages
        .iter()
        .filter_map(|page| page.files.wxml.as_deref())
        .chain(
            project
                .components
                .iter()
                .filter_map(|component| component.files.wxml.as_deref()),
        )
        .filter(move |path| seen.insert(*path))
}

fn load_doc(project: &Project, path: &str) -> Option<WxmlDoc> {
    let source = fs::read_to_string(project.abs(path)).ok()?;
    wxml::parse(path, &source).ok()
}
