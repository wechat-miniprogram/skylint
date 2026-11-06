//! 级联的判别用例：specificity / `!important` / 内联 `style` 三者都按引擎口径排。

use crate::matcher::Tree;
use crate::rules::testutil::Fixture;

/// 单页工程里某个标签的某属性级联结果，升序（末位生效）。
fn cascade(wxml: &str, wxss: &str, tag: &str, property: &str) -> Vec<(String, bool)> {
    let fixture = Fixture::page("cascade", wxml, wxss);
    let project = fixture.project();
    let path = project.pages[0].files.wxml.clone().unwrap();
    let doc = super::load_doc(&project, &path).unwrap();
    let tree = Tree::new(&doc);
    let element = doc
        .elements()
        .find(|element| element.tag == tag)
        .expect("fixture 里没有这个标签");
    super::cascade_entries(&project, &tree, element, property)
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
