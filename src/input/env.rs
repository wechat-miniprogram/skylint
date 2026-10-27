//! 模板作用域里「确定取值」的常量环境。
//!
//! 只收录来源完全静态、且不会被运行时改写的绑定：页面/组件 `data` 与 `properties`
//! 里的字面量（没有对应的动态 `setData`）、以及父级以静态属性传进组件的值。
//! 这些值一旦确定，`wx:if="{{renderer === 'skyline'}}"` 这类条件就能在解析期折叠，
//! `class="{{renderer === 'skyline' ? 'a' : 'b'}}"` 也能收成一档。
//!
//! 作用域是一份 WXML 文件（页面或组件各一份），与小程序的数据作用域一致。

use std::collections::BTreeMap;

/// 一个确定取值。字符串比较是最常用的形态，布尔与数字按 JS 语义保留类型。
#[derive(Debug, Clone, PartialEq)]
pub enum Lit {
    Str(String),
    Bool(bool),
    Num(f64),
}

impl Lit {
    /// JS 真值语义。
    pub fn truthy(&self) -> bool {
        match self {
            Self::Str(value) => !value.is_empty(),
            Self::Bool(value) => *value,
            Self::Num(value) => *value != 0.0 && !value.is_nan(),
        }
    }

    /// 相等比较：两边类型不同时按「不可判定」返回 `None`（`===` 的语义）。
    pub fn strict_eq(&self, other: &Self) -> Option<bool> {
        match (self, other) {
            (Self::Str(a), Self::Str(b)) => Some(a == b),
            (Self::Bool(a), Self::Bool(b)) => Some(a == b),
            (Self::Num(a), Self::Num(b)) => Some(a == b),
            _ => None,
        }
    }

    /// 拼接成字符串（`class` 绑定里的 `+` 用）。
    pub fn text(&self) -> Option<String> {
        match self {
            Self::Str(value) => Some(value.clone()),
            // 数字/布尔的字符串化与 JS 一致到够用的程度：整数值不带小数点。
            Self::Bool(value) => Some(value.to_string()),
            Self::Num(value) if value.fract() == 0.0 => Some(format!("{value:.0}")),
            Self::Num(_) => None,
        }
    }
}

/// 一份 WXML 的常量环境。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Env(BTreeMap<String, Lit>);

impl Env {
    pub fn get(&self, name: &str) -> Option<&Lit> {
        self.0.get(name)
    }

    pub fn insert(&mut self, name: impl Into<String>, value: Lit) {
        self.0.insert(name.into(), value);
    }

    pub fn remove(&mut self, name: &str) {
        self.0.remove(name);
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &Lit)> {
        self.0.iter()
    }
}
