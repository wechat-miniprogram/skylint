//! `native-*` 两条规则共用的属性求值：哪些声明真的写了。
//!
//! 组件能力表（[`crate::rules::wxml::tags::CssBox`] / `padding_attr`）在 `wxml::tags`，
//! 这里不再存第二份标签表。
//!
//! 判据：`CssBox::None` 的组件连布局节点都不建，CSS 盒属性整体失效；
//! `CssBox::BorderBoxTail` 有布局节点、padding 也写进去了，但子节点是独立布局根，
//! padding / border / gap 没有扣减机会。

use crate::input::wxml::WxmlElement;
use crate::matcher::Tree;
use crate::project::Project;

/// 这些属性里哪些写了「非初值」，且全部可判定。
///
/// `None`：有选择器判不了，可能改写结果——调用方一律当「没写」（宁可漏报）。
/// `native-padding` 与 `native-layout-ignored` 的判定只差属性列表和「什么算写了」。
pub(crate) fn declared<'a>(
    project: &Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
    properties: &[&'a str],
    effective: impl Fn(&str, &str) -> bool,
) -> Option<Vec<&'a str>> {
    let mut hit = Vec::new();
    for property in properties {
        let (resolved, undecidable) =
            crate::rules::wxml::resolved_property_with(project, tree, element, property);
        if undecidable {
            return None;
        }
        if resolved.is_some_and(|resolved| effective(property, &resolved.value)) {
            hit.push(*property);
        }
    }
    Some(hit)
}
