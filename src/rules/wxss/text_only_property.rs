//! `text-overflow` / `text-decoration*` 只在文本节点上生效，且不继承到后代。
//!
//! 根因是同一件事：两个属性都只作用于**行盒里的文字**，引擎把它们放在
//! `ComputedStyle` 的非继承数据里。WebView 则相反——`<view>` 上的 `text-overflow`
//! 会作用到它自己的行盒，`text-decoration` 会被绘制到后代的文字上。所以命中的容器
//! 里有 `<text>` 也不豁免：那条声明根本传不到它。
//!
//! 判定漏斗对两个属性完全一样，属性表只描述三处差异：
//!
//! ```text
//! visit_matching_rules（跳过命不中的样式块）
//!   → 按属性表挑出「在当前取值下真的会画出效果」的声明
//!   → broken_selectors（静态命中里一个能承载的节点都没有的选择器）
//!   → 报在声明处
//! ```
//!
//! 各自还有一半前提：
//!
//! - `text-overflow` 要 `white-space: nowrap` **且 `overflow` 不是 `visible`** 才可能
//!   出省略号；缺一个，WebView 里文本只会换行或溢出显示，两端一致（`.book_author` 这类
//!   flex 容器、`.intro_content` 这类靠 `max-height` 裁剪的多行块都属于这种）。
//! - `text-decoration` 只在真的说了线型时才画线：`none` 是取消，`-color` / `-style`
//!   单独出现时线型仍是初值 `none`，两端都空白。
//!
//! 命中节点是自定义组件、或属性带插值时静态判断不了，一律放过（宁可漏报）。多行 clamp
//! 的弱化另由 `skyline/unsupported-property` 与 `skyline/legacy-flexbox` 报；多值线型由
//! `skyline/text-decoration-line` 报。

use std::collections::HashMap;
use std::rc::Rc;

use crate::input::wxml::{WxmlDoc, WxmlElement, WxmlNode};
use crate::input::wxss::WxssDeclaration;
use crate::matcher::Tree;
use crate::project::Project;
use crate::rules::wxml::clips_overflow;
use crate::rules::wxml::is_builtin;
use crate::rules::wxml::resolved_property;
use crate::rules::{Rule, RuleMeta, VisualCase, VisualExpect};
use crate::{Diagnostic, Severity};

use super::value_util::at_declaration;
use super::visit_matching_rules;

pub const ID: &str = "skyline/text-only-property";

/// 简写里的线型关键字；一个都没有时这条声明不会画出任何线。
const LINE_KEYWORDS: [&str; 4] = ["underline", "overline", "line-through", "blink"];

/// 属性表：共享漏斗里唯一的差异点。
///
/// 每一项是一组判定完全同源的属性（`text-decoration` 简写与它的长写共享同一份宿主
/// 判定与文案）。新增「只对文本节点生效」的属性只需往表里加一项。
struct TextProperty {
    /// 触发规则的属性名，小写。
    names: &'static [&'static str],
    /// 值门槛：这条声明在当前取值下根本画不出效果时返回 `false`（两端一致）。
    draws: fn(&WxssDeclaration) -> bool,
    /// 命中节点在 Skyline 下照样承载这条声明时返回 `true`——包括「两端都不生效」。
    keeps: fn(&Project, &Tree, &WxmlElement) -> bool,
    /// 结论文案；第二个参数是失效的选择器，已按「`, `」拼好。
    message: fn(&WxssDeclaration, &str) -> String,
    /// 改法。
    advice: &'static str,
}

const PROPERTIES: &[TextProperty] = &[
    TextProperty {
        names: &["text-overflow"],
        draws: |decl| decl.value.trim().eq_ignore_ascii_case("ellipsis"),
        keeps: overflow_keeps,
        message: overflow_message,
        advice: "用 `<text>` 包裹文本（Skyline 下可配合 `overflow` / `max-lines` 截断），或删掉该声明",
    },
    TextProperty {
        // `text-decoration-thickness` 不在内——属性本身就不生效，由
        // `skyline/unsupported-property` 负责。
        names: &["text-decoration", "text-decoration-line"],
        draws: decoration_draws,
        keeps: decoration_keeps,
        message: decoration_message,
        advice: "把文本包进 `<text>` 并在它上面写装饰；`<view>` 上的装饰在 Skyline 下不会绘制",
    },
];

pub struct TextOnlyProperty;

impl TextOnlyProperty {
    pub const ID: &'static str = ID;
}

/// 视觉用例：`<view>` 上声明省略，WebView 在右端画出省略号，Skyline 只裁切；
/// `<view>` 上的下划线在 WebView 绘制、在 Skyline 消失。
static VISUAL_CASES: &[VisualCase] = &[
    VisualCase {
        wxml: r#"<view class="wrap">
  <view class="title" id="target">流浪地球计划要延续两千五百年，这段标题故意写得足够长</view>
</view>"#,
        wxss: ".wrap { box-sizing: content-box; width: 200px; padding: 8px; background: #fff; }\n.title { font-size: 16px; line-height: 24px; color: #222; text-overflow: ellipsis; white-space: nowrap; overflow: hidden; }\n",
        js: None,
        expect: VisualExpect::Differ,
        note: "WebView 在 200px 宽的 `<view>` 末尾画省略号：「千」之后收成「…」；Skyline 的 text-overflow 只对 `<text>` 生效，文本照常排到右边被裁掉——「五」完整、「百」只剩左半边。两端 bbox 一致，差异只在右端约 24 CSS px",
        target: "#target",
    },
    VisualCase {
        wxml: r#"<view class="link" id="target">查看商品详情页面</view>"#,
        wxss: ".link { box-sizing: content-box; width: 160px; padding: 12px; font-size: 20px; color: #222; background: #fff; text-decoration: underline; }\n",
        js: None,
        expect: VisualExpect::Differ,
        note: "`<view>` 上的下划线：WebView 把装饰画到文本上（文字下方一条 160 CSS 宽的下划线），Skyline 只对 `<text>` 绘制，这段文字没有下划线；两端 bbox 一致，差异只在这条线上",
        target: "#target",
    },
];

impl Rule for TextOnlyProperty {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Warning,
            description: "text-overflow / text-decoration* 只在文本节点（`<text>` / `<span>` / `<navigator>`）上生效、且不继承到后代：写在 `<view>` 等容器上会静默失效",
        }
    }

    fn visual_cases(&self) -> &'static [VisualCase] {
        VISUAL_CASES
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut docs: HashMap<String, Option<Rc<WxmlDoc>>> = HashMap::new();

        visit_matching_rules(project, |sheet, rule, out| {
            let doc = &sheet.doc;
            for decl in &rule.declarations {
                let name = decl.name.to_ascii_lowercase();
                let Some(property) = PROPERTIES
                    .iter()
                    .find(|property| property.names.contains(&name.as_str()))
                else {
                    continue;
                };
                if !(property.draws)(decl) {
                    continue;
                }
                // 逗号分隔的选择器各自独立：`.a, .b` 里只要有一条命中的节点承载不了
                // 这条声明，这条声明对那批节点就是失效的。
                let offenders = super::broken_selectors(
                    project,
                    sheet,
                    &rule.selectors,
                    &mut docs,
                    |tree, element| (property.keeps)(project, tree, element),
                );
                if offenders.is_empty() {
                    continue;
                }
                let selectors = offenders.join("`, `");
                out.push(at_declaration(
                    ID,
                    doc,
                    decl,
                    Severity::Warning,
                    (property.message)(decl, &selectors),
                    property.advice,
                ));
            }
        })
    }
}

fn overflow_message(decl: &WxssDeclaration, selectors: &str) -> String {
    format!(
        "`{}: {}` 在 Skyline 下只作用于 `<text>`；选择器 `{selectors}` 在某处命中的节点不是 `<text>`，声明不会生效",
        decl.name,
        decl.value.trim()
    )
}

fn decoration_message(decl: &WxssDeclaration, selectors: &str) -> String {
    format!(
        "`{}` 在 Skyline 下只作用于 `<text>` / `<span>` / `<input>` / `<navigator>`；选择器 `{selectors}` 在某处命中的节点都不是文本节点，装饰不会显示",
        decl.name
    )
}

// ---- text-overflow ----

/// 这个命中不构成「WebView 能省略、Skyline 不能」的差异。
///
/// - 宿主是 `<text>` / `<span>` / `<navigator>` 或自定义组件：Skyline 下这条声明本来生效；
/// - 宿主自己没有任何行内内容（文本全在块级子节点里）：两个渲染器都没有可省略的行盒；
/// - 其余宿主在 WebView 里也得要 `white-space: nowrap` **且真的裁掉溢出**（`overflow` 不是
///   `visible`）才可能省略；两个条件缺一个，文本只是换行或溢出显示，两端一致。
fn overflow_keeps(project: &Project, tree: &Tree, element: &WxmlElement) -> bool {
    if !is_builtin(&element.tag) || matches!(element.tag.as_str(), "text" | "span" | "navigator") {
        return true;
    }
    if !has_inline_content(element) {
        return true;
    }
    !(white_space_nowrap(project, tree, element) && clips_overflow(project, tree, element))
}

/// 宿主自己的行盒里有没有东西可省略：直接文本，或行内级子元素（`text`/`span`/
/// `navigator` 与自定义组件——后者渲染结果静态不可知，按可能有处理）。
fn has_inline_content(element: &WxmlElement) -> bool {
    element.children.iter().any(|child| match child {
        WxmlNode::Text(text) => text.is_meaningful(),
        WxmlNode::Element(child) => {
            matches!(child.tag.as_str(), "text" | "span" | "navigator") || !is_builtin(&child.tag)
        }
        _ => false,
    })
}

/// `white-space` 是继承属性（引擎里标了继承）：
/// 自己没写就看最近的祖先。
fn white_space_nowrap(project: &Project, tree: &Tree, element: &WxmlElement) -> bool {
    let mut current = Some(element);
    while let Some(node) = current {
        if let Some(resolved) = resolved_property(project, tree, node, "white-space") {
            let value = resolved.value.trim().to_ascii_lowercase();
            return value == "nowrap" || value == "pre";
        }
        current = tree.parent_of(node);
    }
    false
}

// ---- text-decoration ----

/// 装饰会画在这个节点上吗。自定义组件内部结构不可见，按「可能」处理。
fn can_decorate(element: &WxmlElement) -> bool {
    if !is_builtin(&element.tag) {
        return true;
    }
    matches!(
        element.tag.as_str(),
        "text" | "span" | "input" | "navigator"
    )
}

/// 只有说了线型的声明才会画线。
///
/// `text-decoration: none` 是「取消」，写在谁身上都无害；`-color` / `-style` 单独出现时
/// 线型仍是初值 `none`；简写里必须含线型关键字才算真的画线。
fn decoration_draws(decl: &WxssDeclaration) -> bool {
    let value = decl.value.trim();
    if value.eq_ignore_ascii_case("none") {
        return false;
    }
    if !decl.name.eq_ignore_ascii_case("text-decoration") {
        return true;
    }
    let lower = value.to_ascii_lowercase();
    LINE_KEYWORDS.iter().any(|keyword| lower.contains(keyword))
}

/// 装饰不继承，命中节点里有 `<text>` 也不豁免；只有子树里压根没有文字时两端才都不画。
fn decoration_keeps(_project: &Project, _tree: &Tree, element: &WxmlElement) -> bool {
    can_decorate(element) || !has_text(element)
}

/// 子树里有没有可装饰的文本（含 `<text>` 这类行内级后代与自定义组件）。
/// 完全没有文本时两边都不画任何东西，不构成差异。
fn has_text(element: &WxmlElement) -> bool {
    element.children.iter().any(|child| match child {
        WxmlNode::Text(text) => text.is_meaningful(),
        WxmlNode::Element(child) => can_decorate(child) || has_text(child),
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::Rule;
    use crate::rules::testutil::wxss::{self, Fixture};

    fn check(name: &str, wxml: &str, wxss: &str) -> Vec<Diagnostic> {
        wxss::check(&TextOnlyProperty, name, wxml, wxss)
    }

    // ---- positive ----

    /// 视觉用例与单测共用同一份 WXML/WXSS：每条都必须被本规则命中。
    #[test]
    fn visual_cases_are_reported() {
        for case in VISUAL_CASES {
            let fixture = Fixture::page("text-only-property-visual", case.wxml, case.wxss);
            let hits = crate::rules::testutil::check_rule(&fixture, &TextOnlyProperty)
                .iter()
                .filter(|diagnostic| diagnostic.rule == ID)
                .count();
            assert!(hits >= 1, "{}: {}", case.note, case.wxml);
        }
    }

    #[test]
    fn preprocessor_style_source_is_checked_too() {
        // 真实工程的样式源常常只有 .scss / .less；按后缀跳过等于这条规则从未生效。
        let fixture = Fixture::new("scss-source");
        fixture
            .write(
                "project.config.json",
                r#"{"miniprogramRoot":"","setting":{"useCompilerPlugins":["sass"]}}"#,
            )
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write(
                "pages/index/index.wxml",
                r#"<view class="title">标题</view>"#,
            )
            .write(
                "pages/index/index.scss",
                ".title { text-overflow: ellipsis; white-space: nowrap; overflow: hidden; }",
            );
        let diagnostics = TextOnlyProperty.check(&fixture.project());
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].path, "pages/index/index.scss");
    }

    #[test]
    fn view_with_plain_text_is_reported() {
        let diagnostics = check(
            "plain",
            r#"<view class="title">标题</view>"#,
            ".title { text-overflow: ellipsis; white-space: nowrap; overflow: hidden; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].rule, ID);
        assert_eq!(diagnostics[0].path, "pages/index/index.wxss");
    }

    #[test]
    fn view_without_inline_content_is_not_a_divergence() {
        // 文本全在块级子节点里：宿主自己没有行盒，WebView 同样不产生省略号。
        let diagnostics = check(
            "nested-view",
            r#"<view class="title"><view class="inner">x</view></view>"#,
            ".title { text-overflow: ellipsis; white-space: nowrap; overflow: hidden; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn no_nowrap_in_webview_means_no_difference() {
        // 没有 `white-space: nowrap` 时 WebView 只会换行，不产生省略号：两端一致。
        let diagnostics = check(
            "no-nowrap",
            r#"<view class="title">标题</view>"#,
            ".title { text-overflow: ellipsis; overflow: hidden; max-height: 4.5em; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn nowrap_without_overflow_is_fine() {
        // `overflow: visible`（默认）时文本只会溢出、不会出现省略号，WebView 也一样。
        let diagnostics = check(
            "no-overflow",
            r#"<view class="title">标题</view>"#,
            ".title { text-overflow: ellipsis; white-space: nowrap; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn overflow_on_one_axis_is_enough() {
        // `overflow-x: hidden` 同样让 `text-overflow` 生效。
        let diagnostics = check(
            "axis",
            r#"<view class="title">标题</view>"#,
            ".title { text-overflow: ellipsis; white-space: nowrap; overflow-x: hidden; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    }

    #[test]
    fn flex_container_without_nowrap_is_fine() {
        // `.book_author` 那种：flex 容器的子项都是 flex item，WebView 也不会省略。
        let diagnostics = check(
            "flex-container",
            r#"<view class="author"><text class="name">阿乙</text></view>"#,
            ".author { display: flex; text-overflow: ellipsis; overflow: hidden; }
             .name { white-space: nowrap; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn dynamic_nowrap_is_skipped() {
        let diagnostics = check(
            "dynamic-nowrap",
            r#"<view class="title">x</view>"#,
            ".title { text-overflow: ellipsis; white-space: {{nowrap}}; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    /// 共享样式在页面里失效、在组件里正常（反过来也一样）也要报：按作用域分别判。
    #[test]
    fn shared_style_broken_in_one_scope_is_reported() {
        let fixture = Fixture::new("shared-scope");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                r#"{"usingComponents":{"c":"/components/c/c"}}"#,
            )
            .write(
                "pages/index/index.wxml",
                r#"<view class="title">页面上的标题</view><c/>"#,
            )
            .write("pages/index/index.wxss", "@import \"/styles/shared.wxss\";")
            .write("components/c/c.json", "{}")
            .write(
                "components/c/c.wxml",
                r#"<text class="title">组件里的标题</text>"#,
            )
            .write("components/c/c.wxss", "@import \"/styles/shared.wxss\";")
            .write(
                "styles/shared.wxss",
                ".title { text-overflow: ellipsis; white-space: nowrap; overflow: hidden; }",
            );
        let diagnostics = TextOnlyProperty.check(&fixture.project());
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].path, "styles/shared.wxss");
    }

    #[test]
    fn decoration_on_view_is_reported() {
        let diagnostics = check(
            "view-decoration",
            r#"<view class="link">详情</view>"#,
            ".link { text-decoration: underline; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].rule, ID);
        assert!(diagnostics[0].message.contains("text-decoration"));
    }

    #[test]
    fn decoration_on_container_with_text_descendant_is_still_reported() {
        // 装饰不继承：容器里的 <text> 拿不到这条声明。
        let diagnostics = check(
            "container-decoration",
            r#"<view class="link"><text>详情</text></view>"#,
            ".link { text-decoration-line: underline; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    }

    #[test]
    fn subtree_without_text_is_not_reported() {
        // 子树里没有文本：没有可装饰的东西。
        let diagnostics = check(
            "no-text",
            r#"<view class="box"><image src="x"/></view>"#,
            ".box { text-decoration: underline; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    /// `text-overflow` 与 `text-decoration` 同处一条声明块时各报各的。
    #[test]
    fn both_properties_in_one_block_are_reported_separately() {
        let diagnostics = check(
            "both",
            r#"<view class="title">标题</view>"#,
            ".title { text-overflow: ellipsis; white-space: nowrap; overflow: hidden; text-decoration: underline; }",
        );
        assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
        assert!(diagnostics.iter().all(|diagnostic| diagnostic.rule == ID));
        assert!(
            diagnostics[0].message.contains("text-overflow")
                && diagnostics[1].message.contains("text-decoration")
        );
    }

    /// 同一条声明块里重复写同名属性：每条都要判，不能被前一条的值挡掉。
    #[test]
    fn repeated_declarations_are_judged_individually() {
        let diagnostics = check(
            "repeated",
            r#"<view class="title">标题</view>"#,
            ".title { text-overflow: clip; text-overflow: ellipsis; white-space: nowrap; overflow: hidden; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(diagnostics[0].message.contains("ellipsis"));
    }

    // ---- negative ----

    #[test]
    fn text_element_is_fine() {
        let diagnostics = check(
            "text",
            r#"<text class="title">标题</text>"#,
            ".title { text-overflow: ellipsis; white-space: nowrap; overflow: hidden; }",
        );
        assert!(diagnostics.is_empty());
    }

    /// 行内级后代（`<text>`）属于宿主的行盒，所以它不算「没有行内内容」——但这里
    /// 中间隔了一层块级 `<view>`，宿主的行盒里仍然什么都没有。
    #[test]
    fn inline_descendant_behind_a_block_child_is_not_enough() {
        let diagnostics = check(
            "descendant",
            r#"<view class="list"><view class="row"><text>标题</text></view></view>"#,
            ".list { text-overflow: ellipsis; white-space: nowrap; overflow: hidden; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    /// 直接的 `<text>` 子节点算行内内容：这是真差异（WebView 出省略号、Skyline 不出）。
    #[test]
    fn inline_descendant_is_reported() {
        let diagnostics = check(
            "inline-descendant",
            r#"<view class="list"><text>标题</text></view>"#,
            ".list { text-overflow: ellipsis; white-space: nowrap; overflow: hidden; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    }

    /// `white-space` 从祖先继承：WebView 里宿主的行盒同样不换行。
    #[test]
    fn inherited_nowrap_counts() {
        let diagnostics = check(
            "inherited-nowrap",
            r#"<view class="wrap"><view class="title">标题</view></view>"#,
            ".wrap { white-space: nowrap; } .title { text-overflow: ellipsis; overflow: hidden; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    }

    #[test]
    fn span_and_navigator_are_fine() {
        let diagnostics = check(
            "text-nodes",
            r#"<span class="a">x</span><navigator class="b">y</navigator>"#,
            ".a, .b { text-overflow: ellipsis; white-space: nowrap; overflow: hidden; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn unmatched_selector_is_fine() {
        let diagnostics = check(
            "unmatched",
            r#"<view class="other">x</view>"#,
            ".title { text-overflow: ellipsis; white-space: nowrap; overflow: hidden; }",
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn clip_is_fine() {
        let diagnostics = check(
            "clip",
            r#"<view class="title">x</view>"#,
            ".title { text-overflow: clip; }",
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn descendant_selector_is_matched() {
        // 后代选择器现在能命中视图：`.b` 是 `<view>`，承载不了 text-overflow。
        let diagnostics = check(
            "complex",
            r#"<view class="a"><view class="b">x</view></view>"#,
            ".a .b { text-overflow: ellipsis; white-space: nowrap; overflow: hidden; }",
        );
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn descendant_selector_on_text_is_fine() {
        let diagnostics = check(
            "complex-text",
            r#"<view class="a"><text class="b">x</text></view>"#,
            ".a .b { text-overflow: ellipsis; white-space: nowrap; overflow: hidden; }",
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn fully_dynamic_class_is_fine() {
        let diagnostics = check(
            "dynamic",
            r#"<view class="{{cls}}">x</view>"#,
            ".title { text-overflow: ellipsis; white-space: nowrap; overflow: hidden; text-decoration: underline; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    /// 动态 class 里写死的 token 是静态命中：元素是 view，两条声明都失效。
    #[test]
    fn dynamic_class_with_static_token_is_still_matched() {
        let diagnostics = check(
            "dynamic-token",
            r#"<view class="link {{active}}">详情</view>"#,
            ".link { text-decoration: underline; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].rule, ID);
    }

    /// 逗号分隔的选择器各判各的：`.a` 命中 view，`.b` 命中的是 text。
    #[test]
    fn selectors_in_a_list_are_judged_individually() {
        let diagnostics = check(
            "selector-list",
            r#"<view class="a">x</view><text class="b">y</text>"#,
            ".a, .b { text-overflow: ellipsis; white-space: nowrap; overflow: hidden; }",
        );
        assert_eq!(diagnostics.len(), 1);
        assert!(
            diagnostics[0].message.contains("`.a`"),
            "{}",
            diagnostics[0].message
        );
        assert!(!diagnostics[0].message.contains("`.b`"));
    }

    #[test]
    fn app_wxss_matches_across_pages() {
        let fixture = Fixture::new("global");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", r#"<view class="title">x</view>"#)
            .write(
                "app.wxss",
                ".title { text-overflow: ellipsis; white-space: nowrap; overflow: hidden; }",
            );
        let project = fixture.project();
        assert_eq!(TextOnlyProperty.check(&project).len(), 1);
    }

    /// 宿主在 `<import>` 进来的模板定义体里：模板渲染在页面作用域，声明照样落在
    /// 一个非 `<text>` 的 `<view>` 上。可达性闸门若只看页面 WXML，整条声明会被跳过。
    #[test]
    fn host_inside_an_imported_template_is_reported() {
        let fixture = Fixture::new("template-host");
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
                r#"<template name="t"><view class="title">标题</view></template>"#,
            )
            .write(
                "pages/index/index.wxss",
                ".title { text-overflow: ellipsis; white-space: nowrap; overflow: hidden; }",
            );
        let project = fixture.project();
        assert_eq!(TextOnlyProperty.check(&project).len(), 1);
    }

    #[test]
    fn decoration_on_text_is_fine() {
        let diagnostics = check(
            "text-decoration",
            r#"<text class="link">详情</text>"#,
            ".link { text-decoration: underline; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn decoration_on_navigator_is_fine() {
        let diagnostics = check(
            "navigator",
            r#"<navigator class="link">详情</navigator>"#,
            ".link { text-decoration: underline; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn decoration_on_input_is_fine() {
        let diagnostics = check(
            "input",
            r#"<input class="field"/>"#,
            ".field { text-decoration-color: red; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    /// 只写 `text-decoration-color` / `-style`：没有线型，两端都不画。
    #[test]
    fn color_and_style_alone_are_not_reported() {
        for (name, css) in [
            ("only-color", ".link { text-decoration-color: red; }"),
            ("only-style", ".link { text-decoration-style: dashed; }"),
        ] {
            let diagnostics = check(name, r#"<view class="link">详情</view>"#, css);
            assert!(diagnostics.is_empty(), "{name}: {diagnostics:?}");
        }
    }

    #[test]
    fn none_is_fine() {
        let diagnostics = check(
            "none",
            r#"<view class="link">详情</view>"#,
            ".link { text-decoration: none; text-decoration-line: none; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn complex_selector_is_fine() {
        // 组合符 / 伪类的命中范围静态判不了，放过。
        let diagnostics = check(
            "complex-decoration",
            r#"<view class="link">详情</view>"#,
            ".link:hover { text-decoration: underline; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    /// 未注册标签按自定义组件处理：渲染结果静态不可知。
    #[test]
    fn custom_component_is_fine() {
        let diagnostics = check(
            "custom",
            r#"<my-text class="title"/>"#,
            ".title { text-overflow: ellipsis; white-space: nowrap; overflow: hidden; text-decoration: underline; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    /// 真实组件的内部结构在页面作用域里查不到，同样放过。
    #[test]
    fn custom_component_fixture_is_fine() {
        let fixture = Fixture::new("component");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                r#"{"usingComponents":{"c":"/components/c/c"}}"#,
            )
            .write("pages/index/index.wxml", r#"<c class="link"/>"#)
            .write(
                "pages/index/index.wxss",
                ".link { text-decoration: underline; }",
            )
            .write("components/c/c.json", "{}")
            .write("components/c/c.wxml", "<text>x</text>")
            .write("components/c/c.wxss", "");
        let diagnostics = TextOnlyProperty.check(&fixture.project());
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }
}
