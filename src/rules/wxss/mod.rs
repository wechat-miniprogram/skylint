//! WXSS 语义类规则。
//!
//! 一条规则一个文件：手写判定的少量规则、纯数据驱动的属性/值支持表、以及
//! 「值的个数/结构」类限制各占一个文件。它们共用 [`visit_rules`] /
//! [`visit_declarations`] 两层遍历与 [`value_util`] 的值分词工具。

pub mod box_sizing;
pub mod calc_angle;
pub mod calc_min_max;
pub mod dict_table;
pub mod filter_limit;
pub mod flex_direction;
pub mod gradient_limit;
pub mod gradient_unsupported;
pub mod multi_value_limit;
pub mod no_pseudo;
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

pub use box_sizing::BoxSizing;
pub use calc_angle::CalcAngle;
pub use calc_min_max::CalcMinMax;
pub use filter_limit::FilterLimit;
pub use flex_direction::FlexDirection;
pub use gradient_limit::GradientLimit;
pub use gradient_unsupported::GradientUnsupported;
pub use multi_value_limit::MultiValueLimit;
pub use no_pseudo::NoPseudo;
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
use crate::project::{Project, Scope, StyleSheet};

use selector::{Match, SimpleSelector};

/// 这条声明的作用域里需要一起看的 WXML。
pub(crate) fn scope_wxml<'a>(project: &'a Project, scope: &Scope) -> Vec<&'a str> {
    match scope {
        Scope::Global => project.global_wxml().collect(),
        // 样式图不会产生 Loose。
        Scope::Loose(_) => Vec::new(),
        _ => project.scope_wxml(scope).into_iter().collect(),
    }
}

/// 命中节点是否全都承载不了这条声明；返回「在某处失效」的选择器原文。
///
/// 只看静态命中（[`Match::Yes`]）：动态属性命中的是**别的**节点，`一个动态 class
/// 不能把已经确定失效的静态命中洗白，也不能单独成为一个命中。
pub(crate) fn broken_selectors(
    project: &Project,
    sheet: &StyleSheet,
    selectors: &[WxssSelector],
    docs: &mut HashMap<String, Option<WxmlDoc>>,
    can_host: impl Fn(&WxmlElement) -> bool,
) -> Vec<String> {
    let mut out = Vec::new();
    for selector in selectors {
        let Some(simple) = SimpleSelector::parse(selector) else {
            continue;
        };
        // 伪元素生成的是文本节点，声明的承载对象就是它，不看宿主元素。
        if simple.text_pseudo_element() {
            continue;
        }
        // 按作用域分别判：共享样式在 A 页失效、B 页正常也要报。
        let mut broken = false;
        for scope in &sheet.scopes {
            let (mut matched, mut hostable) = (false, false);
            for path in scope_wxml(project, scope) {
                let Some(doc) = docs
                    .entry(path.to_string())
                    .or_insert_with(|| crate::rules::wxml::load_doc(project, path))
                    .as_ref()
                else {
                    continue;
                };
                for element in doc.elements() {
                    if simple.matches(element) == Match::Yes {
                        matched = true;
                        hostable |= can_host(element);
                    }
                }
            }
            broken |= matched && !hostable;
        }
        if broken {
            out.push(selector.text.trim().to_string());
        }
    }
    out
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
    mut f: impl FnMut(&WxssDoc, &WxssDeclaration, &mut Vec<Diagnostic>),
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for sheet in &project.styles {
        for rule in &sheet.doc.rules {
            for declaration in &rule.declarations {
                f(&sheet.doc, declaration, &mut out);
            }
        }
    }
    out
}
