//! 用 `scrollViewContext.scrollTo` 前必须先拿到 scroll-view 的节点引用。
//!
//! `scrollTo(ref, ...)` 的第一个参数是 `NodesRef.ref` 回调里拿到的 ref；没有
//! `createSelectorQuery().select(..).ref(cb)` 这一链，ref 就是 undefined，滚动不会发生。
//! 判定按**文件**做：只要文件里出现了 `scrollViewContext.scrollTo`，就要求同一份文件
//! 里至少有一次 `.ref` 调用。
//!
//! 证据：`skyline-worklet/references/base/scroll-view-context.md:47-52`（要点第一条
//! 「需要先通过 createSelectorQuery().select().ref() 获取 scroll-view 的引用」）。

use crate::Severity;
use crate::project::Project;
use crate::rules::{EvidenceCase, EvidenceExpect, EvidenceRun, Rule, RuleMeta, VisualFile};

use super::{Callee, at_span};

const SCROLL_TO: Callee = Callee::Suffix("scrollViewContext.scrollTo");
const REF: Callee = Callee::Qualified("ref");
const MESSAGE: &str = "用 `scrollViewContext.scrollTo` 前必须先用 `createSelectorQuery().select(..).ref(cb)` 拿到引用，否则 ref 是 undefined";

pub struct ScrollContextRef;

/// 三个 Skyline 工程，只差「有没有先拿到 ref」。
///
/// `nodes_ref` 在模拟器里只有 `this.createSelectorQuery()` 这条链有 `.ref`，返回的
/// `res.ref` 交给 `scrollViewContext.scrollTo` 时能越过框架的 ref 校验；拿不到 ref 时
/// 传 `undefined`，框架会打一行固定的 `scrollTo: fail invalid parameter ref` 并直接返回。
///
/// 有效 ref 触发的真实滚动在本模拟器里观测不到：`scrollTo` 一旦走到原生滚动就会卡住
/// appservice（事件循环停住，`scrollOffset` 回调与 evaluate 都不返回），所以「有 ref →
/// scrollTop 120」只能作为手工证据，自动用例只断言「ref 校验通过 / 未通过」与「无 ref 时
/// 不滚动」。
const APP_JSON: &str = r#"{"pages":["pages/index/index"],"renderer":"skyline","componentFramework":"glass-easel","lazyCodeLoading":"requiredComponents","window":{"navigationStyle":"custom"}}"#;
const APP_JS: &str = "App({})\n";
const PAGE_JSON: &str = r#"{"renderer":"skyline","navigationStyle":"custom"}"#;
const PAGE_WXML: &str = r#"<scroll-view id="sv" type="list" scroll-y style="height: 300px;"><view style="height: 1000px;">tall</view></scroll-view>"#;
const PAGE_JS_NO_REF: &str = r#"
Page({
  onReady() {
    setTimeout(() => {
      wx.worklet.scrollViewContext.scrollTo(undefined, { top: 120 });
      setTimeout(() => {
        wx.createSelectorQuery()
          .select('#sv')
          .scrollOffset((r) => {
            globalThis.__evidence_scroll_top = r && r.scrollTop;
          })
          .exec();
      }, 500);
    }, 300);
  },
});
"#;
const PAGE_JS_WITH_REF: &str = r#"
Page({
  onReady() {
    setTimeout(() => {
      try {
        this.createSelectorQuery()
          .select('#sv')
          .ref((r) => {
            try {
              wx.worklet.scrollViewContext.scrollTo(r.ref, { top: 120 });
              console.log('EVIDENCE_REF_CALL_OK');
            } catch (e) {
              console.log('EVIDENCE_REF_GUARD_PASS ' + e.message);
            }
          })
          .exec();
      } catch (e) {
        console.log('EVIDENCE_REF_EXEC_ERR ' + e.message);
      }
    }, 300);
  },
});
"#;
const REF_WARNING: &str = "scrollTo: fail invalid parameter ref";

static EVIDENCE_CASES: &[EvidenceCase] = &[
    EvidenceCase {
        name: "without-ref-warns",
        files: &[
            VisualFile {
                path: "app.json",
                content: APP_JSON,
            },
            VisualFile {
                path: "app.js",
                content: APP_JS,
            },
            VisualFile {
                path: "pages/index/index.json",
                content: PAGE_JSON,
            },
            VisualFile {
                path: "pages/index/index.wxml",
                content: PAGE_WXML,
            },
            VisualFile {
                path: "pages/index/index.js",
                content: PAGE_JS_NO_REF,
            },
        ],
        project_config: None,
        run: EvidenceRun::Serve {
            page: "pages/index/index",
        },
        expect: EvidenceExpect::Console {
            must: &[REF_WARNING],
            must_not: &[],
        },
        note: "没拿 ref 就调 scrollTo：框架直接返回并打固定 warning，滚动不发生",
    },
    EvidenceCase {
        name: "without-ref-scroll-top-zero",
        files: &[
            VisualFile {
                path: "app.json",
                content: APP_JSON,
            },
            VisualFile {
                path: "app.js",
                content: APP_JS,
            },
            VisualFile {
                path: "pages/index/index.json",
                content: PAGE_JSON,
            },
            VisualFile {
                path: "pages/index/index.wxml",
                content: PAGE_WXML,
            },
            VisualFile {
                path: "pages/index/index.js",
                content: PAGE_JS_NO_REF,
            },
        ],
        project_config: None,
        run: EvidenceRun::Serve {
            page: "pages/index/index",
        },
        expect: EvidenceExpect::Runtime {
            expr: "globalThis.__evidence_scroll_top",
            equals: "0",
        },
        note: "同一份无 ref 的页面：scrollTo 之后 scrollTop 仍为 0",
    },
    EvidenceCase {
        name: "with-ref-passes-guard",
        files: &[
            VisualFile {
                path: "app.json",
                content: APP_JSON,
            },
            VisualFile {
                path: "app.js",
                content: APP_JS,
            },
            VisualFile {
                path: "pages/index/index.json",
                content: PAGE_JSON,
            },
            VisualFile {
                path: "pages/index/index.wxml",
                content: PAGE_WXML,
            },
            VisualFile {
                path: "pages/index/index.js",
                content: PAGE_JS_WITH_REF,
            },
        ],
        project_config: None,
        run: EvidenceRun::Serve {
            page: "pages/index/index",
        },
        expect: EvidenceExpect::Console {
            must: &["EVIDENCE_REF_GUARD_PASS workletUIModule is not defined"],
            must_not: &[REF_WARNING],
        },
        note: "阴性对照：拿到 ref 后同一调用越过 ref 校验（不再报 invalid parameter ref），只在本模拟器的 JS 线程上止于 workletUIModule 缺失；真实 120px 滚动无法自动观测",
    },
];

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

    fn evidence_cases(&self) -> &'static [EvidenceCase] {
        EVIDENCE_CASES
    }

    fn check(&self, project: &Project) -> Vec<crate::Diagnostic> {
        let mut out = Vec::new();
        for file in &project.scripts {
            let calls = file.doc.calls();
            let Some(first) = calls.iter().find(|call| SCROLL_TO.matches(&call.callee)) else {
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

    use super::ScrollContextRef;
    use crate::rules::testutil::js::{messages, single};

    const SCROLL_TO: &str =
        "function scroll() { 'worklet'; scrollViewContext.scrollTo(this.ref.value, { top: 1 }); }";

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
        assert_eq!(messages(&ScrollContextRef, &project), Vec::<String>::new());
    }

    #[test]
    fn ref_on_a_named_receiver_is_accepted() {
        let (_f, project) = single(
            "ref-named",
            "query.select('.x').ref(cb); scrollViewContext.scrollTo(ref, { top: 1 });",
        );
        assert_eq!(messages(&ScrollContextRef, &project), Vec::<String>::new());
    }

    #[test]
    fn files_without_scroll_to_are_ignored() {
        let (_f, project) = single(
            "ref-negative",
            "const query = wx.createSelectorQuery(); query.select('.x').exec();",
        );
        assert_eq!(messages(&ScrollContextRef, &project), Vec::<String>::new());
    }

    #[test]
    fn ref_in_another_file_does_not_cover_this_one() {
        let (f, _) = single(
            "ref-other-file",
            "scrollViewContext.scrollTo(this.ref.value, { top: 1 });",
        );
        f.write("pages/other/other.js", "query.select('.x').ref(cb);")
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
