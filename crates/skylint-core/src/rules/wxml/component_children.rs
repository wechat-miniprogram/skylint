//! 组件对直接子节点的约束：只允许某类子节点、必须恰好一个子节点、条件必需的子节点。
//!
//! 约束写在 `data/component_children.txt` 里，Rust 侧只按约束类型解释表格。
//! 属性一律只认静态值（`WxmlAttribute::is_static`），`{{}}` 插值跳过——静态阶段
//! 无法确定实际取值，宁可漏报。
//!
//! 子节点按 `input/wxml.rs` 的摊平语义计数：`<block>` / `wx:for` / `wx:if` 不产生
//! 自己的层级，所以同一个 `wx:if` 的 if 与 else 分支会同时挂在一个父节点下。
//! `exactly:<n>` 数的是「能同时渲染的子元素」的最大值：同一个 `wx:if` 的不同分支互斥，
//! 取各分支里最多的那一支；不同分支组之间相加。真实工程 `miniprogram-skit_1` 的
//! `components/index-block/index-block.wxml:62,139` 就在
//! `<horizontal-drag-gesture-handler>` 里用 `wx:if` / `wx:else` 各包一个 `<swiper>`；
//! 按摊平结果直接计数会把它误报成 2 个子节点。

use crate::diag::{Diagnostic, Severity};
use crate::input::wxml::{WxmlDoc, WxmlElement};
use crate::project::Project;
use crate::rules::data_table::{self, Columns};
use crate::rules::{Rule, RuleMeta};

use super::{element_diagnostic, load_doc, wxml_files};
use super::branch::{element_children, max_rendered_children};

pub const ID: &str = "skyline/component-children";

/// 子节点约束表。
const CONSTRAINTS_DATA: &str = include_str!("../data/component_children.txt");

/// 表格「约束」列的第一段决定怎么读，其余部分是参数。
enum Constraint {
    /// 直接元素子节点只能是这些标签。`text` 是特殊值，表示文本节点也算合法。
    Only(Vec<&'static str>),
    /// 直接元素子节点个数必须正好是 n；文本子节点不计。
    Exactly(usize),
    /// 自身条件成立时，直接子节点里必须有 `child_name=child_value`。
    RequiresChild {
        child_name: &'static str,
        child_value: &'static str,
        /// 多个条件必须同时成立（`@` 分隔）。
        conditions: Vec<Trigger>,
    },
}

/// 一个触发条件。`value` 为 `None` 时只看属性是否存在（未被静态置为假）——
/// WXML 布尔属性的写法太多（`refresher-enabled`、`refresher-enabled=""`、`"{{true}}"`），
/// 静默阶段无法区分 `{{true}}` 与 `{{refreshing}}`，所以只排除「确实没开」。
struct Trigger {
    name: &'static str,
    value: Option<&'static str>,
}

struct Row {
    tag: &'static str,
    constraint: Constraint,
    severity: Severity,
    help: &'static str,
}

fn rows() -> Vec<Row> {
    data_table::read(CONSTRAINTS_DATA, data_table::split_on('\t'), |_| {
        Columns::new(4, "`组件 | 约束 | 级别 | 说明` 四列")
    })
    .expect("组件子节点约束表格式错误")
    .rows
    .into_iter()
    .filter_map(parse_row)
    .collect()
}

fn parse_row(row: data_table::Row<'static>) -> Option<Row> {
    let (tag, raw, severity, help) = (row.fields[0], row.fields[1], row.fields[2], row.fields[3]);
    let severity = match severity {
        "error" => Severity::Error,
        "warning" => Severity::Warning,
        "info" => Severity::Info,
        _ => return None,
    };
    Some(Row {
        tag,
        constraint: parse_constraint(raw)?,
        severity,
        help,
    })
}

fn parse_constraint(raw: &'static str) -> Option<Constraint> {
    let (kind, rest) = raw.split_once(':')?;
    match kind {
        "only" => Some(Constraint::Only(rest.split('|').collect())),
        "exactly" => rest.parse().ok().map(Constraint::Exactly),
        "requires-child" => {
            let (child, triggers) = rest.split_once('@')?;
            let (child_name, child_value) = child.split_once('=')?;
            let conditions = triggers
                .split('@')
                .map(|trigger| {
                    let trigger = trigger.trim();
                    let (name, value) = trigger.split_once('=').unwrap_or((trigger, ""));
                    Trigger {
                        name: name.trim(),
                        value: (!value.trim().is_empty()).then(|| value.trim()),
                    }
                })
                .collect();
            Some(Constraint::RequiresChild {
                child_name: child_name.trim(),
                child_value: child_value.trim(),
                conditions,
            })
        }
        _ => None,
    }
}

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

fn trigger_text(trigger: &Trigger) -> String {
    match trigger.value {
        Some(value) => format!(" {}=\"{value}\"", trigger.name),
        None => format!(" {}", trigger.name),
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
            if count == *expected {
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
            child_name,
            child_value,
            conditions,
        } => {
            // 任一触发条件不成立就不报；动态值无法确认，一律按不成立处理。
            let satisfied = conditions.iter().all(|condition| match condition.value {
                Some(value) => matches!(check_attr(element, condition.name, value), AttrCheck::Ok),
                None => !attr_is_off(element, condition.name),
            });
            if !satisfied {
                return Vec::new();
            }
            let mut unknown = false;
            for child in element_children(element) {
                match check_attr(child, child_name, child_value) {
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
                conditions.first().map(trigger_text).unwrap_or_default(),
                child_name,
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
        let rows = rows();
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
                for row in rows.iter().filter(|row| row.tag == element.tag) {
                    diagnostics.extend(violations(&doc, element, row, path));
                }
            }
        }
        diagnostics
    }
}

#[cfg(test)]
mod tests {
    use crate::rules::testutil::wxml::{Fixture, check_rule, project};
    use super::ComponentChildren;
    use super::rows;
    use crate::Severity;

    fn check(fixture: &Fixture) -> Vec<crate::Diagnostic> {
        check_rule(fixture, &ComponentChildren)
    }

    // ---- 数据表 ----

    #[test]
    fn data_table_is_well_formed() {
        let rows = rows();
        assert_eq!(rows.len(), 10);
        assert!(
            rows.iter()
                .all(|row| !row.tag.is_empty() && !row.help.is_empty())
        );
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
