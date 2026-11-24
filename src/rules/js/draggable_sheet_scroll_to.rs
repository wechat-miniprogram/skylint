//! `DraggableSheetContext.scrollTo` 的 `size` 与 `pixels` 不能同时传。
//!
//! 运行时两者同时出现时只按 `size` 处理，`pixels` 被静默忽略（`size` 取值 0–1，
//! 与 `pixels` 是两套目标位置语义）。只按 callee 的 `.scrollTo` 尾匹配：
//! `draggableSheetContext.scrollTo`、`this.sheetCtx.scrollTo` 都覆盖，
//! `scrollViewContext.scrollTo` 的参数表里没有这两个键，不会误命中。
//!
//! 证据：`skyline-scroll-api/references/api/draggable-sheet-context.md:32`
//! （「`size` 和 `pixels` 同时传入时，仅 size 生效」）。
//!
//! ## 证据用例：为什么没有
//!
//! 本条**不可自动证**。沙箱实测（页面里放 `<draggable-sheet initial-child-size="0.5">`，
//! `node().scrollTo` 依次传 `{size:0.9}` / `{size:0.1}` / `{size:0.5,pixels:100}`）：
//!
//! - `#sheet` 与子节点的 `boundingClientRect` 在每次调用后与初始值逐字相同
//!   （页面侧看到的是布局盒，原生层的 transform 位移取不到，`computedStyle` 也不可用）；
//! - `scrollTo({pixels: ...})` 会把 appservice 事件循环卡死，之后的
//!   `scrollOffset` 回调与 evaluate 都不再返回；
//! - 唯一稳定的信号是 `{}` 触发的 `parameter must contain size or pixels` warning，
//!   它只区分「有没有传键」，区分不了 size 与 pixels 的优先级。
//!
//! 没有能归因到「pixels 被忽略」的稳定读数，硬写一条几何断言只会是空过或超时，
//! 因此不写自动用例，留给真实设备或截图证据。

use crate::Severity;
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};

use super::{Callee, at_span};

const SCROLL_TO: Callee = Callee::Qualified("scrollTo");

pub struct DraggableSheetScrollTo;

impl DraggableSheetScrollTo {
    pub const ID: &'static str = "skyline/draggable-sheet-scroll-to";
}

impl Rule for DraggableSheetScrollTo {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "DraggableSheetContext.scrollTo 同时传 size 与 pixels 时 pixels 被忽略",
        }
    }

    fn check(&self, project: &Project) -> Vec<crate::Diagnostic> {
        let mut out = Vec::new();
        for file in &project.scripts {
            for call in file.doc.calls() {
                if !SCROLL_TO.matches(&call.callee) {
                    continue;
                }
                let source = file.doc.source();
                let conflicting = call.args.iter().any(|arg| {
                    // 引擎只认 `JS_IS_NUMBER`：`size: undefined` 时照旧用 pixels。
                    let value_is_number = |key: &str| {
                        arg.object_keys
                            .iter()
                            .position(|name| name == key)
                            .and_then(|index| arg.object_key_spans.get(index))
                            .map(|(_, end)| {
                                let rest = &source[*end as usize..arg.end as usize];
                                let rest = rest.trim_start().trim_start_matches(':').trim_start();
                                !matches!(
                                    rest.split([',', '}']).next().unwrap_or("").trim(),
                                    "undefined" | "null" | "void 0"
                                )
                            })
                            .unwrap_or(false)
                    };
                    value_is_number("size") && value_is_number("pixels")
                });
                if conflicting {
                    out.push(at_span(
                        Self::ID,
                        Severity::Warning,
                        file,
                        call.start,
                        call.end,
                        "`size` 与 `pixels` 同时传入时只按 `size` 处理，`pixels` 会被忽略",
                    ));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;

    use super::DraggableSheetScrollTo;
    use crate::rules::testutil::js::{messages, single};

    #[test]
    fn size_and_pixels_together_are_reported() {
        let (_f, project) = single(
            "sheet-positive",
            r#"
            this.sheetCtx.scrollTo({ size: 0.5, pixels: 100 });
            draggableSheetContext.scrollTo({ duration: 300, size: 1, pixels: 50 });
            this.sheet.scrollTo({ pixels: 20, animated: true, size: 0.2 });
            "#,
        );
        let diagnostics = DraggableSheetScrollTo.check(&project);
        assert_eq!(diagnostics.len(), 3, "{diagnostics:#?}");
        assert!(
            diagnostics
                .iter()
                .all(|d| d.severity == crate::Severity::Warning)
        );
        assert!(diagnostics[0].message.contains("`pixels` 会被忽略"));
    }

    #[test]
    fn only_one_of_size_and_pixels_is_accepted() {
        let (_f, project) = single(
            "sheet-negative-one",
            r#"
            draggableSheetContext.scrollTo({ size: 0.5 });
            draggableSheetContext.scrollTo({ pixels: 100 });
            draggableSheetContext.scrollTo({ duration: 300 });
            "#,
        );
        assert_eq!(
            messages(&DraggableSheetScrollTo, &project),
            Vec::<String>::new()
        );
    }

    #[test]
    fn keys_in_separate_arguments_do_not_conflict() {
        let (_f, project) = single(
            "sheet-negative-args",
            "draggableSheetContext.scrollTo({ size: 0.5 }, { pixels: 100 });",
        );
        assert_eq!(
            messages(&DraggableSheetScrollTo, &project),
            Vec::<String>::new()
        );
    }

    #[test]
    fn computed_keys_are_not_static() {
        let (_f, project) = single(
            "sheet-negative-computed",
            "draggableSheetContext.scrollTo({ size: 0.5, [pixels]: 100 });",
        );
        assert_eq!(
            messages(&DraggableSheetScrollTo, &project),
            Vec::<String>::new()
        );
    }

    #[test]
    fn calls_that_are_not_scroll_to_are_ignored() {
        let (_f, project) = single(
            "sheet-negative-other",
            r#"
            this.setData({ size: 0.5, pixels: 100 });
            scrollTo({ size: 0.5, pixels: 100 });
            sheetCtx.scrollIntoView({ size: 0.5, pixels: 100 });
            "#,
        );
        assert_eq!(
            messages(&DraggableSheetScrollTo, &project),
            Vec::<String>::new()
        );
    }
}
