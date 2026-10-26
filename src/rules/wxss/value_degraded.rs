//! 属性可用，但该值在 Skyline 下被降级 / 映射 / 无效果，表现与 WebView 不同。
//!
//! 与 [`super::unsupported_value`] 的分界：这里是「值有效但行为不同」（warning），
//! 那里是「值根本无效，声明被静默忽略」（error）。数据以 const 形式定义在本模块。
//!
//! 例外是 `will-change`：引擎不认的值只是拿不到性能提示、没有视觉差异，按 info 报
//! （见 [`check_will_change`]）。

use crate::input::wxss::{ValueToken, WxssDeclaration, WxssDoc};
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

use super::dict_table::normalize_value;
use super::value_util::{
    all_calls, at_value, at_value_with, is_dynamic, is_ident, top_level_calls,
};
use super::visit_declarations;

const EXACT_HELP: &str = "按实际渲染效果调整取值，或改用 Skyline 下表现一致的值";
const ONLY_HELP: &str = "去掉不在支持范围内的值，只保留 auto 或 contents";
const FUNCTION_HELP: &str =
    "改用 Skyline 支持的等价写法（例如用 background-image 代替 content: url()）";
const ENV_HELP: &str =
    "仅支持 safe-area-inset-top / -right / -bottom / -left 四个变量，其他尺寸改用固定值或自行计算";
const FONT_WEIGHT_HELP: &str = "Skyline 在 font_weight.cc 里先 round 到百位、再在 SetFontWeight(int) 的 switch 里回落 normal，所以 950 会变成普通字重；直接写 400/500/…/900";
const WILL_CHANGE_HELP: &str =
    "保留也无害；若要按 Skyline 的能力优化，可用 contents（提层）或 scroll-position";
const FONT_FAMILY_HELP: &str = "把 system-ui 换成具体字体名（如 -apple-system / sans-serif），并保留一个 Skyline 可用的兜底字体";

pub struct ValueDegraded;

impl ValueDegraded {
    pub const ID: &'static str = "skyline/value-degraded";
}

impl Rule for ValueDegraded {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "属性在 Skyline 下可用，但该值的表现与 WebView 不同（被降级 / 映射 / 无效果）",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit_declarations(project, |doc, decl, out| {
            if decl.value.trim().is_empty() {
                return;
            }
            check_exact(doc, decl, out);
            check_only(doc, decl, out);
            check_function(doc, decl, out);
            check_font_weight(doc, decl, out);
            check_font_family(doc, decl, out);
            check_will_change(doc, decl, out);
            check_env(doc, decl, out);
        })
    }
}

/// `[exact]`：整值与表里的值相等（与 `[unsupported-value]` 同一匹配口径）。
fn check_exact(doc: &WxssDoc, decl: &WxssDeclaration, out: &mut Vec<Diagnostic>) {
    let name = decl.name.to_ascii_lowercase();
    let Some(value) = normalize_value(&decl.value) else {
        return;
    };
    let Some((_, _, note)) = EXACT_VALUES.iter().find(|(names, candidate, _)| {
        *candidate == value.as_str() && names.contains(&name.as_str())
    }) else {
        return;
    };
    out.push(at_value(
        ValueDegraded::ID,
        doc,
        decl,
        format!(
            "`{name}: {}` 在 Skyline 下的表现与 WebView 不同：{note}",
            decl.value.trim()
        ),
        EXACT_HELP,
    ));
}

/// `[only]`：值里的每个 ident 都必须在允许列表内。
fn check_only(doc: &WxssDoc, decl: &WxssDeclaration, out: &mut Vec<Diagnostic>) {
    let name = decl.name.to_ascii_lowercase();
    let Some((_, allowed, note)) = ONLY_VALUES
        .iter()
        .find(|(property, _, _)| *property == name.as_str())
    else {
        return;
    };
    if is_dynamic(&decl.tokens) {
        return;
    }
    let mut offenders: Vec<&str> = Vec::new();
    for token in &decl.tokens {
        let ValueToken::Ident(ident) = token else {
            continue;
        };
        if allowed
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(ident))
        {
            continue;
        }
        if !offenders
            .iter()
            .any(|seen| seen.eq_ignore_ascii_case(ident))
        {
            offenders.push(ident);
        }
    }
    if offenders.is_empty() {
        return;
    }
    let list = offenders
        .iter()
        .map(|ident| format!("`{ident}`"))
        .collect::<Vec<_>>()
        .join("、");
    out.push(at_value(
        ValueDegraded::ID,
        doc,
        decl,
        format!(
            "`{name}: {}` 在 Skyline 下的表现与 WebView 不同：{}，{list} 不生效",
            decl.value.trim(),
            note
        ),
        ONLY_HELP,
    ));
}

/// `[function]`：值里出现指定函数调用。
fn check_function(doc: &WxssDoc, decl: &WxssDeclaration, out: &mut Vec<Diagnostic>) {
    let name = decl.name.to_ascii_lowercase();
    let Some(note) = top_level_calls(&decl.tokens).into_iter().find_map(|call| {
        FUNCTION_VALUES
            .iter()
            .find(|(property, function, _)| {
                *property == name.as_str() && call.name.eq_ignore_ascii_case(function)
            })
            .map(|(_, _, note)| *note)
    }) else {
        return;
    };
    out.push(at_value(
        ValueDegraded::ID,
        doc,
        decl,
        format!(
            "`{name}: {}` 在 Skyline 下不受支持：{note}",
            decl.value.trim()
        ),
        FUNCTION_HELP,
    ));
}

/// `will-change`：引擎只认 `auto` / `contents` / `scroll-position`，其余 ident 被静默忽略。
///
/// 与其它降级值不同的是**没有视觉差异**——`will-change` 只是性能提示，引擎也只有
/// `contents` 会设置 repaint boundary（`style/style_builder.cc:1178-1186`），
/// 所以按 info 报，不能沿用「表现与 WebView 不同」的 warning 口径。
///
/// `scroll-position` 在引擎里是有效值（`css_properties/will_change.cc:30-51`），不报。
fn check_will_change(doc: &WxssDoc, decl: &WxssDeclaration, out: &mut Vec<Diagnostic>) {
    if !decl.name.eq_ignore_ascii_case("will-change") {
        return;
    }
    if decl.value.trim().is_empty() || is_dynamic(&decl.tokens) {
        return;
    }
    let mut offenders: Vec<&str> = Vec::new();
    for token in &decl.tokens {
        let ValueToken::Ident(ident) = token else {
            continue;
        };
        // `auto` / `none` 与 WebView 一样什么都不做；`contents` / `scroll-position` 引擎认。
        if ["auto", "none", "contents", "scroll-position"]
            .iter()
            .any(|supported| supported.eq_ignore_ascii_case(ident))
        {
            continue;
        }
        if !offenders
            .iter()
            .any(|seen| seen.eq_ignore_ascii_case(ident))
        {
            offenders.push(ident);
        }
    }
    if offenders.is_empty() {
        return;
    }
    let list = offenders
        .iter()
        .map(|ident| format!("`{ident}`"))
        .collect::<Vec<_>>()
        .join("、");
    out.push(at_value_with(
        ValueDegraded::ID,
        doc,
        decl,
        Severity::Info,
        format!(
            "`will-change: {}` 里的 {list} 不会被 Skyline 识别：只是拿不到这层性能提示，渲染结果不受影响",
            decl.value.trim()
        ),
        WILL_CHANGE_HELP,
    ));
}

/// font-weight 的数值：Skyline 先四舍五入到百位，再按 switch 选字重，落不到 100–900 时回落 normal。
///
/// 源码：`skyline/css/css_properties/font_weight.cc` 的 `FontWeight::ApplyValue`
/// 里 `round_num = std::round(num / 100)` 后 `SetFontWeight(round_num * 100)`；
/// `skyline/style/computed_style.h` 的 `SetFontWeight(int)` 只映射 100/200/…/900，
/// `default` 分支取 `FontWeight::kNormal`（= kW400，见 `common/dart_definitions.h`）。
/// 「四舍五入 + 越界回落」是两段语义，数据表表达不了，所以按属性特化判定。
fn check_font_weight(doc: &WxssDoc, decl: &WxssDeclaration, out: &mut Vec<Diagnostic>) {
    if !decl.name.eq_ignore_ascii_case("font-weight") {
        return;
    }
    // 只判单个数字；normal / bold / bolder / lighter / 动态值走其他规则或直接跳过。
    let [ValueToken::Number(text)] = decl.tokens.as_slice() else {
        return;
    };
    let Ok(number) = text.parse::<f64>() else {
        return;
    };
    if number % 100.0 == 0.0 && (100.0..=900.0).contains(&number) {
        return;
    }
    let rounded = (number / 100.0).round() * 100.0;
    let actual = if (100.0..=900.0).contains(&rounded) {
        rounded
    } else {
        400.0
    };
    let message = if actual == rounded {
        format!(
            "`font-weight: {text}` 会被四舍五入到 {}（Skyline 按 100 的倍数取字重）",
            actual as i64
        )
    } else {
        format!(
            "`font-weight: {text}` 会回落到 {}（四舍五入后超出 100–900）",
            actual as i64
        )
    };
    out.push(at_value(
        ValueDegraded::ID,
        doc,
        decl,
        message,
        FONT_WEIGHT_HELP,
    ));
}

/// `font-family` 里的 `system-ui`：`font_family.cc:63-67` 的 `SystemUi` 分支只 `Diag`
/// 不 push，若整条只有它，字体退回默认。
fn check_font_family(doc: &WxssDoc, decl: &WxssDeclaration, out: &mut Vec<Diagnostic>) {
    if !decl.name.eq_ignore_ascii_case("font-family") || is_dynamic(&decl.tokens) {
        return;
    }
    if !decl.tokens.iter().any(|token| is_ident(token, "system-ui")) {
        return;
    }
    out.push(at_value(
        ValueDegraded::ID,
        doc,
        decl,
        format!(
            "`font-family: {}` 里的 `system-ui` 在 Skyline 下被丢弃，若整条只含它字体会退回默认字体",
            decl.value.trim()
        ),
        FONT_FAMILY_HELP,
    ));
}

/// `[env]`：不按属性，扫全部声明值里的 `env(<ident>)`（可能嵌在 `calc()` 里）。
fn check_env(doc: &WxssDoc, decl: &WxssDeclaration, out: &mut Vec<Diagnostic>) {
    // 自定义属性只是存值，真正生效在使用处；报在定义处是噪音。
    if decl.name.starts_with("--") {
        return;
    }
    let mut unknown: Vec<String> = Vec::new();
    let mut has_fallback = false;
    for call in all_calls(&decl.tokens) {
        if call.name != "env" {
            continue;
        }
        let Some(ValueToken::Ident(var)) = call.args.first() else {
            continue;
        };
        if ENV_VARIABLES
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(var))
        {
            continue;
        }
        if !unknown.iter().any(|seen| seen.eq_ignore_ascii_case(var)) {
            unknown.push(var.clone());
        }
        has_fallback |= call.args.iter().any(|arg| matches!(arg, ValueToken::Comma));
    }
    if unknown.is_empty() {
        return;
    }
    let calls = unknown
        .iter()
        .map(|var| format!("env({var})"))
        .collect::<Vec<_>>()
        .join("、");
    let tail = if has_fallback {
        "；fallback 会让声明仍然有效，但拿不到期望的变量值"
    } else {
        ""
    };
    out.push(at_value(
        ValueDegraded::ID,
        doc,
        decl,
        format!("{calls} 不在 Skyline 支持的 env() 变量列表内{tail}"),
        ENV_HELP,
    ));
}

// 来源：/Volumes/2tb-ssd/nebula/Work/skyline-skills/skills/skyline-wxss/references/*.md
// 语义：属性本身受支持，但该值在 Skyline 下被降级 / 映射 / 无效果，表现与 WebView 不同。
//       「值根本无效」属于 wxss_support 表（error），不在这里。

/// `[exact]` 条目：`(属性名列表, 整值, 说明)`。
///
/// 属性名列表对应原表一格多属性；值 trim + 小写后整值相等才命中。
const EXACT_VALUES: &[(&[&str], &str, &str)] = &[
    (
        &["animation-fill-mode"],
        "none",
        "可写，但实际按 forwards 处理",
    ), // animation.md:78
    (
        &["animation-fill-mode"],
        "backwards",
        "可写，但实际按 forwards 处理",
    ), // animation.md:78
    (
        &["word-break"],
        "keep-all",
        "可解析，但渲染层未实现，实际等同 normal",
    ), // text.md:58
    (&["word-break"], "break-word", "实际被映射为 normal"), // text.md:59
    (&["background-repeat"], "space", "实际被降级为 repeat"), // image_property_utils.h:512 兜底
    (&["background-repeat"], "round", "实际被降级为 repeat"), // image_property_utils.h:512 兜底
    (&["mask-repeat"], "space", "实际被降级为 repeat"),     // image_property_utils.h:512 兜底
    (&["mask-repeat"], "round", "实际被降级为 repeat"),     // image_property_utils.h:512 兜底
];

/// `[only]` 条目：`(属性名, 允许的 ident, 说明)`；值里每个 ident 都必须在列表内。
///
/// 目前为空：`will-change` 曾按「只支持 auto / contents」放在这里，但引擎实际认
/// `contents` 与 `scroll-position`（`css_properties/will_change.cc:30-51`），且不支持
/// 的自定义 ident 只是不被当作性能提示、没有视觉差异，用 warning 报是夸大。改成
/// [`check_will_change`] 单独判定。
const ONLY_VALUES: &[(&str, &[&str], &str)] = &[];

/// `[function]` 条目：`(属性名, 函数名, 说明)`；值里出现该函数调用即命中。
const FUNCTION_VALUES: &[(&str, &str, &str)] = &[
    ("content", "url", "图片不会渲染，改用 background-image"), // layout.md:56
];

/// `[env]` 白名单：不按属性，扫全部声明值里的 `env()`。
const ENV_VARIABLES: &[&str] = &[
    "safe-area-inset-top",
    "safe-area-inset-right",
    "safe-area-inset-bottom",
    "safe-area-inset-left",
];

#[cfg(test)]
mod tests {

    use crate::rules::Rule;

    use super::ValueDegraded;
    use crate::rules::testutil::wxss::{Fixture, messages, single};

    #[test]
    fn exact_values_report_the_actual_behaviour() {
        let (_f, project) = single(
            "degraded-exact",
            r#"
            .a { animation-fill-mode: none; }
            .b { animation-fill-mode: backwards; }
            .c { word-break: keep-all; }
            .d { word-break: break-word; }
            .e { background-repeat: space; }
            .f { mask-repeat: round; }
            .g { BACKGROUND-REPEAT: SPACE; }
            "#,
        );
        let diags = ValueDegraded.check(&project);
        assert_eq!(diags.len(), 7);
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Warning));
        assert!(diags[0].message.contains("animation-fill-mode: none"));
        assert!(diags[0].message.contains("forwards"));
        assert!(diags.iter().any(|d| d.message.contains("break-word")));
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("mask-repeat: round"))
        );
        assert_eq!(diags[0].help.as_deref(), Some(super::EXACT_HELP));
    }

    #[test]
    fn exact_accepts_supported_values_and_dynamic_values() {
        let (_f, project) = single(
            "degraded-exact-negative",
            r#"
            .a { animation-fill-mode: forwards; }
            .b { animation-fill-mode: both; }
            .c { word-break: break-all; }
            .d { word-break: normal; }
            .e { background-repeat: no-repeat; }
            .f { background-repeat: var(--repeat); }
            .g { mask-repeat: repeat; }
            .h { mask-repeat: repeat-x; }
            .i { mask-repeat: space; }
            "#,
        );
        // `.i` 是唯一命中项：前面几行只用来确认合法值与动态值不报。
        let diags = ValueDegraded.check(&project);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("mask-repeat"));
    }

    #[test]
    fn other_properties_are_not_affected() {
        let (_f, project) = single(
            "degraded-other-properties",
            r#"
            .a { transition: transform; }
            .b { animation-name: content; }
            .c { background-image: url(x.png); }
            .d { background: url(x.png) space; }
            "#,
        );
        assert_eq!(messages(&ValueDegraded, &project), Vec::<String>::new());
    }

    #[test]
    fn will_change_reports_unsupported_idents_as_info() {
        let (_f, project) = single(
            "degraded-will-change",
            r#"
            .a { will-change: transform; }
            .b { will-change: transform, opacity; }
            "#,
        );
        let diags = ValueDegraded.check(&project);
        assert_eq!(diags.len(), 2);
        // 只是拿不到性能提示，渲染结果不变：不能按 warning 报成「表现不同」。
        assert!(
            diags.iter().all(|d| d.severity == crate::Severity::Info),
            "{diags:#?}"
        );
        assert!(diags[0].message.contains("`transform`"));
        assert!(diags[1].message.contains("`transform`"));
        assert!(diags[1].message.contains("`opacity`"));
    }

    #[test]
    fn will_change_accepts_every_value_the_engine_understands() {
        // 引擎认 auto / contents / scroll-position（`will_change.cc:30-51`），
        // 之前把 scroll-position 当「不生效」报是误报。
        let (_f, project) = single(
            "degraded-will-change-supported",
            r#"
            .a { will-change: auto; }
            .b { will-change: none; }
            .c { will-change: contents; }
            .d { will-change: scroll-position; }
            .e { will-change: var(--layer); }
            "#,
        );
        assert_eq!(messages(&ValueDegraded, &project), Vec::<String>::new());
    }

    #[test]
    fn font_weight_numbers_report_the_effective_weight() {
        let (_f, project) = single(
            "degraded-font-weight",
            r#"
            .a { font-weight: 450; }
            .b { font-weight: 950; }
            .c { font-weight: 50; }
            .d { font-weight: 0; }
            .e { font-weight: 100.5; }
            "#,
        );
        let diags = ValueDegraded.check(&project);
        assert_eq!(diags.len(), 5);
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Warning));
        assert!(diags[0].message.contains("font-weight: 450"));
        assert!(diags[0].message.contains("四舍五入到 500"));
        assert!(diags[1].message.contains("回落到 400"));
        assert!(diags[1].message.contains("超出 100–900"));
        assert!(diags[2].message.contains("四舍五入到 100"));
        assert!(diags[3].message.contains("回落到 400"));
        assert!(diags[4].message.contains("四舍五入到 100"));
    }

    #[test]
    fn font_weight_accepts_hundreds_keywords_and_dynamic_values() {
        let (_f, project) = single(
            "degraded-font-weight-negative",
            r#"
            .a { font-weight: 100; }
            .b { font-weight: 400; }
            .c { font-weight: 700; }
            .d { font-weight: 900; }
            .e { font-weight: normal; }
            .f { font-weight: bold; }
            .g { font-weight: bolder; }
            .h { font-weight: var(--w); }
            "#,
        );
        assert_eq!(messages(&ValueDegraded, &project), Vec::<String>::new());
    }

    #[test]
    fn function_calls_report_content_url() {
        let (_f, project) = single(
            "degraded-function",
            r#"
            .a { content: url(icon.png); }
            .b { content: url("a.png") url("b.png"); }
            "#,
        );
        let diags = ValueDegraded.check(&project);
        assert_eq!(diags.len(), 2);
        assert!(diags.iter().all(|d| d.message.contains("content: url(")));
        assert!(diags[0].message.contains("background-image"));
    }

    #[test]
    fn function_accepts_string_and_keyword_content() {
        let (_f, project) = single(
            "degraded-function-negative",
            r#"
            .a { content: "x"; }
            .b { content: normal; }
            .c { content: none; }
            .d { content: var(--label); }
            .e { background-image: url(a.png); }
            "#,
        );
        assert_eq!(messages(&ValueDegraded, &project), Vec::<String>::new());
    }

    #[test]
    fn font_family_system_ui_is_reported() {
        let (_f, project) = single(
            "degraded-font-family",
            r#"
            .a { font-family: system-ui; }
            .b { font-family: system-ui, -apple-system, sans-serif; }
            .c { font-family: SYSTEM-UI, sans-serif; }
            "#,
        );
        let diags = ValueDegraded.check(&project);
        assert_eq!(diags.len(), 3);
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Warning));
        assert!(diags[0].message.contains("`system-ui`"));
        assert!(diags[1].message.contains("退回默认字体"));
        assert_eq!(diags[0].help.as_deref(), Some(super::FONT_FAMILY_HELP));
    }

    #[test]
    fn font_family_without_system_ui_and_dynamic_values_are_accepted() {
        let (_f, project) = single(
            "degraded-font-family-negative",
            r#"
            .a { font-family: sans-serif; }
            .b { font-family: -apple-system, BlinkMacSystemFont; }
            .c { font-family: "system-ui"; }
            .d { font-family: var(--font); }
            "#,
        );
        assert_eq!(messages(&ValueDegraded, &project), Vec::<String>::new());
    }

    #[test]
    fn env_reports_variables_outside_the_whitelist() {
        let (_f, project) = single(
            "degraded-env",
            r#"
            .a { padding: env(foo); }
            .b { height: calc(100% - env(bar)); }
            .c { padding: env(foo) env(foo); }
            "#,
        );
        let diags = ValueDegraded.check(&project);
        assert_eq!(diags.len(), 3);
        assert!(diags[0].message.contains("env(foo)"));
        assert!(diags[1].message.contains("env(bar)"));
        // 同一个值里的重复变量只列一次。
        assert_eq!(diags[2].message.matches("env(foo)").count(), 1);
    }

    #[test]
    fn env_accepts_the_four_safe_area_variables() {
        let (_f, project) = single(
            "degraded-env-negative",
            r#"
            .a { padding-top: env(safe-area-inset-top); }
            .b { padding-right: env(safe-area-inset-right); }
            .c { padding-bottom: env(safe-area-inset-bottom); }
            .d { padding-left: env(safe-area-inset-left); }
            .e { padding-bottom: env(safe-area-inset-bottom, 20px); }
            .f { --gap: env(foo); }
            "#,
        );
        assert_eq!(messages(&ValueDegraded, &project), Vec::<String>::new());
    }

    #[test]
    fn env_with_fallback_still_reports_and_explains() {
        let (_f, project) = single(
            "degraded-env-fallback",
            r#"
            .a { padding: env(keyboard-height, 10px); }
            "#,
        );
        let diags = ValueDegraded.check(&project);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("env(keyboard-height)"));
        assert!(diags[0].message.contains("fallback"));
    }

    #[test]
    fn preprocessor_sources_are_checked_too() {
        let f = Fixture::new("degraded-less");
        f.write(
            "project.config.json",
            r#"{"miniprogramRoot":"","setting":{"useCompilerPlugins":["less"]}}"#,
        )
        .write(
            "app.json",
            r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
        )
        .write("pages/index/index.json", "{}")
        .write("pages/index/index.wxml", "<view/>")
        .write(
            "pages/index/index.less",
            ".a { will-change: transform; .b { word-break: keep-all; } }",
        );

        let project = f.project();
        let diags = ValueDegraded.check(&project);
        assert_eq!(diags.len(), 2);
        assert!(
            diags.iter().all(|d| d.path == "pages/index/index.less"),
            "{:?}",
            diags.iter().map(|d| &d.path).collect::<Vec<_>>()
        );
    }
}
