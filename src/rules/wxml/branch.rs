//! 条件分支的互斥判定与「能同时渲染的子节点数」。
//!
//! `input/wxml.rs` 把 `wx:if` / `wx:elif` / `wx:else` 摊平成同一层的兄弟，并在每个
//! 元素上留下分支栈（`WxmlElement::branches`）。结构类规则要区分「真的两个兄弟」
//! 与「互斥分支里的两个候选」，口径必须一致，否则同一个写法会在不同规则里得出
//! 相反结论。

use crate::input::wxml::{WxmlElement, WxmlNode};

/// 两个元素是否分属同一个 `wx:if` 的不同分支：互斥，运行时不会同时渲染。
///
/// 只要有一层 `wx:if` 把两者分到不同分支就不可能同时渲染；嵌套时靠分支栈逐层
/// 比对，任一层互斥即互斥。
pub(crate) fn exclusive(a: &WxmlElement, b: &WxmlElement) -> bool {
    a.branches.iter().any(|left| {
        b.branches
            .iter()
            .any(|right| left.group == right.group && left.arm != right.arm)
    })
}

pub(crate) fn element_children(element: &WxmlElement) -> impl Iterator<Item = &WxmlElement> {
    element.children.iter().filter_map(|node| match node {
        WxmlNode::Element(element) => Some(element),
        _ => None,
    })
}

/// 能同时渲染的直接子元素数量的**最大值**。
///
/// 同一个 `wx:if` 的不同分支互斥，取各分支里最多的那一支；不同分支组之间可以同时
/// 渲染，直接相加。因此 `if` 一个子节点 + `else` 两个子节点算 2，而不是按摊平结果
/// 数出的 3，也不是贪心保留的 1。
pub(crate) fn max_rendered_children(element: &WxmlElement) -> usize {
    count_rendered(&element_children(element).collect::<Vec<_>>(), 0)
}

fn count_rendered(children: &[&WxmlElement], depth: usize) -> usize {
    let mut total = 0;
    let mut groups: Vec<(u32, Vec<&WxmlElement>)> = Vec::new();
    for &child in children {
        match child.branches.get(depth) {
            None => total += 1,
            Some(branch) => match groups.iter_mut().find(|(group, _)| *group == branch.group) {
                Some((_, bucket)) => bucket.push(child),
                None => groups.push((branch.group, vec![child])),
            },
        }
    }
    for (_, bucket) in groups {
        let mut arms: Vec<(u32, Vec<&WxmlElement>)> = Vec::new();
        for child in bucket {
            let arm = child.branches[depth].arm;
            match arms.iter_mut().find(|(value, _)| *value == arm) {
                Some((_, list)) => list.push(child),
                None => arms.push((arm, vec![child])),
            }
        }
        total += arms
            .into_iter()
            .map(|(_, list)| count_rendered(&list, depth + 1))
            .max()
            .unwrap_or(0);
    }
    total
}
