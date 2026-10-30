//! Skyline 组件属性值白名单：写了但会被静默忽略的取值，以及跨属性的生效条件。
//!
//! 约束以本文件的常量定义，来源：
//! - /Volumes/2tb-ssd/nebula/Work/skyline-skills/skills/skyline-components/references/{media,form,layout,special,scroll}/*.md
//! - /Volumes/2tb-ssd/nebula/Work/skyline-skills/skills/skyline-overview/references/introduction/component-support.md
//! - /Volumes/2tb-ssd/nebula/Work/skyline/package/skyline/lib/widgets/{curve,hero}.dart（缓动名全集）
//!
//! 同步方式：只收录文档明确写「Skyline 不支持 / 仅 WebView 支持 / 取值范围」的属性取值；
//!   枚举值以文档表格为准，条目行尾注释写 references 相对路径与行号。组件不能拆成多条规则。
//!
//! 静态值直接判；动态绑定先按表达式推导候选集（`WxmlAttribute::candidates`），
//! 只有**所有候选都非法**（或 `[requires]` 里所有候选都命中触发条件）时才报——
//! 「无论运行时走哪条分支都错」这个结论不依赖运行时数据，因此不会误报。
//! 候选枚举不出来（取值来自数据字段、成员访问、函数调用）时一律跳过。
//! 无值属性视为布尔 true，足以确认 `circular=*` 这类触发条件。
//! 已知未覆盖：textarea 的 placeholder-class 同为 WebView-only，但知识库无 textarea
//!   证据文件，按不确定不报处理。

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::diag::{Diagnostic, Severity};
use crate::input::wxml::{WxmlAttribute, WxmlElement};
use crate::matcher::Tree;
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};

use super::structure::find_attr;
use super::{span_diagnostic, visit, walk_skyline};

#[derive(Debug, Clone, Copy)]
enum Constraint {
    /// 合法值白名单；值以 `*` 结尾表示前缀匹配。
    Enum(&'static [&'static str]),
    /// 只在 WebView 下生效的值：命中即报。非命中值交给同 key 的 `Enum`，
    /// 这样裁剪模式才能拿到专门的 help。
    WebviewOnly(&'static [&'static str]),
    /// 属性本身不支持：出现即报。
    Unsupported,
    /// 数值范围。
    Number(NumberRange),
}

#[derive(Debug, Clone, Copy)]
enum NumberRange {
    /// 非负整数（≥ 0）。
    NonNegative,
    /// 闭区间。
    Between(f64, f64),
}

impl Constraint {
    fn rejects(&self, value: &str) -> bool {
        match self {
            Self::Enum(values) => !contains(values, value),
            Self::WebviewOnly(values) => contains(values, value),
            Self::Unsupported => true,
            Self::Number(NumberRange::NonNegative) => !value
                .parse::<f64>()
                .is_ok_and(|number| number >= 0.0 && number.fract() == 0.0),
            Self::Number(NumberRange::Between(min, max)) => !value
                .parse::<f64>()
                .is_ok_and(|number| number >= *min && number <= *max),
        }
    }
}

fn contains(values: &[&'static str], value: &str) -> bool {
    values.iter().any(|&item| match item.strip_suffix('*') {
        Some(prefix) => value
            .get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix)),
        None => item.eq_ignore_ascii_case(value),
    })
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    constraint: Constraint,
    help: Option<&'static str>,
}

/// 一行属性约束：`tags` / `attrs` 可写多个，共用同一约束。
struct AttributeRule {
    tags: &'static [&'static str],
    attrs: &'static [&'static str],
    constraint: Constraint,
    help: Option<&'static str>,
}

/// 跨属性生效条件：触发属性命中时，要求属性必须满足约束。
#[derive(Debug, Clone, Copy)]
struct Requires {
    tag: &'static str,
    trigger: Trigger,
    requirement: Requirement,
    help: Option<&'static str>,
}

#[derive(Debug, Clone, Copy)]
enum Trigger {
    /// `attr=*`：属性存在且取值非空；无值属性（布尔 true）同样触发。
    Present(&'static str),
    /// `attr=v` 精确匹配。
    Equals(&'static str, &'static str),
    NotAny(&'static str, &'static [&'static str]),
}

#[derive(Debug, Clone, Copy)]
enum Requirement {
    /// `attr=value`：属性必须存在且等于该值。
    Equals(&'static str, &'static str),
    /// `attr`：属性必须存在。
    Present(&'static str),
    /// `attr=!(a|b)`：属性不得等于其中任一值。
    NotAny(&'static str, &'static [&'static str]),
}

impl Trigger {
    fn attr(&self) -> &'static str {
        match self {
            Self::Present(attr) | Self::Equals(attr, _) | Self::NotAny(attr, _) => attr,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct Table {
    entries: HashMap<&'static str, HashMap<&'static str, Vec<Entry>>>,
    requires: HashMap<&'static str, Vec<Requires>>,
}

impl Table {
    fn lookup(&self, tag: &str, attr: &str) -> Option<&[Entry]> {
        let attrs = self.entries.get(tag)?;
        if let Some(entries) = attrs.get(attr) {
            return Some(entries);
        }
        // 属性名以 `*` 结尾表示前缀匹配（如 safe-password-*）；精确名优先。
        attrs
            .iter()
            .filter_map(|(name, entries)| {
                let prefix = name.strip_suffix('*')?;
                attr.starts_with(prefix).then_some((prefix.len(), entries))
            })
            .max_by_key(|(len, _)| *len)
            .map(|(_, entries)| entries.as_slice())
    }
}

/// WebView 的 `<image mode>` 裁剪取值：Skyline 不支持它们，但两个渲染器都认。
const IMAGE_CROP_MODES: &[&str] = &[
    "top",
    "bottom",
    "center",
    "left",
    "right",
    "top left",
    "top right",
    "bottom left",
    "bottom right",
];

const ATTRIBUTES: &[AttributeRule] = &[
    AttributeRule {
        tags: &["image"],
        attrs: &["mode"],
        constraint: Constraint::WebviewOnly(IMAGE_CROP_MODES),
        help: Some("Skyline 不支持 WebView 的裁剪模式，改用 aspectFill 配合 CSS 定位"),
    }, // media/image.md:42-54
    AttributeRule {
        tags: &["text"],
        attrs: &["overflow"],
        constraint: Constraint::Enum(&["visible", "clip", "fade", "ellipsis", "unset"]),
        help: None,
    }, // media/text.md:50-55；text_shadow_node.cc:280-281 有 unset 分支
    AttributeRule {
        tags: &["image"],
        attrs: &["mode"],
        constraint: Constraint::Enum(&[
            "scaleToFill",
            "aspectFit",
            "aspectFill",
            "widthFix",
            "heightFix",
            "auto",
        ]),
        help: Some("合法值：scaleToFill、aspectFit、aspectFill、widthFix、heightFix、auto"),
    }, // media/image.md:32-38；image_shadow_node.cc:99-101 有 auto
    AttributeRule {
        tags: &["input"],
        attrs: &["type"],
        constraint: Constraint::Enum(&["text", "number", "idcard", "digit", "nickname"]),
        help: Some("合法值：text、number、idcard、digit、nickname"),
    }, // form/input.md:44-50
    AttributeRule {
        tags: &["input"],
        attrs: &["confirm-type"],
        constraint: Constraint::Enum(&["send", "search", "next", "go", "done", "return"]),
        help: Some("合法值：send、search、next、go、done、return"),
    }, // form/input.md:54-60；input.dart:1121-1122 映射 return
    AttributeRule {
        tags: &["swiper"],
        attrs: &["layout-type"],
        constraint: Constraint::Enum(&[
            "normal",
            "stackLeft",
            "stackRight",
            "tinder",
            "transformer",
        ]),
        help: None,
    }, // layout/swiper.md:44-50
    AttributeRule {
        tags: &["swiper"],
        attrs: &["transformer-type"],
        constraint: Constraint::Enum(&[
            "scaleAndFade",
            "accordion",
            "threeD",
            "zoomIn",
            "zoomOut",
            "deepthPage",
        ]),
        help: None,
    }, // layout/swiper.md:56-63
    AttributeRule {
        tags: &["swiper"],
        attrs: &["indicator-type"],
        constraint: Constraint::Enum(&[
            "normal",
            "worm",
            "wormThin",
            "wormUnderground",
            "wormThinUnderground",
            "expand",
            "jump",
            "jumpWithOffset",
            "scroll",
            "scrollFixedCenter",
            "slide",
            "slideUnderground",
            "scale",
            "swap",
            "swapYRotation",
            "color",
        ]),
        help: None,
    }, // layout/swiper.md:67-84
    AttributeRule {
        tags: &["swiper"],
        attrs: &["direction"],
        constraint: Constraint::Enum(&["all", "positive", "negative"]),
        help: None,
    }, // layout/swiper.md:109-114
    AttributeRule {
        tags: &["snapshot"],
        attrs: &["mode"],
        constraint: Constraint::Enum(&["view", "picture"]),
        help: Some("合法值：view、picture"),
    }, // special/snapshot.md:11
    AttributeRule {
        tags: &["share-element"],
        attrs: &["rect-tween-type"],
        constraint: Constraint::Enum(&[
            "materialRectArc",
            "materialRectCenterArc",
            "linear",
            "cubic-bezier*",
            "fastLinearToSlowEaseIn",
            "ease",
            "easeIn",
            "easeInToLinear",
            "easeInSine",
            "easeInQuad",
            "easeInCubic",
            "easeInQuart",
            "easeInQuint",
            "easeInExpo",
            "easeInCirc",
            "easeInBack",
            "easeOut",
            "linearToEaseOut",
            "easeOutSine",
            "easeOutQuad",
            "easeOutCubic",
            "easeOutQuart",
            "easeOutQuint",
            "easeOutExpo",
            "easeOutCirc",
            "easeOutBack",
            "easeInOut",
            "easeInOutSine",
            "easeInOutQuad",
            "easeInOutCubic",
            "easeInOutQuart",
            "easeInOutQuint",
            "easeInOutExpo",
            "easeInOutCirc",
            "easeInOutBack",
            "slowMiddle",
            "elasticIn",
            "elasticOut",
            "elasticInOut",
            "easeInElastic",
            "easeOutElastic",
            "easeInOutElastic",
            "bounceIn",
            "bounceOut",
            "bounceInOut",
            "easeInBounce",
            "easeOutBounce",
            "easeInOutBounce",
            "fastOutSlowIn",
        ]),
        help: Some("用 materialRectArc / materialRectCenterArc / linear 或 cubic-bezier(...)"),
    }, // special/share-element.md:85-98；缓动名全集见 curve.dart:23-70
    AttributeRule {
        tags: &["share-element"],
        attrs: &["shuttle-on-push", "shuttle-on-pop"],
        constraint: Constraint::Enum(&["to", "from"]),
        help: Some("合法值：to（目标页面节点）、from（源页面节点）"),
    }, // special/share-element.md:78-83
    AttributeRule {
        tags: &["scroll-view"],
        attrs: &["refresher-default-style"],
        constraint: Constraint::Enum(&["black", "white", "none"]),
        help: None,
    }, // scroll/scroll-view.md:195
    AttributeRule {
        tags: &["scroll-view"],
        attrs: &["scroll-into-view-alignment"],
        constraint: Constraint::Enum(&["start", "center", "end", "nearest"]),
        help: None,
    }, // scroll/scroll-view.md:113
    AttributeRule {
        tags: &["scroll-view"],
        attrs: &["associative-container"],
        constraint: Constraint::Enum(&["draggable-sheet", "nested-scroll-view", "pop-gesture"]),
        help: None,
    }, // scroll/scroll-view.md:182-185
    AttributeRule {
        tags: &[
            "vertical-drag-gesture-handler",
            "horizontal-drag-gesture-handler",
            "pan-gesture-handler",
            "tap-gesture-handler",
            "double-tap-gesture-handler",
            "long-press-gesture-handler",
            "scale-gesture-handler",
            "force-press-gesture-handler",
        ],
        attrs: &["native-view"],
        constraint: Constraint::Enum(&["scroll-view", "swiper"]),
        help: Some("仅支持 scroll-view、swiper"),
    }, // gesture/gesture-system.md:15-22,56-67；gesture/gesture-negotiation.md:127-129
    AttributeRule {
        tags: &["scroll-view"],
        attrs: &["paging-enabled"],
        constraint: Constraint::Unsupported,
        help: Some("Skyline 没有分页滚动实现，需要分页时改用 swiper"),
    }, // 引擎 shadow/、widgets/、package/skyline/lib 均无 paging
    AttributeRule {
        tags: &["picker-view"],
        attrs: &["indicator-class", "mask-style"],
        constraint: Constraint::Unsupported,
        help: Some("Skyline 下该属性不生效，去掉后按 Skyline 样式重写"),
    }, // introduction/component-support.md:50
    AttributeRule {
        tags: &["input"],
        attrs: &["placeholder-class"],
        constraint: Constraint::Unsupported,
        help: Some("仅 WebView 生效，Skyline 下会被忽略，删除后按 Skyline 样式重写"),
    }, // form/input.md:306-313
    AttributeRule {
        tags: &["input"],
        attrs: &["safe-password-*"],
        constraint: Constraint::Unsupported,
        help: Some("仅 WebView 生效，Skyline 下会被忽略，删除后使用 Skyline 输入框"),
    }, // form/input.md:313
    AttributeRule {
        tags: &["text"],
        attrs: &["max-lines"],
        constraint: Constraint::Number(NumberRange::NonNegative),
        help: Some("合法值：非负整数（0 表示不限制行数）"),
    }, // media/text.md:46；text_shadow_node.h:102-103「0 means not apply max-lines constraints」
    AttributeRule {
        tags: &["draggable-sheet"],
        attrs: &["initial-child-size", "min-child-size", "max-child-size"],
        constraint: Constraint::Number(NumberRange::Between(0.0, 1.0)),
        help: Some("尺寸是相对父容器的比例，取值 0~1"),
    }, // scroll/draggable-sheet.md:26-28,343
];

const REQUIRES: &[Requires] = &[
    Requires {
        tag: "swiper",
        trigger: Trigger::Present("transformer-type"),
        requirement: Requirement::Equals("layout-type", "transformer"),
        help: Some(
            "transformer-type 仅当 layout-type=\"transformer\" 时生效，需同时设置 layout-type",
        ),
    }, // layout/swiper.md:54
    Requires {
        tag: "swiper",
        trigger: Trigger::NotAny("indicator-type", &["normal"]),
        requirement: Requirement::NotAny("layout-type", &["stackLeft", "stackRight", "tinder"]),
        help: Some("stackLeft/stackRight/tinder 布局下 indicator-type 只能是 normal"),
    }, // layout/swiper.md:269
    Requires {
        tag: "swiper",
        trigger: Trigger::Present("circular"),
        requirement: Requirement::NotAny(
            "indicator-type",
            &["scrollFixedCenter", "swap", "swapYRotation"],
        ),
        help: Some("这三个 indicator-type 不支持 circular，去掉 circular 或改用其它指示器动画"),
    }, // layout/swiper.md:270
    Requires {
        tag: "text",
        trigger: Trigger::Equals("overflow", "ellipsis"),
        requirement: Requirement::Present("max-lines"),
        help: Some("只要一行省略改用 `overflow=\"clip\"`；要限制行数就同时写 `max-lines`"),
    }, // text_shadow_node.h:102-103：max_lines=0 表示不施加行数约束，overflow 与 maxLines 是两个独立属性
];

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = Table::default();
        for rule in ATTRIBUTES {
            for &tag in rule.tags {
                for &attr in rule.attrs {
                    table
                        .entries
                        .entry(tag)
                        .or_default()
                        .entry(attr)
                        .or_default()
                        .push(Entry {
                            constraint: rule.constraint,
                            help: rule.help,
                        });
                }
            }
        }
        for rule in REQUIRES {
            table.requires.entry(rule.tag).or_default().push(*rule);
        }
        table
    })
}

pub struct ComponentAttribute;

impl ComponentAttribute {
    pub const ID: &'static str = "skyline/component-attribute";
}

impl Rule for ComponentAttribute {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "Skyline 会静默忽略的组件属性取值与组合（枚举白名单 / 不支持的属性 / 数值范围 / 跨属性条件）",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let table = table();
        visit(project, |doc, out| {
            let tree = Tree::new(doc);
            walk_skyline(doc, |element, ancestors| {
                for attr in &element.attributes {
                    let name = attr.name.to_ascii_lowercase();
                    let Some(entries) = table.lookup(&element.tag, &name) else {
                        continue;
                    };
                    let Some((entry, value)) = judge_attribute(entries, attr) else {
                        continue;
                    };
                    // `<image mode>` 的取值既不是 Skyline 的合法值、也不是 WebView 的裁剪模式时
                    // （`mode="aspectFit|aspectFill|widthFix"` 这类管道串），两个渲染器都回退成
                    // 拉伸填满：两端一致，不是迁移差异。
                    if element.tag == "image"
                        && name == "mode"
                        && matches!(entry.constraint, Constraint::Enum(_))
                    {
                        continue;
                    }
                    out.push(
                        span_diagnostic(
                            Self::ID,
                            doc,
                            attr.start,
                            attr.end,
                            doc.path(),
                            Severity::Warning,
                            message(&element.tag, &name, value, &entry.constraint),
                        )
                        .help(help(entry)),
                    );
                }
                let Some(rules) = table.requires.get(element.tag.as_str()) else {
                    return;
                };
                for rule in rules {
                    let Some(attr) = find_attr(element, rule.trigger.attr()) else {
                        continue;
                    };
                    let Some(value) = trigger_value(attr, &rule.trigger) else {
                        continue;
                    };
                    let Verdict::Violated(current) = judge(element, &rule.requirement) else {
                        continue;
                    };
                    // `<text overflow="ellipsis">` 缺 `max-lines`：WebView 的行内 `<text>`
                    // 没 nowrap / clamp 时本来就换行不省略，两端一致。
                    let needs_baseline = rule.trigger.attr() == "overflow"
                        && matches!(rule.requirement, Requirement::Present("max-lines"));
                    if needs_baseline
                        && !super::webview_truncates(project, &tree, element, ancestors)
                    {
                        continue;
                    }
                    // WebView 靠祖先的 `-webkit-line-clamp` 截几行，Skyline 就得补同样行数的
                    // `max-lines`；行数写进消息，用户能直接照做。
                    let lines = needs_baseline
                        .then(|| super::clamp_lines(project, &tree, element, ancestors))
                        .flatten();
                    out.push(
                        span_diagnostic(
                            Self::ID,
                            doc,
                            attr.start,
                            attr.end,
                            doc.path(),
                            Severity::Warning,
                            requires_message(&element.tag, rule, &value, current.as_deref(), lines),
                        )
                        .help(
                            rule.help
                                .unwrap_or("调整这两个属性的取值，避免组合被忽略")
                                .to_string(),
                        ),
                    );
                }
            });
        })
    }
}

/// 属性取值空间的判定结果：命中的约束，以及用于展示的取值。
///
/// 静态值直接判；动态绑定只在**所有候选都非法**时报——「无论运行时取哪个值都错」，
/// 这是任何执行路径都成立的结论，不会误报。候选枚举不出来时放过。
fn judge_attribute<'a>(
    entries: &'a [Entry],
    attr: &'a WxmlAttribute,
) -> Option<(&'a Entry, Value<'a>)> {
    match (attr.is_static, attr.candidates.as_deref()) {
        (true, _) => {
            // 无值属性是布尔 true：`[unsupported]` 只看属性是否存在，
            // 取值类约束（枚举 / 区间）无法判定，跳过。
            let value = attr.value.as_deref().map(str::trim).unwrap_or("");
            let entry = entries.iter().find(|entry| match entry.constraint {
                Constraint::Unsupported => true,
                _ => !value.is_empty() && entry.constraint.rejects(value),
            })?;
            Some((entry, Value::Static(value)))
        }
        (false, Some(candidates)) => {
            if candidates.is_empty() {
                return None;
            }
            let entry = entries.iter().find(|entry| match entry.constraint {
                // 属性本身不支持与取值无关，存在即可报。
                Constraint::Unsupported => true,
                _ => candidates
                    .iter()
                    .all(|value| entry.constraint.rejects(value)),
            })?;
            Some((entry, Value::AllInvalid(candidates)))
        }
        (false, None) => None,
    }
}

/// 属性可能的取值：静态值一个；动态绑定是候选集（枚举不出时为空）。
fn values(attr: &WxmlAttribute) -> Vec<&str> {
    match (attr.is_static, attr.candidates.as_deref()) {
        (true, _) => vec![attr.value.as_deref().map(str::trim).unwrap_or_default()],
        (false, Some(candidates)) => candidates.iter().map(String::as_str).collect(),
        (false, None) => Vec::new(),
    }
}

/// 用于构造消息的取值：静态值，或「全部候选都非法」时的候选列表。
enum Value<'a> {
    Static(&'a str),
    AllInvalid(&'a [String]),
}

/// 触发条件命中时返回用于展示的取值，无值属性返回空串。
///
/// 动态绑定只在**所有候选都命中触发条件**时才算触发：`overflow="{{a ? 'ellipsis' : 'clip'}}"`
/// 有两半概率不触发，不能按触发处理。
fn trigger_value(attr: &WxmlAttribute, trigger: &Trigger) -> Option<String> {
    if attr.is_static {
        let value = attr.value.as_deref().map(str::trim).unwrap_or_default();
        let hit = match trigger {
            // 无值属性（`<swiper circular>`）等于布尔 true，必须能触发。
            Trigger::Present(_) => attr.value.is_none() || !value.is_empty(),
            Trigger::Equals(_, expected) => {
                !value.is_empty() && value.eq_ignore_ascii_case(expected)
            }
            Trigger::NotAny(_, banned) => !value.is_empty() && !contains(banned, value),
        };
        return hit.then(|| value.to_string());
    }
    let candidates = attr.candidates.as_deref()?;
    let hit = match trigger {
        // 布尔属性的取值决定是否生效（`circular="{{cond}}"`），存在不等于 true。
        Trigger::Present(_) => false,
        Trigger::Equals(_, expected) => candidates
            .iter()
            .all(|value| value.eq_ignore_ascii_case(expected)),
        Trigger::NotAny(_, banned) => candidates.iter().all(|value| !contains(banned, value)),
    };
    hit.then(|| candidates.join(" / "))
}

enum Verdict {
    /// 要求成立，或要求属性是动态值、静态阶段无法判断。
    Ok,
    /// 要求不成立；`None` 表示要求属性缺失。
    Violated(Option<String>),
}

fn judge(element: &WxmlElement, requirement: &Requirement) -> Verdict {
    match requirement {
        Requirement::Equals(name, expected) => {
            let Some(attr) = find_attr(element, name) else {
                // 缺失即走默认值，按「不存在」处理：要求 `=v` 不成立。
                return Verdict::Violated(None);
            };
            let values = values(attr);
            if values.is_empty() {
                // 动态绑定枚举不出取值：判不了。
                return Verdict::Ok;
            }
            // 要求成立需要「任一可能取值都等于期望值」；全都不等才算确定不成立。
            if values
                .iter()
                .any(|value| value.eq_ignore_ascii_case(expected))
            {
                Verdict::Ok
            } else {
                Verdict::Violated(Some(values.join(" / ")))
            }
        }
        Requirement::Present(name) => match find_attr(element, name) {
            // 声明了就算满足（含无值属性与动态绑定，静态阶段判不了它的取值）。
            Some(_) => Verdict::Ok,
            None => Verdict::Violated(None),
        },
        Requirement::NotAny(name, banned) => {
            let Some(attr) = find_attr(element, name) else {
                // 缺失同样按「不存在」处理：不等于任一被禁值（如 layout-type 缺省 normal），
                // 所以不报。
                return Verdict::Ok;
            };
            let values = values(attr);
            // 只有所有可能取值都被禁才算确定违规（`layout-type="{{a ? 'tinder' : 'normal'}}"` 不算）。
            if !values.is_empty() && values.iter().all(|value| contains(banned, value)) {
                Verdict::Violated(Some(values.join(" / ")))
            } else {
                Verdict::Ok
            }
        }
    }
}

fn requires_message(
    tag: &str,
    rule: &Requires,
    value: &str,
    current: Option<&str>,
    lines: Option<u32>,
) -> String {
    let trigger = match value {
        "" => rule.trigger.attr().to_string(),
        value => format!("{}=\"{value}\"", rule.trigger.attr()),
    };
    match &rule.requirement {
        Requirement::Equals(name, expected) => {
            let current = match current {
                Some(value) => format!("{name}=\"{value}\""),
                None => format!("未设置 {name}"),
            };
            format!(
                "`<{tag} {trigger}>` 的 `{}` 需要 `{name}=\"{expected}\"`，当前 {current}",
                rule.trigger.attr()
            )
        }
        Requirement::Present(name) => match lines {
            Some(lines) => format!(
                "`<{tag} {trigger}>` 缺少 `{name}`：WebView 靠祖先的 `-webkit-line-clamp: {lines}` 截 {lines} 行，Skyline 下按属性截断，补 `{name}=\"{lines}\"`"
            ),
            None => format!(
                "`<{tag} {trigger}>` 缺少 `{name}`：WebView 靠单行省略（`nowrap` + `overflow`）截断，Skyline 下按属性截断，单行补 `{name}=\"1\"`"
            ),
        },
        Requirement::NotAny(name, _) => format!(
            "`<{tag} {trigger}>` 的 `{}` 在 `{name}=\"{}\"` 时不生效，会被忽略",
            rule.trigger.attr(),
            current.unwrap_or_default()
        ),
    }
}

fn message(tag: &str, attr: &str, value: Value<'_>, constraint: &Constraint) -> String {
    // 静态值写进属性里；候选集只写属性名，候选列在句末——`mode="{{a ? 'x' : 'y'}}"`
    // 展开成 `mode="x / y"` 会读成「写了一个带斜杠的值」。
    let (subject, note) = match value {
        Value::Static(value) => (format!("`<{tag} {attr}=\"{value}\">`"), String::new()),
        Value::AllInvalid(candidates) => (
            format!("`<{tag} {attr}>`"),
            format!(
                "（动态取值的候选只有 {}，全部非法）",
                candidates
                    .iter()
                    .map(|candidate| format!("`{candidate}`"))
                    .collect::<Vec<_>>()
                    .join("、")
            ),
        ),
    };
    let body = match constraint {
        Constraint::Unsupported => {
            format!("`<{tag}>` 的 `{attr}` 属性在 Skyline 下不支持，会被静默忽略")
        }
        Constraint::Number(NumberRange::NonNegative) => format!("{subject} 必须是非负整数"),
        Constraint::Number(NumberRange::Between(min, max)) => {
            format!("{subject} 必须是 {min}~{max} 的数值")
        }
        // `<image mode>` 命中这里说明它既不是 Skyline 的值、也不是 WebView 的裁剪模式：
        // 两个渲染器都不认，已在 check 里跳过。
        Constraint::Enum(_) | Constraint::WebviewOnly(_) => {
            format!("{subject} 不是 Skyline 支持的值，会被忽略")
        }
    };
    format!("{body}{note}")
}

fn help(entry: &Entry) -> String {
    if let Some(help) = entry.help {
        return help.to_string();
    }
    match &entry.constraint {
        Constraint::Enum(values) => format!(
            "合法值：{}",
            values
                .iter()
                .map(|value| value.trim_end_matches('*'))
                .collect::<Vec<_>>()
                .join("、")
        ),
        Constraint::WebviewOnly(_) => "改用 aspectFill 配合 CSS 定位".to_string(),
        Constraint::Unsupported => "Skyline 不支持该属性，建议删除".to_string(),
        Constraint::Number(NumberRange::Between(min, max)) => format!("合法值：{min}~{max}"),
        Constraint::Number(NumberRange::NonNegative) => "合法值：非负整数".to_string(),
    }
}

#[cfg(test)]
mod tests {

    use crate::rules::testutil::wxml::{Fixture, check_rule, project};

    use super::ComponentAttribute;

    #[test]
    fn enum_violations_are_reported_on_the_attribute() {
        let diagnostics = check_rule(
            &project(
                "attr-enum",
                r#"<view><image mode="cover"/><text overflow="hidden"/></view>"#,
            ),
            &ComponentAttribute,
        );
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("hidden"));
        assert_eq!(diagnostics[0].severity, crate::Severity::Warning);
    }

    /// 引擎实际接受、但旧白名单漏掉的取值：补上后不再误报。
    #[test]
    fn engine_accepted_values_are_allowed() {
        let diagnostics = check_rule(
            &project(
                "attr-engine-values",
                r#"<image mode="auto"/><text overflow="unset"/><input confirm-type="return"/><text max-lines="0"/>
<image mode="cover"/><text overflow="hidden"/><input confirm-type="enter"/><text max-lines="-1"/>"#,
            ),
            &ComponentAttribute,
        );
        assert_eq!(diagnostics.len(), 3, "{diagnostics:#?}");
        assert!(
            diagnostics
                .iter()
                .all(|d| d.severity == crate::Severity::Warning)
        );
        assert!(diagnostics[0].message.contains("hidden"));
        assert!(diagnostics[1].message.contains("enter"));
        assert!(diagnostics[2].message.contains("max-lines"));
    }

    /// WebView 有、引擎没有实现的分页滚动属性。
    #[test]
    fn paging_enabled_is_reported() {
        let diagnostics = check_rule(
            &project("attr-paging", r#"<scroll-view paging-enabled/>"#),
            &ComponentAttribute,
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("paging-enabled"));
    }

    /// `overflow="ellipsis"` 不带 `max-lines` 时引擎不施加行数约束，省略号不会出现。
    ///
    /// 前提是 WebView 侧本来就会省略：行内 `<text>` 没 nowrap / clamp 时会直接换行。
    #[test]
    fn overflow_ellipsis_requires_max_lines() {
        let fixture = project(
            "attr-ellipsis",
            r#"<text class="clip" overflow="ellipsis"/>
<text class="clip" overflow="ellipsis" max-lines="2"/>
<text class="clip" overflow="clip"/>
<text class="clip" overflow="{{o}}" max-lines="2"/>
<text overflow="ellipsis"/>"#,
        );
        fixture.write(
            "pages/index/index.wxss",
            ".clip { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }",
        );
        let diagnostics = check_rule(&fixture, &ComponentAttribute);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("max-lines"));
        // 行数来自 WebView 的单行省略，直接写进消息，用户能照做。
        assert!(
            diagnostics[0].message.contains("max-lines=\"1\""),
            "{}",
            diagnostics[0].message
        );
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (1, 20));
    }

    /// 祖先靠 `-webkit-line-clamp` 截几行，就提示补几行。
    #[test]
    fn max_lines_hint_follows_ancestor_clamp() {
        let fixture = project(
            "attr-ellipsis-clamp",
            r#"<view class="title"><text overflow="ellipsis">书名</text></view>"#,
        );
        fixture.write(
            "pages/index/index.wxss",
            ".title { display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }",
        );
        let diagnostics = check_rule(&fixture, &ComponentAttribute);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(
            diagnostics[0].message.contains("max-lines=\"2\""),
            "{}",
            diagnostics[0].message
        );
    }

    /// WebView 下也不省略时不报：`<text>` 是行内元素，`max-width` 不生效、文本直接换行。
    #[test]
    fn ellipsis_without_webview_baseline_is_skipped() {
        let diagnostics = check_rule(
            &project(
                "attr-ellipsis-baseline",
                r#"<view style="width: 100px;"><text style="max-width: 100px;" overflow="ellipsis">共 3 章</text></view>"#,
            ),
            &ComponentAttribute,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn valid_enum_values_are_allowed() {
        let diagnostics = check_rule(
            &project(
                "attr-enum-ok",
                r#"<image mode="aspectfill"/>
<swiper layout-type="transformer" indicator-type="wormThin" direction="positive"/>
<share-element rect-tween-type="cubic-bezier(0.4, 0, 0.2, 1)" shuttle-on-push="from"/>
<scroll-view scroll-into-view-alignment="CENTER" associative-container="nested-scroll-view"/>
<input type="nickname" confirm-type="done"/>
<input type="text" confirm-type="return"/>
<snapshot mode="picture"/>
        <text overflow="ellipsis" max-lines="2"/>
<text overflow="unset"/>
<image mode="auto"/>
<text max-lines="0"/>"#,
            ),
            &ComponentAttribute,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// 候选全非法：无论运行时取哪个值都错，这条结论成立，可以报。
    #[test]
    fn all_candidates_invalid_is_reported() {
        let diagnostics = check_rule(
            &project(
                "attr-dynamic-all-invalid",
                r#"<input confirm-type="{{ok ? 'sendX' : 'foo'}}"/>"#,
            ),
            &ComponentAttribute,
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(
            diagnostics[0].message.contains("候选只有 `sendX`、`foo`"),
            "{}",
            diagnostics[0].message
        );
    }

    /// 候选里有一个合法值就不报：运行时换个分支就对了。
    #[test]
    fn mixed_candidates_are_skipped() {
        let diagnostics = check_rule(
            &project(
                "attr-dynamic-mixed",
                r#"<image mode="{{ok ? 'aspectFit' : 'foo'}}"/>"#,
            ),
            &ComponentAttribute,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// 触发条件只有「所有候选都命中」时才算触发。
    #[test]
    fn dynamic_trigger_needs_every_candidate() {
        let fixture = project(
            "attr-dynamic-trigger",
            r#"<text class="clip" overflow="{{a ? 'ellipsis' : 'clip'}}"/>
<text class="clip" overflow="{{'ellipsis'}}"/>"#,
        );
        fixture.write(
            "pages/index/index.wxss",
            ".clip { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }",
        );
        let diagnostics = check_rule(&fixture, &ComponentAttribute);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].line, 2);
    }

    #[test]
    fn dynamic_and_valueless_enum_attributes_are_skipped() {
        let diagnostics = check_rule(
            &project(
                "attr-dynamic",
                r#"<image mode="{{m}}"/><image mode="a{{m}}"/><share-element shuttle-on-pop/><text max-lines="{{n}}"/>"#,
            ),
            &ComponentAttribute,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// 无值属性是布尔 true：`[unsupported]` 的属性出现即无效，不能因为没写值就跳过。
    #[test]
    fn unsupported_attribute_without_value_is_reported() {
        let diagnostics = check_rule(
            &project(
                "attr-bare",
                r#"<picker-view indicator-class/><input placeholder-class/>"#,
            ),
            &ComponentAttribute,
        );
        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
        assert!(diagnostics.iter().all(|d| d.message.contains("不支持")));
        assert!(diagnostics[0].message.contains("indicator-class"));
        assert!(diagnostics[1].message.contains("placeholder-class"));
    }

    /// `<image mode>` 两个渲染器都不认的取值：两端都回退成拉伸填满，不是迁移差异。
    #[test]
    fn garbage_image_modes_are_not_blamed_on_skyline() {
        // `mode` 写成一串用竖线拼起来的候选值：WebView 的 `_checkMode` 同样不认
        // （未知值直接 return，background-size 保持默认），两端一致。
        let diagnostics = check_rule(
            &project(
                "attr-mode-garbage",
                r#"<image mode="aspectFit|aspectFill|widthFix"/>"#,
            ),
            &ComponentAttribute,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn webview_only_crop_modes_get_specific_help() {
        let diagnostics = check_rule(
            &project(
                "attr-crop",
                r#"<image mode="top left"/><image mode="center"/>"#,
            ),
            &ComponentAttribute,
        );
        assert_eq!(diagnostics.len(), 2);
        for diagnostic in &diagnostics {
            assert!(diagnostic.message.contains("不是 Skyline 支持的值"));
            assert!(diagnostic.help.as_deref().unwrap().contains("aspectFill"));
        }
        assert!(diagnostics[0].message.contains("top left"));
    }

    #[test]
    fn unsupported_attributes_are_reported() {
        let diagnostics = check_rule(
            &project(
                "attr-unsupported",
                r#"<picker-view indicator-class="a" mask-style="b"/><input placeholder-class="ph"/>"#,
            ),
            &ComponentAttribute,
        );
        assert_eq!(diagnostics.len(), 3);
        assert!(
            diagnostics
                .iter()
                .all(|d| d.message.contains("不支持") && d.severity == crate::Severity::Warning)
        );
    }

    /// `text_support_space_and_decode` 硬编码开启，两个属性都已实现。
    #[test]
    fn text_space_and_decode_are_supported() {
        let diagnostics = check_rule(
            &project(
                "attr-text-space-decode",
                r#"<text space="nbsp" decode="true"/><text space/><text decode/>"#,
            ),
            &ComponentAttribute,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn unsupported_attributes_are_scoped_to_their_component() {
        let diagnostics = check_rule(
            &project(
                "attr-scope",
                r#"<view space="nbsp" decode="true"/><text mode="cover"/><image overflow="hidden"/>"#,
            ),
            &ComponentAttribute,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn number_constraints_are_reported() {
        let diagnostics = check_rule(
            &project(
                "attr-number",
                r#"<text max-lines="-1"/><text max-lines="2.5"/><text max-lines="abc"/>
<draggable-sheet initial-child-size="1.5" min-child-size="-0.1" max-child-size="2"/>"#,
            ),
            &ComponentAttribute,
        );
        assert_eq!(diagnostics.len(), 6);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("非负整数") && d.message.contains("max-lines"))
        );
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("0~1 的数值") && d.message.contains("min-child-size"))
        );
    }

    #[test]
    fn number_constraints_accept_bounds_and_skip_dynamic() {
        let diagnostics = check_rule(
            &project(
                "attr-number-ok",
                r#"<text max-lines="0"/><text max-lines="1"/><text max-lines="1.0"/><text max-lines="{{n}}"/>
<draggable-sheet initial-child-size="0" min-child-size="0.25" max-child-size="1"/>"#,
            ),
            &ComponentAttribute,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// `"renderer": "webview"` 的页面不迁 Skyline，整页跳过。
    #[test]
    fn opted_out_page_is_skipped() {
        let fixture = Fixture::new("attr-opt-out");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index","pages/webview/webview"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", "<view/>")
            .write("pages/webview/webview.json", r#"{"renderer":"webview"}"#)
            .write(
                "pages/webview/webview.wxml",
                r#"<image mode="cover"/><text space="nbsp"/>"#,
            );
        let diagnostics = check_rule(&fixture, &ComponentAttribute);
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn requires_transformer_type_needs_transformer_layout() {
        let diagnostics = check_rule(
            &project(
                "attr-requires-transformer",
                r#"<swiper layout-type="normal" transformer-type="threeD"/>
<swiper transformer-type="scaleAndFade"/>
<swiper layout-type="transformer" transformer-type="threeD"/>
<swiper layout-type="transformer"/>"#,
            ),
            &ComponentAttribute,
        );
        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("transformer-type"));
        assert!(
            diagnostics[0]
                .message
                .contains(r#"layout-type="transformer""#)
        );
        assert!(
            diagnostics[0]
                .message
                .contains(r#"当前 layout-type="normal""#)
        );
        // 无 layout-type 即默认 normal，同样不满足要求。
        assert!(diagnostics[1].message.contains("未设置 layout-type"));
        assert_eq!((diagnostics[1].line, diagnostics[1].column), (2, 9));
        assert!(
            diagnostics[0]
                .help
                .as_deref()
                .unwrap()
                .contains("layout-type")
        );
    }

    #[test]
    fn requires_indicator_type_needs_normal_under_stack_layouts() {
        let diagnostics = check_rule(
            &project(
                "attr-requires-indicator",
                r#"<swiper layout-type="stackLeft" indicator-type="worm"/>
<swiper layout-type="stackRight" indicator-type="slide"/>
<swiper layout-type="tinder" indicator-type="scale"/>
<swiper layout-type="stackLeft" indicator-type="normal"/>
<swiper layout-type="normal" indicator-type="worm"/>
<swiper layout-type="transformer" indicator-type="slide"/>"#,
            ),
            &ComponentAttribute,
        );
        let hits: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.message.contains("`indicator-type`"))
            .collect();
        assert_eq!(hits.len(), 3, "{diagnostics:#?}");
        assert!(hits[0].message.contains(r#"layout-type="stackLeft""#));
        assert!(hits[1].message.contains(r#"layout-type="stackRight""#));
        assert!(hits[2].message.contains(r#"layout-type="tinder""#));
        assert_eq!((hits[2].line, hits[2].column), (3, 30));
    }

    #[test]
    fn requires_circular_conflicts_with_indicator_animations() {
        let diagnostics = check_rule(
            &project(
                "attr-requires-circular",
                r#"<swiper indicator-type="swap" circular/>
<swiper indicator-type="swap" circular="true"/>
<swiper indicator-type="scrollFixedCenter" circular="{{true}}"/>
<swiper indicator-type="swapYRotation" circular="{{circular}}"/>
<swiper indicator-type="normal" circular/>
<swiper indicator-type="swap"/>
<swiper circular/>"#,
            ),
            &ComponentAttribute,
        );
        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
        // 无值属性等于布尔 true，同样要触发。
        assert!(diagnostics[0].message.contains("<swiper circular>"));
        assert!(diagnostics[0].message.contains(r#"indicator-type="swap""#));
        assert!(diagnostics[1].message.contains(r#"circular="true""#));
        assert!(diagnostics[0].help.as_deref().unwrap().contains("circular"));
    }

    #[test]
    fn requires_is_skipped_for_dynamic_values() {
        let diagnostics = check_rule(
            &project(
                "attr-requires-dynamic",
                r#"<swiper indicator-type="{{type}}" circular/>
<swiper layout-type="{{layout}}" transformer-type="threeD"/>
<swiper layout-type="{{layout}}" indicator-type="worm"/>"#,
            ),
            &ComponentAttribute,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn native_view_only_accepts_scroll_view_and_swiper() {
        let diagnostics = check_rule(
            &project(
                "attr-native-view",
                r#"<vertical-drag-gesture-handler native-view="view"/>
<vertical-drag-gesture-handler native-view="scroll-view"/>
<vertical-drag-gesture-handler native-view="SWIPER"/>
<horizontal-drag-gesture-handler native-view="swiper"/>
<pan-gesture-handler native-view="view"/>
<view native-view="view"/>"#,
            ),
            &ComponentAttribute,
        );
        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("native-view"));
        assert!(diagnostics[1].message.contains("pan-gesture-handler"));
        assert!(
            diagnostics[0]
                .help
                .as_deref()
                .unwrap()
                .contains("scroll-view")
        );
    }

    #[test]
    fn input_webview_only_attributes_are_unsupported() {
        let diagnostics = check_rule(
            &project(
                "attr-input-webview",
                r#"<input type="text"/><input placeholder-class="ph"/><input safe-password-nonce="n"/><input safe-password-length="6"/>"#,
            ),
            &ComponentAttribute,
        );
        assert_eq!(diagnostics.len(), 3, "{diagnostics:#?}");
        assert!(diagnostics.iter().all(|d| d.message.contains("不支持")));
        assert!(diagnostics[0].message.contains("placeholder-class"));
        assert!(diagnostics[1].message.contains("safe-password-nonce"));
        assert!(diagnostics[2].message.contains("safe-password-length"));
    }
}
