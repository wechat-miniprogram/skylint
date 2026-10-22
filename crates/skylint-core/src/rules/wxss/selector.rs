//! 简单选择器：只解析能静态判定的形状（标签 / `.class` / `#id` 及其组合）。
//!
//! 跨文件规则要回答「这条声明会落到哪些 WXML 节点上」。组合符（空格、`>`、`+`、`~`）、
//! 伪类、属性选择器一律放弃——静态判定不了就当作「没看见」，宁可漏报。
//! 动态属性（`class="a {{x}}"`）返回 [`Match::Unknown`]，同样是为了不漏报变误报。

use crate::input::wxml::WxmlElement;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimpleSelector {
    tag: Option<String>,
    /// `*`：不约束标签。
    universal: bool,
    id: Option<String>,
    classes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Match {
    No,
    Yes,
    /// 依赖动态属性，静态判断不了。
    Unknown,
}

impl SimpleSelector {
    /// 解析失败返回 `None`（含组合符 / 伪类 / 属性选择器 / 空串）。
    pub fn parse(text: &str) -> Option<Self> {
        let mut rest = text.trim();
        if rest.is_empty() {
            return None;
        }
        let mut selector = Self {
            tag: None,
            universal: false,
            id: None,
            classes: Vec::new(),
        };

        if let Some(len) = ident_len(rest) {
            selector.tag = Some(rest[..len].to_ascii_lowercase());
            rest = &rest[len..];
        } else if let Some(tail) = rest.strip_prefix('*') {
            selector.universal = true;
            rest = tail;
        } else if !rest.starts_with(['.', '#']) {
            return None;
        }

        while !rest.is_empty() {
            let (kind, tail) = rest.split_at(1);
            let len = ident_len(tail)?;
            if len == 0 {
                return None;
            }
            let name = &tail[..len];
            match kind {
                "." => selector.classes.push(name.to_string()),
                "#" => selector.id = Some(name.to_string()),
                _ => return None,
            }
            rest = &tail[len..];
        }

        let empty = !selector.universal
            && selector.tag.is_none()
            && selector.id.is_none()
            && selector.classes.is_empty();
        (!empty).then_some(selector)
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

fn ident_len(text: &str) -> Option<usize> {
    let len = text
        .char_indices()
        .take_while(|(_, ch)| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
        .map(|(i, ch)| i + ch.len_utf8())
        .last()?;
    (len > 0).then_some(len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::wxml;

    fn element(source: &str) -> WxmlElement {
        let doc = wxml::parse("x.wxml", source).unwrap();
        doc.elements().next().unwrap().clone()
    }

    #[test]
    fn parses_tag_class_and_id() {
        let selector = SimpleSelector::parse("view.list#main").unwrap();
        assert_eq!(selector.tag.as_deref(), Some("view"));
        assert_eq!(selector.classes, vec!["list"]);
        assert_eq!(selector.id.as_deref(), Some("main"));
        assert!(SimpleSelector::parse("*").is_some());
    }

    #[test]
    fn rejects_complex_shapes() {
        for text in [
            ".a .b",
            ".a > .b",
            "view::after",
            "[data-x]",
            "a:hover",
            "",
            ".",
            " ",
        ] {
            assert!(SimpleSelector::parse(text).is_none(), "{text}");
        }
    }

    #[test]
    fn matches_static_attributes() {
        let el = element(r#"<view class="a b" id="main"/>"#);
        assert_eq!(
            SimpleSelector::parse("view.a").unwrap().matches(&el),
            Match::Yes
        );
        assert_eq!(
            SimpleSelector::parse("#main").unwrap().matches(&el),
            Match::Yes
        );
        assert_eq!(
            SimpleSelector::parse("text").unwrap().matches(&el),
            Match::No
        );
        assert_eq!(SimpleSelector::parse(".c").unwrap().matches(&el), Match::No);
        assert_eq!(
            SimpleSelector::parse("#other").unwrap().matches(&el),
            Match::No
        );
    }

    #[test]
    fn dynamic_attributes_are_unknown() {
        let el = element(r#"<view class="a {{x}}"/>"#);
        assert_eq!(
            SimpleSelector::parse(".a").unwrap().matches(&el),
            Match::Unknown
        );
        assert_eq!(
            SimpleSelector::parse("view").unwrap().matches(&el),
            Match::Yes
        );
        let dynamic_id = element(r#"<view id="{{x}}" class="a"/>"#);
        assert_eq!(
            SimpleSelector::parse("#x").unwrap().matches(&dynamic_id),
            Match::Unknown
        );
    }
}
