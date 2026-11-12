//! `::before` / `::after` 与宿主里的文本并排时，Skyline 会把伪元素排成独立一行。
//!
//! ## 引擎事实
//!
//! - Skyline 只实现 `::before` / `::after` 两个伪元素（`css/selector.h:96-97` 的
//!   `pseudo_element_type`、`css/css_pseudo_element.h:13-17`；fp 侧解析见
//!   `float-pigment-css/src/parser/selector.rs:292-305`）。
//! - `content` 只支持字符串：`Content::ApplyValue` 把 `none` / `normal` 归空，
//!   其它标识符与 `url()` 都不受支持（`css/css_properties/content.cc:60-83`）。
//!   伪节点在**声明了 `content` 时**才创建（`shadow/shadow_node.cc:595-616`，
//!   更新路径在 `:684-702`；空字符串也算已声明）。
//! - 伪节点的盒类型按宿主是不是 TextShadowNode 分派：**宿主是 `<text>` 时**伪元素是
//!   inline / inline-block；**其它宿主只支持 block / flex**
//!   （`shadow/pseudo_node.cc:47-52`）。非文本宿主的伪节点会自己建一个 Text 根组件
//!   （`:55-104`），并作为宿主的 Yoga 子节点挂进去（`shadow/shadow_node.cc:595-647`
//!   的 `ConnectToShadowTree` / widget 插入，`widgets/yoga_connector.cc:95-107`
//!   的 `YogaAppendChild`）——也就是说它是宿主的 flex item / block 子盒。
//! - 宿主没写 `display` 时，Skyline 默认是 `flex`（开了 `defaultDisplayBlock` 则是
//!   `block`），`flex-direction` 默认是 `column`（开了 `defaultDisplayBlock` 则是
//!   `row`）：`css/css_properties/display.cc:43-50`、
//!   `css/css_properties/flex_direction.cc:39-48`。
//!
//! WebView 里 `::before/::after` 默认是 inline 盒，和相邻裸文本 / `<text>` 同在
//! 一个行内格式化上下文里，所以同一行；Skyline 里它是块级/flex 子盒，和文本各占
//! 一行。宿主显式 `display: flex` 时两端都把它当 flex item（WebView 也会 blockify
//! 伪元素），没有差异——这种宿主上的方向差异归 `skyline/flex-direction`。
//!
//! ## 判据
//!
//! 同时满足才报：
//!
//! 1. 伪元素规则里有**非空字符串** `content`（`content: ""` / `none` / `normal`
//!    不产生可见盒子；`var()` / `attr()` 静态不可知，放过）；
//! 2. 宿主是 `<view>`（其它内置组件有自己的内部布局，自定义组件的子树静态不可知，
//!    都不臆测）；
//! 3. 宿主自己的 `display` 不是 `flex` / `inline-flex` / `grid`（显式 flex 时两端
//!    一致；缺省 display 在两种开关下都会让伪元素与文本分行，因此照报）；
//! 4. 宿主里至少有一个**行内内容**：有意义的裸文本，或靠默认 display 参与行内流的
//!    `text` / `image` / `span` 子元素（复用 `inline_run` 的判据）。
//!
//! 修法：给宿主加 `display: flex; flex-direction: row`；或把伪元素换成真实节点，
//! 和文本一起包进同一个 `<text>`。只把 `content` 换成 `display:inline-block` 不够：
//! 非文本宿主的伪节点在 Skyline 下不按 inline 盒处理。

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::diag::{Diagnostic, Severity};
use crate::input::wxml::{WxmlDoc, WxmlNode};
use crate::input::wxss::WxssSelector;
use crate::matcher::{Dialect, Tree, Tri};
use crate::project::{Project, StyleSheet};
use crate::rules::{Rule, RuleMeta};

use super::scope_wxml;
use crate::rules::wxml::inline_run::is_inline_member;
use crate::rules::wxml::{cascade_entries_with, is_flex_value, load_doc};

pub const ID: &str = "skyline/pseudo-inline-layout";

pub struct PseudoInlineLayout;

impl Rule for PseudoInlineLayout {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Warning,
            description: "伪元素与文本并排时 Skyline 下会换行：伪元素是块级盒，WebView 下是 inline",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        let mut docs: HashMap<String, Option<Rc<WxmlDoc>>> = HashMap::new();
        // 同一个规则块（同一条 content 声明）只报一次。
        let mut reported: HashSet<(String, usize)> = HashSet::new();

        for sheet in &project.styles {
            for rule in &sheet.doc.rules {
                let Some(content) = rule
                    .declarations
                    .iter()
                    .rev()
                    .find(|declaration| declaration.name.eq_ignore_ascii_case("content"))
                else {
                    continue;
                };
                if !has_visible_content(&content.value) {
                    continue;
                }
                let Some(selector) = rule.selectors.iter().find_map(|selector| {
                    let name = pseudo_element(selector)?;
                    let host_selector = strip_trailing_pseudo(&selector.text, name)?;
                    if pseudo_skipped(sheet, host_selector, name) {
                        return None;
                    }
                    has_matching_host(project, sheet, host_selector, &mut docs).then_some(selector)
                }) else {
                    continue;
                };
                if !reported.insert((sheet.doc.path().to_string(), content.offset)) {
                    continue;
                }
                let (line, column) = sheet.doc.position(content.offset);
                let (end_line, end_column) = sheet.doc.position(content.value_end());
                out.push(
                    Diagnostic::new(
                        ID,
                        Severity::Warning,
                        format!(
                            "`{}` 的伪元素与宿主里的文本同处一个行内上下文：WebView 下同行，Skyline 下伪元素是块级盒、会独占一行",
                            selector.text
                        ),
                        sheet.doc.path(),
                        line,
                        column,
                    )
                    .end(end_line, end_column)
                    .help("给宿主加 `display: flex; flex-direction: row`（跨渲染器一致）；或把伪元素换成真实节点、和这一行文本一起包进同一个 `<text>`"),
                );
            }
        }
        out
    }
}

/// 选择器里的 `::before` / `::after`。单冒号旧写法在 fp 解析层也被当成伪元素
/// （`parser/selector.rs:357-377`），这里一视同仁。
fn pseudo_element(selector: &WxssSelector) -> Option<&'static str> {
    selector
        .pseudos
        .iter()
        .find_map(|pseudo| match pseudo.name.as_str() {
            "before" => Some("before"),
            "after" => Some("after"),
            _ => None,
        })
}

/// 去掉选择器末尾的伪元素，得到伪元素的宿主选择器。
///
/// 仅当伪元素确实位于选择器末尾时成立（CSS 里伪元素之后不能再接后代 / 兄弟）；
/// 解析层已经确认存在该伪元素，这里只做文本裁剪，避免碰属性值里的 `::before`。
fn strip_trailing_pseudo<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let trimmed = text.trim_end();
    for prefix in ["::", ":"] {
        if let Some(start) = trimmed.len().checked_sub(prefix.len() + name.len()) {
            let suffix = &trimmed[start..];
            if suffix.eq_ignore_ascii_case(&format!("{prefix}{name}")) {
                let host = trimmed[..start].trim_end();
                return (!host.is_empty()).then_some(host);
            }
        }
    }
    None
}

/// `content` 是否会生成一个非空盒子。
///
/// 只认字符串字面量：`content: "!"`。空串、`none` / `normal` 不产生可见内容；
/// `var()` / `attr()` / `counter()` 在静态阶段无法知道最终值，按漏报处理。
fn has_visible_content(value: &str) -> bool {
    let value = value.trim();
    let inner = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|rest| rest.strip_suffix('\''))
        });
    inner.is_some_and(|inner| !inner.is_empty())
}

/// 伪元素本身是否脱离文档流 / 不生成盒子。
///
/// `::before { position: absolute }` 这类 1px 边框技巧在两端都不参与行内流；
/// `display: none` 同理。同一选择器在样式表里出现多次时按源码顺序取最后一个值。
fn pseudo_skipped(sheet: &StyleSheet, host_selector: &str, name: &str) -> bool {
    let mut position = None;
    let mut display = None;
    for rule in &sheet.doc.rules {
        let matches = rule.selectors.iter().any(|selector| {
            pseudo_element(selector) == Some(name)
                && strip_trailing_pseudo(&selector.text, name) == Some(host_selector)
        });
        if !matches {
            continue;
        }
        for declaration in &rule.declarations {
            let value = declaration.value.trim().to_ascii_lowercase();
            if declaration.name.eq_ignore_ascii_case("position") {
                position = Some(value);
            } else if declaration.name.eq_ignore_ascii_case("display") {
                display = Some(value);
            }
        }
    }
    position.is_some_and(|value| matches!(value.as_str(), "absolute" | "fixed"))
        || display.is_some_and(|value| value == "none")
}

/// 宿主元素本身是否值得判定（标签 / display）。
fn is_candidate_host(
    project: &Project,
    tree: &Tree<'_>,
    element: &crate::input::wxml::WxmlElement,
) -> bool {
    // 只判普通 `<view>`：其它组件的内部盒模型由各自渲染器决定，自定义组件的子树
    // 静态不可知，都不在这里猜。
    if element.tag != "view" {
        return false;
    }
    if has_dynamic_style(element) {
        return false;
    }
    // 显式 flex 时两端都按 flex item 排；grid 在 Skyline 下没有对应实现；
    // `display: none` 不生成盒子。模板里判不了时不能当成「没有 display」。
    let Some(display) = cascade_entries_with(project, tree, element, "display") else {
        return false;
    };
    !display.last().is_some_and(|entry| {
        let value = entry.value.trim().to_ascii_lowercase();
        is_flex_value(&value) || matches!(value.as_str(), "grid" | "inline-grid" | "none")
    })
}

/// 宿主里是否有裸文本或靠默认 display 参与行内流的文本级子元素。
fn has_inline_content(
    project: &Project,
    tree: &Tree<'_>,
    element: &crate::input::wxml::WxmlElement,
) -> bool {
    element.children.iter().any(|child| match child {
        WxmlNode::Text(text) => text.is_meaningful(),
        WxmlNode::Element(child) => is_inline_member(project, tree, child),
        WxmlNode::Comment => false,
    })
}

/// 在样式表的所有作用域里找至少一个满足条件的宿主。
fn has_matching_host(
    project: &Project,
    sheet: &StyleSheet,
    host_selector: &str,
    docs: &mut HashMap<String, Option<Rc<WxmlDoc>>>,
) -> bool {
    for scope in &sheet.scopes {
        for path in scope_wxml(project, scope) {
            let Some(doc) = docs
                .entry(path.to_string())
                .or_insert_with(|| load_doc(project, path))
                .as_ref()
            else {
                continue;
            };
            // 模板定义体 / `<include>` 片段的样式按引用它的单元求值（`cascade_entries`），
            // 宿主自身的 `display` 与子节点的行内性都不依赖实例化处的祖先，照常判。
            let tree = Tree::new(doc);
            for element in doc.elements() {
                if tree.matches(element, host_selector, Dialect::Skyline) != Tri::Yes {
                    continue;
                }
                if is_candidate_host(project, &tree, element)
                    && has_inline_content(project, &tree, element)
                {
                    return true;
                }
            }
        }
    }
    false
}

fn has_dynamic_style(element: &crate::input::wxml::WxmlElement) -> bool {
    element
        .attributes
        .iter()
        .any(|attr| attr.name.eq_ignore_ascii_case("style") && !attr.is_static)
}

#[cfg(test)]
mod tests {
    use super::PseudoInlineLayout;
    use crate::rules::Rule;
    use crate::rules::testutil::wxml::Fixture;

    fn check(fixture: &Fixture) -> Vec<crate::Diagnostic> {
        PseudoInlineLayout.check(&fixture.project())
    }

    fn page(name: &str, wxml: &str, wxss: &str) -> Fixture {
        Fixture::page(name, wxml, wxss)
    }

    #[test]
    fn pseudo_icon_with_text_wraps() {
        let diagnostics = check(&page(
            "pseudo-pos",
            r#"<view class="warn">提交退款申请后不可取消</view>"#,
            r#".warn::before { content: "!"; display: inline-block; width: 32rpx; }"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].severity, crate::Severity::Warning);
        assert!(diagnostics[0].message.contains("::before"));
    }

    #[test]
    fn pseudo_with_text_element_child_wraps() {
        let diagnostics = check(&page(
            "pseudo-text-child",
            r#"<view class="memo">客服电话：<text>1010-5335</text></view>"#,
            r#".memo::after { content: ">"; }"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    #[test]
    fn single_colon_pseudo_element_is_the_same() {
        let diagnostics = check(&page(
            "pseudo-legacy",
            r#"<view class="warn">文案</view>"#,
            r#".warn:before { content: "!"; }"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    #[test]
    fn absolute_or_hidden_pseudo_does_not_participate_in_inline_flow() {
        let diagnostics = check(&page(
            "pseudo-absolute",
            r#"<view class="warn">文案</view>"#,
            r#".warn::before { content: " "; position: absolute; height: 1px; }
               .warn::after { content: "!"; display: none; }"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn only_block_children_do_not_use_inline_flow() {
        let diagnostics = check(&page(
            "pseudo-block-child",
            r#"<view class="warn"><view class="icon"/></view>"#,
            r#".warn::before { content: "!"; }"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn flex_host_has_no_difference() {
        let diagnostics = check(&page(
            "pseudo-flex",
            r#"<view class="warn">文案</view>"#,
            r#".warn { display: flex; flex-direction: row; }
               .warn::before { content: "!"; }"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn text_host_keeps_pseudo_inline() {
        let diagnostics = check(&page(
            "pseudo-text-host",
            r#"<text class="warn">文案</text>"#,
            r#".warn::before { content: "!"; }"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn empty_content_has_no_box() {
        let diagnostics = check(&page(
            "pseudo-empty",
            r#"<view class="warn">文案</view>"#,
            r#".warn::before { content: ""; }"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn content_none_or_var_is_not_statically_visible() {
        let diagnostics = check(&page(
            "pseudo-none",
            r#"<view class="warn">文案</view>"#,
            r#".warn::before { content: none; }
               .warn::after { content: var(--icon); }"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn explicit_block_text_child_does_not_count_as_inline() {
        let diagnostics = check(&page(
            "pseudo-block-text",
            r#"<view class="warn"><text class="stack">文案</text></view>"#,
            r#".warn::before { content: "!"; }
               .stack { display: block; }"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn out_of_flow_text_child_does_not_count_as_inline() {
        let diagnostics = check(&page(
            "pseudo-absolute",
            r#"<view class="warn"><text class="float">文案</text></view>"#,
            r#".warn::before { content: "!"; }
               .float { position: absolute; }"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn dynamic_style_is_unknown() {
        let diagnostics = check(&page(
            "pseudo-dynamic",
            r#"<view class="warn" style="{{display}}">文案</view>"#,
            r#".warn::before { content: "!"; }"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ancestor_selector_matches_the_host() {
        let diagnostics = check(&page(
            "pseudo-ancestor",
            r#"<view class="panel"><view class="warn">文案</view></view>"#,
            r#".panel .warn::before { content: "!"; }"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    #[test]
    fn selector_without_a_matching_host_is_not_reported() {
        let diagnostics = check(&page(
            "pseudo-dead",
            r#"<view class="other">文案</view>"#,
            r#".warn::before { content: "!"; }"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn only_one_diagnostic_per_content_declaration() {
        let diagnostics = check(&page(
            "pseudo-dedupe",
            r#"<view class="warn">一</view><view class="warn">二</view>"#,
            r#".warn::before { content: "!"; }"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    #[test]
    fn content_in_an_attribute_string_is_not_a_pseudo() {
        let diagnostics = check(&page(
            "pseudo-attr",
            r#"<view class="warn" data-label="::before">文案</view>"#,
            r#".warn[data-label="::before"] { color: red; }
               .warn::after { content: "!"; }"#,
        ));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("::after"));
    }

    /// 宿主在 `<import>` 进来的模板定义体里：模板渲染在页面作用域，样式按引用它的
    /// 单元求值；宿主自己的 `display` / 子节点行内性都不依赖实例化处的祖先，能判。
    #[test]
    fn host_inside_an_imported_template_is_reported() {
        let fixture = Fixture::new("pseudo-template");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write(
                "pages/index/index.wxml",
                r#"<import src="/tpls/t.wxml"/><template is="t"/>"#,
            )
            .write(
                "tpls/t.wxml",
                r#"<template name="t"><view class="warn">文案</view></template>"#,
            )
            .write(
                "pages/index/index.wxss",
                r#".warn::before { content: "!"; }"#,
            );
        let diagnostics = check(&fixture);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    /// 模板里宿主的 `display` 判不了（`.outer` 在实例化处、属性条件超受限文法）：
    /// 运行时是 flex 时伪元素与文本同行，不能当成「没有 display」报出来。
    #[test]
    fn undecidable_host_display_is_not_judged() {
        let fixture = Fixture::new("pseudo-undecidable");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write(
                "pages/index/index.wxml",
                r#"<import src="/tpls/t.wxml"/><view class="outer"><template is="t"/></view>"#,
            )
            .write(
                "tpls/t.wxml",
                r#"<template name="t"><view class="warn" data-x="1">文案</view></template>"#,
            )
            .write(
                "pages/index/index.wxss",
                ".outer .warn[data-x] { display: flex; }\n.warn::before { content: \"!\"; }",
            );
        let diagnostics = check(&fixture);
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }
}
