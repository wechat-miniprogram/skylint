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
use crate::matcher::{self, Dialect, Tree, Tri};
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

/// 这条声明是否被**同元素的渲染器分支**关掉了。
///
/// 迁移工程常见写法：基础块服务 WebView，另一个分支（`.x.skyline { display: unset;
/// -webkit-line-clamp: unset }`）在 Skyline 下把它整体重置。把这些基础声明报成
/// 「Skyline 不支持」等于反着看。
///
/// 不猜类名命名：只认 `renderer` 的取值词汇（`skyline` / `webview`），并且要到
/// WXML 里**验证**那个节点的 `class` 真的由 `renderer` 拼出来——否则一个凑巧叫
/// `.skyline` 的普通修饰类就会把真问题消音。作者把类叫 `.sky` 或改用 `wx:if`
/// 分支时认不出来，按漏报处理。
pub(crate) fn overridden_in_renderer_branch(
    project: &Project,
    sheet: &StyleSheet,
    rule: &WxssRule,
    property: &str,
    target: Target,
    docs: &mut HashMap<String, Option<WxmlDoc>>,
) -> bool {
    let branch = match target {
        Target::Skyline => "skyline",
        Target::Webview => "webview",
        Target::Both => return false,
    };
    let mine: Vec<String> = rule
        .selectors
        .iter()
        .map(|selector| normalize_selector(&selector.text))
        .collect();
    let variant = sheet.doc.rules.iter().find(|other| {
        !std::ptr::eq(*other, rule)
            && other
                .declarations
                .iter()
                .any(|decl| decl.name.eq_ignore_ascii_case(property))
            && other.selectors.iter().any(|selector| {
                strip_class(&selector.text, branch)
                    .is_some_and(|base| mine.contains(&normalize_selector(&base)))
            })
    });
    let Some(_) = variant else {
        return false;
    };
    // 变异存在还不够：必须证明确实有这个渲染器分支的元素存在。
    rule.selectors
        .iter()
        .any(|selector| class_depends_on_renderer(project, sheet, &selector.text, docs))
}

/// 选择器主体所在的节点，`class` 是否真的由 `renderer` 拼出来。
///
/// 只看**主体的类**（`.a .b` 里的 `.b`）：祖先类对这个判断没有意义，而整条选择器
/// 求值容易因某个祖先的动态 `class` 变成 `Unknown`。
///
/// 不查 `Env` 的词元求值：组件 `properties` 里的 `renderer` 默认值通常是 `'webview'`，
/// 而运行时的值来自父级传参，拿默认值判会把真分支直接判掉。
pub(crate) fn class_depends_on_renderer(
    project: &Project,
    sheet: &StyleSheet,
    selector: &str,
    docs: &mut HashMap<String, Option<WxmlDoc>>,
) -> bool {
    let Some(subject) = last_compound(selector) else {
        return false;
    };
    if matcher::shape(subject).is_none_or(|shape| shape.pseudo_element || shape.runtime_root) {
        return false;
    }
    let Some(classes) = matcher::class_names(subject) else {
        return false;
    };
    if classes.is_empty() {
        return false;
    }
    for scope in &sheet.scopes {
        for path in scope_wxml(project, scope) {
            let Some(doc) = docs
                .entry(path.to_string())
                .or_insert_with(|| crate::rules::wxml::load_doc(project, path))
                .as_ref()
            else {
                continue;
            };
            let mut hit = false;
            crate::rules::wxml::walk(doc, |element, _| {
                if element.in_template {
                    return;
                }
                let Some(attr) = element.attributes.iter().find(|attr| attr.name == "class") else {
                    return;
                };
                if !attr.references_renderer {
                    return;
                }
                let carries = match &attr.class_tokens {
                    Some(tokens) => classes.iter().all(|class| tokens.is_definite(class, true)),
                    None => false,
                };
                if carries {
                    hit = true;
                }
            });
            if hit {
                return true;
            }
        }
    }
    false
}

/// `.a .skyline`：同一元素的两个类被写成了后代关系时，返回（前缀, 分支类名）。
///
/// 只认 `renderer` 的**取值词汇**（`skyline` / `webview`）——它们是平台给定的，
/// 而类名本身可以是任意拼法（`.is-sky`、`.sky`），那样就认不出来，按漏报处理。
pub(crate) fn same_node_renderer_class<'a>(
    project: &Project,
    sheet: &StyleSheet,
    selector: &'a str,
    docs: &mut HashMap<String, Option<WxmlDoc>>,
) -> Option<(&'a str, &'a str)> {
    let (prefix, last) = split_last_compound(selector)?;
    let branch = last.strip_prefix('.')?;
    if !matches!(branch, "skyline" | "webview") {
        return None;
    }
    if matcher::shape(prefix).is_none_or(|shape| shape.pseudo_element || shape.runtime_root) {
        return None;
    }
    class_depends_on_renderer(project, sheet, prefix, docs).then_some((prefix, branch))
}

/// 按最后一个顶层组合符拆成（前缀, 最后一段）。
fn split_last_compound(text: &str) -> Option<(&str, &str)> {
    let mut depth = 0i32;
    let mut split = None;
    for (index, ch) in text.char_indices().rev() {
        match ch {
            ')' | ']' => depth += 1,
            '(' | '[' => depth -= 1,
            ' ' | '>' | '+' | '~' if depth == 0 => {
                split = Some(index);
                break;
            }
            _ => {}
        }
    }
    let split = split?;
    let prefix = text[..split].trim_end();
    let last = text[split..]
        .trim_start_matches(|ch: char| ch.is_whitespace() || matches!(ch, '>' | '+' | '~'))
        .trim_end();
    (!prefix.is_empty() && !last.is_empty()).then_some((prefix, last))
}

fn last_compound(text: &str) -> Option<&str> {
    Some(match split_last_compound(text) {
        Some((_, last)) => last,
        None => text.trim(),
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

/// 命中节点是不是全都靠 `<text max-lines="N">` 自己完成截断（且 N 与老式
/// `-webkit-line-clamp` 一致）。
///
/// 老式 clamp 堆叠（`display: -webkit-box` + `-webkit-line-clamp`）在被命中的
/// 节点都是 `<text>` / `<span>` 且带同一行数的 `max-lines` 时属于**两端各写一份**：
/// WebView 用堆叠、Skyline 用属性，结果一致。这时报「声明被丢弃」是噪音。
pub(crate) fn truncation_by_max_lines(
    project: &Project,
    sheet: &StyleSheet,
    selectors: &[WxssSelector],
    docs: &mut HashMap<String, Option<WxmlDoc>>,
    clamp: &str,
) -> bool {
    let Some(lines) = clamp.trim().parse::<u32>().ok() else {
        return false;
    };
    let mut matched = false;
    let mut all = true;
    for scope in &sheet.scopes {
        for path in scope_wxml(project, scope) {
            let Some(doc) = docs
                .entry(path.to_string())
                .or_insert_with(|| crate::rules::wxml::load_doc(project, path))
                .as_ref()
            else {
                return false;
            };
            let tree = Tree::new(doc);
            for selector in selectors {
                for element in doc.elements() {
                    if tree.matches(element, selector.text.trim(), Dialect::WebView) != Tri::Yes {
                        continue;
                    }
                    matched = true;
                    let text_like = matches!(element.tag.as_str(), "text" | "span");
                    let same_lines = element
                        .attributes
                        .iter()
                        .find(|attr| attr.name == "max-lines")
                        .and_then(|attr| attr.value.as_deref())
                        .and_then(|value| value.trim().parse::<u32>().ok())
                        .is_some_and(|value| value == lines);
                    all &= text_like && same_lines;
                }
            }
        }
    }
    matched && all
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
