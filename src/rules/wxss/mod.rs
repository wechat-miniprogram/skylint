//! WXSS 语义类规则。
//!
//! 一条规则一个文件：手写判定的少量规则、纯数据驱动的属性/值支持表、以及
//! 「值的个数/结构」类限制各占一个文件。它们共用 [`visit_rules`] /
//! [`visit_declarations`] 两层遍历与 [`value_util`] 的值分词工具。

pub mod attribute_selector;
pub mod box_sizing;
pub mod calc_angle;
pub mod calc_min_max;
pub mod dead_selector;
pub mod dict_table;
pub mod filter_limit;
pub mod fixed_layer;
pub mod flex_direction;
pub mod gradient_limit;
pub mod gradient_unsupported;
pub mod implicit_color;
pub mod legacy_flexbox;
pub mod media_query;
pub mod multi_value_limit;
pub mod no_pseudo;
pub mod scroll_view_height;
pub mod selector;
pub mod shorthand_partial;
pub mod tag_selector_isolation;
pub mod text_decoration_line;
pub mod text_decoration_target;
pub mod text_overflow_ellipsis;
pub mod transition_property;
pub mod unsupported_property;
pub mod unsupported_value;
pub mod value_degraded;
pub mod value_util;

pub use attribute_selector::AttributeSelector;
pub use box_sizing::BoxSizing;
pub use calc_angle::CalcAngle;
pub use calc_min_max::CalcMinMax;
pub use dead_selector::DeadSelector;
pub use filter_limit::FilterLimit;
pub use fixed_layer::FixedLayer;
pub use flex_direction::FlexDirection;
pub use gradient_limit::GradientLimit;
pub use gradient_unsupported::GradientUnsupported;
pub use implicit_color::ImplicitColor;
pub use legacy_flexbox::LegacyFlexbox;
pub use media_query::MediaQuery;
pub use multi_value_limit::MultiValueLimit;
pub use no_pseudo::NoPseudo;
pub use scroll_view_height::ScrollViewHeight;
pub use shorthand_partial::ShorthandPartial;
pub use tag_selector_isolation::TagSelectorIsolation;
pub use text_decoration_line::TextDecorationLine;
pub use text_decoration_target::TextDecorationTarget;
pub use text_overflow_ellipsis::TextOverflowEllipsis;
pub use transition_property::TransitionProperty;
pub use unsupported_property::UnsupportedProperty;
pub use unsupported_value::UnsupportedValue;
pub use value_degraded::ValueDegraded;

use std::collections::HashMap;

use crate::Diagnostic;
use crate::input::wxml::{WxmlDoc, WxmlElement};
use crate::input::wxss::{WxssDeclaration, WxssDoc, WxssRule, WxssSelector};
use crate::matcher::{Dialect, Tree, Tri};
use crate::project::{Project, Scope, StyleSheet};
use crate::rules::Target;

/// 这条声明的作用域里需要一起看的 WXML。
pub(crate) fn scope_wxml<'a>(project: &'a Project, scope: &Scope) -> Vec<&'a str> {
    match scope {
        Scope::Global => project.global_wxml().collect(),
        // 样式图不会产生 Loose。
        Scope::Loose(_) => Vec::new(),
        _ => project.scope_wxml(scope).into_iter().collect(),
    }
}

/// CSS 全局关键字是「恢复默认」，不是「用了不支持的值」：属性在 Skyline 下本来
/// 就不存在，`unset` 只是把 WebView 侧写的那份清掉，报出来是噪音。
pub(crate) fn is_css_wide_keyword(value: &str) -> bool {
    let value = value.trim();
    ["unset", "initial", "revert", "revert-layer"]
        .iter()
        .any(|keyword| value.eq_ignore_ascii_case(keyword))
}

/// 目标渲染器的覆盖块：同一 sheet 里存在「本选择器 + `.skyline`（或 `.webview`）」
/// 的规则又声明了同一个属性时，基础声明在该渲染器下并不生效。
///
/// 迁移工程常用 `.x.skyline { display: unset; -webkit-line-clamp: unset }` 关掉
/// 只给 WebView 写的那套写法，把这些基础声明报成「Skyline 不支持」等于反着看。
pub(crate) fn overridden_by_renderer_class(
    sheet: &StyleSheet,
    rule: &WxssRule,
    property: &str,
    target: Target,
) -> bool {
    let class = match target {
        Target::Skyline => "skyline",
        Target::Webview => "webview",
        Target::Both => return false,
    };
    let mine: Vec<String> = rule
        .selectors
        .iter()
        .map(|selector| normalize_selector(&selector.text))
        .collect();
    sheet.doc.rules.iter().any(|other| {
        !std::ptr::eq(other, rule)
            && other
                .declarations
                .iter()
                .any(|decl| decl.name.eq_ignore_ascii_case(property))
            && other.selectors.iter().any(|selector| {
                strip_class(&selector.text, class)
                    .is_some_and(|base| mine.contains(&normalize_selector(&base)))
            })
    })
}

fn normalize_selector(text: &str) -> String {
    text.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
}

/// 把选择器里指定的类去掉（`.a .b.skyline` → `.a .b`）；没出现该类时返回 `None`。
fn strip_class(text: &str, class: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut found = false;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '.' {
            out.push(ch);
            continue;
        }
        let mut name = String::new();
        while let Some(&next) = chars.peek() {
            if next.is_alphanumeric() || next == '-' || next == '_' {
                name.push(next);
                chars.next();
            } else {
                break;
            }
        }
        if name == class {
            found = true;
        } else {
            out.push('.');
            out.push_str(&name);
        }
    }
    found.then_some(out)
}

/// 这批选择器在作用域里是否有任何可能命中（`Yes` 或 `Unknown`）。
///
/// 用于「条件被丢弃、块内声明对所有设备生效」这类判定：块内选择器一个节点都
/// 对不上时，条件丢失不产生任何可见差异。看不到 WXML 时保守返回 `true`。
pub(crate) fn any_selector_matches(
    project: &Project,
    sheet: &StyleSheet,
    selectors: &[WxssSelector],
    docs: &mut HashMap<String, Option<WxmlDoc>>,
) -> bool {
    let mut seen_doc = false;
    for scope in &sheet.scopes {
        for path in scope_wxml(project, scope) {
            let Some(doc) = docs
                .entry(path.to_string())
                .or_insert_with(|| crate::rules::wxml::load_doc(project, path))
                .as_ref()
            else {
                return true;
            };
            seen_doc = true;
            let tree = Tree::new(doc);
            for selector in selectors {
                let text = selector.text.trim();
                // 伪元素的承载者是生成的文本节点，匹配器一律给 `No`（见
                // `dead-selector` 的豁免说明），不能据此断定块内没有生效。
                if crate::matcher::shape(text).is_none_or(|shape| shape.pseudo_element) {
                    return true;
                }
                if tree.any_match(text, Dialect::WebView) != Tri::No {
                    return true;
                }
            }
        }
    }
    !seen_doc
}

/// 命中节点是否全都承载不了这条声明；返回「在某处失效」的选择器原文。
///
/// 用 WebView 方言求值：这里问的是「这条声明在 WebView 下落在哪些节点上」，
/// 迁移前用户看到的正是 WebView 的渲染结果。
///
/// 只看静态命中（[`Tri::Yes`]）：动态属性 / 运行时状态命中的是**别的**节点，
/// 一个动态 class 不能把已经确定失效的静态命中洗白，也不能单独成为一个命中。
pub(crate) fn broken_selectors(
    project: &Project,
    sheet: &StyleSheet,
    selectors: &[WxssSelector],
    docs: &mut HashMap<String, Option<WxmlDoc>>,
    can_host: impl Fn(&Tree, &WxmlElement) -> bool,
) -> Vec<String> {
    // (选择器, 作用域) → (是否静态命中, 命中里是否有能承载的节点)
    let mut tallies = vec![vec![(false, false); sheet.scopes.len()]; selectors.len()];
    for (scope_index, scope) in sheet.scopes.iter().enumerate() {
        for path in scope_wxml(project, scope) {
            let Some(doc) = docs
                .entry(path.to_string())
                .or_insert_with(|| crate::rules::wxml::load_doc(project, path))
                .as_ref()
            else {
                continue;
            };
            let tree = Tree::new(doc);
            for (selector_index, selector) in selectors.iter().enumerate() {
                let tally = &mut tallies[selector_index][scope_index];
                let text = selector.text.trim();
                for element in doc.elements() {
                    if tree.matches(element, text, Dialect::WebView) == Tri::Yes {
                        tally.0 = true;
                        tally.1 |= can_host(&tree, element);
                    }
                }
            }
        }
    }
    selectors
        .iter()
        .enumerate()
        .filter(|(index, _)| {
            tallies[*index]
                .iter()
                .any(|(matched, hostable)| *matched && !*hostable)
        })
        .map(|(_, selector)| selector.text.trim().to_string())
        .collect()
}

/// 遍历全部样式文件的每一条规则。
///
/// `.wxss` / `.less` / `.scss` 一视同仁：预处理器在 `input::wxss` 里已按同一套
/// 语法摊平成规则（嵌套、`&`、at-rule 容器）。
///
/// 数据来自 `Project.styles`：建图时已经解析过一遍，规则侧不重复读盘。不按
/// `is_skyline` 过滤：这是**迁移**检查工具，目标用户是还在 WebView 上的项目。
/// 若只查已迁移的页面，纯 WebView 工程会一条都不报，「迁移前拿到问题清单」的用途
/// 就落空了。
pub(crate) fn visit_rules(
    project: &Project,
    mut f: impl FnMut(&StyleSheet, &WxssRule, &mut Vec<Diagnostic>),
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for sheet in &project.styles {
        for rule in &sheet.doc.rules {
            f(sheet, rule, &mut out);
        }
    }
    out
}

/// 遍历全部样式文件的每一条声明。
pub(crate) fn visit_declarations(
    project: &Project,
    mut f: impl FnMut(&WxssDoc, &WxssRule, &WxssDeclaration, &mut Vec<Diagnostic>),
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for sheet in &project.styles {
        for rule in &sheet.doc.rules {
            for declaration in &rule.declarations {
                f(&sheet.doc, rule, declaration, &mut out);
            }
        }
    }
    out
}
