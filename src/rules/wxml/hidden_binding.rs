//! `skyline/hidden-binding`：`hidden` 绑定的空串在两端判定相反。
//!
//! Skyline 按**属性是否存在**判定：只要写了 `hidden`，序列化成 `hidden=""` 也算隐藏。
//! WebView 按**字符串真值**判定：`hidden=""` 是 falsy，元素照常显示。
//!
//! 触发形态是表达式可能取到空串：`hidden="{{a && b}}"` 在 `a` 是 String、初始值 `''`
//! 时短路成 `''`，Skyline 整块不渲染、WebView 正常可见；三元里写 `''` 分支同理。
//!
//! 判据刻意收窄到三种能证明「可能产出空串」的形状：裸标识符、`&&` / `||` 链里
//! 可能成为结果的操作数（`&&` 看左侧、`||` 看右侧）、三元里的字符串分支。标识符的
//! 初始值取 [`Project::env`]（单元脚本 `properties` 的字面量默认值与父级静态传值，被 `setData` / `this.data.x = ...`
//! 写过的键已剔除），再看单元脚本顶层 `data` / `properties` 的初始字面量（`Page` /
//! `Component` 及其 `Base({..})` 这类工厂包装都算）；解析不到就不报。比较、
//! `!` / `!!`、`Boolean()`、布尔字面量都不在这些形状里，自然不报。

use glass_easel_template_compiler::parse::expr::Expression;

use crate::diag::{Diagnostic, Severity};
use crate::input::env::Lit;
use crate::project::Project;
use crate::rules::wxml::{for_each_element, span_diagnostic};
use crate::rules::{Rule, RuleMeta, Target, VisualCase, VisualExpect};
pub const ID: &str = "skyline/hidden-binding";

pub struct HiddenBinding;

/// 视觉用例：`data.q` 的初始值是空串，`hidden="{{q}}"` 因此可能产出 `""`。
///
/// Skyline 把 `hidden=""` 当「属性存在」→ 整块不渲染；WebView 按字符串真值把
/// 空串当 falsy → 元素照常显示。`#target` 在 Skyline 侧零墨迹。
static VISUAL_CASES: &[VisualCase] = &[VisualCase {
    wxml: r#"<view class="wrap">
  <view id="target" class="box" hidden="{{q}}">这块在 Skyline 下会消失</view>
  <view class="after">下一块</view>
</view>"#,
    wxss: ".wrap { padding: 20px; }\n.box { height: 60px; background: #cde; }\n.after { background: #eee; }",
    js: Some("Page({ data: { q: '' } });"),
    expect: VisualExpect::Differ,
    note: "`data.q` 初始为空串：Skyline 按属性存在判定，`hidden=\"\"` 也隐藏，蓝色块零墨迹；WebView 按字符串真值把空串当可见，正常显示",
    target: "#target",
}];

impl Rule for HiddenBinding {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Warning,
            description: "`hidden` 绑定可能取到空串：Skyline 按属性存在判定会隐藏，WebView 按字符串真值判定显示",
        }
    }

    fn target(&self) -> Target {
        Target::Skyline
    }

    fn visual_cases(&self) -> &'static [VisualCase] {
        VISUAL_CASES
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        for_each_element(project, self.target(), |site| {
            for attribute in &site.element.attributes {
                if attribute.name != "hidden" {
                    continue;
                }
                // 静态值（`hidden` / `hidden="false"`）与混排绑定不是本规则的形态。
                let Some(expression) = &attribute.expression else {
                    continue;
                };
                if !may_be_empty_string(expression, site.project, site.doc.path()) {
                    continue;
                }
                let value = site.doc.slice(
                    attribute.value_start.unwrap_or(attribute.start),
                    attribute.end,
                );
                out.push(
                    span_diagnostic(
                        ID,
                        site.doc,
                        attribute.start,
                        attribute.end,
                        site.doc.path(),
                        Severity::Warning,
                        format!(
                            "`hidden=\"{value}\"` 可能取到空串：Skyline 按属性存在判定，`hidden=\"\"` 也会隐藏整块；WebView 按字符串真值把空串当可见"
                        ),
                    )
                    .help("把表达式显式布尔化，例如 `!!x` 或比较运算；`hidden` 只应接受布尔语义"),
                );
            }
        });
        out
    }
}

/// 表达式是否可能产出空串。
fn may_be_empty_string(expression: &Expression, project: &Project, path: &str) -> bool {
    match expression {
        Expression::DataField { name, .. } => string_initial(project, path, name),
        // `&&` 的结果可能是左操作数（空串直接成为结果）；`||` 的结果可能是右操作数
        // （前面的空串会继续向后求值）。嵌套链按同样规则向外看。
        Expression::LogicAnd { .. } | Expression::LogicOr { .. } => {
            result_may_be_empty(expression, project, path)
        }
        // 三元任一分支是空串字面量或字符串标识符，就可能把空串交给 `hidden`。
        Expression::Cond {
            true_br, false_br, ..
        } => [true_br, false_br]
            .into_iter()
            .any(|branch| branch_may_be_empty(branch, project, path)),
        _ => false,
    }
}

/// 逻辑链的结果操作数是否可能为空串：`&&` 看左侧、`||` 看右侧，嵌套时递归。
fn result_may_be_empty(expression: &Expression, project: &Project, path: &str) -> bool {
    match expression {
        Expression::DataField { name, .. } => string_initial(project, path, name),
        Expression::LitStr { value, .. } => value.is_empty(),
        Expression::LogicAnd { left, .. } => result_may_be_empty(left, project, path),
        Expression::LogicOr { right, .. } => result_may_be_empty(right, project, path),
        _ => false,
    }
}

fn branch_may_be_empty(branch: &Expression, project: &Project, path: &str) -> bool {
    match branch {
        // 三元分支是普通字符串字面量时只有空串是 falsy，非空串两端都按真值处理。
        Expression::LitStr { value, .. } => value.is_empty(),
        Expression::DataField { name, .. } => string_initial(project, path, name),
        _ => false,
    }
}

/// 标识符的初始值是不是字符串字面量（含 `''`）。
///
/// 先看 [`Project::env`]（`properties` 默认值与父级静态传值）；它没有这个键时再看单元
/// 脚本 `data` 的初始字面量。解析不到一律按 `false`（不报）。
fn string_initial(project: &Project, path: &str, name: &str) -> bool {
    if let Some(value) = project.env(path).and_then(|env| env.get(name)) {
        return matches!(value, Lit::Str(_));
    }
    let data = unit_script(project, path).map(|file| file.doc.data_constants());
    data.as_ref()
        .and_then(|env| env.get(name))
        .is_some_and(|value| matches!(value, Lit::Str(_)))
}

/// 这份 WXML 所属单元的脚本。模板片段不属于任何单元，返回 `None`。
fn unit_script<'a>(project: &'a Project, wxml: &str) -> Option<&'a crate::project::JsFile> {
    let scope = project.scope_of(wxml)?;
    project.scripts.iter().find(|file| file.scope == scope)
}

#[cfg(test)]
mod tests {
    use super::{HiddenBinding, ID};
    use crate::rules::testutil::wxml::{Fixture, check_rule};

    /// 组件夹具：`hidden` 写在组件里，标识符来自组件 `properties` 的默认值。
    fn fixture(name: &str, hidden: &str) -> Fixture {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                r#"{"usingComponents":{"c":"/components/c/c"}}"#,
            )
            .write("pages/index/index.wxml", "<c/>")
            .write("components/c/c.json", r#"{"component":true}"#)
            .write("components/c/c.wxml", &format!("<view {hidden}/>"))
            .write(
                "components/c/c.js",
                r#"Component({ properties: {
                    a: { type: String, value: '' },
                    s: { type: String, value: 'x' },
                    b: { type: Boolean, value: false },
                    n: { type: Number, value: 0 },
                    o: { type: Object, value: {} },
                } });"#,
            );
        fixture
    }

    fn count(name: &str, hidden: &str) -> usize {
        let fixture = fixture(name, hidden);
        check_rule(&fixture, &HiddenBinding)
            .iter()
            .filter(|diagnostic| diagnostic.rule == ID)
            .count()
    }

    /// 页面夹具：标识符来自页面 `data` 的初始字面量。
    fn page_count(name: &str, hidden: &str, js: &str) -> usize {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", &format!("<view {hidden}/>"))
            .write("pages/index/index.js", js);
        check_rule(&fixture, &HiddenBinding)
            .iter()
            .filter(|diagnostic| diagnostic.rule == ID)
            .count()
    }

    #[test]
    fn page_data_string_initial_is_reported() {
        let js = r#"Page({ data: { q: '', r: 'x', flag: false } });"#;
        assert_eq!(page_count("hidden-data-empty", r#"hidden="{{q}}""#, js), 1);
        assert_eq!(page_count("hidden-data-string", r#"hidden="{{r}}""#, js), 1);
        assert_eq!(
            page_count("hidden-data-bool", r#"hidden="{{flag}}""#, js),
            0
        );
    }

    /// `setData` 改写不改变「声明时是字符串」这一事实：本规则只看初始类型。
    #[test]
    fn page_data_written_by_set_data_still_reports() {
        let js = r#"Page({ data: { q: '' }, onLoad() { this.setData({ q: 'x' }); } });"#;
        assert_eq!(page_count("hidden-data-dirty", r#"hidden="{{q}}""#, js), 1);
    }

    /// 顶层工厂包装（`Base({..})` / `fstComponent({..})`）同样承载单元配置对象。
    #[test]
    fn wrapper_factory_data_is_reported() {
        let js = r#"const Base = require('./base'); const config = Base({ data: { q: '' } });"#;
        assert_eq!(page_count("hidden-wrapper", r#"hidden="{{q}}""#, js), 1);
    }

    /// 函数体里的同名对象（请求参数等）不是单元数据，不参与解析。
    #[test]
    fn nested_call_object_is_not_unit_data() {
        let js = r#"Page({ data: {}, onLoad() { wx.request({ data: { q: '' } }); } });"#;
        assert_eq!(page_count("hidden-nested-call", r#"hidden="{{q}}""#, js), 0);
    }

    /// 真实回归：`selectTab`（String，默认 `''`）与比较运算组成的 `&&` 链。
    /// 右操作数的比较不改变「左侧为空串时整链取空串」的事实。
    #[test]
    fn string_left_operand_with_comparison_tail_is_reported() {
        assert_eq!(count("hidden-real", r#"hidden="{{a && a !== b}}""#), 1);
    }

    #[test]
    fn bare_string_binding_is_reported() {
        assert_eq!(count("hidden-bare", r#"hidden="{{a}}""#), 1);
        assert_eq!(count("hidden-bare-nonempty", r#"hidden="{{s}}""#), 1);
    }

    #[test]
    fn logical_chain_operand_is_reported() {
        assert_eq!(count("hidden-and", r#"hidden="{{a && b}}""#), 1);
        assert_eq!(count("hidden-or", r#"hidden="{{b || a}}""#), 1);
        assert_eq!(count("hidden-nested", r#"hidden="{{a && b && b}}""#), 1);
    }

    /// `||` 左侧为空串会继续求值，布尔链尾不会产出空串。
    #[test]
    fn logic_or_with_boolean_tail_is_skipped() {
        assert_eq!(count("hidden-or-bool", r#"hidden="{{a || b}}""#), 0);
    }

    #[test]
    fn ternary_empty_branch_is_reported() {
        assert_eq!(
            count("hidden-cond-empty", r#"hidden="{{b ? 'on' : ''}}""#),
            1
        );
        assert_eq!(
            count("hidden-cond-ident", r#"hidden="{{b ? a : 'on'}}""#),
            1
        );
    }

    #[test]
    fn boolean_forms_are_skipped() {
        assert_eq!(count("hidden-not", r#"hidden="{{!a}}""#), 0);
        assert_eq!(count("hidden-not-not", r#"hidden="{{!!a}}""#), 0);
        assert_eq!(
            count("hidden-boolean-call", r#"hidden="{{Boolean(a)}}""#),
            0
        );
        assert_eq!(count("hidden-compare", r#"hidden="{{a === 'x'}}""#), 0);
        assert_eq!(
            count("hidden-compare-chain", r#"hidden="{{b && a === 'x'}}""#),
            0
        );
        // 已修的真实写法：`!!` 已经把结果布尔化，链左侧不再是字符串。
        assert_eq!(
            count("hidden-not-not-chain", r#"hidden="{{!!a && a !== b}}""#),
            0
        );
    }

    #[test]
    fn non_string_initial_values_are_skipped() {
        assert_eq!(count("hidden-bool-init", r#"hidden="{{b}}""#), 0);
        assert_eq!(count("hidden-num-init", r#"hidden="{{n}}""#), 0);
        assert_eq!(count("hidden-obj-init", r#"hidden="{{o}}""#), 0);
    }

    #[test]
    fn unresolved_and_non_binding_forms_are_skipped() {
        assert_eq!(count("hidden-unknown", r#"hidden="{{missing}}""#), 0);
        assert_eq!(count("hidden-member", r#"hidden="{{a.b}}""#), 0);
        assert_eq!(count("hidden-static", r#"hidden="false""#), 0);
        assert_eq!(count("hidden-empty-attr", "hidden"), 0);
    }

    /// 视觉用例的夹具必须真的命中本规则，否则截图证明的不是规则。
    #[test]
    fn visual_case_triggers_rule() {
        let case = &super::VISUAL_CASES[0];
        let fixture = Fixture::new("hidden-visual");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", case.wxml)
            .write("pages/index/index.wxss", case.wxss)
            .write(
                "pages/index/index.js",
                case.js.unwrap_or("Page({ data: {} });"),
            );
        let diagnostics = check_rule(&fixture, &HiddenBinding);
        assert!(
            diagnostics.iter().any(|diagnostic| diagnostic.rule == ID),
            "视觉用例夹具没有命中规则：{diagnostics:?}"
        );
    }
}
