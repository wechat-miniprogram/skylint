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
//! 只判静态属性：`{{}}` 插值（`WxmlAttribute::is_static == false`）一律跳过，
//! 静态阶段无法确定它们的实际取值。`[requires]` 除外：无值属性视为布尔 true，
//! 这足以确认 `circular=*` 这类触发条件。
//! 已知未覆盖：textarea 的 placeholder-class 同为 WebView-only，但知识库无 textarea
//!   证据文件，按不确定不报处理。

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::diag::{Diagnostic, Severity};
use crate::input::wxml::{WxmlAttribute, WxmlElement};
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};

use super::{load_doc, span_diagnostic, wxml_files};

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
    /// 正整数。
    Positive,
    /// 闭区间。
    Between(f64, f64),
}

impl Constraint {
    fn rejects(&self, value: &str) -> bool {
        match self {
            Self::Enum(values) => !contains(values, value),
            Self::WebviewOnly(values) => contains(values, value),
            Self::Unsupported => true,
            Self::Number(NumberRange::Positive) => !value
                .parse::<f64>()
                .is_ok_and(|number| number >= 1.0 && number.fract() == 0.0),
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
    /// `attr=v` 精确匹配。当前数据没有这种触发条件，保留以覆盖完整的表达式模型。
    #[allow(dead_code)]
    Equals(&'static str, &'static str),
    NotAny(&'static str, &'static [&'static str]),
}

#[derive(Debug, Clone, Copy)]
enum Requirement {
    /// `attr=value`：属性必须存在且等于该值。
    Equals(&'static str, &'static str),
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

const ATTRIBUTES: &[AttributeRule] = &[
    AttributeRule {
        tags: &["image"],
        attrs: &["mode"],
        constraint: Constraint::WebviewOnly(&[
            "top",
            "bottom",
            "center",
            "left",
            "right",
            "top left",
            "top right",
            "bottom left",
            "bottom right",
        ]),
        help: Some("Skyline 不支持 WebView 的裁剪模式，改用 aspectFill 配合 CSS 定位"),
    }, // media/image.md:42-54
    AttributeRule {
        tags: &["text"],
        attrs: &["overflow"],
        constraint: Constraint::Enum(&["visible", "clip", "fade", "ellipsis"]),
        help: None,
    }, // media/text.md:50-55
    AttributeRule {
        tags: &["image"],
        attrs: &["mode"],
        constraint: Constraint::Enum(&[
            "scaleToFill",
            "aspectFit",
            "aspectFill",
            "widthFix",
            "heightFix",
        ]),
        help: Some("合法值：scaleToFill、aspectFit、aspectFill、widthFix、heightFix"),
    }, // media/image.md:32-38
    AttributeRule {
        tags: &["input"],
        attrs: &["type"],
        constraint: Constraint::Enum(&["text", "number", "idcard", "digit", "nickname"]),
        help: Some("合法值：text、number、idcard、digit、nickname"),
    }, // form/input.md:44-50
    AttributeRule {
        tags: &["input"],
        attrs: &["confirm-type"],
        constraint: Constraint::Enum(&["send", "search", "next", "go", "done"]),
        help: Some("合法值：send、search、next、go、done"),
    }, // form/input.md:54-60
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
        tags: &["text"],
        attrs: &["space"],
        constraint: Constraint::Unsupported,
        help: Some("Skyline 不支持 space 属性，空格改用 CSS white-space 控制"),
    }, // media/text.md:327
    AttributeRule {
        tags: &["text"],
        attrs: &["decode"],
        constraint: Constraint::Unsupported,
        help: Some("Skyline 不解码 HTML 实体，直接写目标字符"),
    }, // media/text.md:328
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
        constraint: Constraint::Number(NumberRange::Positive),
        help: Some("合法值：正整数"),
    }, // media/text.md:46
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
        let mut diagnostics = Vec::new();
        for path in wxml_files(project) {
            let Some(doc) = load_doc(project, path) else {
                continue;
            };
            for element in doc.elements() {
                for attr in &element.attributes {
                    if !attr.is_static {
                        continue;
                    }
                    let name = attr.name.to_ascii_lowercase();
                    let Some(entries) = table.lookup(&element.tag, &name) else {
                        continue;
                    };
                    // 无值属性是布尔 true：`[unsupported]` 只看属性是否存在，
                    // 取值类约束（枚举 / 区间）无法判定，跳过。
                    let value = attr.value.as_deref().map(str::trim).unwrap_or("");
                    let Some(entry) = entries.iter().find(|entry| match entry.constraint {
                        Constraint::Unsupported => true,
                        _ => !value.is_empty() && entry.constraint.rejects(value),
                    }) else {
                        continue;
                    };
                    diagnostics.push(
                        span_diagnostic(
                            Self::ID,
                            &doc,
                            attr.start,
                            attr.end,
                            path,
                            Severity::Warning,
                            message(&element.tag, &name, value, &entry.constraint),
                        )
                        .help(help(entry)),
                    );
                }
                let Some(rules) = table.requires.get(element.tag.as_str()) else {
                    continue;
                };
                for rule in rules {
                    let Some(attr) = find_attribute(element, rule.trigger.attr()) else {
                        continue;
                    };
                    if !attr.is_static {
                        continue;
                    }
                    let Some(value) = trigger_value(attr, &rule.trigger) else {
                        continue;
                    };
                    let Verdict::Violated(current) = judge(element, &rule.requirement) else {
                        continue;
                    };
                    diagnostics.push(
                        span_diagnostic(
                            Self::ID,
                            &doc,
                            attr.start,
                            attr.end,
                            path,
                            Severity::Warning,
                            requires_message(&element.tag, rule, &value, current.as_deref()),
                        )
                        .help(
                            rule.help
                                .unwrap_or("调整这两个属性的取值，避免组合被忽略")
                                .to_string(),
                        ),
                    );
                }
            }
        }
        diagnostics
    }
}

fn find_attribute<'a>(element: &'a WxmlElement, name: &str) -> Option<&'a WxmlAttribute> {
    element
        .attributes
        .iter()
        .find(|attr| attr.name.eq_ignore_ascii_case(name))
}

/// 触发条件命中时返回用于展示的取值，无值属性返回空串。
fn trigger_value(attr: &WxmlAttribute, trigger: &Trigger) -> Option<String> {
    let value = attr.value.as_deref().map(str::trim).unwrap_or_default();
    let hit = match trigger {
        // 无值属性（`<swiper circular>`）等于布尔 true，必须能触发。
        Trigger::Present(_) => attr.value.is_none() || !value.is_empty(),
        Trigger::Equals(_, expected) => !value.is_empty() && value.eq_ignore_ascii_case(expected),
        Trigger::NotAny(_, banned) => !value.is_empty() && !contains(banned, value),
    };
    hit.then(|| value.to_string())
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
            let Some(attr) = find_attribute(element, name) else {
                // 缺失即走默认值，按「不存在」处理：要求 `=v` 不成立。
                return Verdict::Violated(None);
            };
            if !attr.is_static {
                return Verdict::Ok;
            }
            let value = attr.value.as_deref().map(str::trim).unwrap_or_default();
            if value.eq_ignore_ascii_case(expected) {
                Verdict::Ok
            } else {
                Verdict::Violated(Some(value.to_string()))
            }
        }
        Requirement::NotAny(name, banned) => {
            let Some(attr) = find_attribute(element, name) else {
                // 缺失同样按「不存在」处理：不等于任一被禁值（如 layout-type 缺省 normal），
                // 所以不报。
                return Verdict::Ok;
            };
            if !attr.is_static {
                return Verdict::Ok;
            }
            let value = attr.value.as_deref().map(str::trim).unwrap_or_default();
            if contains(banned, value) {
                Verdict::Violated(Some(value.to_string()))
            } else {
                Verdict::Ok
            }
        }
    }
}

fn requires_message(tag: &str, rule: &Requires, value: &str, current: Option<&str>) -> String {
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
        Requirement::NotAny(name, _) => format!(
            "`<{tag} {trigger}>` 的 `{}` 在 `{name}=\"{}\"` 时不生效，会被忽略",
            rule.trigger.attr(),
            current.unwrap_or_default()
        ),
    }
}

fn message(tag: &str, attr: &str, value: &str, constraint: &Constraint) -> String {
    match constraint {
        Constraint::Unsupported => {
            format!("`<{tag}>` 的 `{attr}` 属性在 Skyline 下不支持，会被静默忽略")
        }
        Constraint::Number(NumberRange::Positive) => {
            format!("`<{tag} {attr}=\"{value}\">` 必须是正整数")
        }
        Constraint::Number(NumberRange::Between(min, max)) => {
            format!("`<{tag} {attr}=\"{value}\">` 必须是 {min}~{max} 的数值")
        }
        Constraint::Enum(_) | Constraint::WebviewOnly(_) => {
            format!("`<{tag} {attr}=\"{value}\">` 不是 Skyline 支持的值，会被忽略")
        }
    }
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
        Constraint::Number(NumberRange::Positive) => "合法值：正整数".to_string(),
    }
}

#[cfg(test)]
mod tests {

    use crate::rules::testutil::wxml::{Fixture, check_rule, project};

    use super::ComponentAttribute;


    #[test]
    fn enum_violations_are_warnings_on_the_attribute() {
        let diagnostics = check_rule(
            &project(
                "attr-enum",
                r#"<view><image mode="cover"/><text overflow="hidden"/></view>"#,
            ),
            &ComponentAttribute,
        );
        assert_eq!(diagnostics.len(), 2);
        assert!(
            diagnostics
                .iter()
                .all(|d| d.severity == crate::Severity::Warning)
        );
        let image = diagnostics
            .iter()
            .find(|d| d.message.contains("image"))
            .unwrap();
        assert_eq!((image.line, image.column), (1, 14));
        assert_eq!(
            image.help.as_deref().unwrap(),
            "合法值：scaleToFill、aspectFit、aspectFill、widthFix、heightFix"
        );
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
<snapshot mode="picture"/>
<text overflow="ellipsis"/>"#,
            ),
            &ComponentAttribute,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
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
                r#"<text space/><picker-view indicator-class/>"#,
            ),
            &ComponentAttribute,
        );
        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
        assert!(diagnostics.iter().all(|d| d.message.contains("不支持")));
        assert!(diagnostics[0].message.contains("space"));
        assert!(diagnostics[1].message.contains("indicator-class"));
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
                r#"<text space="nbsp"/><text decode="true"/><picker-view indicator-class="a" mask-style="b"/>"#,
            ),
            &ComponentAttribute,
        );
        assert_eq!(diagnostics.len(), 4);
        assert!(
            diagnostics
                .iter()
                .all(|d| d.message.contains("不支持") && d.severity == crate::Severity::Warning)
        );
        assert!(
            diagnostics
                .iter()
                .any(|d| d.help.as_deref().unwrap().contains("white-space"))
        );
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
                r#"<text max-lines="0"/><text max-lines="2.5"/><text max-lines="abc"/>
<draggable-sheet initial-child-size="1.5" min-child-size="-0.1" max-child-size="2"/>"#,
            ),
            &ComponentAttribute,
        );
        assert_eq!(diagnostics.len(), 6);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("正整数") && d.message.contains("max-lines"))
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
                r#"<text max-lines="1"/><text max-lines="1.0"/><text max-lines="{{n}}"/>
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
