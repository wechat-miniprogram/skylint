//! 把 `selectors`（servo/stylo 的匹配引擎）接到我们的 WXSS 选择器文法上。
//!
//! 只做「参数化」：本地名 / 标识符 / 命名空间都用字符串 newtype，
//! 伪类与伪元素照单全收（名字 + 可选参数原文），具体语义推迟到匹配阶段按方言决定。
//! 这样解析永不因「不认识的伪类」失败——那是引擎的行为差异，不是语法错误。

use std::borrow::Borrow;
use std::collections::hash_map::DefaultHasher;
use std::fmt;
use std::hash::{Hash, Hasher};

use cssparser::{CowRcStr, ParseError};
use precomputed_hash::PrecomputedHash;
use selectors::parser::{
    NonTSPseudoClass, ParseRelative, Parser, PseudoElement, SelectorImpl, SelectorParseErrorKind,
};

/// 标识符 / 本地名 / 属性值：**保留原文大小写**。
///
/// 大小写敏感性不在这里定，按使用点区分：标签名与属性名不敏感（与 HTML 一致），
/// class / id / 属性值敏感。`selectors` 判 class / id 时会把 `CaseSensitivity` 传进来
/// （`MatchingContext` 用 `QuirksMode::NoQuirks`，即敏感），见
/// [`super::element::Node::has_class`]；这里再统一小写就会把 `class="btnItem"` 变成
/// 永远命中不了。
#[derive(Clone, Debug, Default, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Ident(pub String);

impl Borrow<str> for Ident {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl PrecomputedHash for Ident {
    fn precomputed_hash(&self) -> u32 {
        let mut hasher = DefaultHasher::new();
        self.0.hash(&mut hasher);
        hasher.finish() as u32
    }
}

impl cssparser::ToCss for Ident {
    fn to_css<W: fmt::Write>(&self, dest: &mut W) -> fmt::Result {
        cssparser::serialize_identifier(&self.0, dest)
    }
}

/// 不支持任何命名空间：WXSS 没有 `@namespace`。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct Ns;

impl Borrow<()> for Ns {
    fn borrow(&self) -> &() {
        &()
    }
}

impl From<&str> for Ns {
    fn from(_: &str) -> Self {
        Self
    }
}

impl From<&str> for Ident {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

impl AsRef<str> for Ident {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl PrecomputedHash for Ns {
    fn precomputed_hash(&self) -> u32 {
        0
    }
}

impl cssparser::ToCss for Ns {
    fn to_css<W: fmt::Write>(&self, _dest: &mut W) -> fmt::Result {
        Ok(())
    }
}

/// 非树结构伪类：只保留名字（小写）与参数原文。
///
/// `:not()` / `:is()` / `:nth-child()` 由 `selectors` 自己解析成结构化组件，
/// 不会走到这里；走到这里的是 `:hover`、`:host`、`:nth-of-type` 之外的方言扩展。
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct PseudoClass {
    pub name: String,
    /// 带参形式的参数原文（`()` 内），用于 `:dir(rtl)` 这类需要区分参数的场合。
    pub args: Option<String>,
}

impl cssparser::ToCss for PseudoClass {
    fn to_css<W: fmt::Write>(&self, dest: &mut W) -> fmt::Result {
        dest.write_str(":")?;
        dest.write_str(&self.name)?;
        if let Some(args) = &self.args {
            dest.write_str("(")?;
            dest.write_str(args)?;
            dest.write_str(")")?;
        }
        Ok(())
    }
}

impl NonTSPseudoClass for PseudoClass {
    fn is_active_or_hover(&self) -> bool {
        matches!(self.name.as_str(), "active" | "hover")
    }

    fn is_user_action_state(&self) -> bool {
        matches!(
            self.name.as_str(),
            "active" | "hover" | "focus" | "focus-within"
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct PseudoElem {
    pub name: String,
}

impl cssparser::ToCss for PseudoElem {
    fn to_css<W: fmt::Write>(&self, dest: &mut W) -> fmt::Result {
        dest.write_str("::")?;
        dest.write_str(&self.name)
    }
}

impl PseudoElement for PseudoElem {
    fn is_before_or_after(&self) -> bool {
        matches!(self.name.as_str(), "before" | "after")
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct SelImpl;

impl SelectorImpl for SelImpl {
    type ExtraMatchingData<'a> = ();
    type AttrValue = Ident;
    type Identifier = Ident;
    type LocalName = Ident;
    type NamespaceUrl = Ns;
    type NamespacePrefix = Ns;
    type BorrowedNamespaceUrl = ();
    type BorrowedLocalName = str;
    type NonTSPseudoClass = PseudoClass;
    type PseudoElement = PseudoElem;
}

/// 把 WXSS 选择器文本解析成 `selectors` 的 AST。
pub struct SelParser;

impl<'i> Parser<'i> for SelParser {
    type Impl = SelImpl;
    type Error = SelectorParseErrorKind;

    fn parse_nth_child_of(&self) -> bool {
        true
    }

    fn parse_is_and_where(&self) -> bool {
        true
    }

    fn parse_non_ts_pseudo_class(
        &self,
        name: CowRcStr<'i>,
    ) -> Result<PseudoClass, ParseError<Self::Error>> {
        Ok(PseudoClass {
            name: name.as_ref().to_ascii_lowercase(),
            args: None,
        })
    }

    fn parse_non_ts_functional_pseudo_class(
        &self,
        name: CowRcStr<'i>,
        parser: &mut cssparser::Parser<'i>,
        _after_part: bool,
    ) -> Result<PseudoClass, ParseError<Self::Error>> {
        // 参数原文照收：语义由匹配阶段决定，这里不做校验。
        let args = parser
            .parse_entirely(|input| -> Result<String, ParseError<Self::Error>> {
                let start = input.position();
                while !input.is_exhausted() {
                    input.next_including_whitespace_and_comments()?;
                }
                Ok(input.slice_from(start).trim().to_string())
            })
            .unwrap_or_default();
        Ok(PseudoClass {
            name: name.as_ref().to_ascii_lowercase(),
            args: Some(args),
        })
    }

    fn parse_pseudo_element(
        &self,
        name: CowRcStr<'i>,
    ) -> Result<PseudoElem, ParseError<Self::Error>> {
        // 单冒号形式的 `:before` / `:after` 是 CSS2 写法，按伪元素处理。
        Ok(PseudoElem {
            name: name.as_ref().to_ascii_lowercase(),
        })
    }

    fn parse_functional_pseudo_element(
        &self,
        name: CowRcStr<'i>,
        _args: &mut cssparser::Parser<'i>,
    ) -> Result<PseudoElem, ParseError<Self::Error>> {
        Ok(PseudoElem {
            name: name.as_ref().to_ascii_lowercase(),
        })
    }
}

/// 解析一条选择器；失败返回 `None`（调用方按「看不见」处理）。
pub fn parse(selector: &str) -> Option<selectors::SelectorList<SelImpl>> {
    let mut parser = cssparser::Parser::new(selector);
    selectors::SelectorList::parse(&SelParser, &mut parser, ParseRelative::No).ok()
}
