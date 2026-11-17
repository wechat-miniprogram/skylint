//! 组件使用关系查询。

use std::collections::{HashSet, VecDeque};

use super::{UsageHop, UsageSite, WxmlGraph};

impl WxmlGraph {
    /// 组件模板的 `options`：`(multipleSlots, virtualHost)`。
    ///
    /// 页面与被引入的片段没有这两个开关，返回 `(false, false)`。迁移工具需要它来解释
    /// 「slot 投影为什么走这条边」「宿主元素是否存在」。
    pub(crate) fn component_options(&self, doc_path: &str) -> (bool, bool) {
        self.by_path.get(doc_path).map_or((false, false), |&doc| {
            let options = self.docs[doc].options;
            (options.multiple_slots, options.virtual_host)
        })
    }

    /// 哪些文档以哪个标签使用了这份组件（反链）。
    ///
    /// `component_options` / `render_docs` 是正向（使用点 → 组件实现），这个方法是反向：
    /// 给定组件模板里的一个结论（「这个元素会塌成 0」），回答「它会在哪些页面 / 哪个父组件
    /// 里出现」。只给**直接**使用点，多跳由调用方递归。
    pub(crate) fn component_usages(&self, doc_path: &str) -> Vec<UsageSite> {
        let Some(&target) = self.by_path.get(doc_path) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for doc in &self.docs {
            for (tag, &dest) in &doc.components {
                if dest != target {
                    continue;
                }
                for element in &doc.elements {
                    if element.tag.eq_ignore_ascii_case(tag) {
                        out.push(UsageSite {
                            doc: doc.path.clone(),
                            tag: tag.clone(),
                            line: element.line,
                            column: element.column,
                        });
                    }
                }
            }
        }
        out.sort_by(|left, right| {
            (&left.doc, left.line, left.column).cmp(&(&right.doc, right.line, right.column))
        });
        out.dedup_by(|left, right| {
            left.doc == right.doc && left.line == right.line && left.column == right.column
        });
        out
    }

    /// 递归使用点（BFS）：把「父组件被谁用」也展开，`depth` 从 1 开始。
    ///
    /// 环（A 用 B、B 又用 A）不会无限展开；同一文档只访问一次，命中时取最短跳数。
    pub(crate) fn component_usages_recursive(
        &self,
        doc_path: &str,
        max_depth: u32,
    ) -> Vec<UsageHop> {
        let mut out = Vec::new();
        let mut frontier = vec![doc_path.to_string()];
        let mut visited: HashSet<String> = HashSet::from([doc_path.to_string()]);
        for depth in 1..=max_depth {
            let mut next = Vec::new();
            for doc in &frontier {
                for site in self.component_usages(doc) {
                    // 去重只用于「要不要继续往上展开」，不能用来过滤输出：
                    // 同一文档里同一标签的多个使用点都是真实现场。
                    if visited.insert(site.doc.clone()) {
                        next.push(site.doc.clone());
                    }
                    out.push(UsageHop {
                        site: site.clone(),
                        depth,
                    });
                }
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }
        out
    }

    /// 这份文档（含递归使用的组件与引入的模板）能渲染到的全部文档路径，含自身。
    ///
    /// 用于「同一个页面里两个元素会不会互相覆盖」这类需要跨组件配对的判断：
    /// 组件实现里的元素与使用方文件里的元素同处一个渲染树。
    pub fn render_docs(&self, doc_path: &str) -> Vec<&str> {
        let Some(&start) = self.by_path.get(doc_path) else {
            return Vec::new();
        };
        let mut seen = HashSet::new();
        let mut queue = VecDeque::from([start]);
        let mut out = Vec::new();
        while let Some(doc) = queue.pop_front() {
            if !seen.insert(doc) {
                continue;
            }
            out.push(self.docs[doc].path.as_str());
            for &component in self.docs[doc].components.values() {
                queue.push_back(component);
            }
            // `<include>` 的内容是页面的一部分；`<import>` 只带模板定义，不进这个视图。
            for (fragment, sites) in &self.includes {
                if sites.iter().any(|(includer, _)| *includer == doc) {
                    queue.push_back(*fragment);
                }
            }
        }
        out
    }
}
