//! 选择器的「形状」分类：`tag-selector-isolation` 只需要知道一条选择器是不是
//! 纯标签 / 纯 id，不关心它命中哪个节点（那件事由 [`crate::matcher`] 负责）。

use crate::input::wxss::{SelectorPart, WxssSelector};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimpleSelector {
    tag: Option<String>,
    /// `*`：不约束标签。
    universal: bool,
    id: Option<String>,
    classes: Vec<String>,
}

/// 不影响「命中哪个节点」的伪类 / 伪元素：引擎支持它、且不改变节点集合。
///
/// 依据：`css/css_parser_impl.h:271-329` 支持的 7 个伪类里，`:not()` / `:nth-child()`
/// 带参数会缩小命中集合，其余五个不影响节点本身；伪元素 `::before` / `::after`
/// 只是在该节点上生成一个子节点。
fn is_transparent_pseudo(element: bool, name: &str) -> bool {
    match name {
        "before" | "after" => true,
        _ if element => false,
        _ => matches!(
            name,
            "empty" | "host" | "only-child" | "first-child" | "last-child"
        ),
    }
}

impl SimpleSelector {
    /// 解析失败返回 `None`（含组合符 / 属性选择器 / 带参数或不受支持的伪类 / 空串）。
    pub fn parse(selector: &WxssSelector) -> Option<Self> {
        if selector.parts.is_empty() {
            return None;
        }
        let mut parsed = Self {
            tag: None,
            universal: false,
            id: None,
            classes: Vec::new(),
        };
        for part in &selector.parts {
            match part {
                SelectorPart::Tag(name) => parsed.tag = Some(name.clone()),
                SelectorPart::Universal => parsed.universal = true,
                SelectorPart::Class(name) => parsed.classes.push(name.clone()),
                SelectorPart::Id(name) => parsed.id = Some(name.clone()),
                SelectorPart::Pseudo {
                    name,
                    element,
                    args,
                } => {
                    if *args || !is_transparent_pseudo(*element, name) {
                        return None;
                    }
                }
                SelectorPart::Other => return None,
            }
        }
        Some(parsed)
    }

    /// 纯标签选择器（`view`、`*` 之外的）。WebView 下它会命中组件内部节点，
    /// Skyline 默认不跨组件匹配。
    pub fn is_tag_only(&self) -> bool {
        self.tag.is_some() && self.id.is_none() && self.classes.is_empty()
    }

    /// 纯 id 选择器（`#foo`）。
    pub fn is_id_only(&self) -> bool {
        self.id.is_some() && self.classes.is_empty()
    }

    pub fn tag(&self) -> Option<&str> {
        self.tag.as_deref()
    }

    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::wxss;

    /// 走真实的 wxss 解析，取第一条选择器的部件。
    fn selector(text: &str) -> Option<SimpleSelector> {
        let source = format!("{text} {{ color: red; }}");
        let doc = wxss::parse("x.wxss", &source).ok()?;
        SimpleSelector::parse(doc.rules.first()?.selectors.first()?)
    }

    #[test]
    fn parses_tag_class_and_id() {
        let parsed = selector("view.list#main").unwrap();
        assert_eq!(parsed.tag.as_deref(), Some("view"));
        assert_eq!(parsed.classes, vec!["list"]);
        assert_eq!(parsed.id.as_deref(), Some("main"));
        assert!(selector("*").is_some());
    }

    #[test]
    fn rejects_complex_shapes() {
        for text in [
            ".a .b",
            ".a > .b",
            "[data-x]",
            "a:hover",
            "a:not(.b)",
            "a:nth-child(2)",
            "a::placeholder",
            "",
            ".",
            " ",
        ] {
            assert!(selector(text).is_none(), "{text}");
        }
    }

    #[test]
    fn ignores_pseudos_that_do_not_change_the_matched_node() {
        assert_eq!(
            selector("view::after").unwrap().tag.as_deref(),
            Some("view")
        );
        for text in [
            "view:first-child",
            "view:last-child",
            "view:only-child",
            "view:empty",
            "view:host",
            "view:before",
        ] {
            assert!(selector(text).is_some(), "{text}");
        }
    }
}
