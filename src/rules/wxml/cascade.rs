//! 样式级联：元素某个属性有哪些候选声明、怎么排序、最后取哪一条。
//!
//! 排序口径对齐引擎（`style/cascade_priority.h`）：`!important` 最高，其次内联
//! `style` 胜过任何选择器，再比 specificity，最后比出现顺序。[`cascade_entries`]
//! 返回升序，末位生效。
//!
//! 样式作用域是唯一口径（[`crate::project::Project::document_covers`]）：单元按自己的
//! 作用域求值，独立模板文件与 `<include>` 片段按引用它的单元求值。所有规则都走这里，
//! 不要再另写一份筛选。
//!
//! 模板定义体 / 片段里的祖先链不在本文件：按真实实例化路径匹配（[`TemplateMatch`]）。

use std::cell::OnceCell;
use std::rc::Rc;

use crate::input::wxml::WxmlElement;
use crate::matcher::{Dialect, FlatNode, Tree, Tri, node_of};
use crate::project::Project;
use crate::wxml_graph::AncestorNodePaths;

use super::structure;

/// 一条参与级联的声明，带溯源信息。
///
/// 排序口径对齐引擎（`style/cascade_priority.h`）：importance 最高，其次 inline 胜过
/// 任何选择器，再比 specificity，最后比出现顺序。`cascade_entries` 返回升序，末位生效。
pub(crate) struct CascadeEntry<'a> {
    pub value: String,
    /// 来自样式表时的文件与声明；来自内联 `style` 时都是 `None`。
    pub sheet: Option<&'a crate::project::StyleSheet>,
    pub declaration: Option<&'a crate::input::wxss::WxssDeclaration>,
    /// 命中的选择器原文；内联 `style` 为 `None`。
    pub selector: Option<String>,
    /// 选择器优先级（packed u32）；内联 `style` 用 `u32::MAX`。
    pub specificity: u32,
    pub important: bool,
    /// 声明所处的 at-rule 上下文（`@media …`）；内联为空。
    pub at_rules: &'a [crate::input::wxss::WxssAtRule],
    pub inline: bool,
    /// 出现顺序，仅用于同级比较。
    order: usize,
}

/// 元素某个属性的有效取值。
///
/// 与 [`CascadeEntry`] 同源：这里只留「赢了的那一条」需要的信息，需要被覆盖的
/// 声明或 at-rule 上下文时用 `cascade_entries`。
pub(crate) struct Resolved<'a> {
    pub value: String,
    /// 来自样式表时的文件与声明；来自内联 `style` 时都是 `None`。
    pub sheet: Option<&'a crate::project::StyleSheet>,
    pub declaration: Option<&'a crate::input::wxss::WxssDeclaration>,
}

/// 祖先链静态不完整的元素（模板定义体 / `<include>` 片段）的求值上下文。
///
/// 这些元素的真实祖先在别的文档里（`<template is>` 的调用处、引用模板的单元），
/// 单文件匹配不到。改用 [`crate::wxml_graph::WxmlGraph::ancestor_nodes`] 给的**逐条**
/// 实例化路径：只要任一条路径满足整条选择器就算命中。
///
/// 语义是「存在即成立」：诊断落在源码位置，表达不了「只在第 3 个使用点坏」；按
/// 「没有一条命中」去报会在有命中的使用点上变成误报。
struct TemplateMatch<'a> {
    project: &'a Project,
    /// 选择器按哪种渲染器语义求值：Skyline 丢弃属性条件，结论会不同。
    dialect: Dialect,
    subject: FlatNode,
    /// `(逐条祖先路径, 是否截断)`：真有选择器需要时才枚举（枚举一次要回溯整张图）。
    paths: OnceCell<Rc<AncestorNodePaths>>,
}

impl<'a> TemplateMatch<'a> {
    /// 普通单元文档返回 `None`（走严格匹配）。
    fn new(
        project: &'a Project,
        tree: &Tree<'_>,
        element: &WxmlElement,
        dialect: Dialect,
    ) -> Option<Self> {
        if !tree.ancestors_unknown(element) {
            return None;
        }
        Some(Self {
            project,
            dialect,
            subject: node_of(&element.tag, &element.attributes),
            paths: OnceCell::new(),
        })
    }

    fn paths(&self, tree: &Tree<'_>, element: &WxmlElement) -> Rc<AncestorNodePaths> {
        self.paths
            .get_or_init(|| self.project.graph().ancestor_nodes(tree.path(), element))
            .clone()
    }

    /// 这条选择器在这个元素上的判定：`Some(true)` 命中、`Some(false)` 不命中、
    /// `None` 判不了。
    ///
    /// 结论只取决于 (元素, 选择器)，而级联按属性逐条来问，所以在图上按元素缓存
    /// （多属性规则否则会把同一条选择器重判十几次）。
    fn verdict(&self, tree: &Tree<'_>, element: &WxmlElement, selector: &str) -> Option<bool> {
        self.project.graph().cached_verdict(
            tree.doc_id(),
            element,
            self.dialect == Dialect::WebView,
            selector,
            || self.compute_verdict(tree, element, selector),
        )
    }

    fn compute_verdict(
        &self,
        tree: &Tree<'_>,
        element: &WxmlElement,
        selector: &str,
    ) -> Option<bool> {
        // 先做主体过滤：主体都命中不了就不用付匹配 / 路径枚举的代价。
        let flat = tree.flat(selector);
        let subject_could_match = match flat.as_ref() {
            Some(flat) => flat.matches_subject(&self.subject),
            None => tree.subject_matches(element, selector),
        };
        if !subject_could_match {
            return Some(false);
        }
        // 单文件里能确定命中就不用看实例化路径：静态祖先必然是最真实路径上的一段。
        if tree.matches(element, selector, self.dialect) == Tri::Yes {
            return Some(true);
        }
        // 主体可能命中，但选择器落在受限文法之外（属性 / 伪类 / 兄弟组合符）：
        // 说不好会命中哪条使用点。
        let Some(flat) = flat.as_ref() else {
            return None;
        };
        if flat.is_simple() {
            return Some(true);
        }
        let paths = self.paths(tree, element);
        let (paths, truncated) = &*paths;
        if paths.iter().any(|path| flat.matches(&self.subject, path)) {
            return Some(true);
        }
        if paths.is_empty() || *truncated {
            // 一条实例化路径都没有，或路径被上限截掉了：断言不了「不命中」。
            return None;
        }
        Some(false)
    }
}

/// 元素某个属性参与级联的全部声明，升序（末位生效）。
///
/// 每条都带溯源：来自哪个样式表 / 哪条选择器 / 优先级 / 是否 `!important` /
/// at-rule 上下文。需要「为什么这条被覆盖」时看它前一条；需要「这个值会不会变」
/// 时看 `at_rules`。
pub(crate) fn cascade_entries<'a>(
    project: &'a Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
    property: &str,
) -> Vec<CascadeEntry<'a>> {
    cascade_entries_with_dialect_impl(project, tree, element, property, Dialect::Skyline).0
}

/// 同上，但有选择器判不了时返回 `None`。
///
/// 「判不了」指模板定义体 / 片段的祖先链靠实例化路径求值，没有路径或选择器超出受限
/// 文法。此时候选集不完整，不能当成「没写」——调用方一律跳过（判不了就不报）。
pub(crate) fn cascade_entries_with<'a>(
    project: &'a Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
    property: &str,
) -> Option<Vec<CascadeEntry<'a>>> {
    let (entries, undecidable) =
        cascade_entries_with_dialect_impl(project, tree, element, property, Dialect::Skyline);
    (!undecidable).then_some(entries)
}

fn cascade_entries_with_dialect_impl<'a>(
    project: &'a Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
    property: &str,
    dialect: Dialect,
) -> (Vec<CascadeEntry<'a>>, bool) {
    let mut entries = Vec::new();
    let mut order = 0;
    let mut undecidable = false;
    let template = TemplateMatch::new(project, tree, element, dialect);

    for sheet in project
        .covering_sheets(tree.path())
        .iter()
        .map(|&index| &project.styles[index])
    {
        // 只走声明了这个属性的规则：级联是按属性查的，其余规则连选择器都不用匹配。
        for rule in sheet.doc.rules_declaring(property) {
            let declarations: Vec<&crate::input::wxss::WxssDeclaration> = rule
                .declarations
                .iter()
                .filter(|decl| decl.name.eq_ignore_ascii_case(property))
                .collect();
            if declarations.is_empty() {
                continue;
            }
            // 一条规则可能有多个选择器，各自优先级不同：取命中里最高的那个；
            // 判不了时这条规则的可能性记在 `undecidable` 上。
            let mut unknown = false;
            let matched = rule
                .selectors
                .iter()
                .filter_map(|selector| {
                    let text = selector.text.trim();
                    let hit = match &template {
                        None => tree.matches(element, text, dialect) == Tri::Yes,
                        Some(template) => match template.verdict(tree, element, text) {
                            Some(hit) => hit,
                            None => {
                                unknown = true;
                                false
                            }
                        },
                    };
                    hit.then(|| tree.selector_specificity(text).map(|spec| (spec, text)))
                        .flatten()
                })
                .max_by_key(|(spec, _)| *spec);
            undecidable |= unknown;
            let Some((specificity, selector)) = matched else {
                continue;
            };
            for declaration in declarations {
                order += 1;
                entries.push(CascadeEntry {
                    value: declaration.value.trim().to_string(),
                    sheet: Some(sheet),
                    declaration: Some(declaration),
                    selector: Some(selector.to_string()),
                    specificity,
                    important: declaration.important,
                    at_rules: &rule.at_rules,
                    inline: false,
                    order,
                });
            }
        }
    }

    if let Some(style) =
        structure::find_attr(element, "style").and_then(|attr| attr.value.as_deref())
    {
        for (value, important) in inline_properties(style, property) {
            order += 1;
            entries.push(CascadeEntry {
                value,
                sheet: None,
                declaration: None,
                selector: None,
                specificity: u32::MAX,
                important,
                at_rules: &[],
                inline: true,
                order,
            });
        }
    }

    // 升序：importance → specificity → 出现顺序；末位即生效值。
    entries.sort_by_key(|entry| (entry.important, entry.specificity, entry.order));
    (entries, undecidable)
}

/// 元素命中的样式表声明，按级联顺序（内联 `style` 除外）。
///
/// 需要知道「丢掉其中一条之后会落到哪一条」时用它——只看有效取值会丢掉这个信息。
pub(crate) fn matched_declarations<'a>(
    project: &'a Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
    property: &str,
) -> Vec<(
    &'a crate::project::StyleSheet,
    &'a crate::input::wxss::WxssDeclaration,
)> {
    cascade_entries(project, tree, element, property)
        .into_iter()
        .filter_map(|entry| Some((entry.sheet?, entry.declaration?)))
        .collect()
}

/// 元素某个属性的有效取值。
///
/// 判不了时（模板定义体 / 片段的祖先未知、选择器超出受限文法）返回 `None`；需要区分
/// 「确实没写」时用 [`resolved_property_with`]。
pub(crate) fn resolved_property<'a>(
    project: &'a Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
    property: &str,
) -> Option<Resolved<'a>> {
    resolved_property_with(project, tree, element, property).0
}

/// [`resolved_property`] 的方言变体。
pub(crate) fn resolved_property_with_dialect<'a>(
    project: &'a Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
    property: &str,
    dialect: Dialect,
) -> (Option<Resolved<'a>>, bool) {
    let (mut entries, undecidable) =
        cascade_entries_with_dialect_impl(project, tree, element, property, dialect);
    (
        entries.pop().map(|winner| Resolved {
            value: winner.value,
            sheet: winner.sheet,
            declaration: winner.declaration,
        }),
        undecidable,
    )
}

/// 同上，另外告诉你「有没有选择器判不了」。
///
/// 布尔位与 [`cascade_entries_with`] 同源：结论依赖这个属性时，判不了就不能报
/// （宁可漏报）。
pub(crate) fn resolved_property_with<'a>(
    project: &'a Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
    property: &str,
) -> (Option<Resolved<'a>>, bool) {
    resolved_property_with_dialect(project, tree, element, property, Dialect::Skyline)
}

/// 只关心「有没有确定取值」时用它：判不了与没写都返回 `None`。
///
/// 需要区分这两种「没有」时用 [`resolved_property_with`]。
pub(crate) fn resolved_property_checked<'a>(
    project: &'a Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
    property: &str,
) -> Option<Resolved<'a>> {
    let (resolved, undecidable) = resolved_property_with(project, tree, element, property);
    (!undecidable).then_some(resolved).flatten()
}

/// 内联 `style` 里某属性的全部声明：`(值, 是否 !important)`，按出现顺序。
fn inline_properties(style: &str, property: &str) -> Vec<(String, bool)> {
    style
        .split(';')
        .filter_map(|declaration| {
            let (name, value) = declaration.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case(property)
                .then(|| split_important(value.trim()))
        })
        .collect()
}

/// 拆掉值末尾的 `!important`（大小写不敏感）。
fn split_important(value: &str) -> (String, bool) {
    let lower = value.to_ascii_lowercase();
    match lower.rfind("!important") {
        Some(index) => (value[..index].trim().to_string(), true),
        None => (value.trim().to_string(), false),
    }
}

#[cfg(test)]
mod tests {
    //! 级联的判别用例：specificity / `!important` / 内联 `style` 三者都按引擎口径排。

    use crate::matcher::Tree;
    use crate::rules::testutil::Fixture;
    use crate::rules::wxml::load_doc;

    use super::cascade_entries;

    /// 单页工程里某个标签的某属性级联结果，升序（末位生效）。
    fn cascade(wxml: &str, wxss: &str, tag: &str, property: &str) -> Vec<(String, bool)> {
        let fixture = Fixture::page("cascade", wxml, wxss);
        let project = fixture.project();
        let path = project.pages[0].files.wxml.clone().unwrap();
        let doc = load_doc(&project, &path).unwrap();
        let tree = Tree::new(&doc);
        let element = doc
            .elements()
            .find(|element| element.tag == tag)
            .expect("fixture 里没有这个标签");
        cascade_entries(&project, &tree, element, property)
            .into_iter()
            .map(|entry| (entry.value, entry.important))
            .collect()
    }

    fn winner(got: &[(String, bool)]) -> Option<(&str, bool)> {
        got.last()
            .map(|(value, important)| (value.as_str(), *important))
    }

    /// 引擎先比 specificity 再比源码位置（`style/style_resolver.cc` 的 `CompareRules`）。
    /// 按「取最后一条」会得出 `blue`，与引擎相反。
    #[test]
    fn specificity_wins_over_source_order() {
        let got = cascade(
            r#"<view class="a b"/>"#,
            ".a.b { color: red }\n.a { color: blue }",
            "view",
            "color",
        );
        assert_eq!(winner(&got), Some(("red", false)));
    }

    /// `!important` 胜过 specificity 更高但没有 important 的声明。
    #[test]
    fn important_beats_specificity() {
        let got = cascade(
            r#"<view class="a b"/>"#,
            ".a { color: blue !important }\n.a.b { color: red }",
            "view",
            "color",
        );
        assert_eq!(winner(&got), Some(("blue", true)));
    }

    /// 内联 `style` 胜过任何选择器（引擎的 `CSSLocation::kInline`）。
    #[test]
    fn inline_beats_any_selector() {
        let got = cascade(
            r#"<view class="a" style="color: green"/>"#,
            ".a { color: red }",
            "view",
            "color",
        );
        assert_eq!(winner(&got), Some(("green", false)));
    }

    /// 样式表里的 `!important` 胜过内联的非 important 声明。
    #[test]
    fn sheet_important_beats_plain_inline() {
        let got = cascade(
            r#"<view class="a" style="color: green"/>"#,
            ".a { color: red !important }",
            "view",
            "color",
        );
        assert_eq!(winner(&got), Some(("red", true)));
    }

    /// 内联的 `!important` 再压过样式表的 `!important`。
    #[test]
    fn inline_important_beats_sheet_important() {
        let got = cascade(
            r#"<view class="a" style="color: green !important"/>"#,
            ".a { color: red !important }",
            "view",
            "color",
        );
        assert_eq!(winner(&got), Some(("green", true)));
    }

    /// 被覆盖的声明也要保留（溯源），且内联值不带 `!important` 尾巴。
    #[test]
    fn overridden_declarations_are_kept() {
        let got = cascade(
            r#"<view class="a b" style="color: green !important"/>"#,
            ".a { color: red }\n.a.b { color: blue }",
            "view",
            "color",
        );
        assert_eq!(
            got,
            vec![
                ("red".to_string(), false),
                ("blue".to_string(), false),
                ("green".to_string(), true),
            ]
        );
    }
}
