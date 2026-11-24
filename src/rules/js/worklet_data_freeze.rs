//! worklet 里把 `this.data` 整体捕获成变量（整体赋值或解构）会让 `this.data`
//! 被 `Object.freeze` 冻结，`setData` 静默失效。
//!
//! worklet 捕获的外部对象会被冻结以保证跨线程安全。`const { msg } = this.data` 与
//! `const snapshot = this.data` 都会把 `this.data` 整个对象作为 worklet 的外部引用
//! 捕获，于是 `this.data` 被冻结，之后所有 `setData` 都不再生效；正确写法是直接访问
//! 属性（`const msg = this.data.msg`）。
//!
//! 只看 `VariableDeclarator`：init 是字面量 `this.data`，与左侧是标识符、对象解构
//! （含嵌套）还是数组解构无关。`this.props`、`const msg = this.data.msg` 都不在此列。
//! 捕获是否在 worklet 内由
//! [`ThisDataCapture::worklet_scope`] 给出，取最近的外层函数——worklet 里嵌的
//! 普通函数不算 worklet。
//!
//! 证据：`skyline-worklet/references/core/worklet-overview.md:244-258`
//! （「Object.freeze 冻结问题」：worklet 引用的对象类型会被冻结，要直接访问属性，
//! 解构 `this.data` 会让 `setData` 失效）。

use crate::input::js::ThisDataCapture;
use crate::project::{JsFile, Project};
use crate::rules::{
    EvidenceCase, EvidenceExpect, EvidenceRun, Rule, RuleMeta, VisualCase, VisualExpect, VisualFile,
};
use crate::{Diagnostic, Severity};

use super::at_span;

/// 截图对照（阴性）：截图批次的 `project.config.json` 没有开 `setting.compileWorklet`，
/// worklet 不会被转换，`applyAnimatedStyle` 里的函数按普通闭包执行，捕获的 `snapshot`
/// 不会被标记为跨线程引用，随后 `setData` 在两端都生效——终态同为 AFTER。
///
/// 它不证明冻结机制（机制需要 `compileWorklet: true`，见本文件的证据用例），只证明
/// 「同样一份源码在开关关闭时两端都不冻结」这个阴性前提。
static VISUAL_CASES: &[VisualCase] = &[VisualCase {
    wxml: r#"<view class="wrap">
  <view class="target {{accent ? 'accent' : ''}}" id="target">{{label}}</view>
</view>"#,
    wxss: ".wrap { padding: 20px; }\n.target { width: 200px; height: 88px; background-color: #dfe6f5; color: #213; text-align: center; line-height: 88px; font-size: 16px; }\n.target.accent { width: 300px; height: 160px; background-color: #d33; color: #ffffff; line-height: 160px; }\n",
    js: Some(PAGE_JS),
    expect: VisualExpect::Same,
    note: "阴性对照：截图批次未开 compileWorklet，worklet 不转换、snapshot 不冻结，两端 setData 都生效，终态同为 AFTER（BEFORE 只存在 400ms）；0.498% 残差仅为 AFTER 字形栅格化，不是状态差异",
    target: "#target",
}];

const APP_JSON: &str = r#"{"pages":["pages/index/index"],"renderer":"skyline","componentFramework":"glass-easel","lazyCodeLoading":"requiredComponents"}"#;
const APP_JSON_WEBVIEW: &str = r#"{"pages":["pages/index/index"],"renderer":"webview","componentFramework":"glass-easel","lazyCodeLoading":"requiredComponents"}"#;
const APP_JS: &str = "App({})\n";
const PAGE_JSON: &str = r#"{"navigationStyle":"custom"}"#;
const PAGE_WXML: &str = "<view class=\"wrap\">\n  <view class=\"target {{accent ? 'accent' : ''}}\" id=\"target\">{{label}}</view>\n</view>\n";

/// `this.data` 在 worklet 内恒为 undefined（实测），所以必须先在页面作用域把 data 对象
/// 捕获成 `snapshot`，再让 worklet 解构它：这正是「捕获外部对象」的那条路径。
/// 冻结后 `setData` 抛只读错误；800ms 时补一条冻结状态 + 终态标签。
const PAGE_JS: &str = r#"Page({
  data: { label: 'BEFORE', accent: false },
  onReady() {
    const snapshot = this.data
    try {
      this.applyAnimatedStyle('#target', function () {
        'worklet'
        const { accent } = snapshot
        return { opacity: accent ? 0.35 : 1 }
      })
      console.log('EVIDENCE_WORKLET_REGISTERED')
    } catch (e) {
      console.log('EVIDENCE_WORKLET_THROW ' + e.message)
    }
    setTimeout(() => {
      try {
        this.setData({ label: 'AFTER', accent: true })
        console.log('EVIDENCE_SETDATA_OK')
      } catch (e) {
        console.log('EVIDENCE_SETDATA_THROW ' + e.message)
      }
    }, 400)
    setTimeout(() => {
      console.log('EVIDENCE_FROZEN ' + Object.isFrozen(snapshot) + ' label=' + this.data.label)
    }, 800)
  },
});
"#;

const CONFIG_ON: &str =
    r#"{"miniprogramRoot":"","appid":"touristappid","setting":{"compileWorklet":true}}"#;

const FILES_SKYLINE: &[VisualFile] = &[
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
        content: PAGE_JS,
    },
];

const FILES_WEBVIEW: &[VisualFile] = &[
    VisualFile {
        path: "app.json",
        content: APP_JSON_WEBVIEW,
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
        content: PAGE_JS,
    },
];

static EVIDENCE_CASES: &[EvidenceCase] = &[
    EvidenceCase {
        name: "skyline-data-frozen",
        files: FILES_SKYLINE,
        project_config: Some(CONFIG_ON),
        run: EvidenceRun::Serve {
            page: "pages/index/index",
        },
        expect: EvidenceExpect::Console {
            must: &[
                "EVIDENCE_FROZEN true label=BEFORE",
                "EVIDENCE_SETDATA_THROW Cannot assign to read only property",
            ],
            must_not: &["label=AFTER"],
        },
        note: "compileWorklet: true 时 worklet 捕获的 data 对象被冻结（frozen=true），随后的 setData 抛只读错误，标签停在 BEFORE",
    },
    EvidenceCase {
        name: "webview-data-intact",
        files: FILES_WEBVIEW,
        project_config: Some(CONFIG_ON),
        run: EvidenceRun::Serve {
            page: "pages/index/index",
        },
        expect: EvidenceExpect::Console {
            must: &["EVIDENCE_FROZEN false label=AFTER"],
            must_not: &["EVIDENCE_SETDATA_THROW"],
        },
        note: "对照：同一份源码在 WebView 下 updater 不过线程，data 未被冻结（frozen=false），setData 生效，标签变成 AFTER",
    },
];

pub struct WorkletDataFreeze;

impl WorkletDataFreeze {
    pub const ID: &'static str = "skyline/worklet-data-freeze";
}

impl Rule for WorkletDataFreeze {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Error,
            description: "worklet 内把 this.data 赋给变量（含解构）会冻结 this.data，导致 setData 失效",
        }
    }

    fn visual_cases(&self) -> &'static [VisualCase] {
        VISUAL_CASES
    }

    fn evidence_cases(&self) -> &'static [EvidenceCase] {
        EVIDENCE_CASES
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        project
            .scripts
            .iter()
            .flat_map(|file| {
                file.doc
                    .this_data_captures()
                    .iter()
                    .filter(|item| item.worklet_scope)
                    .map(|item| diagnostic(file, item))
            })
            .collect()
    }
}

fn diagnostic(file: &JsFile, item: &ThisDataCapture) -> Diagnostic {
    at_span(
        WorkletDataFreeze::ID,
        Severity::Error,
        file,
        item.start,
        item.end,
        "worklet 内捕获 `this.data`（整体赋值或解构）会把它标记为 Object.freeze 冻结，之后 `setData` 静默失效",
    )
    .help("改为直接访问属性，如 `const msg = this.data.msg`")
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testutil::check_rule;
    use crate::rules::testutil::js::{Fixture, messages, single};

    use super::{VISUAL_CASES, WorkletDataFreeze};

    /// 截图对照在 worklet **外**捕获 `this.data`，规则只看 worklet 内的捕获：
    /// 它是 compileWorklet 未开的阴性对照，不应该让规则报错。
    #[test]
    fn visual_case_is_a_negative_control() {
        let case = &VISUAL_CASES[0];
        let fixture = Fixture::new("freeze-visual");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", case.wxml)
            .write("pages/index/index.wxss", case.wxss)
            .write("pages/index/index.js", case.js.expect("视觉用例需要 JS"));
        assert!(check_rule(&fixture, &WorkletDataFreeze).is_empty());
    }

    #[test]
    fn destructuring_inside_worklet_is_reported() {
        let (_f, project) = single(
            "freeze-positive",
            r#"
            function handleTap() {
              'worklet';
              const { msg } = this.data;
              this.setData({ msg });
            }
            "#,
        );
        let diagnostics = WorkletDataFreeze.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].severity, crate::Severity::Error);
        assert_eq!(diagnostics[0].line, 4);
        assert!(diagnostics[0].message.contains("Object.freeze"));
        assert!(
            diagnostics[0]
                .help
                .as_deref()
                .unwrap()
                .contains("this.data.msg")
        );
    }

    #[test]
    fn whole_capture_inside_worklet_is_reported() {
        let (_f, project) = single(
            "freeze-capture",
            r#"
            function handleTap() {
              'worklet';
              const snapshot = this.data;
            }
            "#,
        );
        let diagnostics = WorkletDataFreeze.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].line, 4);
    }

    #[test]
    fn nested_destructuring_inside_worklet_is_reported() {
        let (_f, project) = single(
            "freeze-nested-pattern",
            r#"
            function handleTap() {
              'worklet';
              const { user: { name } } = this.data;
            }
            "#,
        );
        assert_eq!(WorkletDataFreeze.check(&project).len(), 1);
    }

    #[test]
    fn multiple_captures_and_let_are_all_reported() {
        let (_f, project) = single(
            "freeze-multiple",
            r#"
            const animated = () => {
              'worklet';
              const snapshot = this.data;
              let { a, b } = this.data;
              const { c: { d } } = this.data;
            };
            "#,
        );
        assert_eq!(WorkletDataFreeze.check(&project).len(), 3);
    }

    #[test]
    fn destructuring_inside_a_worklet_object_method_is_reported() {
        let (_f, project) = single(
            "freeze-method",
            "Page({ handle(e) { 'worklet'; const { x } = this.data; } });",
        );
        let diagnostics = WorkletDataFreeze.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    #[test]
    fn captures_outside_worklet_are_ignored() {
        let (_f, project) = single(
            "freeze-negative",
            r#"
            function handleTap() {
              const snapshot = this.data;
              const { msg } = this.data;
              this.setData({ msg });
            }
            "#,
        );
        assert_eq!(messages(&WorkletDataFreeze, &project), Vec::<String>::new());
    }

    #[test]
    fn property_access_and_other_sources_are_ignored() {
        let (_f, project) = single(
            "freeze-negative-property",
            r#"
            function handleTap() {
              'worklet';
              const msg = this.data.msg;
              const { a } = this.props;
              const { b } = this.data.user;
              const [first] = this.data.list;
            }
            "#,
        );
        assert_eq!(messages(&WorkletDataFreeze, &project), Vec::<String>::new());
    }

    #[test]
    fn normal_function_nested_in_a_worklet_is_not_a_worklet_scope() {
        let (_f, project) = single(
            "freeze-nested",
            r#"
            const animated = () => {
              'worklet';
              const normal = () => { const { a } = this.data; };
            };
            "#,
        );
        assert_eq!(messages(&WorkletDataFreeze, &project), Vec::<String>::new());
    }
}
