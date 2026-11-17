//! 条件分支的互斥判定与「能同时渲染的子节点数」。
//!
//! `input/wxml.rs` 把 `wx:if` / `wx:elif` / `wx:else` 摊平成同一层的兄弟，并在每个
//! 元素上留下分支栈（`WxmlElement::branches`）。结构类规则要区分「真的两个兄弟」
//! 与「互斥分支里的两个候选」，口径必须一致，否则同一个写法会在不同规则里得出
//! 相反结论。

use crate::input::wxml::{Branch, WxmlElement, WxmlNode, WxmlText};

/// 两个元素是否分属同一个 `wx:if` 的不同分支：互斥，运行时不会同时渲染。
///
/// 只要有一层 `wx:if` 把两者分到不同分支就不可能同时渲染；嵌套时靠分支栈逐层
/// 比对，任一层互斥即互斥。
///
/// 除同一条 `wx:if` 链之外，还认「同字段不同字面量」的写法：`wx:if="{{x===1}}"`
/// 与另一个兄弟 `wx:if="{{x===2}}"` 是两条独立的链，但 `x` 只能取一个值。
pub(crate) fn exclusive(a: &WxmlElement, b: &WxmlElement) -> bool {
    exclusive_branches(&a.branches, &b.branches)
}

/// 同上，但直接比分支栈——`WxmlText` 与 `WxmlElement` 都能用。
pub(crate) fn exclusive_branches(left: &[Branch], right: &[Branch]) -> bool {
    left.iter()
        .any(|a| right.iter().any(|b| arms_exclusive(a, b)))
}

/// 两个分支为什么互斥：同一个 `wx:if` 链的不同分支，或同一字段的字面量比较互斥。
fn arms_exclusive(a: &Branch, b: &Branch) -> bool {
    if a.group == b.group {
        return a.arm != b.arm;
    }
    match (&a.compare, &b.compare) {
        (Some(left), Some(right)) => left.excludes(right),
        _ => false,
    }
}

pub(crate) fn element_children(element: &WxmlElement) -> impl Iterator<Item = &WxmlElement> {
    element.children.iter().filter_map(|node| match node {
        WxmlNode::Element(element) => Some(element),
        _ => None,
    })
}

/// 参与「能同时渲染的直接子节点数」计数的节点：元素与有内容的文本。
///
/// 文本在运行时也是 `TextShadowNode`，真实占据子节点列表；空白文本只是 WXML
/// 缩进，不产生节点，因此不计。两者都带条件分支栈，按同一套互斥规则分组。
#[derive(Clone, Copy)]
enum RenderedChild<'a> {
    Element(&'a WxmlElement),
    Text(&'a WxmlText),
}

impl<'a> RenderedChild<'a> {
    fn branches(self) -> &'a [Branch] {
        match self {
            Self::Element(element) => &element.branches,
            Self::Text(text) => &text.branches,
        }
    }
}

fn rendered_children(element: &WxmlElement) -> impl Iterator<Item = RenderedChild<'_>> {
    element.children.iter().filter_map(|node| match node {
        WxmlNode::Element(element) => Some(RenderedChild::Element(element)),
        WxmlNode::Text(text) if text.is_meaningful() => Some(RenderedChild::Text(text)),
        _ => None,
    })
}

/// 能同时渲染的直接子元素数量的**最大值**（只数元素）。
pub(crate) fn max_rendered_children(element: &WxmlElement) -> usize {
    count_rendered(
        &element_children(element)
            .map(RenderedChild::Element)
            .collect::<Vec<_>>(),
        0,
    )
}

/// 同上，但有内容的文本子节点也各算一个。
///
/// `<tap-gesture-handler>点我<view/></tap-gesture-handler>` 在运行时有两个直接子节点
/// （文本 + 元素），`Exactly(1)` 与 `RendersFirst` 都必须按 2 判。
pub(crate) fn max_rendered_nodes(element: &WxmlElement) -> usize {
    count_rendered(&rendered_children(element).collect::<Vec<_>>(), 0)
}

/// 同一个 `wx:if` 的不同分支互斥，取各分支里最多的那一支；不同分支组之间可以同时
/// 渲染，直接相加。因此 `if` 一个子节点 + `else` 两个子节点算 2，而不是按摊平结果
/// 数出的 3，也不是贪心保留的 1。
fn count_rendered(children: &[RenderedChild<'_>], depth: usize) -> usize {
    let mut total = 0;
    let mut groups: Vec<(u32, Vec<RenderedChild<'_>>)> = Vec::new();
    for &child in children {
        match child.branches().get(depth) {
            None => total += 1,
            Some(branch) => match groups.iter_mut().find(|(group, _)| *group == branch.group) {
                Some((_, bucket)) => bucket.push(child),
                None => groups.push((branch.group, vec![child])),
            },
        }
    }
    for (_, bucket) in groups {
        let mut arms: Vec<(u32, Vec<RenderedChild<'_>>)> = Vec::new();
        for child in bucket {
            let arm = child.branches()[depth].arm;
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

#[cfg(test)]
mod tests {
    use crate::input::wxml::{WxmlDoc, WxmlElement, WxmlNode, parse};

    fn element<'a>(doc: &'a WxmlDoc, tag: &str) -> &'a WxmlElement {
        doc.elements().find(|element| element.tag == tag).unwrap()
    }

    fn fixture(source: &str) -> WxmlDoc {
        parse("t.wxml", source, &crate::input::env::Env::default()).unwrap()
    }

    /// 元素计数不受文本影响：现有调用方（互斥分组）口径不变。
    #[test]
    fn element_count_ignores_text() {
        let doc = fixture("<tap-gesture-handler>点我<view/></tap-gesture-handler>");
        let handler = element(&doc, "tap-gesture-handler");
        assert_eq!(super::max_rendered_children(handler), 1);
        assert_eq!(super::max_rendered_nodes(handler), 2);
    }

    /// 空白文本只是缩进，不算子节点。
    #[test]
    fn whitespace_text_is_not_counted() {
        let doc = fixture("<sticky-header>\n  <view/>\n</sticky-header>");
        assert_eq!(super::max_rendered_nodes(element(&doc, "sticky-header")), 1);
    }

    /// 文本同样带分支栈：互斥分支里的文本取最多的一支，不叠加。
    #[test]
    fn text_in_mutually_exclusive_branches_counts_once() {
        let doc = fixture(
            "<tap-gesture-handler><block wx:if=\"{{a}}\">点我</block><block wx:else><view/></block></tap-gesture-handler>",
        );
        assert_eq!(
            super::max_rendered_nodes(element(&doc, "tap-gesture-handler")),
            1
        );
    }

    /// 顺带确认解析层真的把文本挂在父元素下（计数口径的前提）。
    #[test]
    fn text_children_are_visible_in_the_parse_tree() {
        let doc = fixture("<view>文字</view>");
        let view = element(&doc, "view");
        assert!(
            view.children
                .iter()
                .any(|node| matches!(node, WxmlNode::Text(text) if text.is_meaningful()))
        );
    }

    /// 两条独立的 `wx:if`：同字段不同字面量 ⇒ 互斥。
    #[test]
    fn same_field_different_literal_is_exclusive() {
        assert!(pair_after(
            "<view><image wx:if=\"{{code === 1}}\"/><image wx:if=\"{{code === 2}}\"/></view>",
            "image",
        ));
        assert!(pair_after(
            "<view><image wx:if=\"{{code == 'a'}}\"/><image wx:if=\"{{code == 'b'}}\"/></view>",
            "image",
        ));
    }

    /// 字段不同就不能推互斥（两个值可以同时成立）。
    #[test]
    fn different_fields_are_not_exclusive() {
        assert!(!pair_after(
            "<view><image wx:if=\"{{a === 1}}\"/><image wx:if=\"{{b === 2}}\"/></view>",
            "image",
        ));
    }

    /// `===` 与 `!==` 对同一个字面量互补；两条否定式不互补。
    #[test]
    fn negation_complements_the_equality() {
        assert!(pair_after(
            "<view><image wx:if=\"{{code === 1}}\"/><image wx:if=\"{{code !== 1}}\"/></view>",
            "image",
        ));
        assert!(!pair_after(
            "<view><image wx:if=\"{{code !== 1}}\"/><image wx:if=\"{{code !== 2}}\"/></view>",
            "image",
        ));
    }

    /// 字面量相同、函数调用、宽松相等的跨类型比较都判不了互斥。
    #[test]
    fn undecidable_comparisons_are_not_exclusive() {
        assert!(!pair_after(
            "<view><image wx:if=\"{{code === 1}}\"/><image wx:if=\"{{code === 1}}\"/></view>",
            "image",
        ));
        assert!(!pair_after(
            "<view><image wx:if=\"{{f(code) === 1}}\"/><image wx:if=\"{{f(code) === 2}}\"/></view>",
            "image",
        ));
        // 宽松相等下 `1` 与 `'1'` 可能同时成立，不能当互斥。
        assert!(!pair_after(
            "<view><image wx:if=\"{{code == 1}}\"/><image wx:if=\"{{code == '1'}}\"/></view>",
            "image",
        ));
    }

    /// 同一条 `wx:if` 链的互斥不变。
    #[test]
    fn if_else_chain_stays_exclusive() {
        assert!(pair_after(
            "<view><image wx:if=\"{{a}}\"/><image wx:else/></view>",
            "image",
        ));
    }

    fn pair_after(source: &str, tag: &str) -> bool {
        let doc = fixture(source);
        let elements: Vec<&WxmlElement> = doc.elements().filter(|one| one.tag == tag).collect();
        assert_eq!(elements.len(), 2, "{source}");
        super::exclusive(elements[0], elements[1])
    }
}
