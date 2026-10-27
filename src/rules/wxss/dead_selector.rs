//! 死选择器：样式表里的选择器在它所有消费者文档里一个节点都匹配不到。
//!
//! 这是**源码级**判定，和渲染器差异无关：用 WebView 方言求值——迁移前用户看到的
//! 正是 WebView 的结果，写错的类名 / 标签在两端都不命中。Skyline 特有的
//! 「属性条件被丢弃 / 整条不进规则集」由 `attribute-selector` 负责，这里不重复。
//!
//! 判据分三层，全部满足才报：
//!
//! 1. 主体带 class（`matcher::subject().has_class`）。纯标签 / 纯属性主体要么是
//!    没用到的通用样式（dead code，#1443 排除），要么由 `attribute-selector` 负责。
//! 2. 选择器里的每个 class 名都在这些文档里**确定出现**（`absorb_classes`）。只在
//!    动态 `class` / `hover-class` 里可能出现的类不算；完全不出现的类属于未使用
//!    CSS——实测 businessview 的 vendored weui 会凭空多出 879 条，全是这类噪音。
//! 3. 在每个作用域的全部 WXML 上求值都是 `No`（任一 `Yes` 命中、任一 `Unknown`
//!    判不了）。
//!
//! 于是报出来的是「类名都对得上、关系却对不上」：迁移时把 `.a.skyline` 写成
//! `.a .skyline`、修饰类挂到错节点、LESS 自嵌套写成 `.x.y` 这类真选择器错误。
//!
//! 必须豁免的情况（否则必然误报，逐条对应代码）：
//!
//! 1. **伪元素**：`::before` / `::after` 的承载者是生成的文本节点，不是宿主元素，
//!    matcher 一律返回 `No`（`matcher/mod.rs` 的 `PseudoElement` 分支）。
//!    含伪元素的选择器整条跳过——见 [`matcher::shape`] 的 `pseudo_element`。
//! 2. **运行时伪类**：`:hover` / `:active` / `:checked` 在 WebView 下是 `Unknown`
//!    （`classify` 的 `NonTSPseudoClass` 分支），单个元素就判不了。嵌套在
//!    `:not()` / `:is()` 里的运行时伪类 `classify` 不会递归，所以这里用
//!    [`matcher::shape`] 递归找一遍，命中就跳过。
//! 3. **`@keyframes`**：里面的 `from` / `to` / 百分比不是选择器，规则靠
//!    `WxssRule::at_rules` 跳过。
//! 4. **`<slot/>`**：slot 位置的内容由父级提供、写在别的文件里，匹配层补「未知
//!    子节点」占位；只要选择器带组合符就判 `Unknown`——投影内容里自己也可能出现
//!    `.body .list` 这种嵌套，组件样式本来就能命中投影内容。代价是含 slot 的组件里
//!    关系写错只能靠人工发现（实测这类组件占了多数）。
//! 5. **`unknown_children`**（`<template is>` / `<include>`）：展开后的内容静态
//!    不可知，同作用域，对**所有**选择器生效。
//! 6. **`componentGenerics`**：泛型占位标签运行时渲染成什么组件静态未知，这份
//!    文档的「不命中」结论不可靠，整条选择器跳过——见 [`declares_generics`]。
//! 7. **运行时根节点**：`page` 与 `:host` 不在 WXML 的元素树里（前者是页面根，
//!    后者是组件宿主），含它们的选择器跳过。
//! 8. **解析失败**：`shape` 返回 `None`，跳过（与 matcher 的 `Unknown` 口径一致）。
//!
//! 另外内联 `<template name>` 的定义体（`WxmlElement::in_template`）真实祖先
//! 静态不可知，带组合符的选择器也不下结论。

use std::collections::{HashMap, HashSet};

use crate::input::wxml::WxmlDoc;
use crate::matcher::{self, Dialect, Tree, Tri};
use crate::project::{Project, StyleSheet};
use crate::rules::wxml::walk;
use crate::rules::{Rule, RuleMeta, Target};
use crate::{Diagnostic, Severity};

use super::{scope_wxml, visit_rules};

pub struct DeadSelector;

impl DeadSelector {
    pub const ID: &'static str = "skyline/dead-selector";
}

impl Rule for DeadSelector {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "样式表里的选择器在它作用到的所有 WXML 里都不命中任何节点，多半是类名 / 标签名写错",
        }
    }

    fn target(&self) -> Target {
        Target::Both
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut docs: HashMap<String, Option<WxmlDoc>> = HashMap::new();
        visit_rules(project, |sheet, rule, out| {
            // `@keyframes` 里的 `from` / `to` / 百分比不是选择器，匹配不到节点是必然的。
            if rule
                .at_rules
                .iter()
                .any(|at| at.name.ends_with("keyframes"))
            {
                return;
            }
            for selector in &rule.selectors {
                let text = selector.text.trim();
                let Some(count) = dead(project, sheet, text, &mut docs) else {
                    continue;
                };
                let (line, column) = sheet.doc.position(selector.offset);
                out.push(
                    Diagnostic::new(
                        Self::ID,
                        Severity::Warning,
                        format!(
                            "`{text}` 在它作用到的 {count} 份 WXML 里不命中任何节点：多半是类名 / 标签名写错，或迁移时加了不存在的修饰类（例如把 `.a.skyline` 写成 `.a .skyline`）"
                        ),
                        sheet.doc.path(),
                        line,
                        column,
                    )
                    .help(
                        "逐个核对选择器里的类名与标签名；动态拼接的 class（`class=\"a {{b}}\"`）不在此列。\
                         如果这条样式想作用于组件内部的节点，Skyline 默认不跨组件匹配，应写进组件自己的 wxss",
                    ),
                );
            }
        })
    }
}

/// 这条选择器是否真的一个节点都不命中；`Some(n)` 里的 `n` 是作用到的文档数。
///
/// `None` 表示结论不可靠（静态判不了 / 有文档看不见），不报。
fn dead(
    project: &Project,
    sheet: &StyleSheet,
    selector: &str,
    docs: &mut HashMap<String, Option<WxmlDoc>>,
) -> Option<usize> {
    if selector.is_empty() {
        return None;
    }
    let shape = matcher::shape(selector)?;
    if shape.pseudo_element || shape.dynamic_pseudo || shape.runtime_root {
        return None;
    }
    let classes = matcher::class_names(selector)?;
    // 纯标签 / 纯属性主体不在这里判：要么是完全没用到的通用样式（dead code，
    // #1443 排除），要么由 `attribute-selector` 负责。只有主体带 class 时，
    // 「类名都在、关系对不上」才等价于选择器写错。
    if !matcher::subject(selector)?.has_class {
        return None;
    }

    let mut paths: Vec<&str> = Vec::new();
    for scope in &sheet.scopes {
        let scope_paths = scope_wxml(project, scope);
        // 有作用域看不到 WXML（页面 opt-out / 文件缺失）：那条作用域里可能命中。
        if scope_paths.is_empty() {
            return None;
        }
        for path in scope_paths {
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    if paths.is_empty() {
        return None;
    }

    let mut unknown = false;
    // 选择器里的类名在文档里「确定出现」的集合：只在动态 class / `hover-class` 里
    // 可能出现的类不算。
    let mut present: HashSet<String> = HashSet::new();
    for path in &paths {
        if declares_generics(project, path) {
            return None;
        }
        let Some(doc) = docs
            .entry(path.to_string())
            .or_insert_with(|| crate::rules::wxml::load_doc(project, path))
            .as_ref()
        else {
            // WXML 读不到 / 解析失败：这份文档里可能命中。
            return None;
        };
        absorb_classes(doc, &mut present);
        match Tree::new(doc).any_match(selector, Dialect::WebView) {
            Tri::Yes => return None,
            Tri::Unknown => unknown = true,
            Tri::No => {}
        }
    }
    if unknown {
        return None;
    }
    // 类名一个都不命中不值得报：完全没用到的 CSS 属于 dead code（#1443 排除），
    // 只在动态 class / `hover-class` 里出现的类静态判不出。只有「类名都确定存在、
    // 关系却对不上」才是选择器写错。
    if !classes.iter().all(|name| present.contains(name)) {
        return None;
    }
    Some(paths.len())
}

/// 收集文档里「确定出现」的 class 名：静态 `class` 的字面词，以及动态 `class`
/// 表达式在任何取值下都会出现的词。
fn absorb_classes(doc: &WxmlDoc, out: &mut HashSet<String>) {
    walk(doc, |element, _| {
        for attr in &element.attributes {
            if attr.name != "class" {
                continue;
            }
            match &attr.class_tokens {
                Some(tokens) => out.extend(tokens.definite.iter().cloned()),
                None => {
                    if let Some(value) = &attr.value {
                        out.extend(value.split_ascii_whitespace().map(str::to_owned));
                    }
                }
            }
        }
    });
}

/// 这份 WXML 所属单元的 json 是否声明了 `componentGenerics`。
///
/// 泛型占位标签（`<selectable>`）运行时渲染成什么组件静态未知，里面的节点
/// 也就无法判断能不能被选中。
fn declares_generics(project: &Project, wxml: &str) -> bool {
    let json = project
        .components
        .iter()
        .find(|component| component.files.wxml.as_deref() == Some(wxml))
        .and_then(|component| project.component_json.get(&component.path))
        .or_else(|| {
            project
                .pages
                .iter()
                .find(|page| page.files.wxml.as_deref() == Some(wxml))
                .and_then(|page| project.page_json.get(&page.route))
        });
    json.and_then(|json| json.get("componentGenerics"))
        .and_then(|generics| generics.as_object())
        .is_some_and(|generics| !generics.is_empty())
}

#[cfg(test)]
mod tests {
    use crate::project::Project;
    use crate::rules::Rule;
    use crate::rules::testutil::wxss::Fixture;

    use super::DeadSelector;

    fn page(name: &str, wxml: &str, wxss: &str) -> (Fixture, Project) {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", wxml)
            .write("pages/index/index.wxss", wxss);
        let project = fixture.project();
        (fixture, project)
    }

    /// 单页工程 + 一个自定义组件；页面只写 `<card/>`。
    fn component(
        name: &str,
        component_wxml: &str,
        component_wxss: &str,
        component_json: &str,
    ) -> (Fixture, Project) {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                r#"{"usingComponents":{"card":"/components/card/index"}}"#,
            )
            .write("pages/index/index.wxml", "<card/>")
            .write("components/card/index.json", component_json)
            .write("components/card/index.wxml", component_wxml)
            .write("components/card/index.wxss", component_wxss);
        let project = fixture.project();
        (fixture, project)
    }

    #[test]
    fn descendant_that_matches_nothing_is_dead() {
        // `.a` 与 `.skyline` 都存在，但分属不同节点：写成后代关系永不命中。
        let (_f, project) = page(
            "dead-descendant",
            r#"<view class="a"/><view class="skyline"/>"#,
            ".a .skyline { color: red; }",
        );
        let diags = DeadSelector.check(&project);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert!(diags[0].message.contains("`.a .skyline`"), "{diags:#?}");
        assert!(diags[0].message.contains("不命中任何节点"), "{diags:#?}");
    }

    #[test]
    fn class_that_never_appears_is_not_reported() {
        // 完全没用到的 CSS 属于 dead code（#1443 排除）；类名一个都对不上时
        // 报出来会把整份 vendor 样式表变成噪音。
        let (_f, project) = page(
            "dead-unused",
            r#"<view class="a"/>"#,
            ".a .nowhere { color: red; }\n.nothing { color: red; }",
        );
        assert!(DeadSelector.check(&project).is_empty());
    }

    #[test]
    fn matched_descendant_is_not_dead() {
        let (_f, project) = page(
            "alive-descendant",
            r#"<view class="a"><view class="b"/></view>"#,
            ".a .b { color: red; }",
        );
        assert!(DeadSelector.check(&project).is_empty());
    }

    #[test]
    fn modifier_on_the_same_node_is_not_dead() {
        // `.a.skyline` 是同一个节点上的两个类，不是后代关系。
        let (_f, project) = page(
            "same-node-modifier",
            r#"<view class="a skyline"/>"#,
            ".a.skyline { color: red; }",
        );
        assert!(DeadSelector.check(&project).is_empty());
    }

    #[test]
    fn slot_projection_is_not_reported() {
        // 组件文档里 `.item` 由父级通过 slot 投影进来，静态看不到。
        let (_f, project) = component(
            "slot-projection",
            r#"<view class="list"><slot/></view>"#,
            ".list .item { color: red; }",
            "{}",
        );
        assert!(DeadSelector.check(&project).is_empty());
    }

    #[test]
    fn slot_makes_combinator_selectors_unknown() {
        // slot 投影进来的内容可以任意嵌套，`.body .list` 在投影内容里仍可能命中，
        // 所以含 slot 的文档不对带组合符的选择器下「0 命中」结论。这是保守的代价：
        // 含 slot 的组件里，关系写错只能靠人工发现。
        let (_f, project) = component(
            "slot-bare",
            r#"<view class="list"><slot/><view class="body"/></view>"#,
            ".body .list { color: red; }",
            "{}",
        );
        assert!(DeadSelector.check(&project).is_empty());
    }

    #[test]
    fn keyframes_are_not_selectors() {
        // `@keyframes` 里的 `from` / `to` / 百分比不是选择器。
        let (_f, project) = page(
            "dead-keyframes",
            r#"<view class="a"/>"#,
            "@keyframes fade { from { opacity: 0; } to { opacity: 1; } }\n.a { animation: fade 1s; }",
        );
        assert!(DeadSelector.check(&project).is_empty());
    }

    #[test]
    fn tag_subject_is_not_checked() {
        // 主体没有 class：要么是没用到的通用样式（dead code），要么由
        // `attribute-selector` 负责。
        let (_f, project) = page(
            "dead-tag-subject",
            r#"<view class="svg"/>"#,
            "video:not(.svg) { opacity: 1; }",
        );
        assert!(DeadSelector.check(&project).is_empty());
    }

    #[test]
    fn class_only_in_hover_class_is_not_reported() {
        // `hover-class` 会在按下时挂上类，静态看不见；这种「类名确定存在」判断不成立。
        let (_f, project) = page(
            "dead-hover-class",
            r#"<view class="item" hover-class="item-hover"/>"#,
            ".item.item-hover { background: #eee; }",
        );
        assert!(DeadSelector.check(&project).is_empty());
    }

    #[test]
    fn pseudo_element_is_ignored() {
        let (_f, project) = page(
            "pseudo-element",
            r#"<view class="a"/>"#,
            ".a::after { content: 'x'; }",
        );
        assert!(DeadSelector.check(&project).is_empty());
    }

    #[test]
    fn runtime_pseudo_class_is_ignored() {
        let (_f, project) = page(
            "runtime-pseudo",
            r#"<view class="a"/>"#,
            ".a:hover { color: red; } .a:not(:active) { color: blue; }",
        );
        assert!(DeadSelector.check(&project).is_empty());
    }

    #[test]
    fn template_include_children_are_unknown() {
        // `<template is>` 展开的内容不在本文档里，`.list .item` 可能来自它。
        let (_f, project) = page(
            "template-children",
            r#"<view class="list"><template is="row"/></view>"#,
            ".list .item { color: red; } .item { color: blue; }",
        );
        assert!(DeadSelector.check(&project).is_empty());
    }

    #[test]
    fn generics_component_is_skipped() {
        // `<selectable>` 是 componentGenerics 占位标签，渲染结果静态未知。
        let (_f, project) = component(
            "generics",
            r#"<view class="list"><selectable/></view>"#,
            ".list .item { color: red; }",
            r#"{"componentGenerics":{"selectable":true}}"#,
        );
        assert!(DeadSelector.check(&project).is_empty());
    }

    #[test]
    fn runtime_root_selectors_are_ignored() {
        // `page` 是页面根、`:host` 是组件宿主，两者都不在 WXML 元素树里。
        let (_f, project) = page(
            "runtime-root",
            r#"<view class="a"/>"#,
            "page { background: #fff; } .a:host { color: red; }",
        );
        assert!(DeadSelector.check(&project).is_empty());
    }

    #[test]
    fn unparseable_selector_is_skipped() {
        let (_f, project) = page(
            "unparseable",
            r#"<view class="a"/>"#,
            ".a: {{ color: red; }",
        );
        assert!(DeadSelector.check(&project).is_empty());
    }
}
