//! callee 形态匹配。
//!
//! `input/js.rs` 只保留点号链文本：链的前缀是调用结果时（`q.select('.x').ref(cb)`）
//! 没有静态名字，用空串占位、保留链尾（`.ref`）。所以规则侧能表达的最强语义就是
//! 「整条相等 / 以某段路径结尾 / 某命名空间下 / 链尾方法名」——统一放在这里，
//! 避免各规则各写一份 `ends_with` 而边界语义不一致（`myX.scrollTo` 该不该命中，
//! 取决于有没有按段比对）。

#[derive(Debug, Clone, Copy)]
pub(crate) enum Callee {
    /// 完全相同：`wx.request`。
    Exact(&'static str),
    /// 相等，或以 `.<path>` 结尾：`scrollViewContext.scrollTo` 命中
    /// `worklet.scrollViewContext.scrollTo`，不命中 `myscrollViewContext.scrollTo`。
    Suffix(&'static str),
    /// 必须以 `.<path>` 结尾（排除同名裸调用）：`this.applyAnimatedStyle` 算，
    /// `applyAnimatedStyle({...})` 这种全局同名函数不算。
    Qualified(&'static str),
    /// 命名空间：相等，或以 `<prefix>.` 开头。
    Namespace(&'static str),
    /// 链尾方法名（最后一段）：`ref`、`q.select('.x').ref` 都命中 `ref`。
    Method(&'static str),
}

impl Callee {
    pub(crate) fn matches(self, callee: &str) -> bool {
        match self {
            Self::Exact(path) => callee == path,
            Self::Suffix(path) => {
                callee == path
                    || callee
                        .strip_suffix(path)
                        .is_some_and(|prefix| prefix.ends_with('.'))
            }
            Self::Qualified(path) => callee
                .strip_suffix(path)
                .is_some_and(|prefix| prefix.ends_with('.')),
            Self::Namespace(prefix) => {
                callee == prefix
                    || callee
                        .strip_prefix(prefix)
                        .is_some_and(|rest| rest.starts_with('.'))
            }
            Self::Method(method) => callee.rsplit('.').next() == Some(method),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Callee;

    #[test]
    fn suffix_matches_whole_segments_only() {
        let pattern = Callee::Suffix("scrollViewContext.scrollTo");
        for hit in [
            "scrollViewContext.scrollTo",
            "worklet.scrollViewContext.scrollTo",
            "this.scrollViewContext.scrollTo",
        ] {
            assert!(pattern.matches(hit), "{hit}");
        }
        for miss in [
            "myscrollViewContext.scrollTo",
            "scrollViewContext.scrollToBack",
            "",
        ] {
            assert!(!pattern.matches(miss), "{miss}");
        }
    }

    #[test]
    fn qualified_requires_a_receiver() {
        let pattern = Callee::Qualified("applyAnimatedStyle");
        assert!(pattern.matches("this.applyAnimatedStyle"));
        assert!(pattern.matches("ctx.anim.applyAnimatedStyle"));
        assert!(!pattern.matches("applyAnimatedStyle"));
        assert!(!pattern.matches("myapplyAnimatedStyle"));
    }

    #[test]
    fn namespace_requires_a_segment_boundary() {
        let pattern = Callee::Namespace("wx.router");
        assert!(pattern.matches("wx.router"));
        assert!(pattern.matches("wx.router.push"));
        assert!(!pattern.matches("wx.routers.push"));
        assert!(!pattern.matches("wx"));
    }

    #[test]
    fn method_matches_the_chain_tail() {
        let pattern = Callee::Method("ref");
        assert!(pattern.matches("ref"));
        assert!(pattern.matches(".ref"));
        assert!(pattern.matches("query.select('.x').ref"));
        assert!(!pattern.matches("query.reference"));
        assert!(!pattern.matches(""));
    }
}
