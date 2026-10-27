//! `skyline/implicit-color`：省略颜色的 `border` / `text-decoration` / 阴影在 Skyline 下
//! 默认取黑色，而 WebView 取元素自身的 `color`。
//!
//! 依据：CSS 里 `border-color`、`text-decoration-color`、阴影颜色的初始值都是
//! `currentColor`，WebView 按此绘制。Skyline 这条路径受实验开关 `current_color` 控制
//! （`common/feature_toggle.h:26`），开关默认关闭（`common/feature_toggle.cc:20-22` 静态初值
//! 为 false，`Initialize()` 才用 `get_experiment("clicfg_skyline_current_color")` 覆盖），
//! 关闭时初始值退化成 `Black()`（`css/css_properties/border_top_color.cc:12-16`、
//! `text_decoration_color.cc:13-17`、`common/graphics/shadow_data.h:20-26`）。
//!
//! 只报「省略颜色、且元素有效 `color` 不是黑色」的声明：显式写了 `currentColor`、
//! 显式写颜色、值里含变量时一律跳过——显式 `currentColor` 的解析不受开关影响
//! （`css/css_parsing_utils.h:256-258`）。
//!
//! 开关是运行时灰度，静态判不出，所以这条按 `Info` 报：它说明「若该实验未开启，
//! 这里的颜色和 WebView 不一致」，而不是断言一定错。

use std::collections::{HashMap, HashSet};

use crate::input::wxml::{WxmlDoc, WxmlElement};
use crate::input::wxss::{ValueToken, WxssDeclaration};
use crate::matcher::Tree;
use crate::project::Project;
use crate::rules::Rule;
use crate::rules::RuleMeta;
use crate::rules::wxml::{load_doc, resolved_property, walk};
use crate::rules::wxss::value_util::at_declaration;
use crate::{Diagnostic, Severity};

/// 省略颜色会退回黑色的属性。
const PROPERTIES: &[&str] = &[
    "border",
    "border-top",
    "border-right",
    "border-bottom",
    "border-left",
    "text-decoration",
    "box-shadow",
    "text-shadow",
];

/// 这些属性里「不是颜色」的关键字。值里出现表外的标识符就当作写了颜色。
const KEYWORDS: &[&str] = &[
    // border width / style
    "thin",
    "medium",
    "thick",
    "none",
    "hidden",
    "solid",
    "dashed",
    "dotted",
    "double",
    "groove",
    "ridge",
    "inset",
    "outset",
    // text-decoration
    "underline",
    "overline",
    "line-through",
    "wavy",
    "auto",
    "from-font",
    "blink",
];

pub struct ImplicitColor;

impl ImplicitColor {
    pub const ID: &'static str = "skyline/implicit-color";
}

impl Rule for ImplicitColor {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Info,
            description: "省略颜色时 Skyline 用黑色而不是元素自身的 color（受实验开关影响）",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        let mut reported: HashSet<(String, usize)> = HashSet::new();
        let mut docs: HashMap<String, Option<WxmlDoc>> = HashMap::new();

        for page in &project.pages {
            if page.opted_out {
                continue;
            }
            let Some(page_wxml) = page.files.wxml.as_deref() else {
                continue;
            };
            let mut paths = vec![page_wxml.to_string()];
            paths.extend(project.page_components(&page.route).into_iter().filter_map(
                |component| {
                    project
                        .components
                        .iter()
                        .find(|unit| unit.path == component)
                        .and_then(|unit| unit.files.wxml.clone())
                },
            ));
            for path in &paths {
                if !docs.contains_key(path) {
                    docs.insert(path.clone(), load_doc(project, path));
                }
            }
            let loaded: Vec<&WxmlDoc> = paths
                .iter()
                .filter_map(|path| docs.get(path).and_then(Option::as_ref))
                .collect();

            for doc in loaded {
                let tree = Tree::new(doc);
                walk(doc, |element, ancestors| {
                    if effective_color(project, &tree, element, ancestors).is_none() {
                        return;
                    }
                    for property in PROPERTIES {
                        let Some(resolved) = resolved_property(project, &tree, element, property)
                        else {
                            continue;
                        };
                        let (Some(sheet), Some(declaration)) =
                            (resolved.sheet, resolved.declaration)
                        else {
                            continue;
                        };
                        if !lacks_color(declaration) {
                            continue;
                        }
                        if !reported.insert((sheet.doc.path().to_string(), declaration.offset)) {
                            continue;
                        }
                        out.push(at_declaration(
                            Self::ID,
                            &sheet.doc,
                            declaration,
                            Severity::Info,
                            format!(
                                "`{}` 没写颜色：Skyline 下这里默认黑色，WebView 下取元素的 `color`",
                                declaration.name
                            ),
                            "显式写出颜色（含 `currentColor`）即可两端一致",
                        ));
                    }
                });
            }
        }
        out
    }
}

/// 元素的有效 `color`：自身或最近祖先上的声明；黑色不算（那种情况两端都是黑色）。
///
/// `color` 是继承属性，而 `resolved_property` 只看元素自身，所以这里手动向上找。
fn effective_color<'a>(
    project: &'a Project,
    tree: &Tree<'_>,
    element: &'a WxmlElement,
    ancestors: &[&'a WxmlElement],
) -> Option<String> {
    let value = std::iter::once(element)
        .chain(ancestors.iter().rev().copied())
        .find_map(|candidate| {
            resolved_property(project, tree, candidate, "color").map(|resolved| resolved.value)
        })?;
    let value = value.trim().to_ascii_lowercase();
    match value.as_str() {
        "black" | "#000" | "#000000" | "#000000ff" => None,
        other => Some(other.to_string()),
    }
}

/// 声明是否「省略了颜色」：没有任何颜色、也没有显式 `currentColor`。
///
/// 值是原文（`value_is_raw`，含 `$x` / `@x` / 插值时）只会有一个 `Other`，按「判不了」跳过。
fn lacks_color(declaration: &WxssDeclaration) -> bool {
    !declaration.tokens.iter().any(|token| match token {
        // 显式颜色；函数（`rgb()` / `var()` / `calc()`）一律当作不可静态判定。
        ValueToken::Color(_) | ValueToken::Function { .. } | ValueToken::Dynamic => true,
        // 表外的标识符就是颜色（含 `currentColor` 与具名颜色）；预处理器变量也算。
        ValueToken::Ident(name) => !KEYWORDS
            .iter()
            .any(|keyword| keyword.eq_ignore_ascii_case(name)),
        ValueToken::Other(_) | ValueToken::Str(_) | ValueToken::Url(_) => true,
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use crate::Diagnostic;
    use crate::rules::Rule;
    use crate::rules::testutil::wxss::Fixture;

    use super::ImplicitColor;

    fn check(name: &str, wxml: &str, wxss: &str) -> Vec<Diagnostic> {
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
        ImplicitColor.check(&project)
    }

    #[test]
    fn border_without_color_on_colored_element_is_reported() {
        let diagnostics = check(
            "implicit-color-border",
            r#"<view class="card"/>"#,
            ".card { color: #fff; border: 1px solid; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("border"));
        assert!(diagnostics[0].message.contains("黑色"));
    }

    #[test]
    fn explicit_color_and_current_color_are_not_reported() {
        let diagnostics = check(
            "implicit-color-explicit",
            r#"<view class="card"/>"#,
            ".card { color: #fff; border: 1px solid currentColor; box-shadow: 0 2px 4px #000; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn black_color_is_not_reported() {
        // 元素本来就是黑色，省略颜色两端一致。
        let diagnostics = check(
            "implicit-color-black",
            r#"<view class="card"/>"#,
            ".card { color: #000; border: 1px solid; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn inherited_color_counts() {
        let diagnostics = check(
            "implicit-color-inherit",
            r#"<view class="wrap"><view class="title"/></view>"#,
            ".wrap { color: #333; }\n.title { border-bottom: 1px solid; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("border-bottom"));
    }

    #[test]
    fn variable_values_are_skipped() {
        let diagnostics = check(
            "implicit-color-var",
            r#"<view class="card"/>"#,
            ".card { color: #fff; border: 1px solid var(--line); }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }
}
