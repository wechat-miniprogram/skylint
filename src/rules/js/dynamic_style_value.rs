//! JS 字符串里的内联样式取值：Skyline 解析不了的写法。
//!
//! 样式值不只在 WXSS 里，也常写在 JS 字符串里再通过 `style="{{xxx}}"` 绑到元素上：
//!
//! ```js
//! wxLoginStyle: 'background-image: -webkit-linear-gradient(left top, #FFD000, #FFBD00)'
//! ```
//!
//! 这类值不在任何 WXSS 文件里，`skyline/gradient-support` 等规则扫不到 WXSS 之外的
//! 写法，本规则补这一层。判据与解析全部复用 WXSS 侧，不另写一份：
//! - 声明块由 `rules/wxss/inline::parse_block` 交给 `input::wxss` 的解析器切成声明，
//!   属性名、取值、词元与样式表里同源；
//! - 渐变函数名取自 `skyline/gradient-support` 的矩阵（`rules/wxss/gradient_support.rs`）：
//!   float-pigment 只认 linear / radial / conic 三个标准名
//!   （`float-pigment-css/src/parser/property_value/gradient.rs:376-438`），厂商前缀名
//!   与 repeating-* 整条声明不生效；mask 的 SetMaskImage 只分派 linear / radial。
//! - 老式 `-webkit-box` 属性名取自 `skyline/legacy-flexbox` 的 `LEGACY_PROPERTIES`，
//!   `display` 的可解析取值取自同模块的 `PARSEABLE_DISPLAY`。
//! - `-webkit-line-clamp` 的修法同 `skyline/unsupported-property`：属性名在引擎里是
//!   无效值，改用 `<text max-lines>`。
//! - 按 unset 回落到初始值的取值复用 `skyline/unsupported-value` 的 `initial_value`
//!   （position: sticky、overflow: auto|scroll、display: grid、vertical-align: text-top …）。
//!
//! 一个字符串只报一次，取第一个命中的问题：先报「整条声明被丢弃」类（老式属性、
//! 非可用 display、`-webkit-line-clamp`、前缀渐变），再报「回落初始值」类。前者是
//! 整块布局失效，后者只是一条属性的取值变了；一条 style 字符串报五条只是噪音。
//!
//! 判不了的写法一律不报：含插值的模板串拼不出结果；取值以 `-` 结尾的拼接半截
//! （`";display:table-" + cell`）不是完整取值。字符串里内联的 HTML 片段
//! （`"<span style='…'>" + text`）不是声明块，解析器直接把它挡在外面。
//!
//! 入口门禁看**内容**、不看名字：字面量里同时有 `;` 与 `:` 才当成声明块。
//! `style` / `wxLoginStyle` 这类名字只是「常常是样式」，`cssText` / `styles` 与
//! 富文本映射表里的键都反例——按名字放行会漏判，按内容判定则由 CSS 语法兜底，
//! 普通文案解析不出声明。
//!
//! 跳过 `npm/**` 构建产物与 `.wxs`（另一套方言）。
//!
//! 已知边界：`*.attrs = { style: … }` 这类把 HTML 属性映射成节点属性的写法会被报
//! （mp-html 就是这种）。机制上是真差异（生成出来的 `<div style="overflow-x:auto">`
//! 在 Skyline 里不滚），只是修法落在第三方库上，所以不额外过滤。
//!
//! reach 口径与 `skyline/unsupported-api` 同向：显式 `renderer: "webview"` 的单元不报
//! （那是「这页不迁 Skyline」的决定）。差别在于这里不要求单元已经在 Skyline 上：
//! 这些值在 WebView 里是好的、迁过去才坏，还没表态的页面正是要检查的对象（同 WXSS
//! 规则的 `unit_wxml` 口径）；`app.js` / `utils/` 这类没有单元归属的脚本判不了归属，
//! 照查。

use crate::input::wxss::{WxssDeclaration, WxssRule};
use crate::project::{Project, Scope};
use crate::rules::wxss::dict_table::{normalize_value, table};
use crate::rules::wxss::gradient_support;
use crate::rules::wxss::inline::parse_block;
use crate::rules::wxss::is_css_wide_keyword;
use crate::rules::wxss::legacy_flexbox::{LEGACY_PROPERTIES, PARSEABLE_DISPLAY};
use crate::rules::wxss::unsupported_value::initial_value;
use crate::rules::wxss::value_util::all_calls;
use crate::rules::{Rule, RuleMeta, VisualCase, VisualExpect};
use crate::{Diagnostic, Severity};

use super::at_span;

pub struct DynamicStyleValue;

/// 可能承载渐变的属性。其余属性上的渐变函数不判——`content: "-webkit-gradient(…)"`
/// 这类写法不在迁移检查的范围内。
const GRADIENT_PROPERTIES: [&str; 6] = [
    "background",
    "background-image",
    "mask",
    "mask-image",
    "border-image",
    "-webkit-mask-image",
];

impl DynamicStyleValue {
    pub const ID: &'static str = "skyline/dynamic-style-value";
}

/// 视觉用例：样式串写在 JS 里、再通过 `style="{{…}}"` 绑到元素上。
///
/// `-webkit-linear-gradient` 整条声明在 Skyline 被丢弃，元素只剩页面底色；
/// WebView 正常画出黄色渐变。
static VISUAL_CASES: &[VisualCase] = &[VisualCase {
    wxml: r#"<view class="login" id="target" style="{{wxLoginStyle}}"></view>"#,
    wxss: ".login { width: 300px; height: 64px; }\n",
    js: Some(
        "Page({ data: { wxLoginStyle: 'background-image: -webkit-linear-gradient(#FFD000, #FFBD00);' } });\n",
    ),
    expect: VisualExpect::Differ,
    note: "style 绑定的 -webkit-linear-gradient：WebView 画出黄色渐变底，Skyline 丢掉整条 background-image，元素只剩页面白底",
    target: "#target",
}];

#[derive(Clone)]
enum Problem {
    /// 整条声明被丢弃：前缀渐变 / repeating 渐变 / mask 的 conic。
    Prefixed(String),
    Repeating(String),
    MaskConic(String),
    /// 老式 `-webkit-box` 弹性盒属性：引擎里没有这个属性名。
    LegacyProperty,
    /// `display` 的取值 fp 解析不了（`-webkit-box` / `-webkit-flex` / `box` …）。
    /// `clamp` 是同一块里的 `-webkit-line-clamp` 原值，修法要一起说。
    DisplayValue {
        clamp: Option<String>,
    },
    /// `-webkit-line-clamp`：引擎把它映射成 kInvalid。
    LineClamp,
    /// 取值让属性按 unset 回落到初始值。
    InitialValue {
        initial: &'static str,
        note: &'static str,
        advice: &'static str,
    },
}

/// 一次命中：哪条声明、什么问题。
struct Finding<'a> {
    decl: &'a WxssDeclaration,
    problem: Problem,
}

impl Rule for DynamicStyleValue {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "JS 字符串里的内联样式用了 Skyline 不支持的写法（前缀渐变、老式弹性盒、不可解析的 display、-webkit-line-clamp、会回落初始值的取值）",
        }
    }

    fn visual_cases(&self) -> &'static [VisualCase] {
        VISUAL_CASES
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        for file in &project.scripts {
            // 构建产物与第三方库里的「样式生成器」不是内联样式；`.wxs` 是另一套方言。
            if file.path.ends_with(".wxs") || file.path.split('/').any(|segment| segment == "npm") {
                continue;
            }
            if webview_only(project, &file.scope) {
                continue;
            }
            for literal in file.doc.strings() {
                // 门禁看内容不看名字：`cssText` / `styles` 同样是内联样式的常见载体。
                if !looks_like_declarations(&literal.value) {
                    continue;
                }
                let Some(block) = parse_block(&file.path, &literal.value) else {
                    continue;
                };
                // 同一个字符串只报一次：诊断都落在字面量起点，重复报只是噪音。
                let Some(finding) = find(&block.rules[0]) else {
                    continue;
                };
                out.push(
                    at_span(
                        Self::ID,
                        Severity::Warning,
                        file,
                        literal.start,
                        literal.start,
                        message(finding.decl, &finding.problem),
                    )
                    .help(help(finding.decl, &finding.problem)),
                );
            }
        }
        out
    }
}

/// 字面量内容像不像声明块：至少一段 `<属性名>: <取值>`。
///
/// 属性名必须是 CSS 标识符（字母 / `-`），普通文案（`request failed: timeout`）挡在
/// 这里；取值不能为空，拼接半截留下的空段也不算。分号不是必要条件：单条声明
/// （`'display: grid'`、`'background-image: -webkit-linear-gradient(…)'`）在工程里
/// 同样常见，要求分号会把它们全漏掉。
fn looks_like_declarations(value: &str) -> bool {
    value.split(';').any(|segment| {
        let Some((name, value)) = segment.split_once(':') else {
            return false;
        };
        let name = name.trim();
        !name.is_empty()
            && name.chars().all(|c| c.is_ascii_alphabetic() || c == '-')
            && !value.trim().is_empty()
    })
}

/// 一段声明里第一个要报的问题：先找「整条声明被丢弃」类，再找「回落初始值」类。
fn find(rule: &WxssRule) -> Option<Finding<'_>> {
    let block = &rule.declarations;
    let complete = |decl: &&WxssDeclaration| {
        // 拼接出来的半截（`";display:table-" + cell`）判不了；
        // 字符串里内联的 HTML 会把引号带进取值（`display:inline-block'`），也跳过。
        !decl.value.ends_with('-') && !decl.value.contains(['"', '\''])
    };
    block
        .iter()
        .filter(complete)
        .find_map(|decl| dropped(decl, block).map(|problem| Finding { decl, problem }))
        .or_else(|| {
            block
                .iter()
                .filter(complete)
                .find_map(|decl| initial(decl).map(|problem| Finding { decl, problem }))
        })
}

/// 整条声明会被丢弃的问题：引擎不认识属性名或取值，级联里更早的值继续生效。
fn dropped(decl: &WxssDeclaration, block: &[WxssDeclaration]) -> Option<Problem> {
    if LEGACY_PROPERTIES.iter().any(|property| is(decl, property)) {
        // `unset` 是迁移工程关掉 WebView 写法的手段，不是「用了不支持的属性」。
        return (!is_css_wide_keyword(&decl.value)).then_some(Problem::LegacyProperty);
    }
    if is(decl, "-webkit-line-clamp") {
        // `unset` 是迁移工程关掉 WebView 写法的手段；`0` 在 WebView 里也不截断
        // （Chromium 只接受正整数），两端一致。
        if is_css_wide_keyword(&decl.value) || normalize_value(&decl.value).as_deref() == Some("0")
        {
            return None;
        }
        return Some(Problem::LineClamp);
    }
    if is(decl, "display") {
        let value = normalize_value(&decl.value)?;
        if PARSEABLE_DISPLAY.contains(&value.as_str()) {
            return None;
        }
        // 渐进增强栈（`display: -webkit-box; display: flex`）里后面还有能解析的取值时，
        // 两端最终都是那一条，前缀写法只是兜底——口径同 `skyline/legacy-flexbox`。
        let later = block
            .iter()
            .position(|other| std::ptr::eq(other, decl))
            .map_or(&[][..], |index| &block[index + 1..])
            .iter()
            .any(|other| {
                is(other, "display")
                    && normalize_value(other.value.as_str())
                        .is_some_and(|value| PARSEABLE_DISPLAY.contains(&value.as_str()))
            });
        return (!later).then_some(Problem::DisplayValue {
            clamp: block
                .iter()
                .find(|other| is(other, "-webkit-line-clamp"))
                .map(|other| other.value.clone()),
        });
    }
    gradient_problem(decl, block)
}

/// 取值不受支持、按 unset 回落到初始值的问题；判据与文案都复用 `skyline/unsupported-value`。
fn initial(decl: &WxssDeclaration) -> Option<Problem> {
    let value = normalize_value(&decl.value)?;
    let name = decl.name.to_ascii_lowercase();
    let (initial, note) = initial_value(&name, &value)?;
    Some(Problem::InitialValue {
        initial,
        note,
        advice: table()
            .value(&name, &value)
            .unwrap_or("改用 Skyline 支持的取值"),
    })
}

/// 声明的属性名是否等于 `property`（ASCII 大小写不敏感）。
fn is(decl: &WxssDeclaration, property: &str) -> bool {
    decl.name.eq_ignore_ascii_case(property)
}

/// 这个脚本是否只会跑在 WebView 里。
///
/// 只有显式 `renderer: "webview"` 才是「这页不迁 Skyline」的决定；页面没表态只是
/// 还没迁，正是要检查的对象。组件按「引用它的页面」判：全部显式 webview 才算，
/// 没人引用时判不了，按要检查处理。
fn webview_only(project: &Project, scope: &Scope) -> bool {
    let opted_out = |route: &str| {
        project
            .pages
            .iter()
            .any(|page| page.route == route && page.opted_out)
    };
    match scope {
        Scope::Page(route) => opted_out(route),
        Scope::Component(path) => {
            let pages = project.component_pages(path);
            !pages.is_empty() && pages.iter().all(|route| opted_out(route))
        }
        Scope::Global | Scope::Loose(_) => false,
    }
}

/// 调用名（已小写）：`background: url(a.png), -webkit-linear-gradient(...)` 给出两个。
fn calls(decl: &WxssDeclaration) -> Vec<String> {
    all_calls(&decl.tokens)
        .into_iter()
        .map(|call| call.name)
        .collect()
}

/// 只有 WebView 认的厂商前缀写法。`-moz-` / `-ms-` 两端都不认，不算迁移差异。
fn is_prefixed(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.starts_with("-webkit-") && name.ends_with("-gradient")
}

fn is_repeating(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.starts_with("repeating-") && name.ends_with("-gradient")
}

/// 这段声明里第一个不受支持的渐变调用。
fn gradient_problem(decl: &WxssDeclaration, block: &[WxssDeclaration]) -> Option<Problem> {
    if !GRADIENT_PROPERTIES
        .iter()
        .any(|property| is(decl, property))
    {
        return None;
    }
    let mask = is(decl, "mask") || is(decl, "mask-image") || is(decl, "-webkit-mask-image");
    // 前缀写法常和标准写法成对出现（autoprefixer 产物）：同一条声明块里已经有该
    // 属性的标准渐变兜底时，两端渲染一致，不必报。标准名以 gradient-support 的
    // 矩阵为准，本规则不再自带名单。
    let has_fallback = block.iter().any(|other| {
        is(other, &decl.name)
            && calls(other)
                .iter()
                .any(|call| gradient_support::parses(call))
    });
    calls(decl).into_iter().find_map(|call| {
        if is_repeating(&call) {
            Some(Problem::Repeating(call))
        } else if is_prefixed(&call) {
            (!has_fallback).then_some(Problem::Prefixed(call))
        } else if mask && call.eq_ignore_ascii_case("conic-gradient") {
            Some(Problem::MaskConic(call))
        } else {
            None
        }
    })
}

fn message(decl: &WxssDeclaration, problem: &Problem) -> String {
    let (name, value) = (decl.name.as_str(), truncate(&decl.value, 120));
    match problem {
        Problem::Prefixed(call) => format!(
            "内联样式 `{name}` 的取值 `{value}` 用了 `{call}()`；Skyline 只认标准渐变函数名，整条声明会被丢弃"
        ),
        Problem::Repeating(call) => format!(
            "内联样式 `{name}` 的取值 `{value}` 用了 `{call}()`；Skyline 不解析 repeating 渐变，整条声明会被丢弃"
        ),
        Problem::MaskConic(call) => format!(
            "内联样式 `{name}` 的取值 `{value}` 里的 `{call}()` 不被 Skyline 的 mask 支持，整条声明会被丢弃"
        ),
        Problem::LegacyProperty => format!(
            "内联样式 `{name}` 是老式 `-webkit-box` 弹性盒属性，Skyline 没有这个属性名，整条声明会被丢弃"
        ),
        Problem::DisplayValue { .. } => format!(
            "内联样式 `display: {value}` 不是 Skyline 能解析的取值，整条声明会被丢弃，元素退回默认盒模型"
        ),
        Problem::LineClamp => format!(
            "内联样式 `-webkit-line-clamp: {value}` 不起作用：Skyline 不认这个属性名，整条声明会被丢弃，文本会完整铺开"
        ),
        Problem::InitialValue { initial, note, .. } => format!(
            "内联样式 `{name}: {value}` 在 Skyline 下不被支持，引擎按 unset 处理并回落到初始值 {initial}：它会覆盖字符串里更早的同名取值，而不是保留那一条{note}"
        ),
    }
}

fn help(decl: &WxssDeclaration, problem: &Problem) -> String {
    match problem {
        Problem::Prefixed(call) => {
            // 两端颜色相同就等价于纯色，比换标准渐变更简单。
            let color = arguments(&decl.value, call)
                .and_then(two_same_stops)
                .filter(|_| is(decl, "background") || is(decl, "background-image"));
            match color {
                Some(color) => format!(
                    "改用标准语法 `linear-gradient(...)`；两端颜色相同（`{color}`），也可以直接写成 `background-color: {color}`"
                ),
                None => "改用标准语法 `linear-gradient(...)` / `radial-gradient(...)` / `conic-gradient(...)`".to_string(),
            }
        }
        Problem::Repeating(_) => {
            "改用非 repeating 的 `linear-gradient(...)`，或用图片实现重复图案".to_string()
        }
        Problem::MaskConic(_) => {
            "改用 `linear-gradient(...)` / `radial-gradient(...)`，或用 `url()` 遮罩图".to_string()
        }
        Problem::LegacyProperty => {
            "改用 flex 布局（`display: flex` 配 `flex-direction` / `align-items` / `justify-content` / `flex: 1`）".to_string()
        }
        // 同一条 style 字符串里带 `-webkit-line-clamp` 时，修法要把截断一起说清楚。
        Problem::DisplayValue { clamp: Some(lines) } => format!(
            "这块多行省略改用 `<text max-lines=\"{}\" overflow=\"ellipsis\">`；布局本身改用 `display: flex` 配 `flex-direction` / `align-items`",
            line_count(lines)
        ),
        Problem::DisplayValue { clamp: None } => {
            let value = normalize_value(&decl.value).unwrap_or_default();
            match value.as_str() {
                "-webkit-flex" => "改用标准写法 `display: flex`".to_string(),
                "-webkit-box" => "改用 `display: flex`；配套的 `-webkit-box-*` 属性分别换成 flex-direction / align-items / justify-content / flex".to_string(),
                _ => "改用 Skyline 能解析的取值：none / block / inline / inline-block / flex / inline-flex".to_string(),
            }
        }
        Problem::LineClamp => format!(
            "多行省略改用 `<text max-lines=\"{}\" overflow=\"ellipsis\">`",
            line_count(&decl.value)
        ),
        Problem::InitialValue { advice, .. } => advice.to_string(),
    }
}

/// `-webkit-line-clamp` 的行数；不是纯数字时用 `N` 占位（修法仍然成立）。
fn line_count(value: &str) -> String {
    normalize_value(value)
        .filter(|value| value.chars().all(|c| c.is_ascii_digit()))
        .unwrap_or_else(|| "N".to_string())
}

/// 调用实参原文（不含外层括号），按括号嵌套配对。
fn arguments<'a>(value: &'a str, call: &str) -> Option<&'a str> {
    let inner = value
        .get(value.find(call)? + call.len()..)?
        .strip_prefix('(')?;
    let mut depth = 1usize;
    for (index, c) in inner.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&inner[..index]);
                }
            }
            _ => {}
        }
    }
    None
}

/// 最后两段实参相同（忽略大小写与首尾空白）时给出这个色值。
fn two_same_stops(args: &str) -> Option<&str> {
    let args: Vec<&str> = args.split(',').map(str::trim).collect();
    let [.., previous, last] = args.as_slice() else {
        return None;
    };
    (!last.is_empty() && previous.eq_ignore_ascii_case(last)).then_some(*last)
}

fn truncate(value: &str, limit: usize) -> String {
    match value.char_indices().nth(limit) {
        Some((index, _)) => format!("{}…", &value[..index]),
        None => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;

    use super::{DynamicStyleValue, VISUAL_CASES};
    use crate::rules::testutil::check_rule;
    use crate::rules::testutil::js::{Fixture, messages, single};

    /// 组件工程：页面按 `renderer` 渲染，页面引用 `components/c/c`。
    fn component_project(name: &str, renderer: &str) -> (Fixture, crate::project::Project) {
        let f = Fixture::new(name);
        f.write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                &format!(
                    r#"{{"renderer":"{renderer}","usingComponents":{{"c":"/components/c/c"}}}}"#
                ),
            )
            .write("pages/index/index.wxml", "<c/>")
            .write("pages/index/index.js", "Page({});")
            .write("components/c/c.json", r#"{"component":true}"#)
            .write(
                "components/c/c.js",
                "Component({ data: { cardStyle: 'background-image: -webkit-linear-gradient(red, blue)' } });",
            );
        let project = f.project();
        (f, project)
    }

    /// 单页工程：app / 页面 json 都由用例给，脚本里放一条前缀渐变。
    fn page_project(name: &str, app: &str, page: &str) -> (Fixture, crate::project::Project) {
        let f = Fixture::new(name);
        f.write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write("app.json", app)
            .write("pages/index/index.json", page)
            .write("pages/index/index.wxml", "<view/>")
            .write(
                "pages/index/index.js",
                "Page({ data: { cardStyle: 'background-image: -webkit-linear-gradient(red, blue)' } });",
            );
        let project = f.project();
        (f, project)
    }

    /// 视觉用例的同一份 WXML/WXSS/JS 必须真的能触发本规则。
    #[test]
    fn visual_case_triggers_rule() {
        let case = &VISUAL_CASES[0];
        let fixture = Fixture::new("dynamic-style-visual");
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
        let diagnostics = check_rule(&fixture, &DynamicStyleValue);
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.rule == DynamicStyleValue::ID),
            "视觉用例未触发规则：{diagnostics:#?}"
        );
    }

    #[test]
    fn prefixed_gradient_in_a_style_string_is_reported() {
        let (_f, project) = single(
            "dynamic-style-prefixed",
            r#"Page({ data: {
                wxLoginStyle: 'background-image: -webkit-linear-gradient(left top, #FFD000, #FFBD00); color: #222222;border:none',
            } });"#,
        );
        let diagnostics = DynamicStyleValue.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        let diagnostic = &diagnostics[0];
        assert_eq!(diagnostic.path, "pages/index/index.js");
        assert_eq!(diagnostic.severity, crate::Severity::Warning);
        assert!(diagnostic.message.contains("background-image"));
        assert!(diagnostic.message.contains("-webkit-linear-gradient"));
        assert!(diagnostic.message.contains("整条声明会被丢弃"));
        let help = diagnostic.help.as_deref().unwrap_or_default();
        assert!(help.contains("linear-gradient"), "{help}");

        // 诊断落在字符串字面量的起点（含引号的那一列）。
        let doc = &project.scripts[0].doc;
        let offset = doc.source().find("'background-image").unwrap();
        assert_eq!(doc.position(offset), (diagnostic.line, diagnostic.column));
    }

    #[test]
    fn standard_gradient_in_a_style_string_is_accepted() {
        let (_f, project) = single(
            "dynamic-style-standard",
            r#"Page({ data: {
                wxLoginStyle: 'background-image: linear-gradient(to bottom, #FFD000, #FFBD00); color: #222',
            } });"#,
        );
        assert_eq!(messages(&DynamicStyleValue, &project), Vec::<String>::new());
    }

    /// 普通文案 / URL 没有 `<属性名>: <取值>` 形状，门禁直接挡住。
    #[test]
    fn plain_text_strings_are_ignored() {
        let (_f, project) = single(
            "dynamic-style-text",
            r#"Page({ data: {
                style: '网络异常，请稍后重试',
                tip: 'request failed: timeout',
                log: '找不到页面: 404',
                link: 'https://example.com/a:b',
                ratio: '16:9',
            } });"#,
        );
        assert_eq!(messages(&DynamicStyleValue, &project), Vec::<String>::new());
    }

    /// 组件只被显式 `renderer: "webview"` 的页面用到：那页不迁 Skyline，不能报。
    #[test]
    fn components_used_only_by_webview_pages_are_not_reported() {
        let (_f, project) = component_project("dynamic-style-webview-component", "webview");
        assert!(DynamicStyleValue.check(&project).is_empty());
    }

    #[test]
    fn components_used_by_skyline_pages_are_reported() {
        let (_f, project) = component_project("dynamic-style-skyline-component", "skyline");
        assert_eq!(DynamicStyleValue.check(&project).len(), 1);
    }

    /// 还没表态的页面正是迁移对象：app 级没开 Skyline 也要报。
    #[test]
    fn pages_not_yet_migrated_are_reported() {
        let (_f, project) = page_project(
            "dynamic-style-unmigrated",
            r#"{"pages":["pages/index/index"]}"#,
            "{}",
        );
        assert_eq!(DynamicStyleValue.check(&project).len(), 1);
    }

    /// 页面自己声明了 `renderer: "webview"`：这页不迁 Skyline，不能报。
    #[test]
    fn webview_pages_are_not_reported() {
        let (_f, project) = page_project(
            "dynamic-style-webview-page",
            r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            r#"{"renderer":"webview"}"#,
        );
        assert!(DynamicStyleValue.check(&project).is_empty());
    }

    /// 同一条声明块里已有标准渐变兜底时，两端渲染一致，不必报。
    #[test]
    fn a_standard_fallback_in_the_same_style_string_wins() {
        let (_f, project) = single(
            "dynamic-style-fallback",
            r#"Page({ data: {
                cardStyle: 'background-image: -webkit-linear-gradient(#fff, #000); background-image: linear-gradient(#fff, #000)',
            } });"#,
        );
        assert_eq!(messages(&DynamicStyleValue, &project), Vec::<String>::new());
    }

    #[test]
    fn mask_conic_gradient_is_reported() {
        let (_f, project) = single(
            "dynamic-style-mask",
            r#"Page({ data: { cardStyle: 'mask-image: conic-gradient(red, blue)' } });"#,
        );
        let diagnostics = DynamicStyleValue.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("mask-image"));
        assert!(diagnostics[0].message.contains("conic-gradient"));
    }

    /// 真实工程里的写法：两端颜色相同，修法可以更直接。
    #[test]
    fn identical_stops_suggest_a_plain_background_color() {
        let (_f, project) = single(
            "dynamic-style-solid",
            r#"Page({ data: {
                cardStyle: 'background-image: -webkit-linear-gradient(left top, #FFD100, #FFD100)',
            } });"#,
        );
        let diagnostics = DynamicStyleValue.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        let help = diagnostics[0].help.as_deref().unwrap_or_default();
        assert!(help.contains("background-color: #FFD100"), "{help}");
    }

    /// 含插值的模板串拼不出结果，不判。
    #[test]
    fn interpolated_templates_are_ignored() {
        let (_f, project) = single(
            "dynamic-style-template",
            "Page({ data: { cardStyle: `background-image: ${g}` } });",
        );
        assert_eq!(messages(&DynamicStyleValue, &project), Vec::<String>::new());
    }

    /// 老式 `-webkit-box` 属性：引擎里没有这个属性名。
    #[test]
    fn legacy_flexbox_properties_are_reported() {
        let (_f, project) = single(
            "dynamic-style-legacy-property",
            r#"Page({ data: { cardStyle: '-webkit-box-align: center; -webkit-box-pack: center' } });"#,
        );
        let diagnostics = DynamicStyleValue.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("-webkit-box-align"));
        assert!(
            diagnostics[0]
                .help
                .as_deref()
                .unwrap_or_default()
                .contains("flex"),
            "应该是 flex 布局的修法"
        );
    }

    /// 标准的 flex 声明不该报。
    #[test]
    fn flex_layout_values_are_not_reported() {
        let (_f, project) = single(
            "dynamic-style-legacy-negative",
            r#"Page({ data: { cardStyle: 'display: flex; flex-direction: column; align-items: center' } });"#,
        );
        assert_eq!(messages(&DynamicStyleValue, &project), Vec::<String>::new());
    }

    #[test]
    fn unparseable_display_values_are_reported() {
        let (_f, project) = single(
            "dynamic-style-display",
            r#"Page({ data: { cardStyle: 'display: -webkit-flex; justify-content: center' } });"#,
        );
        let diagnostics = DynamicStyleValue.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("display: -webkit-flex"));
        assert!(
            diagnostics[0]
                .help
                .as_deref()
                .unwrap_or_default()
                .contains("display: flex")
        );
    }

    #[test]
    fn parseable_display_values_are_not_reported() {
        let (_f, project) = single(
            "dynamic-style-display-negative",
            r#"Page({ data: { cardStyle: 'display: inline-flex; display: block' } });"#,
        );
        assert_eq!(messages(&DynamicStyleValue, &project), Vec::<String>::new());
    }

    /// 渐进增强栈：后面还有能解析的 `display` 时，前缀写法只是兜底。
    #[test]
    fn a_later_parseable_display_wins() {
        let (_f, project) = single(
            "dynamic-style-display-stack",
            r#"Page({ data: { cardStyle: 'display: -webkit-box; display: -webkit-flex; display: flex' } });"#,
        );
        assert_eq!(messages(&DynamicStyleValue, &project), Vec::<String>::new());
    }

    #[test]
    fn line_clamp_is_reported_with_max_lines_advice() {
        let (_f, project) = single(
            "dynamic-style-line-clamp",
            r#"Page({ data: { cardStyle: '-webkit-line-clamp: 2' } });"#,
        );
        let diagnostics = DynamicStyleValue.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("-webkit-line-clamp"));
        let help = diagnostics[0].help.as_deref().unwrap_or_default();
        assert!(
            help.contains("max-lines=\"2\"") && help.contains("overflow=\"ellipsis\""),
            "{help}"
        );
    }

    /// 迁移工程用 `unset` 关掉 WebView 侧写法，不算「用了不支持的属性」。
    #[test]
    fn line_clamp_unset_is_not_reported() {
        let (_f, project) = single(
            "dynamic-style-line-clamp-unset",
            r#"Page({ data: { cardStyle: '-webkit-line-clamp: unset' } });"#,
        );
        assert_eq!(messages(&DynamicStyleValue, &project), Vec::<String>::new());
    }

    /// 字符串里内联的 HTML 片段不是声明块；拼接出来的 `display:table-` 半截判不了；
    /// `-webkit-line-clamp: 0` 在 WebView 里也不截断。
    #[test]
    fn html_fragments_and_concatenation_halves_are_ignored() {
        let (_f, project) = single(
            "dynamic-style-html-fragment",
            r#"Page({ data: {
                spanStyle: "<span style='color:#FF8000;display:inline-block'>" + text + "</span>",
                tableStyle: ";display:table-" + cell,
                clampStyle: "<span style='color:#858687CC;-webkit-line-clamp: 0'>" + text,
            } });"#,
        );
        assert_eq!(messages(&DynamicStyleValue, &project), Vec::<String>::new());
    }

    /// 按 unset 回落初始值：判据与文案都来自 `skyline/unsupported-value`。
    #[test]
    fn values_that_fall_back_to_initial_are_reported() {
        let (_f, project) = single(
            "dynamic-style-initial",
            r#"Page({ data: { cardStyle: 'position: sticky; top: 88rpx' } });"#,
        );
        let diagnostics = DynamicStyleValue.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(
            diagnostics[0]
                .message
                .contains("回落到初始值 `position: relative`"),
            "{}",
            diagnostics[0].message
        );
        assert!(
            diagnostics[0]
                .help
                .as_deref()
                .unwrap_or_default()
                .contains("sticky-section")
        );
    }

    /// 支持的取值、以及 `var()` 这类判不了的动态值都不报。
    #[test]
    fn supported_and_dynamic_values_are_not_reported() {
        let (_f, project) = single(
            "dynamic-style-initial-negative",
            r#"Page({ data: { cardStyle: 'position: relative; display: var(--d); overflow: hidden' } });"#,
        );
        assert_eq!(messages(&DynamicStyleValue, &project), Vec::<String>::new());
    }

    /// 一条 style 字符串只报一次；「整条声明被丢弃」类优先于「回落初始值」类。
    #[test]
    fn one_style_string_reports_once_with_dropped_problems_first() {
        let (_f, project) = single(
            "dynamic-style-priority",
            r#"Page({ data: {
                aStyle: 'display:-webkit-box;-webkit-box-orient:vertical;-webkit-line-clamp:2',
                bStyle: 'position: sticky; display: -webkit-box',
            } });"#,
        );
        let diagnostics = DynamicStyleValue.check(&project);
        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
        // a：命中位置最靠前的 `display`，help 把 max-lines 的修法一起说清楚。
        assert!(diagnostics[0].message.contains("display: -webkit-box"));
        assert!(
            diagnostics[0]
                .help
                .as_deref()
                .unwrap_or_default()
                .contains("max-lines")
        );
        // b：`position: sticky` 写在前面，但丢弃类优先。
        assert!(diagnostics[1].message.contains("display: -webkit-box"));
    }

    /// 门禁按内容判：名字不像 `…Style` 也照查（`cssText` / `styles` 是常见载体）。
    #[test]
    fn declaration_blocks_are_judged_by_content_not_by_name() {
        let (_f, project) = single(
            "dynamic-style-content-gate",
            r#"Page({ data: {
                cssText: 'display: -webkit-box;',
                styles: 'position: sticky; top: 0;',
            }, methods: { render() { return { caption: 'display:table-caption;text-align:center' }; } } });"#,
        );
        let diagnostics = DynamicStyleValue.check(&project);
        assert_eq!(diagnostics.len(), 3, "{diagnostics:#?}");
        assert!(diagnostics.iter().all(|d| d.rule == DynamicStyleValue::ID));
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("display: table-caption"))
        );
    }

    /// 单条声明没有分号也算（工程里很常见）；名字不像 `…Style` 也照查。
    #[test]
    fn single_declaration_strings_are_judged_by_content() {
        let (_f, project) = single(
            "dynamic-style-name-gate",
            r#"Page({ data: {
                tip: 'display: -webkit-box',
                label: '-webkit-box-orient: vertical',
            }, methods: { render() { return { style: format('display: -webkit-box') }; } } });"#,
        );
        let diagnostics = DynamicStyleValue.check(&project);
        assert_eq!(diagnostics.len(), 3, "{diagnostics:#?}");
    }

    /// 变量声明与赋值目标上的字符串同样只看内容，不看名字。
    #[test]
    fn declarator_and_assignment_contexts_are_judged() {
        let (_f, project) = single(
            "dynamic-style-contexts",
            r#"const card = 'display: -webkit-box';
            Page({ data: {}, onLoad() { this.login = 'display: -webkit-flex'; } });"#,
        );
        let diagnostics = DynamicStyleValue.check(&project);
        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("display: -webkit-box"));
        assert!(diagnostics[1].message.contains("display: -webkit-flex"));
    }

    /// 构建产物里的样式生成器不是内联样式。[`Fixture::write`] 的 `npm/` 路径。
    #[test]
    fn npm_build_output_is_not_scanned() {
        let f = Fixture::new("dynamic-style-npm");
        f.write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", "<view/>")
            .write("pages/index/index.js", "Page({});")
            .write(
                "npm/foo/index.js",
                "var cardStyle = 'display: -webkit-box;';\n",
            );
        assert!(DynamicStyleValue.check(&f.project()).is_empty());
    }

    /// `.wxs` 是另一套方言，不判。
    #[test]
    fn wxs_files_are_not_scanned() {
        let f = Fixture::new("dynamic-style-wxs");
        f.write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write(
                "pages/index/index.wxml",
                r#"<wxs module="m" src="./index.wxs"/><view/>"#,
            )
            .write("pages/index/index.js", "Page({});")
            .write(
                "pages/index/index.wxs",
                "var cardStyle = 'display: -webkit-box;';\nmodule.exports = { cardStyle: cardStyle };\n",
            );
        assert!(DynamicStyleValue.check(&f.project()).is_empty());
    }
}
