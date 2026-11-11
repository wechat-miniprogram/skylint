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

use std::collections::HashSet;

use crate::input::wxml::WxmlElement;
use crate::input::wxss::{ValueToken, WxssDeclaration};
use crate::matcher::Tree;
use crate::project::Project;
use crate::rules::Rule;
use crate::rules::RuleMeta;
use crate::rules::wxml::{ElementDoc, element_docs, resolved_property_with, walk_skyline};
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

        for ElementDoc { doc, reach } in element_docs(project) {
            let tree = Tree::new(&doc);
            walk_skyline(&doc, |element, ancestors| {
                // 颜色判不了就不能断言「有效 color 不是黑色」：不报。
                let (color, undecidable) = effective_color(project, &tree, element, ancestors);
                if undecidable || color.is_none() {
                    return;
                }
                for property in PROPERTIES {
                    let (resolved, undecidable) =
                        resolved_property_with(project, &tree, element, property);
                    if undecidable {
                        continue;
                    }
                    let Some(resolved) = resolved else {
                        continue;
                    };
                    let (Some(sheet), Some(declaration)) = (resolved.sheet, resolved.declaration)
                    else {
                        continue;
                    };
                    if !lacks_color(declaration) {
                        continue;
                    }
                    if invisible(project, &tree, element, declaration) != Some(false) {
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
                        reach.help("显式写出颜色（含 `currentColor`）即可两端一致"),
                    ));
                }
            });
        }
        out
    }
}

/// 页面的基色：`page { color: … }` / `:host { color: … }`。
///
/// 页面根不在元素树里，`resolved_property` 看不到它；后写的声明生效。
///
/// 覆盖范围按 [`Project::covering_sheets`]：单元用自己那几张，`<include>` 片段用
/// 引用它的单元的样式表。片段不属于任何单元，按单元作用域取不到东西，用它会把
/// 片段里的元素全部当成「没有基色」跳过。
fn page_color(project: &Project, tree: &Tree<'_>) -> Option<String> {
    project
        .covering_sheets(tree.path())
        .iter()
        .map(|&index| &project.styles[index])
        .flat_map(|sheet| &sheet.doc.rules)
        .filter(|rule| {
            rule.selectors.iter().any(|selector| {
                let text = selector.text.trim();
                text.eq_ignore_ascii_case("page") || text.eq_ignore_ascii_case(":host")
            })
        })
        .flat_map(|rule| &rule.declarations)
        .filter(|declaration| declaration.name.eq_ignore_ascii_case("color"))
        .map(|declaration| declaration.value.clone())
        .last()
}

/// 声明本身不可见（宽度 0 / `none` / 颜色由 longhand 给出）：省略颜色不会产生可见差异。
///
/// `None` = longhand 判不了，不能断言「不可见」。
fn invisible(
    project: &Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
    declaration: &WxssDeclaration,
) -> Option<bool> {
    let name = declaration.name.to_ascii_lowercase();
    let has = |word: &str| {
        declaration.tokens.iter().any(
            |token| matches!(token, ValueToken::Ident(ident) if ident.eq_ignore_ascii_case(word)),
        )
    };
    if name == "text-decoration" {
        if has("none") {
            return Some(true);
        }
        // 颜色写在了 longhand 里：简写只是没重复它。
        let (resolved, undecidable) =
            resolved_property_with(project, tree, element, "text-decoration-color");
        if undecidable {
            return None;
        }
        return Some(resolved.is_some());
    }
    if !name.starts_with("border") {
        return Some(false);
    }
    // 宽度为 0 或边框样式为 none：没有可见边框。
    let zero = declaration.tokens.iter().any(|token| match token {
        ValueToken::Number(value)
        | ValueToken::Dimension { value, .. }
        | ValueToken::Percentage(value) => value.parse::<f64>().is_ok_and(|number| number == 0.0),
        _ => false,
    });
    if zero || has("none") {
        return Some(true);
    }
    // 颜色写在了 longhand 里：简写只是没重复它。
    let longhand = match name.as_str() {
        "border" => "border-color".to_string(),
        other => format!("{other}-color"),
    };
    let (resolved, undecidable) = resolved_property_with(project, tree, element, &longhand);
    if undecidable {
        return None;
    }
    Some(resolved.is_some())
}

/// 元素的有效 `color`：自身或最近祖先上的声明；黑色不算（那种情况两端都是黑色）。
///
/// `color` 是继承属性，而 `resolved_property` 只看元素自身，所以这里手动向上找。
/// 布尔位是「有选择器判不了」：这时返回的 `None` 不代表确定没有颜色。
fn effective_color<'a>(
    project: &'a Project,
    tree: &Tree<'_>,
    element: &'a WxmlElement,
    ancestors: &[&'a WxmlElement],
) -> (Option<String>, bool) {
    // `inherit` / `currentColor` 等都解析成父级的值：继续往上找真正的颜色。
    let mut value = None;
    for candidate in std::iter::once(element).chain(ancestors.iter().rev().copied()) {
        let (resolved, undecidable) = resolved_property_with(project, tree, candidate, "color");
        if undecidable {
            return (None, true);
        }
        let Some(resolved) = resolved else {
            continue;
        };
        let raw = resolved.value.trim().to_ascii_lowercase();
        match raw.as_str() {
            "inherit" | "unset" | "currentcolor" | "current-color" | "revert" => continue,
            // `initial` 是 CSS 的 `canvastext`（黑），与「没写」等效。
            "initial" => return (None, false),
            _ => {
                value = Some(resolved.value);
                break;
            }
        }
    }
    let Some(value) = value.or_else(|| page_color(project, tree)) else {
        return (None, false);
    };
    let value = value.trim().to_ascii_lowercase();
    // 变量与函数求解不了：运行时可能是黑色，也可能是任意色，按「判不了」跳过。
    if value.contains("var(") || value.contains("env(") || value.contains("{{") {
        return (None, false);
    }
    let compact: String = value
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect();
    let black = matches!(compact.as_str(), "black" | "#000" | "#000000" | "#000000ff")
        || compact.starts_with("rgb(0,0,0")
        || compact.starts_with("rgba(0,0,0");
    (if black { None } else { Some(value) }, false)
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

    /// `color: inherit` 沿祖先链解析到黑色时，无颜色的 border 两端都是黑。
    #[test]
    fn inherited_color_is_followed() {
        let diagnostics = check(
            "inherit-black",
            r#"<view class="a"><view class="b"/></view>"#,
            ".a { color: #000; } .b { color: inherit; border: 1px solid; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
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

    /// 颜色写在 longhand 里：简写只是没重复它，两端一致。
    #[test]
    fn text_decoration_color_longhand_is_not_reported() {
        let diagnostics = check(
            "implicit-color-decoration-longhand",
            r#"<text class="price">a</text>"#,
            ".price { color: #999; text-decoration: line-through; text-decoration-color: #999; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// `<include>` 片段按引用它的单元取基色：`page { color }` 写在页面样式里，
    /// 片段元素继承的是同一份页面基色。改前片段按单元作用域取样式表（恒为空），
    /// 这里会把「本该报」的省略颜色静默吞掉。
    #[test]
    fn include_fragment_takes_the_page_base_color() {
        let fixture = Fixture::new("implicit-color-include");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", r#"<include src="./part"/>"#)
            .write(
                "pages/index/index.wxss",
                "page { color: #fff; }\n.card { border: 1px solid; }",
            )
            .write("pages/index/part.wxml", r#"<view class="card"/>"#);
        let diagnostics = ImplicitColor.check(&fixture.project());
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("border"));
    }

    /// 片段里 `color` 判不了（祖先未知 / 选择器超出受限文法）：不能当成「没写」。
    #[test]
    fn undecidable_fragment_color_is_not_reported() {
        let fixture = Fixture::new("implicit-color-undecidable");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", r#"<include src="./part"/>"#)
            .write("pages/index/part.wxml", r#"<view class="card"/>"#)
            .write(
                "pages/index/index.wxss",
                ".card { color: #fff; border: 1px solid; }\n.ghost + .card { color: #000; }",
            );
        let diagnostics = ImplicitColor.check(&fixture.project());
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }
}
