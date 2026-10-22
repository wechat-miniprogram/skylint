//! WXSS 语义类规则。
//!
//! 三条 P0 规则都只依赖「单条规则内的原始声明」，不做级联、不解析值、不匹配选择器。
//!
//! 文件划分按「判断形状」而不是按主题：`baseline` 是手写判定的少量规则，
//! `dict` 是纯数据驱动的属性/值支持表，`shape` 是「值的个数/结构」类限制
//! （多值数量、函数参数约束）。三者共用同一个 [`visit`]。

pub mod baseline;
pub mod dict;
pub mod shape;

pub use baseline::{BoxSizing, FlexDirection, NoPseudo};

use std::collections::HashSet;

use crate::Diagnostic;
use crate::input::wxss::{self, WxssDoc};
use crate::project::Project;

/// 遍历所有需要检查的 WXSS 文件：全部页面 + 全部自定义组件。
///
/// 不按 `is_skyline` 过滤：这是**迁移**检查工具，目标用户是还在 WebView 上的项目。
/// 若只查已迁移的页面，纯 WebView 工程会一条都不报，「迁移前拿到问题清单」的用途
/// 就落空了。
///
/// 单文件读失败 / 解析失败 / 非 `.wxss` 源（`.less`、`.scss`）一律跳过，不终止扫描。
pub(crate) fn visit(
    project: &Project,
    mut f: impl FnMut(&WxssDoc, &mut Vec<Diagnostic>),
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let paths = project
        .pages
        .iter()
        .filter_map(|page| page.files.wxss.as_ref())
        .chain(
            project
                .components
                .iter()
                .filter_map(|c| c.files.wxss.as_ref()),
        );

    for rel in paths {
        if !seen.insert(rel.as_str()) {
            continue;
        }
        if !rel.to_ascii_lowercase().ends_with(".wxss") {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(project.abs(rel)) else {
            continue;
        };
        let Ok(doc) = wxss::parse(rel, &source) else {
            continue;
        };
        f(&doc, &mut out);
    }
    out
}
