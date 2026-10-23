//! 组件对直接子节点的约束：只允许某类子节点、必须恰好一个子节点、条件必需的子节点。
//!
//! 约束直接写在下方 `ROWS` 里，按组件标签查表。属性一律只认静态值
//! （`WxmlAttribute::is_static`），`{{}}` 插值跳过——静态阶段无法确定实际取值，
//! 宁可漏报。
//!
//! 子节点按 `input/wxml.rs` 的摊平语义计数：`<block>` / `wx:for` / `wx:if` 不产生
//! 自己的层级，所以同一个 `wx:if` 的 if 与 else 分支会同时挂在一个父节点下。
//! `Exactly(n)` 数的是「能同时渲染的子元素」的最大值：同一个 `wx:if` 的不同分支互斥，
//! 取各分支里最多的那一支；不同分支组之间相加。真实工程 `miniprogram-skit_1` 的
//! `components/index-block/index-block.wxml:62,139` 就在
//! `<horizontal-drag-gesture-handler>` 里用 `wx:if` / `wx:else` 各包一个 `<swiper>`；
//! 按摊平结果直接计数会把它误报成 2 个子节点。
//!
//! # 约束语义
//!
//! - `Only`：直接元素子节点只能是这些标签。`text` 是特殊值，表示「文本节点也允许」，
//!   其余按标签名精确匹配。
//! - `Exactly`：直接元素子节点个数必须正好是 n；文本子节点（含空白）一律不计，因为
//!   WXML 里换行缩进就是文本节点。
//! - `RequiresChild`：自身条件全部成立时，直接子节点里必须有一个该属性取到目标值的
//!   节点。多个条件必须同时成立；不带 `=值` 的条件只要求属性存在（未被静态置为假）。
//!   动态属性值不触发。
//!
//! # 依据
//!
//! - navigator：skyline-overview/references/introduction/component-support.md:23
//!   「navigator | 只能嵌套 text 组件或文本节点」；同文件 :150-162 给出正反例
//!   （❌ `<navigator><view>不能这样用</view></navigator>`）。
//! - 手势组件单子节点：skyline-components/references/gesture/gesture-system.md:186
//!   MUST「必须使用单子节点：手势组件只能包含一个直接子节点」；同文件 :11-22 的表
//!   列出 8 个手势组件（tap / double-tap / long-press / pan / horizontal-drag /
//!   vertical-drag / scale / force-press），:9 说明它们是「虚组件，不参与布局渲染，
//!   事件由其直接子节点响应」——所以 0 个子节点等于手势没有响应目标，同为 error。
//! - scroll-view 自定义刷新节点：skyline-components/references/scroll/scroll-view.md
//!   :195（`refresher-default-style` 取 black/white/none）、:214-222（none 的示例
//!   必须给 `<view slot="refresher">`）、:377 Bug&Tip 第 4 条「自定义下拉刷新节点需
//!   声明 slot="refresher"」。取 none 时不给自定义节点，下拉刷新区就是空白，故为
//!   warning 而不是 error。触发还要 `refresher-enabled`（:193 默认 false）：没开
//!   下拉刷新时 `refresher-default-style` 不产生任何刷新节点。

use crate::diag::{Diagnostic, Severity};
use crate::input::wxml::{WxmlDoc, WxmlElement};
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};

use super::branch::{element_children, max_rendered_children};
use super::{element_diagnostic, load_doc, wxml_files};

pub const ID: &str = "skyline/component-children";

/// 子节点约束。
enum Constraint {
    /// 直接元素子节点只能是这些标签。`text` 是特殊值，表示文本节点也算合法。
    Only(&'static [&'static str]),
    /// 直接元素子节点个数必须正好是 n；文本子节点不计。
    Exactly(u32),
    /// 自身条件成立时，直接子节点里必须有 `child_attr=child_value`。
    RequiresChild {
        child_attr: &'static str,
        child_value: &'static str,
        /// 多个条件必须同时成立。
        own: &'static [Condition],
    },
}

/// 一个触发条件。
enum Condition {
    /// `attr="value"`：静态值与 value 相等（忽略大小写）才算成立。
    Equals(&'static str, &'static str),
    /// `attr`：属性存在且未被静态置为假，即 `attr=!(false|0)`——WXML 布尔属性的写法
    /// 太多（`refresher-enabled`、`refresher-enabled=""`、`"{{true}}"`），静态阶段
    /// 无法区分 `{{true}}` 与 `{{refreshing}}`，所以只排除「确实没开」。
    Present(&'static str),
}

struct Row {
    tag: &'static str,
    constraint: Constraint,
    severity: Severity,
    help: &'static str,
}

/// 组件对直接子节点的约束表，组件标签唯一。
const ROWS: &[Row] = &[
    Row {
        tag: "navigator",
        constraint: Constraint::Only(&["text"]),
        severity: Severity::Error,
        help: "把 `<navigator>` 的内容改成 `<text>` 或文本节点；需要自定义布局时改用 `bind:tap` 的 `<view>`",
    },
    Row {
        tag: "pan-gesture-handler",
        constraint: Constraint::Exactly(1),
        severity: Severity::Error,
        help: "用一个 `<view>` 包住手势内容，`<pan-gesture-handler>` 内只留这一个子节点",
    },
    Row {
        tag: "double-tap-gesture-handler",
        constraint: Constraint::Exactly(1),
        severity: Severity::Error,
        help: "用一个 `<view>` 包住手势内容，`<double-tap-gesture-handler>` 内只留这一个子节点",
    },
    Row {
        tag: "long-press-gesture-handler",
        constraint: Constraint::Exactly(1),
        severity: Severity::Error,
        help: "用一个 `<view>` 包住手势内容，`<long-press-gesture-handler>` 内只留这一个子节点",
    },
    Row {
        tag: "horizontal-drag-gesture-handler",
        constraint: Constraint::Exactly(1),
        severity: Severity::Error,
        help: "用一个 `<view>` 包住手势内容，`<horizontal-drag-gesture-handler>` 内只留这一个子节点",
    },
    Row {
        tag: "vertical-drag-gesture-handler",
        constraint: Constraint::Exactly(1),
        severity: Severity::Error,
        help: "用一个 `<view>` 包住手势内容，`<vertical-drag-gesture-handler>` 内只留这一个子节点",
    },
    Row {
        tag: "scale-gesture-handler",
        constraint: Constraint::Exactly(1),
        severity: Severity::Error,
        help: "用一个 `<view>` 包住手势内容，`<scale-gesture-handler>` 内只留这一个子节点",
    },
    Row {
        tag: "force-press-gesture-handler",
        constraint: Constraint::Exactly(1),
        severity: Severity::Error,
        help: "用一个 `<view>` 包住手势内容，`<force-press-gesture-handler>` 内只留这一个子节点",
    },
    Row {
        tag: "tap-gesture-handler",
        constraint: Constraint::Exactly(1),
        severity: Severity::Error,
        help: "用一个 `<view>` 包住手势内容，`<tap-gesture-handler>` 内只留这一个子节点",
    },
    Row {
        tag: "scroll-view",
        constraint: Constraint::RequiresChild {
            child_attr: "slot",
            child_value: "refresher",
            own: &[
                Condition::Equals("refresher-default-style", "none"),
                Condition::Present("refresher-enabled"),
            ],
        },
        severity: Severity::Warning,
        help: "给 `<scroll-view refresher-default-style=\"none\">` 补一个 `<view slot=\"refresher\">` 作为自定义刷新节点",
    },
];

enum AttrCheck {
    Ok,
    Violation,
    /// 动态绑定，静态阶段无法判定。
    Unknown,
}

fn find_attr<'a>(
    element: &'a WxmlElement,
    name: &str,
) -> Option<&'a crate::input::wxml::WxmlAttribute> {
    element
        .attributes
        .iter()
        .find(|attr| attr.name.eq_ignore_ascii_case(name))
}

fn check_attr(element: &WxmlElement, name: &str, expected: &str) -> AttrCheck {
    match find_attr(element, name) {
        None => AttrCheck::Violation,
        Some(attr) if !attr.is_static => AttrCheck::Unknown,
        Some(attr) => match attr.value.as_deref() {
            Some(value) if value.trim().eq_ignore_ascii_case(expected) => AttrCheck::Ok,
            _ => AttrCheck::Violation,
        },
    }
}

/// 属性是否被明确关掉：缺失，或静态假值（`false` / `0`）。
/// 动态值（`{{...}}`，包括 `{{true}}`）无法区分，按「可能开启」处理。
fn attr_is_off(element: &WxmlElement, name: &str) -> bool {
    match find_attr(element, name) {
        None => true,
        Some(attr) if attr.is_static => attr
            .value
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| value.eq_ignore_ascii_case("false") || value == "0"),
        Some(_) => false,
    }
}

fn condition_text(condition: &Condition) -> String {
    match condition {
        Condition::Equals(name, value) => format!(" {name}=\"{value}\""),
        Condition::Present(name) => format!(" {name}"),
    }
}

/// 允许的子节点渲染成文档里的写法：`text` 指 `<text>` 组件与裸文本节点，其余按标签名。
fn render_allowed(allowed: &[&str]) -> String {
    allowed
        .iter()
        .map(|name| match *name {
            "text" => "`<text>` 或文本节点".to_string(),
            tag => format!("`<{tag}>`"),
        })
        .collect::<Vec<_>>()
        .join(" 或 ")
}

fn violations(doc: &WxmlDoc, element: &WxmlElement, row: &Row, path: &str) -> Vec<Diagnostic> {
    let on_element = |message: String| {
        element_diagnostic(ID, doc, element, path, row.severity, message).help(row.help)
    };
    match &row.constraint {
        Constraint::Only(allowed) => element_children(element)
            // 文本子节点不是元素，天然不在候选集里：`text` 只表示它们合法。
            .filter(|child| !allowed.contains(&child.tag.as_str()))
            .map(|child| {
                element_diagnostic(
                    ID,
                    doc,
                    child,
                    path,
                    row.severity,
                    format!(
                        "`<{}>` 只能嵌套 {}，当前子节点是 `<{}>`",
                        element.tag,
                        render_allowed(allowed),
                        child.tag
                    ),
                )
                .help(row.help)
            })
            .collect(),
        Constraint::Exactly(expected) => {
            let count = max_rendered_children(element);
            if count == *expected as usize {
                return Vec::new();
            }
            let message = if count == 0 {
                format!(
                    "`<{}>` 里没有元素子节点；手势组件不参与布局，事件只能由唯一的直接子节点接收，当前没有响应目标",
                    element.tag
                )
            } else {
                format!(
                    "`<{}>` 有 {count} 个直接子元素；手势组件不参与布局，事件只绑定到唯一的直接子节点",
                    element.tag
                )
            };
            vec![on_element(message)]
        }
        Constraint::RequiresChild {
            child_attr,
            child_value,
            own,
        } => {
            // 任一触发条件不成立就不报；动态值无法确认，一律按不成立处理。
            let satisfied = own.iter().all(|condition| match condition {
                Condition::Equals(name, value) => {
                    matches!(check_attr(element, name, value), AttrCheck::Ok)
                }
                Condition::Present(name) => !attr_is_off(element, name),
            });
            if !satisfied {
                return Vec::new();
            }
            let mut unknown = false;
            for child in element_children(element) {
                match check_attr(child, child_attr, child_value) {
                    AttrCheck::Ok => return Vec::new(),
                    // 子节点的该属性是动态值：可能是目标节点，静态阶段无法否定。
                    AttrCheck::Unknown => unknown = true,
                    AttrCheck::Violation => {}
                }
            }
            if unknown {
                return Vec::new();
            }
            vec![on_element(format!(
                "`<{}{}>` 没有 `{}=\"{}\"` 的直接子节点，自定义刷新节点不会被渲染",
                element.tag,
                own.first().map(condition_text).unwrap_or_default(),
                child_attr,
                child_value
            ))]
        }
    }
}

pub struct ComponentChildren;

impl Rule for ComponentChildren {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Error,
            description: "组件对直接子节点的结构约束（只允许某类子节点 / 必须唯一子节点 / 条件必需的子节点）",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        for path in wxml_files(project) {
            let Some(doc) = load_doc(project, path) else {
                continue;
            };
            for element in doc.elements() {
                // 模板定义体的真实子节点在实例化处，静态不可知。
                if element.in_template {
                    continue;
                }
                for row in ROWS.iter().filter(|row| row.tag == element.tag) {
                    diagnostics.extend(violations(&doc, element, row, path));
                }
            }
        }
        diagnostics
    }
}

#[cfg(test)]
mod tests {
    use super::{ComponentChildren, ROWS};
    use crate::Severity;
    use crate::rules::testutil::wxml::{Fixture, check_rule, project};

    fn check(fixture: &Fixture) -> Vec<crate::Diagnostic> {
        check_rule(fixture, &ComponentChildren)
    }

    // ---- 约束表 ----

    #[test]
    fn component_tags_are_unique() {
        let mut tags: Vec<&str> = ROWS.iter().map(|row| row.tag).collect();
        tags.sort_unstable();
        let count = tags.len();
        tags.dedup();
        assert_eq!(tags.len(), count, "组件约束表里有重复的组件标签");
    }

    // ---- navigator：只能放 text / 文本节点 ----

    #[test]
    fn navigator_with_view_is_reported() {
        let diagnostics = check(&project(
            "navigator-view",
            r#"<navigator url="/pages/a/a"><view>不能这样用</view></navigator>"#,
        ));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert!(diagnostics[0].message.contains("只能嵌套"));
        // 报在违规的子节点上，不是 navigator 上。
        assert_eq!(diagnostics[0].column, 30);
    }

    #[test]
    fn every_disallowed_navigator_child_is_reported() {
        let diagnostics = check(&project(
            "navigator-two",
            r#"<navigator url="/x"><view/><image/></navigator>"#,
        ));
        assert_eq!(diagnostics.len(), 2);
        assert!(
            diagnostics
                .iter()
                .all(|d| d.message.contains("<navigator>"))
        );
    }

    #[test]
    fn navigator_with_text_element_is_allowed() {
        let diagnostics = check(&project(
            "navigator-text",
            r#"<navigator url="/x"><text>点击跳转</text></navigator>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn navigator_with_plain_text_is_allowed() {
        let diagnostics = check(&project(
            "navigator-plain",
            r#"<navigator url="/x">点击跳转</navigator>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn navigator_with_text_node_and_text_element_is_allowed() {
        let diagnostics = check(&project(
            "navigator-mixed",
            r#"<navigator url="/x">同意<text>《用户协议》</text></navigator>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn navigator_with_only_whitespace_children_is_allowed() {
        let diagnostics = check(&project(
            "navigator-blank",
            "<navigator url=\"/x\">\n  \n</navigator>",
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    // ---- 手势组件：恰好一个直接元素子节点 ----

    #[test]
    fn gesture_handler_without_child_is_reported() {
        let diagnostics = check(&project("gesture-empty", "<pan-gesture-handler/>"));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert!(diagnostics[0].message.contains("没有元素子节点"));
    }

    #[test]
    fn gesture_handler_with_two_children_is_reported() {
        let diagnostics = check(&project(
            "gesture-two",
            r#"<pan-gesture-handler worklet:ongesture="onPan"><view/><view/></pan-gesture-handler>"#,
        ));
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("2 个直接子元素"));
        assert_eq!(diagnostics[0].column, 2);
    }

    #[test]
    fn every_gesture_handler_is_checked() {
        let diagnostics = check(&project(
            "gesture-all",
            concat!(
                r#"<tap-gesture-handler><view/><view/></tap-gesture-handler>"#,
                r#"<double-tap-gesture-handler><view/><view/></double-tap-gesture-handler>"#,
                r#"<long-press-gesture-handler><view/><view/></long-press-gesture-handler>"#,
                r#"<horizontal-drag-gesture-handler><view/><view/></horizontal-drag-gesture-handler>"#,
                r#"<vertical-drag-gesture-handler><view/><view/></vertical-drag-gesture-handler>"#,
                r#"<scale-gesture-handler><view/><view/></scale-gesture-handler>"#,
                r#"<force-press-gesture-handler><view/><view/></force-press-gesture-handler>"#,
            ),
        ));
        assert_eq!(diagnostics.len(), 7, "{diagnostics:#?}");
    }

    #[test]
    fn gesture_handler_with_one_child_is_allowed() {
        let diagnostics = check(&project(
            "gesture-one",
            concat!(
                "<pan-gesture-handler worklet:ongesture=\"onPan\">\n",
                "  <view class=\"wrapper\">\n",
                "    <slot></slot>\n",
                "  </view>\n",
                "</pan-gesture-handler>\n",
            ),
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn gesture_handler_with_one_child_and_whitespace_text_is_allowed() {
        let diagnostics = check(&project(
            "gesture-whitespace",
            "<tap-gesture-handler>\n\n  <view/>\n\n</tap-gesture-handler>",
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// 互斥分支只算一个：`wx:if` / `wx:else` 各包一个 `<view>` 是合法的。
    /// 真实工程 `miniprogram-skit_1` 的 `components/index-block/index-block.wxml:62,139`
    /// 就是这个形状（两个 `<swiper>`）。
    #[test]
    fn mutually_exclusive_branches_are_counted_once() {
        let diagnostics = check(&project(
            "gesture-branches",
            concat!(
                r#"<pan-gesture-handler>"#,
                r#"<block wx:if="{{a}}"><view/></block>"#,
                r#"<block wx:else><view/></block>"#,
                r#"</pan-gesture-handler>"#,
            ),
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn swiper_if_else_inside_gesture_handler_is_allowed() {
        let diagnostics = check(&project(
            "gesture-swiper-branches",
            concat!(
                r#"<horizontal-drag-gesture-handler native-view="swiper">"#,
                r#"<swiper wx:if="{{type === 'medium'}}"><swiper-item/></swiper>"#,
                r#"<swiper wx:else><swiper-item/></swiper>"#,
                r#"</horizontal-drag-gesture-handler>"#,
            ),
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn two_children_in_the_same_branch_are_reported() {
        let diagnostics = check(&project(
            "gesture-same-branch",
            concat!(
                r#"<pan-gesture-handler>"#,
                r#"<block wx:if="{{a}}"><view/><view/></block>"#,
                r#"</pan-gesture-handler>"#,
            ),
        ));
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("2 个直接子元素"));
    }

    #[test]
    fn independent_conditions_are_counted_together() {
        // 两个独立的 `wx:if` 不互斥，运行时可能同时渲染。
        let diagnostics = check(&project(
            "gesture-independent",
            concat!(
                r#"<pan-gesture-handler>"#,
                r#"<view wx:if="{{a}}"/>"#,
                r#"<view wx:if="{{b}}"/>"#,
                r#"</pan-gesture-handler>"#,
            ),
        ));
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("2 个直接子元素"));
    }

    #[test]
    fn single_branch_under_gesture_handler_is_allowed() {
        let diagnostics = check(&project(
            "gesture-one-branch",
            concat!(
                r#"<pan-gesture-handler>"#,
                r#"<block wx:if="{{a}}"><view/></block>"#,
                r#"</pan-gesture-handler>"#,
            ),
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// 分支子节点数不相等时，按最多的一支计数：else 分支有两个子节点就是 2。
    #[test]
    fn branches_with_different_child_counts_take_the_largest() {
        let diagnostics = check(&project(
            "gesture-uneven-branches",
            concat!(
                r#"<pan-gesture-handler>"#,
                r#"<block wx:if="{{a}}"><view/></block>"#,
                r#"<block wx:else><view/><view/></block>"#,
                r#"</pan-gesture-handler>"#,
            ),
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("2 个直接子元素"));
    }

    /// 模板定义体里的手势组件只在实例化处渲染，真实子节点静态不可知，不判。
    #[test]
    fn gesture_handler_inside_inline_template_is_not_judged() {
        let diagnostics = check(&project(
            "gesture-template",
            r#"<template name="t"><horizontal-drag-gesture-handler><template is="row"/></horizontal-drag-gesture-handler></template>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// 两个不同的 `wx:if` 组各自只留一个子节点，但两组可以同时渲染 → 2。
    #[test]
    fn independent_branch_groups_add_up() {
        let diagnostics = check(&project(
            "gesture-two-groups",
            concat!(
                r#"<pan-gesture-handler>"#,
                r#"<block wx:if="{{a}}"><view/></block>"#,
                r#"<block wx:if="{{b}}"><view/></block>"#,
                r#"</pan-gesture-handler>"#,
            ),
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    #[test]
    fn other_components_are_not_counted() {
        let diagnostics = check(&project("view-multi", "<view><view/><view/><view/></view>"));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    // ---- scroll-view：refresher-default-style="none" 必须给自定义刷新节点 ----

    #[test]
    fn scroll_view_with_none_style_and_no_slot_child_is_reported() {
        let diagnostics = check(&project(
            "refresher-missing",
            r#"<scroll-view refresher-enabled="{{true}}" refresher-default-style="none"><view>内容</view></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, Severity::Warning);
        assert!(diagnostics[0].message.contains(r#"slot="refresher""#));
    }

    /// 没开 `refresher-enabled` 时 `refresher-default-style` 不产生刷新节点，不能报。
    #[test]
    fn scroll_view_without_refresher_enabled_is_not_reported() {
        let diagnostics = check(&project(
            "refresher-disabled",
            r#"<scroll-view refresher-default-style="none"><view>内容</view></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// `refresher-enabled="{{false}}"` 是动态值，无法区分真假 → 仍可能开启，继续报。
    #[test]
    fn scroll_view_with_statically_disabled_refresher_is_not_reported() {
        let diagnostics = check(&project(
            "refresher-static-false",
            r#"<scroll-view refresher-enabled="false" refresher-default-style="none"><view>内容</view></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn scroll_view_with_enabled_refresher_and_empty_body_is_reported() {
        let diagnostics = check(&project(
            "refresher-empty",
            r#"<scroll-view refresher-enabled refresher-default-style="none"/>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    #[test]
    fn scroll_view_with_none_style_and_slot_child_is_allowed() {
        let diagnostics = check(&project(
            "refresher-ok",
            concat!(
                r#"<scroll-view refresher-enabled="{{true}}" refresher-default-style="none">"#,
                r#"<view slot="refresher" class="custom-refresher">下拉刷新</view>"#,
                r#"<view>内容</view>"#,
                r#"</scroll-view>"#,
            ),
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn scroll_view_with_other_default_style_is_allowed() {
        let diagnostics = check(&project(
            "refresher-black",
            r#"<scroll-view refresher-enabled="{{true}}" refresher-default-style="black"><view>内容</view></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn scroll_view_with_dynamic_default_style_is_skipped() {
        let diagnostics = check(&project(
            "refresher-dynamic",
            r#"<scroll-view refresher-default-style="{{style}}"><view>内容</view></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn scroll_view_without_refresher_style_is_allowed() {
        let diagnostics = check(&project(
            "refresher-absent",
            r#"<scroll-view scroll-y><view>内容</view></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn slot_child_outside_direct_children_does_not_count() {
        let diagnostics = check(&project(
            "refresher-nested",
            concat!(
                r#"<scroll-view refresher-enabled refresher-default-style="none">"#,
                r#"<view><view slot="refresher">下拉刷新</view></view>"#,
                r#"</scroll-view>"#,
            ),
        ));
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn dynamic_slot_child_is_not_reported_as_missing() {
        let diagnostics = check(&project(
            "refresher-dynamic-slot",
            concat!(
                r#"<scroll-view refresher-enabled refresher-default-style="none">"#,
                "<view slot=\"{{slotName}}\">下拉刷新</view>",
                "</scroll-view>",
            ),
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    // ---- 容错 ----

    #[test]
    fn broken_file_does_not_stop_other_files() {
        let fixture = Fixture::new("children-broken");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/bad/bad","pages/good/good"],"renderer":"skyline"}"#,
            )
            .write("pages/bad/bad.json", "{}")
            .write("pages/bad/bad.wxml", "<view>{{ a</view>")
            .write("pages/good/good.json", "{}")
            .write("pages/good/good.wxml", "<pan-gesture-handler/>");

        let diagnostics = check(&fixture);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].path, "pages/good/good.wxml");
    }
}
