//! 内置组件能力表：Skyline 下能渲染的组件、有没有 CSS 盒，与没有可用实现时的后果档位。
//!
//! 三件事实共用同一份名单：
//!
//! - 是不是平台内置组件（[`is_builtin`]）；
//! - 还有多少 CSS 盒能力（[`Capability::css_box`]）：`skyline/native-padding` 与
//!   `skyline/native-layout-ignored` 的唯一数据源；
//! - Skyline 下没有可用实现时的后果档位（黑名单），`skyline/component-support` 按表出文案。

/// 组件在 Skyline 里还剩多少 CSS 盒能力。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CssBox {
    /// 普通 CSS 盒：子节点进它自己的布局树，盒属性正常生效。
    Full,
    /// 有 yoga 节点、padding 也写进去了，但 concrete widget 是 widget 链尾，
    /// 子节点仍是独立布局根。
    BorderBoxTail,
    /// 连布局节点都不建（`InflateNaked()` / `SetSkipStyleBuild()`），CSS 盒属性整体失效。
    None,
}

/// 组件是否暴露 `padding` 属性。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PaddingAttr {
    /// 自带 `padding` 属性，取 `[top, right, bottom, left]`。
    Attribute,
    /// 没有 `padding` 属性，只能在里面包一层普通 `view`。
    Absent,
}

/// 一个内置组件的 CSS 能力。
pub(crate) struct Capability {
    pub tag: &'static str,
    pub css_box: CssBox,
    /// CSS 背景色是否还会绘制。只在 [`CssBox::None`] 上有区分：list/grid 四件套在
    /// style builder 的白名单里只剩背景色，sticky / nested-scroll 系列连 computed
    /// style 都不建。
    pub paints_background: bool,
    pub padding_attr: PaddingAttr,
}

impl CssBox {
    /// 这个组件连 yoga 节点都没有：CSS 布局属性整体失效。
    pub(crate) fn ignores_layout(self) -> bool {
        matches!(self, Self::None)
    }
}

/// Skyline 下没有可用实现的组件，后果分三档。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Loss {
    /// 组件在 Skyline 下没有实现：盒子与内容都不出现。
    Unavailable,
    /// 盒子与 CSS 仍在，只丢掉原生绘制或原生能力。
    PaintingLost,
    /// 只能在整页 `"renderer": "webview"` 下工作。
    PageOptOut,
}

/// 黑名单的一行：标签、后果档位、替代方案。
pub(crate) struct Blocked {
    pub tag: &'static str,
    pub loss: Loss,
    pub advice: &'static str,
}

/// Skyline 组件支持表。
///
/// 来源：公开的组件支持文档
/// 同步方式：按「完全支持的组件」「基本支持的组件」「Skyline 新增组件」更新本表，
/// 按「暂不支持的组件」更新 `BLOCKED_COMPONENTS`。
/// 标签一律小写。
const SUPPORTED_COMPONENTS: &[&str] = &[
    "view",
    "cover-view",
    "button",
    "scroll-view",
    "swiper",
    "swiper-item",
    "input",
    "textarea",
    "navigator",
    "map",
    "canvas",
    "radio",
    "radio-group",
    "label",
    "checkbox",
    "checkbox-group",
    "picker",
    "camera",
    "root-portal",
    "form",
    "ad",
    "official-account",
    "live-player",
    "live-pusher",
    "voip-room",
    "icon",
    "slider",
    "switch",
    "share-element",
    "page-container",
    "text",
    "image",
    "cover-image",
    "video",
    "picker-view",
    "picker-view-column",
    "rich-text",
    "page-meta",
    "span",
    "sticky-header",
    "sticky-section",
    "list-view",
    "grid-view",
    "list-builder",
    "grid-builder",
    "nested-scroll-header",
    "nested-scroll-body",
    "draggable-sheet",
    "open-container",
    "snapshot",
    "tap-gesture-handler",
    "double-tap-gesture-handler",
    "long-press-gesture-handler",
    "pan-gesture-handler",
    "scale-gesture-handler",
    "horizontal-drag-gesture-handler",
    "vertical-drag-gesture-handler",
    "force-press-gesture-handler",
];

/// 内置组件的 CSS 盒能力表。
///
/// 只收录「非普通 CSS 盒」的组件，以及少数明确确认 `Full` 的例外；未收录的内置组件
/// 按普通 CSS 盒处理（两条 `native-*` 规则都不报），宁可漏报。
///
/// 条目来源：既有单测（`native_layout_ignored` / `native_padding`）、
/// `native_layout_ignored` 的 sticky-section 视觉用例与引擎行为（NoCssBox 的组件
/// 走 `InflateNaked()` / `SetSkipStyleBuild()`，连布局节点都不建）。
///
/// 待确认的标签（没有仓库内证据，先不收录——两条规则都不报）：
/// `cover-view` / `picker-view` / `rich-text` / `page-container` / `root-portal` /
/// `share-element` / `open-container` / `snapshot` / 手势容器，以及原生媒体组件
/// （`input` / `textarea` / `video` / `camera` / `map` / `canvas` / `live-player` 等）。
/// 它们缺失时只会少报，不会误报；确认后再逐条补。
pub(crate) const CAPABILITIES: &[Capability] = &[
    // 没有 CSS 盒：CSS 布局属性整体失效。
    Capability {
        tag: "list-view",
        css_box: CssBox::None,
        paints_background: true,
        padding_attr: PaddingAttr::Attribute,
    },
    Capability {
        tag: "grid-view",
        css_box: CssBox::None,
        paints_background: true,
        padding_attr: PaddingAttr::Attribute,
    },
    Capability {
        tag: "list-builder",
        css_box: CssBox::None,
        paints_background: true,
        padding_attr: PaddingAttr::Attribute,
    },
    Capability {
        tag: "grid-builder",
        css_box: CssBox::None,
        paints_background: true,
        padding_attr: PaddingAttr::Attribute,
    },
    Capability {
        tag: "sticky-section",
        css_box: CssBox::None,
        paints_background: false,
        padding_attr: PaddingAttr::Attribute,
    },
    Capability {
        tag: "sticky-header",
        css_box: CssBox::None,
        paints_background: false,
        padding_attr: PaddingAttr::Attribute,
    },
    Capability {
        tag: "nested-scroll-header",
        css_box: CssBox::None,
        paints_background: false,
        padding_attr: PaddingAttr::Absent,
    },
    Capability {
        tag: "nested-scroll-body",
        css_box: CssBox::None,
        paints_background: false,
        padding_attr: PaddingAttr::Absent,
    },
    // 有 CSS 盒，但 concrete widget 是 widget 链尾：padding / border / gap 不内缩子节点。
    Capability {
        tag: "scroll-view",
        css_box: CssBox::BorderBoxTail,
        paints_background: true,
        padding_attr: PaddingAttr::Attribute,
    },
    Capability {
        tag: "swiper",
        css_box: CssBox::BorderBoxTail,
        paints_background: true,
        padding_attr: PaddingAttr::Absent,
    },
    Capability {
        tag: "draggable-sheet",
        css_box: CssBox::BorderBoxTail,
        paints_background: true,
        padding_attr: PaddingAttr::Absent,
    },
    Capability {
        tag: "picker-view-column",
        css_box: CssBox::BorderBoxTail,
        paints_background: true,
        padding_attr: PaddingAttr::Absent,
    },
    // 明确的普通 CSS 盒例外：子节点走它自己的 yoga 树，padding 正常生效。
    Capability {
        tag: "swiper-item",
        css_box: CssBox::Full,
        paints_background: true,
        padding_attr: PaddingAttr::Absent,
    },
];

/// CSS 能力表命中才返回整行；未收录的内置组件与未知自定义组件都返回 `None`。
pub(crate) fn capability(tag: &str) -> Option<&'static Capability> {
    CAPABILITIES.iter().find(|row| row.tag == tag)
}

/// Skyline 下不可用的组件：后果档位 + 替代方案。来源同 `SUPPORTED_COMPONENTS`。
const BLOCKED_COMPONENTS: &[Blocked] = &[
    Blocked {
        tag: "web-view",
        loss: Loss::PageOptOut,
        advice: r#"该页面配置 `"renderer": "webview"`"#,
    },
    // 手势容器没有实现，拖拽内容整块不参与布局；替代是手势 + worklet 自己驱动位移。
    Blocked {
        tag: "movable-area",
        loss: Loss::Unavailable,
        advice: "手势 + worklet 动画方案",
    },
    Blocked {
        tag: "movable-view",
        loss: Loss::Unavailable,
        advice: "手势 + worklet 动画方案",
    },
    Blocked {
        tag: "editor",
        loss: Loss::Unavailable,
        advice: "该组件在 Skyline 下不可用",
    },
    // 盒子与 CSS 底纹照常渲染，缺的是原生进度条填充（见 `skyline/component-support` 的视觉用例）。
    Blocked {
        tag: "progress",
        loss: Loss::PaintingLost,
        advice: "用 `view` + 百分比宽度自绘进度条",
    },
    Blocked {
        tag: "match-media",
        loss: Loss::Unavailable,
        advice: "用 `wx.getWindowInfo` 自己判断后渲染",
    },
    // 知识库把该组件拼成 `keyboard-accessary`（官方名为 keyboard-accessory），
    // 我们按官方名收录，否则真实使用永远命中不了。
    Blocked {
        tag: "keyboard-accessory",
        loss: Loss::Unavailable,
        advice: "input 的 worklet:onkeyboardheightchange 回调",
    },
    Blocked {
        tag: "navigation-bar",
        loss: Loss::Unavailable,
        advice: r#"Skyline 只能用自定义导航（页面配置 `"navigationStyle": "custom"`）"#,
    },
    Blocked {
        tag: "xr-frame",
        loss: Loss::Unavailable,
        advice: "该组件在 Skyline 下不可用",
    },
];

/// 黑名单命中才返回整行；其余内置组件与未知自定义组件都返回 `None`。
pub(crate) fn blocked(tag: &str) -> Option<&'static Blocked> {
    BLOCKED_COMPONENTS.iter().find(|row| row.tag == tag)
}

/// 组件是否属于平台内置组件（含 Skyline 下不可用的那些）。
///
/// 「是内置组件」与「Skyline 能渲染」是两件事：`web-view` / `movable-view` 这类
/// 仍是平台组件，只是 Skyline 不支持。结构规则判「是不是自定义组件边界」看它，
/// 判「Skyline 能不能用」看 [`blocked`]。
pub(crate) fn is_builtin(tag: &str) -> bool {
    SUPPORTED_COMPONENTS.contains(&tag) || blocked(tag).is_some()
}

#[cfg(test)]
mod tests {
    use super::{
        CAPABILITIES, CssBox, Loss, SUPPORTED_COMPONENTS, blocked, capability, is_builtin,
    };

    /// 能力表的三档都要有行，档位才不会退化成死代码。
    #[test]
    fn every_css_box_tier_has_a_row() {
        for css_box in [CssBox::Full, CssBox::BorderBoxTail, CssBox::None] {
            assert!(
                CAPABILITIES.iter().any(|row| row.css_box == css_box),
                "{css_box:?} 没有任何组件"
            );
        }
    }

    /// 能力表只描述内置组件，且同一个标签不重复。
    #[test]
    fn capability_rows_are_unique_builtins() {
        let mut seen = std::collections::HashSet::new();
        for row in CAPABILITIES {
            assert!(
                SUPPORTED_COMPONENTS.contains(&row.tag),
                "{} 不是内置组件",
                row.tag
            );
            assert!(seen.insert(row.tag), "{} 重复", row.tag);
        }
        assert_eq!(capability("swiper-item").unwrap().css_box, CssBox::Full);
        assert!(capability("rich-text").is_none());
    }

    /// 引擎节点工厂注册的内置组件
    /// 必须在白名单里，否则结构规则会把它们当成自定义组件边界而跳过内部约束。
    #[test]
    fn engine_builtin_components_are_known() {
        for tag in ["open-container", "list-builder", "grid-builder"] {
            assert!(is_builtin(tag), "{tag} should be builtin");
        }
    }

    /// 黑名单的三档后果都有组件落在上面，档位才不会退化成死代码。
    #[test]
    fn every_loss_has_a_row() {
        for loss in [Loss::Unavailable, Loss::PaintingLost, Loss::PageOptOut] {
            assert!(
                super::BLOCKED_COMPONENTS.iter().any(|row| row.loss == loss),
                "{loss:?} 没有任何组件"
            );
        }
        assert_eq!(blocked("progress").unwrap().loss, Loss::PaintingLost);
        assert_eq!(blocked("web-view").unwrap().loss, Loss::PageOptOut);
    }
}
