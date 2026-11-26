//! 单元依赖索引：把「谁引用谁」的所有事实收在一处。
//!
//! 仓库里原本有三份遍历实现——`Project::closure`、`webview_pinned_renderer` 自己的
//! BFS、`graph` 的树渲染——它们读的是同一份 `deps`，但各自走一遍。索引把边归集好，
//! 查询方法（[`DependencyIndex::reachable`] / [`Project::page_components`] /
//! [`Project::component_pages`]）都落在它上面，`skylint graph` 只负责把它序列化。
//!
//! 两条边语义不同，不能合成一个集合：
//! - [`UnitDeps::declared`]：单元声明并会被加载的组件（`usingComponents`、
//!   `componentGenerics.*.default`、`componentPlaceholder` 的值）。影响面、构建行为、
//!   迁移顺序都按它算——声明了即使没渲染，组件仍然进包。
//! - [`UnitDeps::templates`] / [`UnitDeps::styles`]：文件级引用，闭包已展开。

use std::collections::{BTreeMap, BTreeSet};

/// 一个单元的依赖边。字段都是排序去重的项目相对路径。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnitDeps {
    /// 声明并会被加载的组件。
    pub declared: Vec<String>,
    /// `<import>` / `<include>` 的模板（传递闭包）。
    pub templates: Vec<String>,
    /// `@import` 的样式（传递闭包）；单元自己的 wxss 不算。
    pub styles: Vec<String>,
}

/// 单元 → 依赖边。key 是页面 route、组件路径或 app 配置文件名（`app.json` / `plugin.json`）。
#[derive(Debug, Clone, Default)]
pub struct DependencyIndex {
    units: BTreeMap<String, UnitDeps>,
}

impl DependencyIndex {
    pub fn insert(&mut self, unit: impl Into<String>, deps: UnitDeps) {
        self.units.insert(unit.into(), deps);
    }

    pub fn get(&self, unit: &str) -> Option<&UnitDeps> {
        self.units.get(unit)
    }

    /// 声明的组件；没有这个单元时返回空切片。
    pub fn declared(&self, unit: &str) -> &[String] {
        self.get(unit).map_or(&[], |deps| &deps.declared)
    }

    pub fn templates(&self, unit: &str) -> &[String] {
        self.get(unit).map_or(&[], |deps| &deps.templates)
    }

    pub fn styles(&self, unit: &str) -> &[String] {
        self.get(unit).map_or(&[], |deps| &deps.styles)
    }

    /// 全部单元，按 key 排序。
    pub fn units(&self) -> impl Iterator<Item = (&str, &UnitDeps)> {
        self.units.iter().map(|(unit, deps)| (unit.as_str(), deps))
    }

    /// 从 `start` 出发沿声明边可达的单元（含起点自身，排序去重）。
    ///
    /// 起点可以同时给页面 route 和 `"app.json"`：后者让 app 级全局组件也算进来。
    pub fn reachable(&self, start: &[&str]) -> Vec<String> {
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut stack: Vec<&str> = start.to_vec();
        while let Some(unit) = stack.pop() {
            if !seen.insert(unit.to_string()) {
                continue;
            }
            for dep in self.declared(unit) {
                if !seen.contains(dep) {
                    stack.push(dep);
                }
            }
        }
        seen.into_iter().collect()
    }
}
