//! `selectors::Element` 的 WXML 适配器。

use std::fmt;

use selectors::OpaqueElement;
use selectors::attr::{AttrSelectorOperation, CaseSensitivity, NamespaceConstraint};
use selectors::bloom::BloomFilter;
use selectors::context::MatchingContext;
use selectors::matching::ElementSelectorFlags;

use crate::matcher::{Dialect, SelImpl, Tree};

#[derive(Clone)]
pub struct Node<'a> {
    pub tree: &'a Tree<'a>,
    pub idx: usize,
    pub dialect: Dialect,
}

impl fmt::Debug for Node<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Node")
            .field("tag", &self.element().tag)
            .finish()
    }
}

impl<'a> Node<'a> {
    fn element(&self) -> &'a crate::input::wxml::WxmlElement {
        self.tree.element_at(self.idx)
    }

    fn at(&self, idx: Option<usize>) -> Option<Self> {
        idx.map(|idx| Self {
            tree: self.tree,
            idx,
            dialect: self.dialect,
        })
    }

    fn attribute(&self, name: &str) -> Option<&'a crate::input::wxml::WxmlAttribute> {
        self.element()
            .attributes
            .iter()
            .find(|attr| attr.name.eq_ignore_ascii_case(name))
    }
}

impl<'a> selectors::Element for Node<'a> {
    type Impl = SelImpl;

    fn opaque(&self) -> OpaqueElement {
        // 指向文档自己持有的元素，地址在文档生命周期内稳定。
        OpaqueElement::new(self.element())
    }

    fn parent_element(&self) -> Option<Self> {
        self.at(self.tree.parent_idx(self.idx))
    }

    fn parent_node_is_shadow_root(&self) -> bool {
        false
    }

    fn containing_shadow_host(&self) -> Option<Self> {
        None
    }

    fn is_pseudo_element(&self) -> bool {
        false
    }

    fn prev_sibling_element(&self) -> Option<Self> {
        self.at(self.tree.sibling_idx(self.idx, false))
    }

    fn next_sibling_element(&self) -> Option<Self> {
        self.at(self.tree.sibling_idx(self.idx, true))
    }

    fn first_element_child(&self) -> Option<Self> {
        self.at(self.tree.first_child_idx(self.idx))
    }

    /// 与 `is_same_type` 一起决定 `:nth-of-type` 的计数口径；WXML 标签大小写已归一，
    /// 与 HTML 文档无关。
    fn is_html_element_in_html_document(&self) -> bool {
        false
    }

    fn has_local_name(&self, local_name: &str) -> bool {
        self.element().tag == local_name
    }

    fn has_namespace(&self, _ns: &()) -> bool {
        false
    }

    fn is_same_type(&self, other: &Self) -> bool {
        self.element().tag == other.element().tag
    }

    fn attr_matches(
        &self,
        _ns: &NamespaceConstraint<&super::imp::Ns>,
        local_name: &super::imp::Ident,
        operation: &AttrSelectorOperation<&super::imp::Ident>,
    ) -> bool {
        if self.dialect == Dialect::Skyline {
            // Skyline 的 `CompoundSelector` 没有 attribute 字段，属性条件在转换时被丢弃
            // （`skyline/css/selector.h:78`）——即 `[foo]` 等价 `*`。
            return true;
        }
        match self.attribute(&local_name.0) {
            None => false,
            // 动态属性：值由运行时决定，本轮判不了。
            Some(attr) if !attr.is_static => {
                self.tree.unresolved.set(true);
                false
            }
            Some(attr) => operation.eval_str(attr.value.as_deref().unwrap_or("")),
        }
    }

    fn match_non_ts_pseudo_class(
        &self,
        _pc: &super::imp::PseudoClass,
        _context: &mut MatchingContext<SelImpl>,
    ) -> bool {
        // 结构伪类由 `selectors` 自己处理；走到这里的都是方言扩展或运行时状态，
        // 静态阶段一律不命中（需要区分时由 `classify` 返回 Unknown）。
        false
    }

    fn match_pseudo_element(
        &self,
        _pe: &super::imp::PseudoElem,
        _context: &mut MatchingContext<SelImpl>,
    ) -> bool {
        false
    }

    fn apply_selector_flags(&self, _flags: ElementSelectorFlags) {}

    fn is_link(&self) -> bool {
        false
    }

    fn is_html_slot_element(&self) -> bool {
        false
    }

    fn has_id(&self, id: &super::imp::Ident, case_sensitivity: CaseSensitivity) -> bool {
        match self.attribute("id") {
            Some(attr) if attr.is_static => case_sensitivity.eq(
                attr.value.as_deref().unwrap_or("").trim().as_bytes(),
                id.0.as_bytes(),
            ),
            Some(_) => {
                self.tree.unresolved.set(true);
                false
            }
            None => false,
        }
    }

    fn has_class(&self, name: &super::imp::Ident, case_sensitivity: CaseSensitivity) -> bool {
        let Some(attr) = self.attribute("class") else {
            return false;
        };
        if attr.is_static {
            return attr
                .value
                .as_deref()
                .unwrap_or("")
                .split_whitespace()
                .any(|class| case_sensitivity.eq(class.as_bytes(), name.0.as_bytes()));
        }
        // 动态 class：原文里写死的 token 仍然确定存在（`class="novel {{renderer}}"`）。
        if attr.raw_value.as_deref().is_some_and(|raw| {
            super::certain_tokens(raw).any(|t| case_sensitivity.eq(t.as_bytes(), name.0.as_bytes()))
        }) {
            return true;
        }
        // 找不到：可能是运行时拼上去的，也可能真的没有——本轮判不了。
        self.tree.unresolved.set(true);
        false
    }

    fn has_custom_state(&self, _name: &super::imp::Ident) -> bool {
        false
    }

    fn imported_part(&self, _name: &super::imp::Ident) -> Option<super::imp::Ident> {
        None
    }

    fn is_part(&self, _name: &super::imp::Ident) -> bool {
        false
    }

    fn is_empty(&self) -> bool {
        !self.element().children.iter().any(|node| match node {
            crate::input::wxml::WxmlNode::Element(_) => true,
            crate::input::wxml::WxmlNode::Text(text) => text.is_meaningful(),
            crate::input::wxml::WxmlNode::Comment => false,
        })
    }

    fn is_root(&self) -> bool {
        self.tree.parent_idx(self.idx).is_none()
    }

    fn add_element_unique_hashes(&self, _filter: &mut BloomFilter) -> bool {
        false
    }
}
