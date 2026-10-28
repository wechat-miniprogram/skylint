//! `scroll-view` 家族组件之间的结构约束：挂载位置、子节点顺序与唯一性、必需属性、
//! 关联容器。约束直接写在下方 `CONSTRAINTS` 里，按子标签查表。
//!
//! 表格表达不了的否定约束（nested-scroll-header 不能在 body 内）留在代码里。
//!
//! # 依据（每条都能指到原句）
//!
//! - sticky-section：scroll/sticky.md「它们必须在 <scroll-view type="custom"> 模式下使用」，
//!   MUST 1「必须在 type="custom" 的 scroll-view 中使用」，NEVER 1「不要在 type="list" 的 scroll-view 中使用」。
//! - list-view / grid-view：scroll/list-grid-view.md「它们必须作为 <scroll-view type="custom"> 或
//!   <sticky-section> 的直接子节点使用」，NEVER 1「不要在 type="list" 的 scroll-view 中使用 list-view/grid-view」。
//! - nested-scroll-header / nested-scroll-body：scroll/nested-scroll.md MUST 1
//!   「外层 scroll-view 必须设置 type="nested"」。
//! - sticky-header 顺序与唯一性：scroll/sticky.md MUST 2「sticky-header 必须是 sticky-section 的第一个
//!   子元素」（:153）、MUST 3「每个 sticky-section 只能有一个 sticky-header」（:154）。
//!   只加「在 sticky-section 内时」的条件约束，挂载位置仍不约束（见下方整条撤销的记录）。
//! - nested-scroll-body 唯一：scroll/nested-scroll.md MUST 4「外层只能有一个 nested-scroll-body」（:166）、
//!   NEVER 2「不要有多个 nested-scroll-body」。
//! - nested-scroll-body 必需：nested-scroll.md:38 只渲染第一个 body；
//!   skyline_scroll_view.dart:1559-1562 用 `firstWhere((child) => child is NestedScrollBody)`
//!   且没有 orElse，缺 body 时直接抛 StateError（`?? SizedBox()` 是死代码）。
//! - nested-scroll-header 与 body 的顺序无关：skyline_scroll_view.dart:1551-1554 用
//!   `children.whereType<NestedScrollHeader>()` 收集，不依赖声明顺序。
//! - 只渲染第一个子节点：nested-scroll.md:29（header）、:38（body）。多写的会被静默丢弃，故为 warning。
//! - list-builder 必需属性：scroll/list-grid-view.md MUST 2「使用 list-builder 时必须指定
//!   child-height（定高模式）」（:271）。grid-view 的 cross-axis-count 不是必填：
//!   grid_view_shadow_node.h:38 默认值为 2，省略时按 2 列渲染。
//! - list-builder type：list-grid-view.md:155「type | string | static | 列表模式：static（定高）/ dynamic（不定高）」。
//! - grid-view type：list-grid-view.md:65「type | string | aligned / masonry」。
//! - 内层 scroll-view 关联容器：nested-scroll.md MUST 2「内层 scroll-view 必须设置
//!   associative-container="nested-scroll-view"」（:164）、NEVER 3「不要忘记给内层 scroll-view 设置」。作用域取
//!   nested-scroll-body：文档示例里内层 scroll-view 在 body 内的 swiper 下（:210-:224），而 header 内的
//!   scroll-view 不参与嵌套滚动（sticky 里的横向 tab 列表就属于这种）。
//! - draggable-sheet 内 scroll-view 关联容器：draggable-sheet.md:50、:342
//!   「内部的 scroll-view 必须设置 associative-container="draggable-sheet"」。
//!
//! # 文档冲突记录
//!
//! skyline-overview/references/introduction/component-support.md 写
//! 「list-view | 列表布局容器，作为 scroll-view type="list" 的直接子节点」，与
//! scroll/list-grid-view.md（必须 custom）以及 SKILL.md MUST 4 冲突。这里按更具体、
//! 且被 scroll-view.md type 表（custom 支持 list-view/grid-view/sticky-section）
//! 印证的 list-grid-view.md 实现。
//!
//! sticky-header 的挂载位置（整条撤销）：scroll/sticky.md NEVER 2 写「不要将
//! sticky-header 放在 sticky-section 之外」，但 scroll/scroll-view.md:54 在 custom 模式的
//! 「支持的子组件」里明确列出 `sticky-section` / `sticky-header`，即 sticky-header 也可以
//! 作为 scroll-view 的直接子节点。生产工程 miniprogram-skit_1 的 rank.wxml 正是
//! <nested-scroll-header><sticky-header> 的写法且在跑。两份文档互相冲突，按「文档没写清
//! 或互相矛盾就不报」的原则，不对 sticky-header 的挂载位置做任何约束。

use crate::diag::{Diagnostic, Severity};
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};

use super::structure::{self, Condition, Constraint, Relation, Target};
use super::{element_diagnostic, is_builtin, visit, walk};

pub const ID: &str = "skyline/scroll-view-family";

const CUSTOM_SCROLL: Target = Target {
    tag: "scroll-view",
    attr: Some(("type", "custom")),
};
const NESTED_SCROLL: Target = Target {
    tag: "scroll-view",
    attr: Some(("type", "nested")),
};
const STICKY_SECTION: Target = Target {
    tag: "sticky-section",
    attr: None,
};
/// 嵌套滚动的头部槽也是 sliver 列表（`skyline_scroll_view.dart:1551-1556` 交给
/// `NestedScrollView.headerSliverBuilder`），放 `list-view` / `sticky-section` 合法。
const NESTED_HEADER: Target = Target {
    tag: "nested-scroll-header",
    attr: None,
};

/// scroll-view 家族结构约束表。
const CONSTRAINTS: &[Constraint] = &[
    Constraint {
        tag: "sticky-section",
        relation: Relation::Ancestor(&[CUSTOM_SCROLL, NESTED_HEADER]),
        severity: Severity::Error,
        help: "把 `<sticky-section>` 放进 `<scroll-view type=\"custom\">` 或 `<nested-scroll-header>`，type 不能是 list/nested",
    },
    Constraint {
        tag: "list-view",
        relation: Relation::Direct(&[CUSTOM_SCROLL, STICKY_SECTION, NESTED_HEADER]),
        severity: Severity::Error,
        help: "把 `<list-view>` 移为 `<scroll-view type=\"custom\">`、`<sticky-section>` 或 `<nested-scroll-header>` 的直接子节点",
    },
    Constraint {
        tag: "grid-view",
        relation: Relation::Direct(&[CUSTOM_SCROLL, STICKY_SECTION, NESTED_HEADER]),
        severity: Severity::Error,
        help: "把 `<grid-view>` 移为 `<scroll-view type=\"custom\">`、`<sticky-section>` 或 `<nested-scroll-header>` 的直接子节点",
    },
    Constraint {
        tag: "nested-scroll-header",
        relation: Relation::Ancestor(&[NESTED_SCROLL]),
        severity: Severity::Error,
        help: "把 `<nested-scroll-header>` 放在外层 `<scroll-view type=\"nested\">` 内",
    },
    Constraint {
        tag: "nested-scroll-body",
        relation: Relation::Ancestor(&[NESTED_SCROLL]),
        severity: Severity::Error,
        help: "把 `<nested-scroll-body>` 放在外层 `<scroll-view type=\"nested\">` 内",
    },
    Constraint {
        tag: "sticky-header",
        relation: Relation::FirstChild(&[STICKY_SECTION]),
        severity: Severity::Error,
        help: "把 `<sticky-header>` 移到 `<sticky-section>` 的第一个元素子节点位置",
    },
    Constraint {
        tag: "sticky-header",
        relation: Relation::Unique(&[STICKY_SECTION]),
        severity: Severity::Error,
        help: "一个 `<sticky-section>` 只保留一个 `<sticky-header>`",
    },
    Constraint {
        tag: "nested-scroll-body",
        relation: Relation::Unique(&[NESTED_SCROLL]),
        severity: Severity::Error,
        help: "外层 `<scroll-view type=\"nested\">` 只保留一个 `<nested-scroll-body>`",
    },
    Constraint {
        tag: "scroll-view",
        relation: Relation::RequiresChildTag {
            child_tag: "nested-scroll-body",
            own: &[Condition::Equals("type", "nested")],
        },
        severity: Severity::Error,
        help: "在 `<scroll-view type=\"nested\">` 内加一个 `<nested-scroll-body>`",
    },
    Constraint {
        tag: "nested-scroll-header",
        relation: Relation::RendersFirst,
        severity: Severity::Warning,
        help: "把头部内容收进一个直接子节点（例如一个 `<view>`）",
    },
    Constraint {
        tag: "nested-scroll-body",
        relation: Relation::RendersFirst,
        severity: Severity::Warning,
        help: "把主体内容收进一个直接子节点（例如一个 `<view>`）",
    },
    Constraint {
        tag: "list-builder",
        relation: Relation::RequiresAttr {
            name: "child-height",
            unless: Some(("type", "dynamic")),
        },
        severity: Severity::Error,
        help: "在 `<list-builder>` 上声明列表项高度 `child-height`（定高模式必填）",
    },
    Constraint {
        tag: "list-builder",
        relation: Relation::AllowedValues {
            name: "type",
            values: &["static", "dynamic"],
        },
        severity: Severity::Error,
        help: "`type` 只能是 `static`（定高）或 `dynamic`（不定高）",
    },
    Constraint {
        tag: "grid-view",
        relation: Relation::AllowedValues {
            name: "type",
            values: &["aligned", "masonry"],
        },
        severity: Severity::Error,
        help: "`type` 只能是 `aligned`（网格）或 `masonry`（瀑布流）",
    },
    Constraint {
        tag: "scroll-view",
        relation: Relation::Within {
            container: Target {
                tag: "nested-scroll-body",
                attr: None,
            },
            name: "associative-container",
            value: "nested-scroll-view",
        },
        severity: Severity::Error,
        help: "给 `<nested-scroll-body>` 内的 `<scroll-view>` 加上 `associative-container=\"nested-scroll-view\"`",
    },
    Constraint {
        tag: "scroll-view",
        relation: Relation::Within {
            container: Target {
                tag: "draggable-sheet",
                attr: None,
            },
            name: "associative-container",
            value: "draggable-sheet",
        },
        severity: Severity::Warning,
        help: "给 `<draggable-sheet>` 内的 `<scroll-view>` 加上 `associative-container=\"draggable-sheet\"`",
    },
];

pub struct ScrollViewFamily;

impl Rule for ScrollViewFamily {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Error,
            description: "scroll-view 家族组件的结构约束（挂载位置 / 顺序 / 唯一性 / 必需属性 / 关联容器）",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit(project, |doc, out| {
            walk(doc, |element, ancestors| {
                structure::check_element(ID, doc, element, ancestors, CONSTRAINTS, doc.path(), out);

                // nested-scroll.md NEVER 1「不要在 nested-scroll-body 下面放 nested-scroll-header」。
                if !element.in_template()
                    && element.tag == "nested-scroll-header"
                    && ancestors
                        .iter()
                        .rev()
                        .take_while(|ancestor| is_builtin(&ancestor.tag))
                        .any(|ancestor| ancestor.tag == "nested-scroll-body")
                {
                    out.push(
                        element_diagnostic(
                            ID,
                            doc,
                            element,
                            doc.path(),
                            Severity::Error,
                            "`<nested-scroll-header>` 不能放在 `<nested-scroll-body>` 内，头部不会随外层滚动",
                        )
                        .help("把 `<nested-scroll-header>` 移到 `<nested-scroll-body>` 之前，与它平级"),
                    );
                }
            });
        })
    }
}

#[cfg(test)]
mod tests {
    use super::ScrollViewFamily;
    use crate::Severity;
    use crate::rules::testutil::wxml::{Fixture, check_rule, project};

    fn check(fixture: &Fixture) -> Vec<crate::Diagnostic> {
        check_rule(fixture, &ScrollViewFamily)
    }

    /// 被 `<include>` 的片段：根节点插到哪由实例化处决定，不能报「位于文件顶层」。
    #[test]
    fn included_fragment_root_is_not_judged_for_position() {
        let fixture = project(
            "include-fragment",
            r#"<scroll-view type="custom"><include src="./row"/></scroll-view>"#,
        );
        fixture.write("pages/index/row.wxml", "<list-view/>");
        let loaded = fixture.project();
        assert!(
            loaded
                .template_files
                .iter()
                .any(|path| path.ends_with("row.wxml")),
            "片段应当进入模板闭包：{:?}",
            loaded.template_files
        );
        let diagnostics = check(&fixture);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");

        // 同样的节点写进页面本体就是真错。
        let page = project(
            "include-page",
            r#"<scroll-view type="custom"><list-view/></scroll-view><list-view/>"#,
        );
        let diagnostics = check(&page);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(diagnostics[0].message.contains("位于文件顶层"));
    }

    // ---- 约束表 ----

    // ---- sticky-section ----

    #[test]
    fn sticky_section_in_list_scroll_is_reported() {
        let diagnostics = check(&project(
            "sticky-section-list",
            r#"<scroll-view type="list"><sticky-section/></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert!(diagnostics[0].message.contains(r#"type="list""#));
    }

    #[test]
    fn sticky_section_without_scroll_view_is_reported() {
        let diagnostics = check(&project(
            "sticky-section-view",
            "<view><sticky-section/></view>",
        ));
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("不在该容器中"));
    }

    #[test]
    fn sticky_section_in_scroll_view_without_type_is_reported() {
        let diagnostics = check(&project(
            "sticky-section-no-type",
            "<scroll-view><sticky-section/></scroll-view>",
        ));
        assert_eq!(diagnostics.len(), 1);
    }

    // ---- list-view / grid-view ----

    #[test]
    fn list_view_in_plain_view_is_reported() {
        let diagnostics = check(&project("list-view-view", "<view><list-view/></view>"));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert!(diagnostics[0].message.contains("sticky-section"));
    }

    #[test]
    fn list_view_in_list_scroll_is_reported() {
        let diagnostics = check(&project(
            "list-view-list",
            r#"<scroll-view type="list"><list-view/></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains(r#"type="list""#));
    }

    #[test]
    fn list_view_at_root_is_reported() {
        let diagnostics = check(&project("list-view-root", "<list-view/>"));
        assert_eq!(diagnostics.len(), 1);
    }

    /// 模板定义体只在实例化处渲染，真实容器静态不可知：不能当成「文件顶层」报。
    #[test]
    fn list_view_inside_inline_template_is_skipped() {
        let diagnostics = check(&project(
            "list-view-template",
            r#"<template name="t"><list-view/></template>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn grid_view_in_plain_view_is_reported() {
        let diagnostics = check(&project(
            "grid-view-view",
            r#"<view><grid-view cross-axis-count="2"/></view>"#,
        ));
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("grid-view"));
    }

    #[test]
    fn grid_view_in_list_scroll_is_reported() {
        let diagnostics = check(&project(
            "grid-view-list",
            r#"<scroll-view type="list"><grid-view cross-axis-count="2"/></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1);
    }

    // ---- nested scroll ----

    #[test]
    fn nested_header_in_list_scroll_is_reported() {
        let diagnostics = check(&project(
            "nested-header-list",
            r#"<scroll-view type="list"><nested-scroll-header/></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains(r#"type="list""#));
    }

    #[test]
    fn nested_header_without_scroll_view_is_reported() {
        let diagnostics = check(&project(
            "nested-header-view",
            "<view><nested-scroll-header/></view>",
        ));
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn nested_body_in_custom_scroll_is_reported() {
        let diagnostics = check(&project(
            "nested-body-custom",
            r#"<scroll-view type="custom"><nested-scroll-body/></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("nested"));
    }

    #[test]
    fn nested_body_without_scroll_view_is_reported() {
        let diagnostics = check(&project(
            "nested-body-view",
            "<view><nested-scroll-body/></view>",
        ));
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn nested_header_inside_body_is_reported() {
        let diagnostics = check(&project(
            "nested-header-body",
            r#"<scroll-view type="nested"><nested-scroll-body><nested-scroll-header/></nested-scroll-body></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("nested-scroll-body"));
    }

    #[test]
    fn nested_header_deep_inside_body_is_reported() {
        let diagnostics = check(&project(
            "nested-header-body-deep",
            r#"<scroll-view type="nested"><nested-scroll-body><view><nested-scroll-header/></view></nested-scroll-body></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1);
    }

    // ---- draggable-sheet ----

    #[test]
    fn draggable_sheet_scroll_without_container_is_reported() {
        let diagnostics = check(&project(
            "draggable-missing",
            r#"<draggable-sheet><scroll-view type="list" scroll-y/></draggable-sheet>"#,
        ));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, Severity::Warning);
        assert!(diagnostics[0].message.contains("associative-container"));
    }

    #[test]
    fn draggable_sheet_scroll_with_wrong_container_is_reported() {
        let diagnostics = check(&project(
            "draggable-wrong",
            r#"<draggable-sheet><scroll-view type="list" associative-container="nested-scroll-view"/></draggable-sheet>"#,
        ));
        assert_eq!(diagnostics.len(), 1);
    }

    // ---- 负例 ----

    /// 文档冲突，故意不报：sticky.md NEVER 2 要求 sticky-header 必须在 sticky-section 内，
    /// 但 scroll-view.md:54 的 custom 模式「支持的子组件」里同时列了 sticky-header，
    /// 生产工程也按后者在 nested-scroll-header 内直接使用 sticky-header。
    #[test]
    fn sticky_header_outside_section_is_not_reported_due_to_conflicting_docs() {
        let diagnostics = check(&project(
            "sticky-conflict",
            concat!(
                r#"<scroll-view type="custom"><sticky-header>标题</sticky-header></scroll-view>"#,
                r#"<scroll-view type="nested"><nested-scroll-header><sticky-header>标题</sticky-header></nested-scroll-header><nested-scroll-body><view/></nested-scroll-body></scroll-view>"#,
                "<view><sticky-header>标题</sticky-header></view>",
            ),
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn sticky_section_under_custom_scroll_is_allowed() {
        let diagnostics = check(&project(
            "sticky-custom",
            r#"<scroll-view type="custom"><sticky-section/></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn dynamic_scroll_view_type_is_skipped() {
        let diagnostics = check(&project(
            "sticky-dynamic",
            r#"<scroll-view type="{{t}}"><sticky-section><sticky-header>标题</sticky-header></sticky-section></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn list_view_under_custom_scroll_is_allowed() {
        let diagnostics = check(&project(
            "list-custom",
            r#"<scroll-view type="custom"><list-view><view/></list-view></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn list_view_under_sticky_section_is_allowed() {
        let diagnostics = check(&project(
            "list-sticky",
            r#"<scroll-view type="custom"><sticky-section><sticky-header>标题</sticky-header><list-view/></sticky-section></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn list_view_under_block_is_allowed() {
        let diagnostics = check(&project(
            "list-block",
            r#"<scroll-view type="custom"><block wx:for="{{list}}"><list-view/></block></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn list_view_under_custom_component_is_skipped() {
        let diagnostics = check(&project(
            "list-custom-comp",
            "<my-box><list-view/></my-box>",
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn grid_view_under_custom_scroll_is_allowed() {
        let diagnostics = check(&project(
            "grid-custom",
            r#"<scroll-view type="custom"><grid-view type="aligned" cross-axis-count="2"/></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn grid_view_under_sticky_section_is_allowed() {
        let diagnostics = check(&project(
            "grid-sticky",
            r#"<scroll-view type="custom"><sticky-section><sticky-header>标题</sticky-header><grid-view cross-axis-count="2"/></sticky-section></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn nested_parts_under_nested_scroll_are_allowed() {
        let diagnostics = check(&project(
            "nested-ok",
            r#"<scroll-view type="nested"><nested-scroll-header><view/></nested-scroll-header><nested-scroll-body><scroll-view type="list" associative-container="nested-scroll-view"/></nested-scroll-body></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn nested_parts_under_block_are_allowed() {
        let diagnostics = check(&project(
            "nested-block",
            r#"<scroll-view type="nested"><block wx:if="{{ok}}"><nested-scroll-header/></block><block wx:else><nested-scroll-body/></block></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn nested_parts_with_dynamic_type_are_skipped() {
        let diagnostics = check(&project(
            "nested-dynamic",
            r#"<scroll-view type="{{t}}"><nested-scroll-header/><nested-scroll-body/></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn draggable_sheet_scroll_with_container_is_allowed() {
        let diagnostics = check(&project(
            "draggable-ok",
            r#"<draggable-sheet><scroll-view type="list" associative-container="draggable-sheet"/></draggable-sheet>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn scroll_view_outside_draggable_sheet_is_allowed() {
        let diagnostics = check(&project(
            "draggable-outside",
            r#"<view><scroll-view type="list"/></view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn plain_list_scroll_page_is_allowed() {
        let diagnostics = check(&project(
            "plain",
            r#"<scroll-view type="list"><view wx:for="{{list}}">{{item}}</view></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    // ---- 页面 + 组件 ----

    #[test]
    fn pages_and_components_are_both_checked() {
        let fixture = Fixture::new("page-and-component");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                r#"{"usingComponents":{"my-box":"/components/box/box"}}"#,
            )
            .write(
                "pages/index/index.wxml",
                r#"<scroll-view type="list"><sticky-section/></scroll-view>"#,
            )
            .write("components/box/box.json", r#"{"component":true}"#)
            .write("components/box/box.wxml", "<view><list-view/></view>");

        let diagnostics = check(&fixture);
        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
        let paths: Vec<_> = diagnostics.iter().map(|d| d.path.as_str()).collect();
        assert!(paths.contains(&"pages/index/index.wxml"));
        assert!(paths.contains(&"components/box/box.wxml"));
    }

    // ---- 容错 ----

    #[test]
    fn broken_file_does_not_stop_other_files() {
        let fixture = Fixture::new("scroll-broken");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/bad/bad","pages/good/good"],"renderer":"skyline"}"#,
            )
            .write("pages/bad/bad.json", "{}")
            .write("pages/bad/bad.wxml", "<view>{{ a</view>")
            .write("pages/good/good.json", "{}")
            .write("pages/good/good.wxml", "<view><list-view/></view>");

        let diagnostics = check(&fixture);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].path, "pages/good/good.wxml");
    }

    // ---- 顺序：sticky-header 必须是 sticky-section 的第一个元素子节点 ----

    #[test]
    fn sticky_header_not_first_in_section_is_reported() {
        let diagnostics = check(&project(
            "sticky-first-view",
            r#"<scroll-view type="custom"><sticky-section><view/><sticky-header>标题</sticky-header></sticky-section></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert!(diagnostics[0].message.contains("第一个元素子节点"));
    }

    #[test]
    fn sticky_header_after_list_view_in_section_is_reported() {
        let diagnostics = check(&project(
            "sticky-first-list",
            r#"<scroll-view type="custom"><sticky-section><list-view><view/></list-view><sticky-header>标题</sticky-header></sticky-section></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("<list-view>"));
    }

    #[test]
    fn sticky_header_first_in_section_is_allowed() {
        let diagnostics = check(&project(
            "sticky-first-ok",
            r#"<scroll-view type="custom"><sticky-section><sticky-header>标题</sticky-header><list-view><view/></list-view></sticky-section></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn sticky_header_first_after_if_block_is_allowed() {
        // block 已摊平：header 仍是 sticky-section 的第一个元素子节点。
        let diagnostics = check(&project(
            "sticky-first-if",
            r#"<scroll-view type="custom"><sticky-section><block wx:if="{{ok}}"><sticky-header>标题</sticky-header></block><list-view><view/></list-view></sticky-section></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn sticky_header_first_after_for_block_is_allowed() {
        let diagnostics = check(&project(
            "sticky-first-for",
            r#"<scroll-view type="custom"><sticky-section><block wx:for="{{list}}"><sticky-header>标题</sticky-header></block><view/></sticky-section></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// 这正是当初撤销「必须在 sticky-section 内」的场景：两种直接子节点都要放过。
    #[test]
    fn sticky_header_direct_child_of_custom_scroll_is_allowed() {
        let diagnostics = check(&project(
            "sticky-first-scroll",
            r#"<scroll-view type="custom"><sticky-header>标题</sticky-header><sticky-section><sticky-header>标题</sticky-header></sticky-section></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// 嵌套滚动的头部槽也是 sliver 列表，list-view / sticky-section 放这里合法。
    #[test]
    fn slivers_are_allowed_inside_nested_scroll_header() {
        let diagnostics = check(&project(
            "nested-header-slivers",
            concat!(
                r#"<scroll-view type="nested">"#,
                r#"<nested-scroll-header><list-view/></nested-scroll-header>"#,
                r#"<nested-scroll-body><view/></nested-scroll-body>"#,
                r#"</scroll-view>"#,
                r#"<scroll-view type="nested">"#,
                r#"<nested-scroll-header><sticky-section><sticky-header>x</sticky-header></sticky-section></nested-scroll-header>"#,
                r#"<nested-scroll-body><view/></nested-scroll-body>"#,
                r#"</scroll-view>"#,
            ),
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    // ---- 唯一性：每个 sticky-section 最多一个 sticky-header ----

    #[test]
    fn two_sticky_headers_in_section_is_reported() {
        let diagnostics = check(&project(
            "sticky-unique",
            r#"<scroll-view type="custom"><sticky-section><sticky-header>一</sticky-header><sticky-header>二</sticky-header></sticky-section></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert!(diagnostics[0].message.contains("2 个 `<sticky-header>`"));
        assert!(diagnostics[0].message.contains("只能有一个"));
    }

    #[test]
    fn three_sticky_headers_in_section_report_each_extra() {
        let diagnostics = check(&project(
            "sticky-unique-three",
            r#"<scroll-view type="custom"><sticky-section><sticky-header>一</sticky-header><sticky-header>二</sticky-header><sticky-header>三</sticky-header></sticky-section></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
        assert!(
            diagnostics
                .iter()
                .all(|d| d.message.contains("3 个 `<sticky-header>`"))
        );
    }

    #[test]
    fn single_sticky_header_is_allowed() {
        let diagnostics = check(&project(
            "sticky-unique-one",
            r#"<scroll-view type="custom"><sticky-section><sticky-header>标题</sticky-header></sticky-section></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn sticky_headers_in_separate_sections_are_allowed() {
        let diagnostics = check(&project(
            "sticky-unique-separate",
            r#"<scroll-view type="custom"><sticky-section><sticky-header>一</sticky-header></sticky-section><sticky-section><sticky-header>二</sticky-header></sticky-section></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    // ---- 唯一性：外层最多一个 nested-scroll-body ----

    #[test]
    fn two_nested_bodies_is_reported() {
        let diagnostics = check(&project(
            "nested-body-unique",
            r#"<scroll-view type="nested"><nested-scroll-body><view/></nested-scroll-body><nested-scroll-body><view/></nested-scroll-body></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert!(
            diagnostics[0]
                .message
                .contains("2 个 `<nested-scroll-body>`")
        );
    }

    #[test]
    fn three_nested_bodies_report_each_extra() {
        let diagnostics = check(&project(
            "nested-body-unique-three",
            concat!(
                r#"<scroll-view type="nested"><nested-scroll-body><view/></nested-scroll-body>"#,
                r#"<nested-scroll-body><view/></nested-scroll-body>"#,
                r#"<nested-scroll-body><view/></nested-scroll-body></scroll-view>"#,
            ),
        ));
        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
    }

    #[test]
    fn nested_bodies_in_separate_scrolls_are_allowed() {
        let diagnostics = check(&project(
            "nested-body-unique-separate",
            concat!(
                r#"<scroll-view type="nested"><nested-scroll-body><view/></nested-scroll-body></scroll-view>"#,
                r#"<scroll-view type="nested"><nested-scroll-body><view/></nested-scroll-body></scroll-view>"#,
            ),
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn nested_bodies_with_dynamic_type_are_skipped() {
        let diagnostics = check(&project(
            "nested-body-unique-dynamic",
            r#"<scroll-view type="{{t}}"><nested-scroll-body><view/></nested-scroll-body><nested-scroll-body><view/></nested-scroll-body></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    // ---- 顺序无关：nested-scroll-header 由 whereType 收集 ----

    #[test]
    fn nested_header_after_body_is_allowed() {
        let diagnostics = check(&project(
            "nested-before",
            r#"<scroll-view type="nested"><nested-scroll-body><view/></nested-scroll-body><nested-scroll-header><view/></nested-scroll-header></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn nested_header_after_body_with_sibling_between_is_allowed() {
        let diagnostics = check(&project(
            "nested-before-between",
            r#"<scroll-view type="nested"><nested-scroll-body><view/></nested-scroll-body><view/><nested-scroll-header><view/></nested-scroll-header></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn nested_header_before_body_is_allowed() {
        let diagnostics = check(&project(
            "nested-before-ok",
            r#"<scroll-view type="nested"><nested-scroll-header><view/></nested-scroll-header><nested-scroll-body><view/></nested-scroll-body></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn nested_body_without_header_is_allowed() {
        let diagnostics = check(&project(
            "nested-before-no-header",
            r#"<scroll-view type="nested"><nested-scroll-body><view/></nested-scroll-body></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn nested_header_without_body_is_reported() {
        let diagnostics = check(&project(
            "nested-before-no-body",
            r#"<scroll-view type="nested"><nested-scroll-header><view/></nested-scroll-header></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert!(diagnostics[0].message.contains("nested-scroll-body"));
        assert!(
            diagnostics[0]
                .help
                .as_deref()
                .unwrap()
                .contains("nested-scroll-body")
        );
    }

    #[test]
    fn nested_scroll_without_body_is_reported() {
        let diagnostics = check(&project(
            "nested-no-body",
            r#"<scroll-view type="nested"><view/></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("nested-scroll-body"));
    }

    #[test]
    fn nested_body_must_be_a_direct_child() {
        let diagnostics = check(&project(
            "nested-body-indirect",
            r#"<scroll-view type="nested"><view><nested-scroll-body/></view></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("nested-scroll-body"));
    }

    // ---- 只渲染第一个直接子节点 ----

    #[test]
    fn nested_header_with_multiple_children_is_reported() {
        let diagnostics = check(&project(
            "nested-header-multi",
            r#"<scroll-view type="nested"><nested-scroll-header><view/><view/></nested-scroll-header><nested-scroll-body><view/></nested-scroll-body></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].severity, Severity::Warning);
        assert!(diagnostics[0].message.contains("2 个直接子元素"));
    }

    #[test]
    fn nested_body_with_multiple_children_is_reported() {
        let diagnostics = check(&project(
            "nested-body-multi",
            r#"<scroll-view type="nested"><nested-scroll-header><view/></nested-scroll-header><nested-scroll-body><view/><view/></nested-scroll-body></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].severity, Severity::Warning);
        assert!(diagnostics[0].message.contains("nested-scroll-body"));
    }

    #[test]
    fn nested_parts_with_single_child_are_allowed() {
        let diagnostics = check(&project(
            "nested-parts-single",
            r#"<scroll-view type="nested"><nested-scroll-header><view/></nested-scroll-header><nested-scroll-body><view/></nested-scroll-body></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn nested_body_with_nested_grandchildren_is_allowed() {
        // 只数直接元素子节点，孙节点不算。
        let diagnostics = check(&project(
            "nested-body-grandchildren",
            r#"<scroll-view type="nested"><nested-scroll-header><view><view/><view/></view></nested-scroll-header><nested-scroll-body><view><view/><view/></view></nested-scroll-body></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    // ---- 必需属性 ----

    #[test]
    fn list_builder_without_child_height_is_reported() {
        let diagnostics = check(&project(
            "list-builder-attr",
            "<view><list-builder/></view>",
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert!(diagnostics[0].message.contains("child-height"));
    }

    #[test]
    fn list_builder_with_type_but_without_child_height_is_reported() {
        let diagnostics = check(&project(
            "list-builder-attr-static",
            r#"<view><list-builder type="static"/></view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    #[test]
    fn list_builder_with_child_height_is_allowed() {
        let diagnostics = check(&project(
            "list-builder-attr-ok",
            r#"<view><list-builder child-height="80"/></view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn list_builder_with_dynamic_child_height_is_skipped() {
        let diagnostics = check(&project(
            "list-builder-attr-dynamic",
            r#"<view><list-builder child-height="{{h}}"/></view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn grid_view_without_cross_axis_count_is_allowed() {
        // grid_view_shadow_node.h:38：cross_axis_count_ 默认 2，省略即可用。
        let diagnostics = check(&project(
            "grid-view-attr",
            r#"<scroll-view type="custom"><grid-view/></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn grid_view_in_sticky_section_without_cross_axis_count_is_allowed() {
        let diagnostics = check(&project(
            "grid-view-attr-sticky",
            r#"<scroll-view type="custom"><sticky-section><grid-view/></sticky-section></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn grid_view_with_cross_axis_count_is_allowed() {
        let diagnostics = check(&project(
            "grid-view-attr-ok",
            r#"<scroll-view type="custom"><grid-view cross-axis-count="3"/></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn grid_view_with_dynamic_cross_axis_count_is_skipped() {
        let diagnostics = check(&project(
            "grid-view-attr-dynamic",
            r#"<scroll-view type="custom"><grid-view cross-axis-count="{{n}}"/></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    // ---- 属性取值 ----

    #[test]
    fn list_builder_with_unknown_type_is_reported() {
        let diagnostics = check(&project(
            "list-builder-type",
            r#"<view><list-builder child-height="80" type="flex"/></view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert!(diagnostics[0].message.contains("`static`"));
        assert!(diagnostics[0].message.contains("`dynamic`"));
    }

    #[test]
    fn list_builder_with_unknown_type_variant_is_reported() {
        let diagnostics = check(&project(
            "list-builder-type-b",
            r#"<view><list-builder child-height="80" type="virtual"/></view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    #[test]
    fn list_builder_with_allowed_types_is_allowed() {
        for (name, wxml) in [
            (
                "list-builder-type-static",
                r#"<view><list-builder child-height="80" type="static"/></view>"#,
            ),
            (
                "list-builder-type-dynamic",
                r#"<view><list-builder type="dynamic"/></view>"#,
            ),
            (
                "list-builder-type-default",
                r#"<view><list-builder child-height="80"/></view>"#,
            ),
        ] {
            let diagnostics = check(&project(name, wxml));
            assert!(diagnostics.is_empty(), "{name}: {diagnostics:#?}");
        }
    }

    #[test]
    fn list_builder_with_dynamic_type_is_skipped() {
        let diagnostics = check(&project(
            "list-builder-type-dynamic-binding",
            r#"<view><list-builder child-height="80" type="{{t}}"/></view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn grid_view_with_unknown_type_is_reported() {
        let diagnostics = check(&project(
            "grid-view-type",
            r#"<scroll-view type="custom"><grid-view cross-axis-count="2" type="grid"/></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("`masonry`"));
    }

    #[test]
    fn grid_view_with_unknown_type_variant_is_reported() {
        let diagnostics = check(&project(
            "grid-view-type-b",
            r#"<scroll-view type="custom"><grid-view cross-axis-count="2" type="waterfall"/></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    #[test]
    fn grid_view_with_allowed_types_is_allowed() {
        for (name, wxml) in [
            (
                "grid-view-type-aligned",
                r#"<scroll-view type="custom"><grid-view cross-axis-count="2" type="aligned"/></scroll-view>"#,
            ),
            (
                "grid-view-type-masonry",
                r#"<scroll-view type="custom"><grid-view cross-axis-count="2" type="masonry"/></scroll-view>"#,
            ),
        ] {
            let diagnostics = check(&project(name, wxml));
            assert!(diagnostics.is_empty(), "{name}: {diagnostics:#?}");
        }
    }

    #[test]
    fn grid_view_with_dynamic_type_is_skipped() {
        let diagnostics = check(&project(
            "grid-view-type-dynamic",
            r#"<scroll-view type="custom"><grid-view cross-axis-count="2" type="{{t}}"/></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    // ---- 关联容器：nested-scroll-body 内层 scroll-view ----

    #[test]
    fn inner_scroll_without_associative_container_is_reported() {
        let diagnostics = check(&project(
            "nested-assoc",
            r#"<scroll-view type="nested"><nested-scroll-body><scroll-view type="list"/></nested-scroll-body></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert!(diagnostics[0].message.contains("associative-container"));
        assert!(diagnostics[0].message.contains("nested-scroll-view"));
    }

    #[test]
    fn deeply_nested_inner_scroll_without_associative_container_is_reported() {
        let diagnostics = check(&project(
            "nested-assoc-deep",
            r#"<scroll-view type="nested"><nested-scroll-body><swiper><swiper-item><scroll-view type="list"/></swiper-item></swiper></nested-scroll-body></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    #[test]
    fn inner_scroll_with_wrong_associative_container_is_reported() {
        let diagnostics = check(&project(
            "nested-assoc-wrong",
            r#"<scroll-view type="nested"><nested-scroll-body><scroll-view type="list" associative-container="draggable-sheet"/></nested-scroll-body></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("当前是"));
    }

    #[test]
    fn inner_scroll_with_associative_container_is_allowed() {
        let diagnostics = check(&project(
            "nested-assoc-ok",
            r#"<scroll-view type="nested"><nested-scroll-body><scroll-view type="list" associative-container="nested-scroll-view"/></nested-scroll-body></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn inner_scroll_with_dynamic_associative_container_is_skipped() {
        let diagnostics = check(&project(
            "nested-assoc-dynamic",
            r#"<scroll-view type="nested"><nested-scroll-body><scroll-view type="list" associative-container="{{c}}"/></nested-scroll-body></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn inner_scroll_under_if_block_is_allowed() {
        let diagnostics = check(&project(
            "nested-assoc-block",
            r#"<scroll-view type="nested"><nested-scroll-body><block wx:if="{{ok}}"><scroll-view type="list" associative-container="nested-scroll-view"/></block></nested-scroll-body></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn scroll_view_in_nested_header_is_allowed() {
        // header 内的 scroll-view 不参与嵌套滚动（sticky 的横向 tab 列表就是这种写法）。
        let diagnostics = check(&project(
            "nested-assoc-header",
            r#"<scroll-view type="nested"><nested-scroll-header><sticky-header><scroll-view type="list" scroll-x/></sticky-header></nested-scroll-header><nested-scroll-body><view/></nested-scroll-body></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn inner_scroll_in_body_under_dynamic_type_is_still_reported() {
        // 约束条件是有没有 nested-scroll-body 祖先，与外壳 type 无关：外壳动态绑定也可能是 nested。
        let diagnostics = check(&project(
            "nested-assoc-type-dynamic",
            r#"<scroll-view type="{{t}}"><nested-scroll-body><scroll-view type="list"/></nested-scroll-body></scroll-view>"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    // ---- 关联容器：draggable-sheet 内的 scroll-view ----

    #[test]
    fn draggable_sheet_scroll_with_dynamic_container_is_skipped() {
        let diagnostics = check(&project(
            "draggable-dynamic",
            r#"<draggable-sheet><scroll-view type="list" associative-container="{{c}}"/></draggable-sheet>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    // ---- 条件分支：同一个 wx:if 的不同分支互斥 ----

    #[test]
    fn nested_bodies_in_if_and_else_are_allowed() {
        let diagnostics = check(&project(
            "nested-body-branches",
            concat!(
                r#"<scroll-view type="nested">"#,
                r#"<block wx:if="{{a}}"><nested-scroll-body><view/></nested-scroll-body></block>"#,
                r#"<block wx:else><nested-scroll-body><view/></nested-scroll-body></block>"#,
                r#"</scroll-view>"#,
            ),
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn sticky_headers_in_if_and_else_are_allowed() {
        let diagnostics = check(&project(
            "sticky-branches",
            concat!(
                r#"<scroll-view type="custom"><sticky-section>"#,
                r#"<block wx:if="{{a}}"><sticky-header>一</sticky-header></block>"#,
                r#"<block wx:else><sticky-header>二</sticky-header></block>"#,
                r#"</sticky-section></scroll-view>"#,
            ),
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn nested_header_in_if_and_body_in_else_are_allowed() {
        let diagnostics = check(&project(
            "nested-before-branches",
            concat!(
                r#"<scroll-view type="nested">"#,
                r#"<block wx:if="{{a}}"><nested-scroll-header><view/></nested-scroll-header></block>"#,
                r#"<block wx:else><nested-scroll-body><view/></nested-scroll-body></block>"#,
                r#"</scroll-view>"#,
            ),
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn unconditional_body_plus_body_in_if_is_reported() {
        // 无条件 body 与 if 里的 body 可能同时渲染。
        let diagnostics = check(&project(
            "nested-body-mixed",
            concat!(
                r#"<scroll-view type="nested"><nested-scroll-body><view/></nested-scroll-body>"#,
                r#"<block wx:if="{{a}}"><nested-scroll-body><view/></nested-scroll-body></block>"#,
                r#"</scroll-view>"#,
            ),
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(
            diagnostics[0]
                .message
                .contains("2 个 `<nested-scroll-body>`")
        );
    }

    #[test]
    fn nested_bodies_in_separate_if_blocks_are_reported() {
        // 两个独立的 wx:if 分别求值，理论上可同时成立。
        let diagnostics = check(&project(
            "nested-body-two-ifs",
            concat!(
                r#"<scroll-view type="nested">"#,
                r#"<block wx:if="{{a}}"><nested-scroll-body><view/></nested-scroll-body></block>"#,
                r#"<block wx:if="{{b}}"><nested-scroll-body><view/></nested-scroll-body></block>"#,
                r#"</scroll-view>"#,
            ),
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    #[test]
    fn nested_if_bodies_are_not_counted_twice() {
        // 外层 if/else 各自又套一层 if：分支栈要能看出最外层互斥。
        let diagnostics = check(&project(
            "nested-if",
            r#"<scroll-view type="nested"><block wx:if="{{a}}"><block wx:if="{{b}}"><nested-scroll-body/></block></block><block wx:else><nested-scroll-body/></block></scroll-view>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn sticky_header_after_sibling_in_else_branch_is_allowed() {
        let diagnostics = check(&project(
            "sticky-first-branches",
            concat!(
                r#"<scroll-view type="custom"><sticky-section>"#,
                r#"<block wx:if="{{a}}"><view/></block>"#,
                r#"<block wx:else><sticky-header>标题</sticky-header></block>"#,
                r#"</sticky-section></scroll-view>"#,
            ),
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn sticky_header_after_view_in_partial_if_is_reported() {
        // 没有 else 的 if 与无条件 header 不互斥，view 可能同时渲染在 header 之前。
        let diagnostics = check(&project(
            "sticky-first-partial-if",
            concat!(
                r#"<scroll-view type="custom"><sticky-section>"#,
                r#"<block wx:if="{{a}}"><view/></block>"#,
                r#"<sticky-header>标题</sticky-header>"#,
                r#"</sticky-section></scroll-view>"#,
            ),
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("<view>"));
    }

    /// 嵌套在内置节点里的 sticky-header 不是容器的直接子节点。
    #[test]
    fn sticky_header_nested_inside_a_view_is_reported() {
        let diagnostics = check(&project(
            "sticky-nested",
            concat!(
                r#"<scroll-view type="custom"><sticky-section>"#,
                r#"<view><sticky-header>标题</sticky-header></view>"#,
                r#"</sticky-section></scroll-view>"#,
            ),
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("直接放在"));
    }

    /// 唯一性只看容器的直接子节点：嵌套在 `<view>` 里的那个由「必须是直接子节点」报。
    #[test]
    fn sticky_header_count_ignores_nested_ones() {
        let diagnostics = check(&project(
            "sticky-unique-nested",
            concat!(
                r#"<scroll-view type="custom"><sticky-section>"#,
                r#"<sticky-header>第一个</sticky-header>"#,
                r#"<view><sticky-header>第二个</sticky-header></view>"#,
                r#"</sticky-section></scroll-view>"#,
            ),
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("直接放在"));
    }

    // ---- renders-first：按能同时渲染的子节点数判 ----

    /// `wx:if` / `wx:else` 各一个子节点：运行期只渲染一个，不能报。
    #[test]
    fn renders_first_ignores_exclusive_branches() {
        let diagnostics = check(&project(
            "renders-first-branches",
            concat!(
                r#"<scroll-view type="nested">"#,
                r#"<nested-scroll-header>"#,
                r#"<view wx:if="{{a}}"/><view wx:else/>"#,
                r#"</nested-scroll-header>"#,
                r#"<nested-scroll-body/>"#,
                r#"</scroll-view>"#,
            ),
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// 分支子节点数不相等时取最多的一支：else 里两个子节点就是 2。
    #[test]
    fn renders_first_counts_the_largest_branch() {
        let diagnostics = check(&project(
            "renders-first-largest",
            concat!(
                r#"<scroll-view type="nested">"#,
                r#"<nested-scroll-header>"#,
                r#"<view wx:if="{{a}}"/><block wx:else><view/><view/></block>"#,
                r#"</nested-scroll-header>"#,
                r#"<nested-scroll-body/>"#,
                r#"</scroll-view>"#,
            ),
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("2 个直接子元素"));
    }
}
