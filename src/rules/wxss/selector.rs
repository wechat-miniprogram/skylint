//! 简单选择器：只解析能静态判定的形状（标签 / `.class` / `#id` 及其组合）。
//!
//! 跨文件规则要回答「这条声明会落到哪些 WXML 节点上」。组合符（空格、`>`、`+`、`~`）、
//! 属性选择器、带参数的伪类（`:not()` / `:nth-child()`）一律放弃——静态判定不了就当作
//! 「没看见」，宁可漏报。不带参数、不影响「命中哪个节点」的伪类（`:first-child` 等与
//! `::before` / `::after`）直接忽略；不受支持的伪类（`:hover` 等）引擎会丢弃整条规则，
//! 同样归为未知。动态属性（`class="a {{x}}"`）返回 [`Match::Unknown`]。

use crate::input::wxml::WxmlElement;
use crate::input::wxss::{SelectorPart, WxssSelector};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimpleSelector {
    tag: Option<String>,
    /// `*`：不约束标签。
    universal: bool,
    id: Option<String>,
    classes: Vec<String>,
    /// 选择器带 `::before` / `::after`（含单冒号写法）：命中的是生成的文本节点。
    text_pseudo: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Match {
    No,
    Yes,
    /// 依赖动态属性，静态判断不了。
    Unknown,
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
            text_pseudo: false,
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
                    if matches!(name.as_str(), "before" | "after") {
                        parsed.text_pseudo = true;
                    }
                }
                SelectorPart::Other => return None,
            }
        }
        Some(parsed)
    }

    /// 选择器命中的是伪元素生成的文本节点（`::before` / `::after`）。
    pub fn text_pseudo_element(&self) -> bool {
        self.text_pseudo
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

    pub fn matches(&self, element: &WxmlElement) -> Match {
        if let Some(tag) = &self.tag
            && &element.tag != tag
        {
            return Match::No;
        }
        if let Some(id) = &self.id {
            match attr(element, "id") {
                Attr::Static(value) if value == id => {}
                Attr::Static(_) | Attr::Missing => return Match::No,
                Attr::Dynamic => return Match::Unknown,
            }
        }
        if !self.classes.is_empty() {
            match attr(element, "class") {
                Attr::Static(value) => {
                    let classes: Vec<&str> = value.split_whitespace().collect();
                    if !self.classes.iter().all(|c| classes.contains(&c.as_str())) {
                        return Match::No;
                    }
                }
                Attr::Missing => return Match::No,
                Attr::Dynamic => return Match::Unknown,
            }
        }
        Match::Yes
    }
}

enum Attr<'a> {
    Static(&'a str),
    /// 值里带插值（`class="a {{x}}"`），静态判定不了。
    Dynamic,
    Missing,
}

fn attr<'a>(element: &'a WxmlElement, name: &str) -> Attr<'a> {
    match element.attributes.iter().find(|a| a.name == name) {
        None => Attr::Missing,
        Some(attribute) if attribute.is_static => {
            Attr::Static(attribute.value.as_deref().unwrap_or("").trim())
        }
        Some(_) => Attr::Dynamic,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::wxml;
    use crate::input::wxss;

    fn element(source: &str) -> WxmlElement {
        let doc = wxml::parse("x.wxml", source).unwrap();
        doc.elements().next().unwrap().clone()
    }

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

    #[test]
    fn matches_static_attributes() {
        let el = element(r#"<view class="a b" id="main"/>"#);
        assert_eq!(selector("view.a").unwrap().matches(&el), Match::Yes);
        assert_eq!(selector("#main").unwrap().matches(&el), Match::Yes);
        assert_eq!(selector("text").unwrap().matches(&el), Match::No);
        assert_eq!(selector(".c").unwrap().matches(&el), Match::No);
        assert_eq!(selector("#other").unwrap().matches(&el), Match::No);
    }

    #[test]
    fn dynamic_attributes_are_unknown() {
        let el = element(r#"<view class="a {{x}}"/>"#);
        assert_eq!(selector(".a").unwrap().matches(&el), Match::Unknown);
        assert_eq!(selector("view").unwrap().matches(&el), Match::Yes);
        let dynamic_id = element(r#"<view id="{{x}}" class="a"/>"#);
        assert_eq!(selector("#x").unwrap().matches(&dynamic_id), Match::Unknown);
    }
}
