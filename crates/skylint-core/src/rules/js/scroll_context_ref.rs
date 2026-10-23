//! 用 `scrollViewContext.scrollTo` 前必须先拿到 scroll-view 的节点引用。
//!
//! `scrollTo(ref, ...)` 的第一个参数是 `NodesRef.ref` 回调里拿到的 ref；没有
//! `createSelectorQuery().select(..).ref(cb)` 这一链，ref 就是 undefined，滚动不会发生。
//! 判定按**文件**做：只要文件里出现了 `scrollViewContext.scrollTo`，就要求同一份文件
//! 里至少有一次 `.ref` 调用。
//!
//! 证据：`skyline-worklet/references/base/scroll-view-context.md:47-52`（要点第一条
//! 「需要先通过 createSelectorQuery().select().ref() 获取 scroll-view 的引用」）。

use crate::project::Project;
use crate::rules::{Rule, RuleMeta};
use crate::Severity;

use super::{Callee, at_span};

const SCROLL_TO: Callee = Callee::Suffix("scrollViewContext.scrollTo");
const REF: Callee = Callee::Qualified("ref");
const MESSAGE: &str = "用 `scrollViewContext.scrollTo` 前必须先用 `createSelectorQuery().select(..).ref(cb)` 拿到引用，否则 ref 是 undefined";

pub struct ScrollContextRef;

impl ScrollContextRef {
    pub const ID: &'static str = "skyline/scroll-context-ref";
}

impl Rule for ScrollContextRef {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "用 scrollViewContext.scrollTo 前没有通过 NodesRef.ref 拿到 scroll-view 引用",
        }
    }

    fn check(&self, project: &Project) -> Vec<crate::Diagnostic> {
        let mut out = Vec::new();
        for file in &project.scripts {
            let calls = file.doc.calls();
            let Some(first) = calls
                .iter()
                .find(|call| SCROLL_TO.matches(&call.callee))
            else {
                continue;
            };
            if calls.iter().any(|call| REF.matches(&call.callee)) {
                continue;
            }
            out.push(at_span(
                Self::ID,
                Severity::Warning,
                file,
                first.start,
                first.end,
                MESSAGE,
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;

    use super::super::testutil::{messages, single};
    use super::ScrollContextRef;

    const SCROLL_TO: &str = "function scroll() { 'worklet'; scrollViewContext.scrollTo(this.ref.value, { top: 1 }); }";

    #[test]
    fn missing_ref_is_reported() {
        let (_f, project) = single("ref-missing", SCROLL_TO);
        let diagnostics = ScrollContextRef.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].severity, crate::Severity::Warning);
        assert!(diagnostics[0].message.contains("ref 是 undefined"));
    }

    #[test]
    fn each_file_reports_at_most_once() {
        let (_f, project) = single(
            "ref-multiple",
            r#"
            function a() { 'worklet'; scrollViewContext.scrollTo(ref, { top: 1 }); }
            function b() { 'worklet'; wx.worklet.scrollViewContext.scrollTo(ref, { top: 2 }); }
            "#,
        );
        let diagnostics = ScrollContextRef.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].line, 2);
    }

    #[test]
    fn ref_through_selector_query_chain_is_accepted() {
        let (_f, project) = single(
            "ref-chain",
            r#"
            function scroll() {
              'worklet';
              this.createSelectorQuery().select('.scrollable').ref((res) => {
                this.ref.value = res.ref;
              }).exec();
              scrollViewContext.scrollTo(this.ref.value, { top: 1 });
            }
            "#,
        );
        assert_eq!(
            messages(&ScrollContextRef, &project),
            Vec::<String>::new()
        );
    }

    #[test]
    fn ref_on_a_named_receiver_is_accepted() {
        let (_f, project) = single(
            "ref-named",
            "query.select('.x').ref(cb); scrollViewContext.scrollTo(ref, { top: 1 });",
        );
        assert_eq!(
            messages(&ScrollContextRef, &project),
            Vec::<String>::new()
        );
    }

    #[test]
    fn files_without_scroll_to_are_ignored() {
        let (_f, project) = single(
            "ref-negative",
            "const query = wx.createSelectorQuery(); query.select('.x').exec();",
        );
        assert_eq!(
            messages(&ScrollContextRef, &project),
            Vec::<String>::new()
        );
    }

    #[test]
    fn ref_in_another_file_does_not_cover_this_one() {
        let (f, _) = single(
            "ref-other-file",
            "scrollViewContext.scrollTo(this.ref.value, { top: 1 });",
        );
        f.write(
            "pages/other/other.js",
            "query.select('.x').ref(cb);",
        )
        .write("pages/other/other.json", "{}")
        .write("pages/other/other.wxml", "<view/>")
        .write(
            "app.json",
            r#"{"pages":["pages/index/index","pages/other/other"],"renderer":"skyline"}"#,
        );
        let project = f.project();
        let diagnostics = ScrollContextRef.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].path, "pages/index/index.js");
    }
}
