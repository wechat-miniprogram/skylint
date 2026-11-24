//! `ScrollViewContext` 的获取前置条件：调用前必须先拿到 context 或节点引用。
//!
//! 一条规则管三组方法，前置条件不同（表见 [`ROWS`]）：
//!
//! | 方法 | 需要的获取方式 | 依据 |
//! |---|---|---|
//! | `triggerRefresh` / `closeRefresh` | `select(..).node()` 拿 context；宿主 `<scroll-view>` 开 `enhanced` + `refresher-enabled` | `skyline-scroll-api/references/api/scroll-view-context.md:7,131` |
//! | `triggerTwoLevel` / `closeTwoLevel` | 同上，另加 `refresher-two-level-enabled` | 同上 `:7,132`；`skyline-components/references/scroll/scroll-view.md:271,291-292` |
//! | `scrollTo` | 第一参是 `select(..).ref(cb)` 拿到的 ref，存进 worklet `SharedValue` | `skyline-worklet/references/base/scroll-view-context.md:13,55-56` |
//!
//! `scrollIntoView` 不在这张表里：它的第一个参数是**选择器字符串**
//! （`scroll-view-context.md`「参数 1: selector」），不需要 ref；把它并进 `scrollTo`
//! 那一行会把所有正确用法判成误报。
//!
//! ## 判定口径
//!
//! - 刷新家族：链尾方法名 + 接收者能追到 `.node`（`res[0].node.triggerRefresh()`，
//!   或同一个函数里 `const ctx = res[0].node; ctx.triggerRefresh()`）。追不到就整条
//!   跳过——`triggerRefresh` 也可能是自定义对象上的同名方法，报出来是误报。
//! - `scrollTo`：接收者必须是 `scrollViewContext`。`res[0].node.scrollTo({ top })`
//!   是 ScrollViewContext 自己的同名方法，第一参是参数对象、不是 ref，不判。
//! - 属性按**单元**看：页面自己的 WXML 与它引用到的组件 / 模板里，只要有一个开了
//!   `enhanced` 的 `<scroll-view>` 同时具备所需属性就算通过——scroll-view 常被
//!   封装在组件里。属性「写了就算」（`enhanced="{{false}}"` 也放行），宁可漏报。
//! - `enhanced` 这一支是**文档 / 源码级判定**：本机模拟器不实现「未开 enhanced 时
//!   只 warn 并 return false」（见 `docs/RULE-VISUAL.md`），所以没有视觉用例。
//!
//! ## ref 的定位口径：从文件级收紧到表达式级
//!
//! 旧规则是「文件里任意一次 `.ref` 就放行」，可能属于别的元素、甚至是用户自定义的
//! `obj.ref()`。现在要求引用与这次调用**同源**，满足其一才放行：
//!
//! 1. `scrollTo(r.ref, ..)`：这次调用落在某条 `select(..).ref(cb)` 链的区间里；
//! 2. `scrollTo(this.sv.value, ..)`：某条 ref 回调里出现过 `this.sv.value = ..ref`
//!    这条赋值（`SharedValue` 中转），按**完整表达式文本**对齐。
//!
//! `obj.ref()` 的接收者里没有 `select(..)`，不算 `NodesRef.ref`。
//!
//! 局限（写清而不是假装没有，方向以漏报为主）：
//!
//! - 不做别名与参数传递追踪：`const r = this.sv.value` 之后再传 `r`、引用经函数参数
//!   转手、经数组 / 对象字段中转，都认不出来，会误报；
//! - 对齐的只是表达式文本，不知道 `select(..)` 选的是哪个节点：同一个 `SharedValue`
//!   被另一个元素的 ref 回调赋值时仍然放行；
//! - `scrollViewContext` 被赋给别的变量名（`const ctx = wx.worklet.scrollViewContext`
//!   之后的 `ctx.scrollTo`）认不出来，会漏报。
//!
//! ## 证据用例：只有 ref 这一支
//!
//! 三条用例随「`scrollTo` 前必须先拿到 ref」那条规则一并并入，note 里的读数是当时实测的：没有
//! ref 时框架打 `scrollTo: fail invalid parameter ref` 且 `scrollTop` 保持 0；拿到
//! ref 的对照页不再打这行。`enhanced` / 刷新属性两支没有视觉也没有证据用例——
//! 模拟器上开与不开 `enhanced` 的返回值、告警、滚动行为都没有差异。

use std::collections::HashMap;

use crate::diag::{Diagnostic, Severity};
use crate::input::js::{CallInfo, FunctionInfo};
use crate::project::{Project, Scope};
use crate::rules::wxml::{for_each_element, structure::find_attr};
use crate::rules::{EvidenceCase, EvidenceExpect, EvidenceRun, Rule, RuleMeta, Target, VisualFile};

use super::{at_span, renders_in_skyline};

const REF_MISSING: &str = "用 `scrollViewContext.scrollTo` 前必须先用 `createSelectorQuery().select(..).ref(cb)` 拿到引用（通常存进 `SharedValue`），否则 ref 是 undefined，这次滚动不会发生";

/// `<scroll-view>` 上「写了就算」的属性。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Attr {
    Enhanced,
    Refresher,
    TwoLevel,
}

impl Attr {
    fn name(self) -> &'static str {
        match self {
            Self::Enhanced => "enhanced",
            Self::Refresher => "refresher-enabled",
            Self::TwoLevel => "refresher-two-level-enabled",
        }
    }

    fn missing(self) -> &'static str {
        match self {
            Self::Enhanced => {
                "宿主 `<scroll-view>` 没有开 `enhanced`：`node()` 拿不到可用的 ScrollViewContext，这个调用只打一条 warning 后返回"
            }
            Self::Refresher => {
                "宿主 `<scroll-view>` 没有开 `refresher-enabled`：没有刷新区，`triggerRefresh` / `closeRefresh` 不会产生下拉刷新"
            }
            Self::TwoLevel => {
                "宿主 `<scroll-view>` 没有开 `refresher-two-level-enabled`：没有二楼下拉入口，`triggerTwoLevel` / `closeTwoLevel` 不生效"
            }
        }
    }
}

/// 一组方法需要的获取方式。
enum Getter {
    /// `NodesRef.node()`：拿到 ScrollViewContext，且宿主开齐这些属性。
    Node(&'static [Attr]),
    /// `NodesRef.ref()`：引用存进 `SharedValue`，作为第一参传入。
    Ref,
}

struct Row {
    /// 链尾方法名。
    methods: &'static [&'static str],
    /// 接收者最后一段必须是什么；`None` 表示不限（刷新家族的方法名是独有的）。
    receiver: Option<&'static str>,
    getter: Getter,
}

/// `方法 → 需要的获取方式`：改这里就是改这条规则的可判范围。
const ROWS: &[Row] = &[
    Row {
        methods: &["triggerRefresh", "closeRefresh"],
        receiver: None,
        getter: Getter::Node(&[Attr::Enhanced, Attr::Refresher]),
    },
    Row {
        methods: &["triggerTwoLevel", "closeTwoLevel"],
        receiver: None,
        getter: Getter::Node(&[Attr::Enhanced, Attr::Refresher, Attr::TwoLevel]),
    },
    Row {
        methods: &["scrollTo"],
        receiver: Some("scrollViewContext"),
        getter: Getter::Ref,
    },
];

/// 单元里的 `<scroll-view>` 具备哪些属性（只在开了 `enhanced` 的元素上统计：
/// 没开 `enhanced` 就拿不到 context，其余属性再多也没用）。
#[derive(Clone, Copy, Default)]
struct Hosts {
    enhanced: bool,
    refresher: bool,
    two_level: bool,
}

impl Hosts {
    fn has(self, attr: Attr) -> bool {
        match attr {
            Attr::Enhanced => self.enhanced,
            Attr::Refresher => self.refresher,
            Attr::TwoLevel => self.two_level,
        }
    }

    fn absorb(&mut self, other: Self) {
        self.enhanced |= other.enhanced;
        self.refresher |= other.refresher;
        self.two_level |= other.two_level;
    }
}

/// 调用的方法名（链尾）。
///
/// `res[0].node.triggerRefresh(..)` 这类链里带计算属性的写法，输入层的 callee 是
/// 空串（整条链放弃），只能从调用区间取。
fn method_tail<'a>(source: &'a str, call: &'a CallInfo) -> &'a str {
    let Some(start) = method_start(source, call) else {
        return call.callee.rsplit('.').next().unwrap_or_default();
    };
    let rest = &source[start..];
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '$')))
        .unwrap_or(rest.len());
    &rest[..end]
}

/// 链尾方法名在源码里的起点；接收者因此是 `call.start..这里`。
///
/// 先退到自己的参数表（无参调用退到右括号），再跳过空白与括号：`ref((res) => ..)`
/// 多一层括号也能落到方法名上。
fn method_start(source: &str, call: &CallInfo) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut i = call
        .args
        .first()
        .map_or(call.end as usize, |arg| arg.start as usize);
    loop {
        while i > 0 && bytes[i - 1].is_ascii_whitespace() {
            i -= 1;
        }
        if i > 0 && matches!(bytes[i - 1], b'(' | b')') {
            i -= 1;
            continue;
        }
        break;
    }
    let end = i;
    while i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || matches!(bytes[i - 1], b'_' | b'$')) {
        i -= 1;
    }
    (i < end).then_some(i)
}

fn receiver_text<'a>(source: &'a str, call: &CallInfo, method_start: usize) -> &'a str {
    source[call.start as usize..method_start]
        .trim()
        .trim_end_matches('.')
}

/// 接收者的最后一段：`wx.worklet.scrollViewContext` → `scrollViewContext`。
fn last_segment(receiver: &str) -> Option<&str> {
    receiver.rsplit('.').next()
}

/// 这次调用是不是 `NodesRef.ref`：链尾 `ref`，且接收者里有 `select(..)` / `selectAll(..)`。
///
/// 只按链尾匹配会把用户自定义的 `obj.ref(cb)` 当成拿节点引用。
fn is_nodes_ref(source: &str, call: &CallInfo) -> bool {
    if method_tail(source, call) != "ref" {
        return false;
    }
    let Some(start) = method_start(source, call) else {
        return false;
    };
    let receiver = &source[call.start as usize..start];
    receiver.contains("select(") || receiver.contains("selectAll(")
}

/// `haystack` 里是否有 `target = <同一句里含 marker 的右侧>`。
///
/// `===` / `==` / `=>` 不算赋值；`target` 必须作为完整表达式出现（前面不能是标识符
/// 字符或 `.`，否则 `x.value` 会命中 `this.x.value`）。
fn assigns(haystack: &str, target: &str, marker: &str) -> bool {
    let mut from = 0;
    while let Some(at) = haystack[from..].find(target) {
        let start = from + at;
        let boundary = haystack[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '$' | '.')));
        let rest = haystack[start + target.len()..].trim_start();
        if boundary
            && let Some(after) = rest.strip_prefix('=')
            && !after.starts_with('=')
            && !after.starts_with('>')
        {
            let statement = after.split([';', '\n']).next().unwrap_or_default();
            if statement.contains(marker) {
                return true;
            }
        }
        from = start + target.len();
    }
    false
}

/// 这次调用是不是落在 `NodesRef.node()` 拿到的 context 上。
///
/// 只认两种写法：接收者最后一段就是 `node`（`res[0].node.triggerRefresh()`），或接收者
/// 在同一个函数里被赋成 `<...>.node`（`const ctx = res[0].node`）。认不出来就整条跳过。
fn is_node_context(
    source: &str,
    receiver: &str,
    call: &CallInfo,
    functions: &[FunctionInfo<'_>],
) -> bool {
    if last_segment(receiver) == Some("node") {
        return true;
    }
    let plain = !receiver.is_empty()
        && receiver
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '$' | '.'));
    if !plain {
        return false;
    }
    let scope = enclosing_function(functions, call)
        .map(|(start, end)| &source[start as usize..end as usize])
        .unwrap_or(source);
    assigns(scope, receiver, ".node")
}

/// 最内层包住这次调用的函数。
fn enclosing_function(functions: &[FunctionInfo<'_>], call: &CallInfo) -> Option<(u32, u32)> {
    functions
        .iter()
        .filter(|f| f.start <= call.start && call.end <= f.end)
        .max_by_key(|f| f.start)
        .map(|f| (f.start, f.end))
}

/// 第一参是否来自 `NodesRef.ref()`：回调里直接用的 `r.ref`，或 ref 回调写进
/// `SharedValue` 的那个表达式。
fn traces_to_ref(source: &str, call: &CallInfo, refs: &[&CallInfo]) -> bool {
    let Some(arg) = call.args.first() else {
        return false;
    };
    let target = source[arg.start as usize..arg.end as usize].trim();
    let encloses = |r: &&CallInfo| r.start <= call.start && call.end <= r.end;
    if target.ends_with(".ref") || arg.ident.is_some() {
        // 回调参数直接当引用用：`ref((r) => scrollTo(r.ref, ..))` / `ref((ref) => ..)`
        return refs.iter().any(encloses);
    }
    if target.ends_with(".value") {
        return refs.iter().any(|r| {
            let chain = &source[r.start as usize..r.end as usize];
            assigns(chain, target, ".ref")
        });
    }
    false
}

/// 每份文档里 `<scroll-view>` 的属性。
fn hosts_by_doc(project: &Project) -> HashMap<String, Hosts> {
    let mut hosts: HashMap<String, Hosts> = HashMap::new();
    for_each_element(project, Target::Skyline, |site| {
        if site.element.tag != "scroll-view"
            || find_attr(site.element, Attr::Enhanced.name()).is_none()
        {
            return;
        }
        let entry = hosts.entry(site.doc.path().to_string()).or_default();
        entry.enhanced = true;
        entry.refresher |= find_attr(site.element, Attr::Refresher.name()).is_some();
        entry.two_level |= find_attr(site.element, Attr::TwoLevel.name()).is_some();
    });
    hosts
}

/// 单元能用的 scroll-view 属性：页面 WXML 加上它引用到的组件 / 模板。
fn unit_hosts(
    project: &Project,
    doc_hosts: &HashMap<String, Hosts>,
    scope: &Scope,
) -> Option<Hosts> {
    let unit = match scope {
        Scope::Page(route) => route,
        Scope::Component(path) => path,
        Scope::Global | Scope::Loose(_) => return None,
    };
    let wxml = project.unit_wxml(unit)?;
    let mut hosts = Hosts::default();
    for doc in project.graph().render_docs(wxml) {
        if let Some(found) = doc_hosts.get(doc) {
            hosts.absorb(*found);
        }
    }
    Some(hosts)
}

pub struct ScrollContext;

impl ScrollContext {
    pub const ID: &'static str = "skyline/scroll-context";
}

impl Rule for ScrollContext {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "调用 ScrollViewContext 前没有拿到 context 或节点引用（刷新缺 `enhanced` / refresher 属性，`scrollTo` 缺 ref）",
        }
    }

    fn evidence_cases(&self) -> &'static [EvidenceCase] {
        EVIDENCE_CASES
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let doc_hosts = hosts_by_doc(project);
        let mut out = Vec::new();
        for file in &project.scripts {
            if !renders_in_skyline(project, &file.scope) {
                continue;
            }
            let source = file.doc.source();
            let refs: Vec<&CallInfo> = file
                .doc
                .calls()
                .iter()
                .filter(|call| is_nodes_ref(source, call))
                .collect();
            let functions = file.doc.functions();
            for call in file.doc.calls() {
                if call.webview_only {
                    continue;
                }
                let name = method_tail(source, call);
                let Some(row) = ROWS.iter().find(|row| row.methods.contains(&name)) else {
                    continue;
                };
                let Some(start) = method_start(source, call) else {
                    continue;
                };
                let receiver = receiver_text(source, call, start);
                if row
                    .receiver
                    .is_some_and(|wanted| last_segment(receiver) != Some(wanted))
                {
                    continue;
                }
                let diagnostic = match row.getter {
                    Getter::Ref => {
                        if traces_to_ref(source, call, &refs) {
                            continue;
                        }
                        at_span(
                            Self::ID,
                            Severity::Warning,
                            file,
                            call.start,
                            call.end,
                            REF_MISSING,
                        )
                    }
                    Getter::Node(attrs) => {
                        if !is_node_context(source, receiver, call, &functions) {
                            continue;
                        }
                        let Some(hosts) = unit_hosts(project, &doc_hosts, &file.scope) else {
                            continue;
                        };
                        let Some(missing) = attrs.iter().copied().find(|attr| !hosts.has(*attr))
                        else {
                            continue;
                        };
                        at_span(
                            Self::ID,
                            Severity::Warning,
                            file,
                            call.start,
                            call.end,
                            missing.missing(),
                        )
                    }
                };
                out.push(diagnostic);
            }
        }
        out
    }
}

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

#[cfg(test)]
mod tests {
    use crate::project::Project;
    use crate::rules::Rule;
    use crate::rules::testutil::js::{Fixture, messages, single};

    use super::ScrollContext;

    /// 一个页面 + 给定 wxml / js。
    fn page(name: &str, wxml: &str, js: &str) -> (Fixture, Project) {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", wxml)
            .write("pages/index/index.js", js);
        let project = fixture.project();
        (fixture, project)
    }

    const REFRESH: &str = r#"
        Page({
          onReady() {
            wx.createSelectorQuery().select('#sv').node().exec((res) => {
              res[0].node.triggerRefresh({ duration: 300 });
            });
          },
        });
    "#;

    #[test]
    fn scroll_to_without_ref_is_reported() {
        let (_f, project) = single(
            "context-ref-missing",
            "function scroll() { 'worklet'; scrollViewContext.scrollTo(undefined, { top: 1 }); }",
        );
        let diagnostics = ScrollContext.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].severity, crate::Severity::Warning);
        assert!(diagnostics[0].message.contains("ref 是 undefined"));
    }

    /// 同文件里另一个元素有 ref：不再按文件级放行。
    #[test]
    fn ref_for_another_element_in_the_same_file_is_still_reported() {
        let (_f, project) = single(
            "context-ref-other-element",
            r#"
            Page({
              onReady() {
                this.createSelectorQuery().select('#other').ref((res) => {
                  this.otherRef.value = res.ref;
                }).exec();
              },
              onTap() {
                'worklet';
                wx.worklet.scrollViewContext.scrollTo(this.scrollRef.value, { top: 120 });
              },
            });
            "#,
        );
        let diagnostics = ScrollContext.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].line, 10);
    }

    /// 用户自定义的 `obj.ref()` 不是 `NodesRef.ref`。
    #[test]
    fn custom_ref_method_is_not_nodes_ref() {
        let (_f, project) = single(
            "context-ref-custom",
            r#"
            function go(o) {
              o.ref((res) => {
                scrollViewContext.scrollTo(res.ref, { top: 1 });
              });
            }
            "#,
        );
        assert_eq!(ScrollContext.check(&project).len(), 1);
    }

    /// 文档里的标准写法：`onLoad` 取 ref 写进 `SharedValue`，`onTap` 在 worklet 里用。
    #[test]
    fn shared_value_from_a_ref_callback_passes() {
        let (_f, project) = single(
            "context-ref-shared",
            r#"
            Page({
              onLoad() {
                this.scrollRef = wx.worklet.shared();
                this.createSelectorQuery()
                  .select('.scrollable')
                  .ref((res) => {
                    this.scrollRef.value = res.ref;
                  })
                  .exec();
              },
              onTap() {
                'worklet';
                wx.worklet.scrollViewContext.scrollTo(this.scrollRef.value, { top: 200 });
              },
            });
            "#,
        );
        assert_eq!(messages(&ScrollContext, &project), Vec::<String>::new());
    }

    /// 回调里直接传 `r.ref` 也算拿到了引用。
    #[test]
    fn ref_inside_the_callback_passes() {
        let (_f, project) = single(
            "context-ref-enclosed",
            r#"
            function scroll() {
              'worklet';
              this.createSelectorQuery().select('.scrollable').ref((res) => {
                this.ref.value = res.ref;
                scrollViewContext.scrollTo(this.ref.value, { top: 1 });
              }).exec();
            }
            "#,
        );
        assert_eq!(messages(&ScrollContext, &project), Vec::<String>::new());
    }

    /// 另一个文件里的 ref 不覆盖本文件。
    #[test]
    fn ref_in_another_file_does_not_cover_this_one() {
        let (fixture, _) = single(
            "context-ref-other-file",
            "scrollViewContext.scrollTo(this.ref.value, { top: 1 });",
        );
        fixture
            .write("pages/other/other.js", "query.select('.x').ref(cb);")
            .write("pages/other/other.json", "{}")
            .write("pages/other/other.wxml", "<view/>")
            .write(
                "app.json",
                r#"{"pages":["pages/index/index","pages/other/other"],"renderer":"skyline"}"#,
            );
        let project = fixture.project();
        let diagnostics = ScrollContext.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].path, "pages/index/index.js");
    }

    /// `ScrollViewContext.scrollTo` 自己的第一参是参数对象，不是 worklet 那个 ref。
    #[test]
    fn scroll_to_on_the_node_result_is_not_judged() {
        let (_f, project) = single(
            "context-scroll-to-node",
            r#"
            Page({
              onReady() {
                wx.createSelectorQuery().select('#sv').node().exec((res) => {
                  res[0].node.scrollTo({ top: 0 });
                });
              },
            });
            "#,
        );
        assert_eq!(messages(&ScrollContext, &project), Vec::<String>::new());
    }

    /// `scrollIntoView` 的第一参是选择器字符串，不需要 ref，不判。
    #[test]
    fn scroll_into_view_is_not_judged() {
        let (_f, project) = single(
            "context-scroll-into-view",
            "function scroll() { 'worklet'; scrollViewContext.scrollIntoView('#target', { animated: true }); }",
        );
        assert_eq!(messages(&ScrollContext, &project), Vec::<String>::new());
    }

    #[test]
    fn refresh_without_enhanced_is_reported() {
        let (_f, project) = page(
            "context-enhanced-missing",
            r#"<scroll-view id="sv" type="list" scroll-y/>"#,
            REFRESH,
        );
        let diagnostics = ScrollContext.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("`enhanced`"));
    }

    #[test]
    fn enhanced_without_refresher_enabled_is_reported() {
        let (_f, project) = page(
            "context-refresher-missing",
            r#"<scroll-view id="sv" type="list" scroll-y enhanced/>"#,
            REFRESH,
        );
        let diagnostics = ScrollContext.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(
            diagnostics[0].message.contains("refresher-enabled"),
            "{}",
            diagnostics[0].message
        );
    }

    #[test]
    fn refresh_with_every_attribute_passes() {
        let (_f, project) = page(
            "context-refresher-ok",
            r#"<scroll-view id="sv" type="list" scroll-y enhanced refresher-enabled/>"#,
            REFRESH,
        );
        assert_eq!(messages(&ScrollContext, &project), Vec::<String>::new());
    }

    #[test]
    fn two_level_needs_its_own_attribute() {
        let js = r#"
            Page({
              onReady() {
                wx.createSelectorQuery().select('#sv').node().exec((res) => {
                  res[0].node.triggerTwoLevel({ duration: 500 });
                });
              },
            });
        "#;
        let (_f, project) = page(
            "context-two-level-missing",
            r#"<scroll-view id="sv" type="list" scroll-y enhanced refresher-enabled/>"#,
            js,
        );
        let diagnostics = ScrollContext.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(
            diagnostics[0]
                .message
                .contains("refresher-two-level-enabled"),
            "{}",
            diagnostics[0].message
        );

        let (_f, project) = page(
            "context-two-level-ok",
            r#"<scroll-view id="sv" type="list" scroll-y enhanced refresher-enabled refresher-two-level-enabled/>"#,
            js,
        );
        assert_eq!(messages(&ScrollContext, &project), Vec::<String>::new());
    }

    /// 同一个函数里把 `res[0].node` 存进变量再调用，算拿到了 context。
    #[test]
    fn node_stored_in_a_variable_is_tracked() {
        let (_f, project) = page(
            "context-node-variable",
            r#"<scroll-view id="sv" type="list" scroll-y enhanced refresher-enabled/>"#,
            r#"
            Page({
              onReady() {
                wx.createSelectorQuery().select('#sv').node().exec((res) => {
                  const ctx = res[0].node;
                  ctx.triggerRefresh({ duration: 300 });
                });
              },
            });
            "#,
        );
        assert_eq!(messages(&ScrollContext, &project), Vec::<String>::new());
    }

    /// 追不到 `.node` 的同名方法不判：可能是别的对象。
    #[test]
    fn refresh_without_node_is_not_judged() {
        let (_f, project) = page(
            "context-no-node",
            r#"<scroll-view id="sv" type="list" scroll-y/>"#,
            "Page({ onReady() { this.panel.triggerRefresh(); } });",
        );
        assert_eq!(messages(&ScrollContext, &project), Vec::<String>::new());
    }

    #[test]
    fn enhanced_in_a_child_component_passes() {
        let fixture = Fixture::new("context-enhanced-component");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                r#"{"usingComponents":{"list":"/components/list/list"}}"#,
            )
            .write("pages/index/index.wxml", r#"<list id="sv"/>"#)
            .write("pages/index/index.js", REFRESH)
            .write("components/list/list.json", r#"{"component":true}"#)
            .write(
                "components/list/list.wxml",
                r#"<scroll-view type="list" scroll-y enhanced refresher-enabled/>"#,
            )
            .write("components/list/list.js", "Component({});");
        let project = fixture.project();
        assert_eq!(messages(&ScrollContext, &project), Vec::<String>::new());
    }

    #[test]
    fn webview_page_is_skipped() {
        let fixture = Fixture::new("context-webview");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                r#"{"renderer":"webview","navigationStyle":"custom"}"#,
            )
            .write(
                "pages/index/index.wxml",
                r#"<scroll-view id="sv" type="list" scroll-y/>"#,
            )
            .write("pages/index/index.js", REFRESH);
        let project = fixture.project();
        assert_eq!(messages(&ScrollContext, &project), Vec::<String>::new());
    }
}
