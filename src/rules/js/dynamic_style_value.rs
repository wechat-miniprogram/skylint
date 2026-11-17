//! JS 字符串里的内联样式取值：Skyline 解析不了的写法。
//!
//! 样式值不只在 WXSS 里，也常写在 JS 字符串里再通过 `style="{{xxx}}"` 绑到元素上：
//!
//! ```js
//! wxLoginStyle: 'background-image: -webkit-linear-gradient(left top, #FFD000, #FFBD00)'
//! ```
//!
//! 这类值不在任何 WXSS 文件里，`skyline/gradient-unsupported` 等规则扫不到。
//! 判据全部复用 WXSS 侧的常量与表格，不另写一份：
//! - 渐变函数名与 `skyline/gradient-unsupported`（`rules/wxss/gradient_unsupported.rs`）
//!   一致：float-pigment 只认 linear / radial / conic 三个标准名
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
//! 判不了的写法一律不报：含插值的模板串拼不出结果，属性名 / 取值结构不完整的段
//! （日志文案、URL、`url(data:image/png;base64,…)` 被 `;` 切开的半截）直接跳过。
//!
//! 入口有硬门禁——先确认这条字符串真的是内联样式，再判取值：
//! - 字面量必须挂在 `…Style` 名字下（对象属性键 / 变量名 / 赋值目标），
//!   `style`、`wxLoginStyle`、`loginActiveStyle` 都算；第三方样式生成器、
//!   HTML→富文本的映射表里的普通字符串不算。
//! - 跳过 `npm/**` 构建产物与 `.wxs`（另一套方言）。
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

use crate::project::{Project, Scope};
use crate::rules::wxss::dict_table::{normalize_value, table};
use crate::rules::wxss::is_css_wide_keyword;
use crate::rules::wxss::legacy_flexbox::{LEGACY_PROPERTIES, PARSEABLE_DISPLAY};
use crate::rules::wxss::unsupported_value::initial_value;
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

use super::at_span;

pub struct DynamicStyleValue;

/// 只存在于 WebView 的旧式渐变函数名，与 `skyline/gradient-unsupported` 的
/// `PREFIXED_GRADIENTS` 保持一致。
const PREFIXED_GRADIENTS: [&str; 4] = [
    "-webkit-linear-gradient",
    "-webkit-radial-gradient",
    "-webkit-repeating-linear-gradient",
    "-webkit-gradient",
];

const STANDARD_GRADIENTS: [&str; 3] = ["linear-gradient", "radial-gradient", "conic-gradient"];

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

/// 声明块里的一段：属性名与取值原文（均已 trim）。
struct Declaration<'a> {
    name: &'a str,
    value: &'a str,
}

impl Declaration<'_> {
    fn is(&self, property: &str) -> bool {
        self.name.eq_ignore_ascii_case(property)
    }
}

#[derive(Clone, Copy)]
enum Problem<'a> {
    /// 整条声明被丢弃：前缀渐变 / repeating 渐变 / mask 的 conic。
    Prefixed(&'a str),
    Repeating(&'a str),
    MaskConic(&'a str),
    /// 老式 `-webkit-box` 弹性盒属性：引擎里没有这个属性名。
    LegacyProperty,
    /// `display` 的取值 fp 解析不了（`-webkit-box` / `-webkit-flex` / `box` …）。
    /// `clamp` 是同一块里的 `-webkit-line-clamp` 原值，修法要一起说。
    DisplayValue {
        clamp: Option<&'a str>,
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
    decl: &'a Declaration<'a>,
    problem: Problem<'a>,
}

impl Rule for DynamicStyleValue {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "JS 字符串里的内联样式用了 Skyline 不支持的写法（前缀渐变、老式弹性盒、不可解析的 display、-webkit-line-clamp、会回落初始值的取值）",
        }
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
                // 只有「某个 `…Style` 名字的值」才可能被绑到 `style` 上。
                if !literal.context.as_deref().is_some_and(is_style_name) {
                    continue;
                }
                let block: Vec<Declaration<'_>> = declarations(&literal.value).collect();
                // 同一个字符串只报一次：诊断都落在字面量起点，重复报只是噪音。
                let Some(finding) = find(&block) else {
                    continue;
                };
                out.push(
                    at_span(
                        Self::ID,
                        Severity::Warning,
                        file,
                        literal.start,
                        literal.start,
                        message(finding.decl, finding.problem),
                    )
                    .help(help(finding.decl, finding.problem)),
                );
            }
        }
        out
    }
}

/// 一段声明里第一个要报的问题：先找「整条声明被丢弃」类，再找「回落初始值」类。
fn find<'a>(block: &'a [Declaration<'a>]) -> Option<Finding<'a>> {
    block
        .iter()
        .find_map(|decl| dropped(decl, block).map(|problem| Finding { decl, problem }))
        .or_else(|| {
            block
                .iter()
                .find_map(|decl| initial(decl).map(|problem| Finding { decl, problem }))
        })
}

/// 整条声明会被丢弃的问题：引擎不认识属性名或取值，级联里更早的值继续生效。
fn dropped<'a>(decl: &Declaration<'a>, block: &[Declaration<'a>]) -> Option<Problem<'a>> {
    if LEGACY_PROPERTIES.iter().any(|property| decl.is(property)) {
        // `unset` 是迁移工程关掉 WebView 写法的手段，不是「用了不支持的属性」。
        return (!is_css_wide_keyword(decl.value)).then_some(Problem::LegacyProperty);
    }
    if decl.is("-webkit-line-clamp") {
        // `unset` 是迁移工程关掉 WebView 写法的手段；`0` 在 WebView 里也不截断
        // （Chromium 只接受正整数），两端一致。
        if is_css_wide_keyword(decl.value) || normalized(decl.value).as_deref() == Some("0") {
            return None;
        }
        return Some(Problem::LineClamp);
    }
    if decl.is("display") {
        let value = normalized(decl.value)?;
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
                other.is("display")
                    && normalized(other.value)
                        .is_some_and(|value| PARSEABLE_DISPLAY.contains(&value.as_str()))
            });
        return (!later).then_some(Problem::DisplayValue {
            clamp: block
                .iter()
                .find(|other| other.is("-webkit-line-clamp"))
                .map(|other| other.value),
        });
    }
    if !GRADIENT_PROPERTIES.iter().any(|property| decl.is(property)) {
        return None;
    }
    gradient_problem(decl, block)
}

/// 取值不受支持、按 unset 回落到初始值的问题；判据与文案都复用 `skyline/unsupported-value`。
fn initial<'a>(decl: &Declaration<'a>) -> Option<Problem<'a>> {
    let value = normalized(decl.value)?;
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

/// 取值归一：去掉 `!important`、trim + 小写；`var()` / `env()` / `{{…}}` 这类动态值
/// 判不了，返回 `None`（与 WXSS 侧 `normalize_value` 同一口径）。
fn normalized(value: &str) -> Option<String> {
    let value = value.trim();
    let stripped = value
        .len()
        .checked_sub("!important".len())
        .filter(|index| value.is_char_boundary(*index))
        .filter(|index| value[*index..].eq_ignore_ascii_case("!important"))
        .map_or(value, |index| value[..index].trim_end());
    normalize_value(stripped)
}

/// 字面量挂的名字是否像「样式值」：`style` / `wxLoginStyle` / `loginActiveStyle`
/// （即 `/(^|[_-])?style$/i`）。
fn is_style_name(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with("style")
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

/// 把字符串当声明块解析：按 `;` 切段，第一个 `:` 分属性与取值。
fn declarations(block: &str) -> impl Iterator<Item = Declaration<'_>> {
    block.split(';').filter_map(|segment| {
        let (name, value) = segment.split_once(':')?;
        let (name, value) = (name.trim(), value.trim());
        // 字符串里内联的 HTML（`"<span style='…display:inline-block'>"`）会把引号、
        // `>` 带进取值；引号之后的部分不再属于这条声明。
        let value = value
            .find(['\'', '"'])
            .map_or(value, |index| value[..index].trim());
        // 属性名只认 CSS 标识符（`[A-Za-z-]`）：被 `;` 切开的 URL 半截、以及
        // 「登录失败: 网络错误」这类文案在这里被排除。
        let name_ok = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphabetic() || c == '-');
        // 拼接出来的半截（`";display:table-" + …`）不是完整取值，判不了。
        (name_ok && !value.is_empty() && !value.ends_with('-'))
            .then_some(Declaration { name, value })
    })
}

/// 取值里出现的函数名（`-webkit-linear-gradient(…)` → `-webkit-linear-gradient`）。
fn function_names(value: &str) -> Vec<&str> {
    value
        .match_indices('(')
        .filter_map(|(paren, _)| {
            let start = value[..paren]
                .char_indices()
                .rev()
                .take_while(|(_, c)| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
                .last()
                .map(|(index, _)| index)?;
            let name = &value[start..paren];
            name.chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '-')
                .then_some(name)
        })
        .collect()
}

fn is_standard(name: &str) -> bool {
    STANDARD_GRADIENTS
        .iter()
        .any(|gradient| name.eq_ignore_ascii_case(gradient))
}

fn is_prefixed(name: &str) -> bool {
    PREFIXED_GRADIENTS
        .iter()
        .any(|gradient| name.eq_ignore_ascii_case(gradient))
}

fn is_repeating(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.starts_with("repeating-") && name.ends_with("-gradient")
}

/// 这段声明里第一个不受支持的渐变调用。
fn gradient_problem<'a>(decl: &Declaration<'a>, block: &[Declaration<'a>]) -> Option<Problem<'a>> {
    let mask = decl.is("mask") || decl.is("mask-image") || decl.is("-webkit-mask-image");
    // 前缀写法常和标准写法成对出现（autoprefixer 产物）：同一条声明块里已经有该
    // 属性的标准渐变兜底时，两端渲染一致，不必报。
    let has_fallback = block.iter().any(|other| {
        other.is(decl.name)
            && function_names(other.value)
                .iter()
                .any(|call| is_standard(call))
    });
    function_names(decl.value).into_iter().find_map(|call| {
        if is_repeating(call) {
            Some(Problem::Repeating(call))
        } else if is_prefixed(call) {
            (!has_fallback).then_some(Problem::Prefixed(call))
        } else if mask && call.eq_ignore_ascii_case("conic-gradient") {
            Some(Problem::MaskConic(call))
        } else {
            None
        }
    })
}

fn message(decl: &Declaration<'_>, problem: Problem<'_>) -> String {
    let (name, value) = (decl.name, truncate(decl.value, 120));
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

fn help(decl: &Declaration<'_>, problem: Problem<'_>) -> String {
    match problem {
        Problem::Prefixed(call) => {
            // 两端颜色相同就等价于纯色，比换标准渐变更简单。
            let color = arguments(decl.value, call)
                .and_then(two_same_stops)
                .filter(|_| decl.is("background") || decl.is("background-image"));
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
            let value = normalized(decl.value).unwrap_or_default();
            match value.as_str() {
                "-webkit-flex" => "改用标准写法 `display: flex`".to_string(),
                "-webkit-box" => "改用 `display: flex`；配套的 `-webkit-box-*` 属性分别换成 flex-direction / align-items / justify-content / flex".to_string(),
                _ => "改用 Skyline 能解析的取值：none / block / inline / inline-block / flex / inline-flex".to_string(),
            }
        }
        Problem::LineClamp => format!(
            "多行省略改用 `<text max-lines=\"{}\" overflow=\"ellipsis\">`",
            line_count(decl.value)
        ),
        Problem::InitialValue { advice, .. } => advice.to_string(),
    }
}

/// `-webkit-line-clamp` 的行数；不是纯数字时用 `N` 占位（修法仍然成立）。
fn line_count(value: &str) -> String {
    normalized(value)
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

    use super::DynamicStyleValue;
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

    #[test]
    fn plain_text_strings_are_ignored() {
        let (_f, project) = single(
            "dynamic-style-text",
            r#"Page({ data: {
                style: '网络异常，请稍后重试',
                tip: '网络异常，请稍后重试',
                log: 'request failed: timeout',
                link: 'https://example.com/a:b',
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

    /// 字符串里内联的 HTML 片段：引号后的内容不是取值，拼接出来的 `display:table-`
    /// 半截判不了，`-webkit-line-clamp: 0` 在 WebView 里也不截断。
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

    /// 入口门禁：名字不像 `…Style` 的字符串不判——第三方样式生成器、映射表都长这样。
    #[test]
    fn strings_not_named_like_styles_are_ignored() {
        let (_f, project) = single(
            "dynamic-style-name-gate",
            r#"Page({ data: {
                tip: 'display: -webkit-box',
                caption: 'display:table-caption;text-align:center',
                label: '-webkit-box-orient: vertical',
            } });"#,
        );
        assert_eq!(messages(&DynamicStyleValue, &project), Vec::<String>::new());
    }

    /// 变量声明与赋值目标上的 `…Style` 名字同样算上下文。
    #[test]
    fn declarator_and_assignment_contexts_are_judged() {
        let (_f, project) = single(
            "dynamic-style-contexts",
            r#"const cardStyle = 'display: -webkit-box';
            Page({ data: {}, onLoad() { this.loginStyle = 'display: -webkit-flex'; } });"#,
        );
        let diagnostics = DynamicStyleValue.check(&project);
        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("display: -webkit-box"));
        assert!(diagnostics[1].message.contains("display: -webkit-flex"));
    }

    /// 名字像样式、但值是调用结果时，字符串是实参，不继承外层名字。
    #[test]
    fn style_named_call_arguments_are_ignored() {
        let (_f, project) = single(
            "dynamic-style-call-arg",
            r#"Page({ data: { style: format('display: -webkit-box') } });"#,
        );
        assert_eq!(messages(&DynamicStyleValue, &project), Vec::<String>::new());
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
                "var cardStyle = 'display: -webkit-box';\n",
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
                "var cardStyle = 'display: -webkit-box';\nmodule.exports = { cardStyle: cardStyle };\n",
            );
        assert!(DynamicStyleValue.check(&f.project()).is_empty());
    }
}
