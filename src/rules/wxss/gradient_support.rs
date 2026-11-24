//! 渐变函数的支持矩阵：属性家族 × 函数 × 子文法。
//!
//! 引擎的实现分三层，矩阵的每一行就是这三层的组合：
//! - **函数名**：float-pigment 的渐变解析只匹配 `linear-gradient` /
//!   `radial-gradient` / `conic-gradient`，`repeating-*` 与厂商前缀写法整条声明被丢弃。
//! - **属性家族**：mask 的落地只分派 linear / radial（`DecoratedBox` 没有 conic
//!   遮罩分支），background 三个都画。两族共用同一份解析，所以子文法口径相同。
//! - **子文法**：radial 只认 `circle` 形状与 `px` 尺寸；linear / radial 的停止位
//!   只收 `%` 与 `px`；conic 的停止位只收角度与 `%`。
//!
//! 后果分三档，修法完全不同，所以每条诊断里逐项标注：
//! - 整条渐变不渲染：函数不被解析、mask 没有 conic、radial 形状不是 circle、
//!   conic 停止位里混进长度；
//! - 只丢这一个停止位：linear / radial 的停止位单位不对，其余部分照常绘制；
//! - 半径按 0 绘制、渐变退化成单色：radial 尺寸不是 px。
//!
//! 一条声明只出一条诊断，message 里列出该声明里**所有**不受支持的点。

use crate::input::wxss::{ValueToken, WxssDeclaration, WxssRule};
use crate::project::Project;
use crate::rules::{Rule, RuleMeta, VisualCase, VisualExpect};
use crate::{Diagnostic, Severity};

use super::value_util::{
    Call, at_value, is_dynamic, is_ident, length_unit, split_tokens, split_top_level, token_text,
    top_level_calls,
};
use super::visit_declarations;

pub struct GradientSupport;

impl GradientSupport {
    pub const ID: &'static str = "skyline/gradient-support";
}

/// 属性家族：引擎按 image 读取的两族属性，决定 conic 有没有渲染分支。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Surface {
    Background,
    Mask,
}

/// 属性名归一到家族根：去掉厂商前缀与 `-image` 长手名后缀。
///
/// `mask-*` 一族在引擎里都走 `SetMaskImage`，所以按族根匹配而不是枚举属性名——
/// `-webkit-mask-image` 这类前缀变体天然覆盖。
fn surface_of(property: &str) -> Option<Surface> {
    let name = property.trim().to_ascii_lowercase();
    let name = match name.strip_prefix('-').and_then(|rest| rest.split_once('-')) {
        Some((_, rest)) => rest.to_string(),
        None => name,
    };
    match name.strip_suffix("-image").unwrap_or(&name) {
        "background" => Some(Surface::Background),
        "mask" => Some(Surface::Mask),
        _ => None,
    }
}

/// 颜色停止位置的文法：两族的引擎分支不同。
#[derive(Clone, Copy)]
enum StopGrammar {
    /// 只把 `%`（Ratio）与 `px`（Px）收进停止点，其它单位只丢这一个停止位。
    PercentOrPx,
    /// conic 的停止位是「角度或百分比」，长度进不了这个文法，整条渐变解析失败。
    AngleOrPercent,
}

impl StopGrammar {
    fn allows(self, unit: &str) -> bool {
        match self {
            Self::PercentOrPx => matches!(unit, "%" | "px"),
            Self::AngleOrPercent => matches!(unit, "%" | "deg" | "rad" | "grad" | "turn"),
        }
    }

    fn impact(self) -> Impact {
        match self {
            Self::PercentOrPx => Impact::ItemDropped,
            Self::AngleOrPercent => Impact::Dropped,
        }
    }

    fn expectation(self) -> &'static str {
        match self {
            Self::PercentOrPx => "% 或 px",
            Self::AngleOrPercent => "角度或 %",
        }
    }
}

/// 矩阵的一行：一个被实现的渐变函数。
struct Gradient {
    /// 标准函数名；只有这三个名字能进 float-pigment 的渐变解析。
    name: &'static str,
    /// 引擎实现了该函数的属性家族；不在其中的家族整条不设置。
    surfaces: &'static [Surface],
    /// 形状关键字白名单；`None` 表示该函数没有形状维度。
    shapes: Option<&'static [&'static str]>,
    /// 显式尺寸允许的单位；空表示该函数没有尺寸维度。
    size_units: &'static [&'static str],
    stops: StopGrammar,
}

const MATRIX: [Gradient; 3] = [
    Gradient {
        name: "linear-gradient",
        surfaces: &[Surface::Background, Surface::Mask],
        shapes: None,
        size_units: &[],
        stops: StopGrammar::PercentOrPx,
    },
    Gradient {
        name: "radial-gradient",
        surfaces: &[Surface::Background, Surface::Mask],
        shapes: Some(&["circle"]),
        size_units: &["px"],
        stops: StopGrammar::PercentOrPx,
    },
    Gradient {
        // mask 的取值分派只处理 linear / radial，conic 静默不设置。
        name: "conic-gradient",
        surfaces: &[Surface::Background],
        shapes: None,
        size_units: &[],
        stops: StopGrammar::AngleOrPercent,
    },
];

/// 一个不受支持的点，以及它的渲染后果。
struct Problem {
    what: String,
    impact: Impact,
}

#[derive(Clone, Copy)]
enum Impact {
    /// 这一层渐变整条不渲染。
    Dropped,
    /// 只丢这一个子项，其余部分照常绘制。
    ItemDropped,
    /// 渐变还画，但尺寸退化成 0 半径。
    Degraded,
}

impl Impact {
    fn phrase(self, surface: Surface) -> &'static str {
        match (self, surface) {
            (Self::Dropped, Surface::Background) => "整条渐变不渲染，这一层背景没有图像",
            (Self::Dropped, Surface::Mask) => "整条遮罩不生效",
            (Self::ItemDropped, _) => "只丢这一个停止位，其余部分照常绘制",
            (Self::Degraded, _) => "半径按 0 绘制，渐变退化成单色",
        }
    }
}

const HELP: &str = "radial-gradient 只实现 circle 形状与 px 尺寸；linear / radial 的停止位只取 % 与 px；conic 的停止位只取角度与 %；repeating 与厂商前缀写法都不被解析。整条不渲染的项要么改成受支持的写法，要么换图片实现";

/// 视觉用例：矩阵的三类后果各一条。
///
/// - 省略形状的 `radial-gradient` 默认 ellipse，Skyline 只认 circle，整张图不渲染。
/// - `linear-gradient` 的 `em` 停止位置被丢弃，颜色过渡的起点从 0 开始。
/// - `repeating-*` 不在解析范围里，整条 background-image 丢失。
static VISUAL_CASES: &[VisualCase] = &[
    VisualCase {
        wxml: r#"<view class="spot" id="target"></view>"#,
        wxss: ".spot { width: 240px; height: 160px; background-color: #ffffff; background-image: radial-gradient(#ff3b30, #007aff); }\n",
        js: None,
        expect: VisualExpect::Differ,
        note: "`radial-gradient` 省略形状即默认 ellipse，Skyline 只认 circle、整张图不渲染：WebView 是红蓝径向渐变，Skyline 只剩白底",
        target: "#target",
    },
    VisualCase {
        wxml: r#"<view class="bar" id="target"></view>"#,
        wxss: ".bar { width: 240px; height: 160px; background-color: #ffffff; background-image: linear-gradient(90deg, #ff3b30 10em, #007aff 100%); }\n",
        js: None,
        expect: VisualExpect::Differ,
        note: "`linear-gradient` 的 `10em` 停止位置在 Skyline 被丢弃：WebView 前 160px 是纯红，Skyline 从 0 就开始向蓝色过渡",
        target: "#target",
    },
    VisualCase {
        wxml: r#"<view class="stripes" id="target"></view>"#,
        wxss: ".stripes { width: 200px; height: 120px; background-image: repeating-linear-gradient(45deg, #e53935 0 10px, #1e88e5 10px 20px); }\n",
        js: None,
        expect: VisualExpect::Differ,
        note: "repeating-linear-gradient：WebView 画出红蓝斜条纹，Skyline 不解析 repeating 渐变，整条 background-image 丢失；该元素没有 background-color 兜底，所以 Skyline 侧整张截图完全透明（alpha=0），连盒子都不可见。同组的 `-2` 证明该渲染器能画渐变，这里的空白不是采集失败",
        target: "#target",
    },
];

impl Rule for GradientSupport {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "渐变函数的支持矩阵：函数名、属性家族与形状 / 尺寸 / 停止位子文法在 Skyline 下只实现了一部分",
        }
    }

    fn visual_cases(&self) -> &'static [VisualCase] {
        VISUAL_CASES
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit_declarations(project, |sheet, rule, decl, out| {
            let doc = &sheet.doc;
            if decl.value.trim().is_empty() || is_dynamic(&decl.tokens) {
                return;
            }
            let Some(surface) = surface_of(&decl.name) else {
                return;
            };
            let mut problems = Vec::new();
            let fallback = has_standard_fallback(rule, decl, surface);
            for call in top_level_calls(&decl.tokens) {
                problems.extend(call_problems(&call, surface, fallback));
            }
            if problems.is_empty() {
                return;
            }
            out.push(at_value(
                Self::ID,
                doc,
                decl,
                message(&decl.name, &problems, surface),
                HELP,
            ));
        })
    }
}

/// 引擎能解析的渐变函数名（[`MATRIX`] 的 name 列）。
///
/// 内联样式入口（`skyline/dynamic-style-value`）复用这份表，不再自带一份名单。
pub(crate) fn parses(name: &str) -> bool {
    MATRIX.iter().any(|row| row.name.eq_ignore_ascii_case(name))
}

fn message(property: &str, problems: &[Problem], surface: Surface) -> String {
    let mut listed: Vec<String> = Vec::new();
    for problem in problems {
        let item = format!("{}（{}）", problem.what, problem.impact.phrase(surface));
        if !listed.contains(&item) {
            listed.push(item);
        }
    }
    format!(
        "`{property}` 的渐变在 Skyline 下不完全支持：{}",
        listed.join("；")
    )
}

/// 单个渐变函数调用上的全部不受支持点。
fn call_problems(call: &Call<'_>, surface: Surface, prefixed_has_fallback: bool) -> Vec<Problem> {
    let Some(row) = MATRIX.iter().find(|row| row.name == call.name) else {
        return name_problems(call, prefixed_has_fallback);
    };
    if !row.surfaces.contains(&surface) {
        return vec![Problem {
            what: format!(
                "mask 的取值分派只处理 linear / radial，`{}()` 不会被设置",
                call.name
            ),
            impact: Impact::Dropped,
        }];
    }
    let parts = split_top_level(call.args);
    let first = parts.first().copied();
    let mut problems = Vec::new();
    let prelude = row.shapes.and_then(|_| radial_prelude(first));
    if let Some(shapes) = row.shapes {
        problems.extend(shape_problems(call, shapes, prelude));
        if let Some(prelude) = prelude {
            problems.extend(size_problems(call, row.size_units, prelude));
        }
    }
    // 前言不是颜色停止：形状 / 尺寸 / 方向段都不参与停止位检查。
    let skip_first = prelude.is_some()
        || first.is_some_and(|first| is_linear_direction(first) || is_conic_prelude(first));
    let stops = if skip_first { &parts[1..] } else { &parts[..] };
    problems.extend(stop_problems(call, row.stops, stops));
    problems
}

/// 不在矩阵里的函数名：repeating 与厂商前缀写法整条声明被丢弃。
fn name_problems(call: &Call<'_>, prefixed_has_fallback: bool) -> Vec<Problem> {
    let name = call.name.as_str();
    if !name.ends_with("-gradient") {
        return Vec::new();
    }
    // 前缀写法常和标准写法成对出现（autoprefixer 产物）：同一属性已有能解析的
    // 标准渐变兜底时两端一致，不必报。
    if name.starts_with('-') {
        return if prefixed_has_fallback {
            Vec::new()
        } else {
            vec![Problem {
                what: format!("`{name}()` 是只有 WebView 认的厂商前缀写法"),
                impact: Impact::Dropped,
            }]
        };
    }
    if name.starts_with("repeating-") {
        return vec![Problem {
            what: format!("`{name}()` 的 repeating 写法不在引擎的渐变解析里"),
            impact: Impact::Dropped,
        }];
    }
    vec![Problem {
        what: format!("`{name}()` 不是引擎实现的渐变函数"),
        impact: Impact::Dropped,
    }]
}

/// 同一属性已有能解析的标准渐变：返回 true 时前缀写法不报。
///
/// 「看起来像标准」不等于能解析：`linear-gradient(left top, …)` 缺 `to`，是旧
/// WebKit 语法，标准语法里非法，两个引擎都会丢掉这条声明——不能当兜底。
fn has_standard_fallback(rule: &WxssRule, decl: &WxssDeclaration, surface: Surface) -> bool {
    rule.declarations.iter().any(|other| {
        other.name.eq_ignore_ascii_case(&decl.name)
            && top_level_calls(&other.tokens).iter().any(|call| {
                MATRIX
                    .iter()
                    .find(|row| row.name == call.name)
                    .is_some_and(|row| {
                        row.surfaces.contains(&surface) && parses_as_standard_gradient(call)
                    })
            })
    })
}

/// 标准渐变的第一个参数不能是裸方向词：`left top` / `top` 属旧 WebKit 语法，
/// 标准写法要带 `to`（`to left top`），否则整条声明在两个引擎里都被丢弃。
fn parses_as_standard_gradient(call: &Call<'_>) -> bool {
    match call.args.first() {
        Some(ValueToken::Ident(word)) => !matches!(
            word.to_ascii_lowercase().as_str(),
            "left" | "right" | "top" | "bottom"
        ),
        _ => true,
    }
}

/// 形状维度：矩阵只白名单了 circle，引擎的 default 分支直接丢掉整张图。
fn shape_problems(
    call: &Call<'_>,
    shapes: &'static [&'static str],
    prelude: Option<&[ValueToken]>,
) -> Vec<Problem> {
    let shape = prelude.and_then(|prelude| {
        split_tokens(prelude).into_iter().find_map(|group| {
            let token = group.first()?;
            (shapes.iter().any(|allowed| is_ident(token, allowed)) || is_ident(token, "ellipse"))
                .then_some(token)
        })
    });
    match shape {
        Some(token) if shapes.iter().any(|allowed| is_ident(token, allowed)) => Vec::new(),
        Some(token) => vec![Problem {
            what: format!(
                "`{}()` 用了 `{}` 形状，Skyline 只实现 {}",
                call.name,
                token_text(token),
                shapes.join(" / ")
            ),
            impact: Impact::Dropped,
        }],
        None => vec![Problem {
            what: format!(
                "`{}()` 省略了形状（默认 ellipse），Skyline 只实现 {}",
                call.name,
                shapes.join(" / ")
            ),
            impact: Impact::Dropped,
        }],
    }
}

/// 尺寸维度：引擎只把 `px` 赋给半径，其它单位让半径停在 0。
fn size_problems(
    call: &Call<'_>,
    size_units: &'static [&'static str],
    prelude: &[ValueToken],
) -> Vec<Problem> {
    let Some(size) = radial_size(prelude) else {
        return Vec::new();
    };
    let Some(unit) = length_unit(size) else {
        return Vec::new();
    };
    if size_units.contains(&unit.as_str()) {
        return Vec::new();
    }
    vec![Problem {
        what: format!(
            "`{}()` 的尺寸 `{}` 只收 {}",
            call.name,
            token_text(size),
            size_units.join(" / ")
        ),
        impact: Impact::Degraded,
    }]
}

fn stop_problems(call: &Call<'_>, grammar: StopGrammar, stops: &[&[ValueToken]]) -> Vec<Problem> {
    let mut problems = Vec::new();
    for stop in stops {
        for token in split_tokens(stop)
            .into_iter()
            .filter_map(|group| group.first())
        {
            let Some(unit) = length_unit(token) else {
                continue;
            };
            if grammar.allows(&unit) {
                continue;
            }
            problems.push(Problem {
                what: format!(
                    "`{}()` 的停止位 `{}` 只收 {}",
                    call.name,
                    token_text(token),
                    grammar.expectation()
                ),
                impact: grammar.impact(),
            });
        }
    }
    problems
}

/// 第一段是否是形状 / 尺寸 / 位置前言，而不是第一个颜色停止。
///
/// 颜色停止必须以颜色开头，所以「第一个 token 是长度 / 百分比」的段一定是尺寸
/// （`radial-gradient(10em, red, blue)`），归入前言——否则会被当成停止位置误判。
fn radial_prelude(first: Option<&[ValueToken]>) -> Option<&[ValueToken]> {
    const KEYWORDS: [&str; 7] = [
        "circle",
        "ellipse",
        "at",
        "closest-side",
        "farthest-side",
        "closest-corner",
        "farthest-corner",
    ];
    let first = first?;
    let is_prelude = split_tokens(first).iter().any(|group| {
        group.first().is_some_and(|token| {
            KEYWORDS.iter().any(|keyword| is_ident(token, keyword))
                || matches!(
                    token,
                    ValueToken::Dimension { .. } | ValueToken::Percentage(_)
                )
        })
    });
    is_prelude.then_some(first)
}

/// 前言里的尺寸：形状关键字之后、`at` 之前的第一个 token。
fn radial_size(prelude: &[ValueToken]) -> Option<&ValueToken> {
    let mut groups = split_tokens(prelude);
    if groups
        .first()
        .and_then(|group| group.first())
        .is_some_and(|token| is_ident(token, "circle") || is_ident(token, "ellipse"))
    {
        groups.remove(0);
    }
    if let Some(at) = groups
        .iter()
        .position(|group| group.first().is_some_and(|token| is_ident(token, "at")))
    {
        groups.truncate(at);
    }
    groups.first().and_then(|group| group.first())
}

fn is_linear_direction(first: &[ValueToken]) -> bool {
    let tokens: Vec<&ValueToken> = split_tokens(first)
        .into_iter()
        .filter_map(|group| group.first())
        .collect();
    match tokens.first() {
        Some(token) if is_ident(token, "to") => tokens.len() > 1,
        Some(ValueToken::Dimension { unit, .. }) => matches!(
            unit.to_ascii_lowercase().as_str(),
            "deg" | "rad" | "grad" | "turn"
        ),
        _ => false,
    }
}

/// conic 的前言是 `from <angle>` / `at <position>`，第一个 token 一定是关键字。
fn is_conic_prelude(first: &[ValueToken]) -> bool {
    first
        .first()
        .is_some_and(|token| is_ident(token, "from") || is_ident(token, "at"))
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;

    use super::GradientSupport;
    use crate::rules::testutil::wxss::{messages, page};

    /// 视觉用例与单测共用同一份 WXML/WXSS：每条都必须被本规则命中。
    #[test]
    fn visual_cases_are_reported() {
        for case in super::VISUAL_CASES {
            let fixture = crate::rules::testutil::Fixture::page(
                "gradient-support-visual",
                case.wxml,
                case.wxss,
            );
            let hits = crate::rules::testutil::check_rule(&fixture, &GradientSupport)
                .iter()
                .filter(|diagnostic| diagnostic.rule == GradientSupport::ID)
                .count();
            assert!(hits >= 1, "{}: {}", case.note, case.wxml);
        }
    }

    #[test]
    fn reports_shape_size_and_stop_units() {
        let (_f, project) = page(
            "gradient-positive",
            r#"<view class="a"/><view class="b"/><view class="c"/><view class="d"/><view class="e"/>"#,
            r#"
            .a { background-image: radial-gradient(ellipse, red 0%, blue 100%); }
            .b { background-image: radial-gradient(circle 10em, red 0%, blue 100%); }
            .c { background-image: radial-gradient(circle, red 0rpx, blue 100%); }
            .d { background-image: linear-gradient(red 10em, blue 100%); }
            .e { background-image: radial-gradient(circle, red 0em, blue 100%); }
            "#,
        );
        let diags = GradientSupport.check(&project);
        assert_eq!(diags.len(), 5);
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Warning));
        assert!(diags.iter().any(|d| d.message.contains("ellipse")));
        assert!(diags.iter().any(|d| d.message.contains("`10em`")));
        assert!(diags.iter().any(|d| d.message.contains("`0rpx`")));
        assert!(diags.iter().any(|d| d.message.contains("`0em`")));
        // 尺寸不是 px 只把半径归零，停止位单位不对只丢停止位：两档后果要分开。
        assert!(diags[1].message.contains("半径按 0 绘制"));
        assert!(diags[3].message.contains("只丢这一个停止位"));
    }

    /// 长度型前言（没有形状关键字）不能被当成颜色停止；省略形状本身也要报。
    #[test]
    fn radial_gradient_with_length_only_prelude_is_a_size_not_a_stop() {
        let (_f, project) = page(
            "gradient-length-prelude",
            r#"<view class="a"/><view class="b"/>"#,
            r#"
            .a { background-image: radial-gradient(10em, red 0%, blue 100%); }
            .b { background-image: radial-gradient(10px, red 0%, blue 100%); }
            "#,
        );
        let diags = GradientSupport.check(&project);
        assert_eq!(diags.len(), 2, "{diags:#?}");
        // `10em` 归在「尺寸」而不是「停止位」。
        assert!(diags[0].message.contains("的尺寸 `10em`"));
        assert!(!diags[0].message.contains("停止位"));
        assert!(diags[0].message.contains("circle"));
        assert!(diags[1].message.contains("circle"));
    }

    /// `rpx` 停止位不被引擎接受，mask 与 `-webkit-mask-image` 也要查。
    #[test]
    fn rpx_stops_and_mask_gradients_are_reported() {
        let (_f, project) = page(
            "gradient-rpx",
            r#"<view class="a"/><view class="b"/><view class="c"/>"#,
            r#"
            .a { background-image: linear-gradient(red 10rpx, blue 100%); }
            .b { mask-image: linear-gradient(red 10em, blue 100%); }
            .c { -webkit-mask-image: radial-gradient(circle, red 10rpx, blue 100%); }
            "#,
        );
        let diags = GradientSupport.check(&project);
        assert_eq!(diags.len(), 3, "{diags:#?}");
        assert!(diags[0].message.contains("`10rpx`"));
        assert!(diags[1].message.contains("`10em`"));
        assert!(diags[2].message.contains("`10rpx`"));
        assert!(diags.iter().all(|d| d.message.contains("只丢这一个停止位")));
    }

    /// 属性家族按归一后的族根匹配：`-webkit-mask-image` 上的前缀渐变也要覆盖。
    #[test]
    fn prefixed_gradient_on_webkit_mask_image_is_reported() {
        let (_f, project) = page(
            "gradient-webkit-mask",
            r#"<view class="a"/><view class="b"/>"#,
            r#"
            .a { -webkit-mask-image: -webkit-linear-gradient(bottom, #fff, transparent); }
            .b { -webkit-mask: -webkit-radial-gradient(circle, #fff, transparent); }
            "#,
        );
        let diags = GradientSupport.check(&project);
        assert_eq!(diags.len(), 2, "{diags:#?}");
        assert!(diags[0].message.contains("-webkit-mask-image"));
        assert!(diags[0].message.contains("-webkit-linear-gradient"));
        assert!(diags[0].message.contains("整条遮罩不生效"));
        assert!(diags[1].message.contains("-webkit-radial-gradient"));
    }

    /// 同一条声明里的多个不受支持点合成一条诊断，message 里逐项列出。
    #[test]
    fn one_declaration_yields_one_diagnostic_listing_every_problem() {
        let (_f, project) = page(
            "gradient-two-problems",
            r#"<view class="a"/>"#,
            r#"
            .a { background-image: radial-gradient(#ff3b30, #007aff), linear-gradient(red 10em, blue 100%); }
            "#,
        );
        let diags = GradientSupport.check(&project);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        let message = &diags[0].message;
        assert!(message.contains("省略了形状"), "{message}");
        assert!(message.contains("整条渐变不渲染"), "{message}");
        assert!(message.contains("`10em`"), "{message}");
        assert!(message.contains("只丢这一个停止位"), "{message}");
    }

    /// 省略形状让整张图不渲染，`10em` 只丢停止位：两种后果不能混成一句。
    #[test]
    fn shape_omission_drops_the_whole_gradient_while_em_only_drops_the_stop() {
        let (_f, project) = page(
            "gradient-consequence",
            r#"<view class="a"/><view class="b"/>"#,
            r#"
            .a { background-image: radial-gradient(#ff3b30, #007aff); }
            .b { background-image: linear-gradient(90deg, #ff3b30 10em, #007aff 100%); }
            "#,
        );
        let diags = GradientSupport.check(&project);
        assert_eq!(diags.len(), 2, "{diags:#?}");
        assert!(diags[0].message.contains("整条渐变不渲染"));
        assert!(!diags[0].message.contains("只丢这一个停止位"));
        assert!(diags[1].message.contains("只丢这一个停止位"));
        assert!(!diags[1].message.contains("整条渐变不渲染"));
    }

    /// conic 的停止位只收角度与 `%`：长度让整条渐变解析失败（mask 也不画 conic）。
    #[test]
    fn conic_stop_length_drops_the_whole_gradient_and_mask_conic_is_not_implemented() {
        let (_f, project) = page(
            "gradient-conic",
            r#"<view class="a"/><view class="b"/><view class="c"/>"#,
            r#"
            .a { background-image: conic-gradient(red 10px, blue 100%); }
            .b { mask-image: conic-gradient(red, blue); }
            .c { background-image: conic-gradient(from 45deg, red 0deg, blue 90deg); }
            "#,
        );
        let diags = GradientSupport.check(&project);
        assert_eq!(diags.len(), 2, "{diags:#?}");
        assert!(diags[0].message.contains("`10px`"));
        assert!(diags[0].message.contains("整条渐变不渲染"));
        assert!(diags[1].message.contains("conic-gradient"));
        assert!(diags[1].message.contains("整条遮罩不生效"));
    }

    #[test]
    fn prefixed_gradient_is_reported_unless_a_standard_fallback_exists() {
        let (_f, project) = page(
            "gradient-prefixed",
            r#"<view class="a"/><view class="b"/>"#,
            r#"
            .a { background-image: -webkit-linear-gradient(bottom, #fff, transparent); }
            .b { background-image: -webkit-linear-gradient(bottom, #fff, transparent);
                 background-image: linear-gradient(to top, #fff, transparent); }
            "#,
        );
        let diags = GradientSupport.check(&project);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert!(diags[0].message.contains("-webkit-linear-gradient"));
    }

    #[test]
    fn repeating_and_mask_conic_gradients_are_reported() {
        let (_f, project) = page(
            "gradient-unsupported-positive",
            r#"<view class="a"/><view class="b"/><view class="c"/><view class="d"/>"#,
            r#"
            .a { background-image: repeating-linear-gradient(red 0 10px, blue 10px 20px); }
            .b { background: repeating-radial-gradient(circle, red 0 10px, blue 10px 20px); }
            .c { mask-image: conic-gradient(red, blue); }
            .d { mask: conic-gradient(red, blue); }
            "#,
        );
        let diags = GradientSupport.check(&project);
        assert_eq!(diags.len(), 4, "{diags:#?}");
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Warning));
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("repeating-linear-gradient"))
        );
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("repeating-radial-gradient"))
        );
        assert!(diags.iter().any(|d| d.message.contains("conic-gradient")));
    }

    /// `linear-gradient(left top, …)` 缺 `to`，是旧 WebKit 语法，两个引擎都会丢掉它。
    /// 它不算「标准兜底」，否则同一条规则里的前缀写法会被消掉。
    #[test]
    fn a_legacy_direction_standard_gradient_is_not_a_fallback() {
        let (_f, project) = page(
            "gradient-legacy-direction",
            r#"<view class="a"/><view class="b"/><view class="c"/><view class="d"/>"#,
            r#"
            .a { background-image: linear-gradient(left top, #ffd000, #ffbd00);
                 background-image: -webkit-linear-gradient(left top, #ffd000, #ffbd00); }
            .b { background-image: linear-gradient(to left top, #ffd000, #ffbd00);
                 background-image: -webkit-linear-gradient(left top, #ffd000, #ffbd00); }
            .c { background-image: linear-gradient(45deg, #ffd000, #ffbd00);
                 background-image: -webkit-linear-gradient(left top, #ffd000, #ffbd00); }
            .d { background-image: linear-gradient(#ffd000, #ffbd00);
                 background-image: -webkit-linear-gradient(left top, #ffd000, #ffbd00); }
            "#,
        );
        let diags = GradientSupport.check(&project);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert!(diags[0].message.contains("-webkit-linear-gradient"));
    }

    #[test]
    fn supported_gradients_and_vars_are_accepted() {
        let (_f, project) = page(
            "gradient-negative",
            r#"<view class="a"/><view class="b"/><view class="c"/><view class="d"/><view class="f"/><view class="g"/><view class="h"/><view class="i"/><view class="j"/>"#,
            r#"
            .a { background-image: radial-gradient(circle 50px at 10% 20%, red 0%, blue 100%); }
            .b { background-image: linear-gradient(to right, red 0%, blue 100px); }
            .c { background-image: conic-gradient(from 45deg, red 0%, blue 100%); }
            .d { background-image: radial-gradient(circle, red var(--stop), blue 100%); }
            .f { background-image: linear-gradient(to right, red 0%, rgba(0, 0, 0, .5) 50%); }
            .g { background-image: radial-gradient(circle, red 0px, blue 30px); }
            .h { mask-image: linear-gradient(red, blue); }
            .i { -webkit-mask-image: radial-gradient(circle, red 0%, blue 100%); }
            .j { background-image: conic-gradient(red 0deg, blue 90deg); }
            "#,
        );
        assert_eq!(messages(&GradientSupport, &project), Vec::<String>::new());
    }

    /// WXML 静态 `style` 属性里的前缀渐变同样要查，诊断落在属性值上。
    #[test]
    fn inline_style_attribute_gradient_is_reported() {
        let fixture = crate::rules::testutil::Fixture::page(
            "gradient-inline-style",
            r##"<view class="a" style="background: -webkit-linear-gradient(#fff, #000)"></view>"##,
            ".a { width: 100px }\n",
        );
        let diagnostics = GradientSupport.check(&fixture.project());
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("-webkit-linear-gradient"));
        assert_eq!(diagnostics[0].path, "pages/index/index.wxml");
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (1, 36));
    }
}
