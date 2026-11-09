//! worklet 里解构 `this.data` 会让 `this.data` 被 `Object.freeze` 冻结，`setData` 静默失效。
//!
//! worklet 捕获的外部对象会被冻结以保证跨线程安全。`const { msg } = this.data` 这种
//! 解构会把 `this.data` 整个对象作为 worklet 的外部引用捕获，于是 `this.data` 被冻结，
//! 之后所有 `setData` 都不再生效；正确写法是直接访问属性
//! （`const msg = this.data.msg`）。
//!
//! 只看 `VariableDeclarator`：id 是对象解构、init 是字面量 `this.data`。`this.props`、
//! 数组解构、`const msg = this.data.msg` 都不在此列。解构是否在 worklet 内由
//! [`ThisDataDestructure::worklet_scope`] 给出，取最近的外层函数——worklet 里嵌的
//! 普通函数不算 worklet。
//!
//! 证据：`skyline-worklet/references/core/worklet-overview.md:244-258`
//! （「Object.freeze 冻结问题」：worklet 引用的对象类型会被冻结，要直接访问属性，
//! 解构 `this.data` 会让 `setData` 失效）。

use crate::input::js::ThisDataDestructure;
use crate::project::{JsFile, Project};
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

use super::at_span;

pub struct WorkletDataFreeze;

impl WorkletDataFreeze {
    pub const ID: &'static str = "skyline/worklet-data-freeze";
}

impl Rule for WorkletDataFreeze {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Error,
            description: "worklet 内解构 this.data 会冻结 this.data，导致 setData 失效",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        project
            .scripts
            .iter()
            .flat_map(|file| {
                file.doc
                    .this_data_destructures()
                    .iter()
                    .filter(|item| item.worklet_scope)
                    .map(|item| diagnostic(file, item))
            })
            .collect()
    }
}

fn diagnostic(file: &JsFile, item: &ThisDataDestructure) -> Diagnostic {
    at_span(
        WorkletDataFreeze::ID,
        Severity::Error,
        file,
        item.start,
        item.end,
        "worklet 内解构 `this.data` 会把它标记为 Object.freeze 冻结，之后 `setData` 静默失效",
    )
    .help("改为直接访问属性，如 `const msg = this.data.msg`")
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testutil::js::{messages, single};

    use super::WorkletDataFreeze;

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
    fn multiple_destructures_and_let_are_all_reported() {
        let (_f, project) = single(
            "freeze-multiple",
            r#"
            const animated = () => {
              'worklet';
              let { a, b } = this.data;
              const { c } = this.data;
            };
            "#,
        );
        assert_eq!(WorkletDataFreeze.check(&project).len(), 2);
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
    fn destructuring_outside_worklet_is_ignored() {
        let (_f, project) = single(
            "freeze-negative",
            r#"
            function handleTap() {
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
