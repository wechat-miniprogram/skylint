//! 内置组件在 Skyline 里「有没有 CSS 盒」的分类表，`native-padding` 与
//! `native-layout-ignored` 共用同一份事实。
//!
//! 判据：子节点是否被 `SetAsRoot()` 成独立布局根。
//!
//! - `shadow/shadow_node.cc:195-203`：父 widget 链尾不是 `YGConnector` 就 `SetAsRoot()`；
//! - `shadow/shadow_node.cc:211-226`：`CreateConcreteWidget()` 返回空才建 `YGConnector`；
//! - `widgets/yoga_connector.cc:95-107`：全仓唯一把子 yoga 节点挂进父树的入口；
//! - `shadow/shadow_node.cc:269-277`：`InflateNaked()` 只塞 concrete widget，连 yoga 节点都不建；
//! - `widgets/yoga.cc:616-624` + `package/skyline/lib/widgets/yoga.dart:575-584`：concrete
//!   widget 拿到的约束是 border box，padding 没有扣减机会。
//!
//! 排除 `swiper-item`：它 `CreateConcreteWidget()` 返回空 → 走 `YGConnector`，子节点进它
//! 自己的 yoga 树，CSS 正常生效（`shadow/components/swiper_item_shadow_node.cc:30-32`）。

/// 组件自身还有多少 CSS 能用。
pub(crate) enum NativeKind {
    /// 没有 CSS 盒（`InflateNaked` / `SetSkipStyleBuild`）。`background` 表示背景色是否
    /// 还能画出来：list/grid 四件套在 style builder 的白名单里只剩背景色
    /// （`style/style_builder.cc:155-166`、`:446-535`），sticky/nested-scroll 连
    /// computed style 都不建（`shadow/context.cc:867`）。
    NoCssBox { background: bool },
    /// 有 yoga 节点、padding 也写进去了（`style_builder.cc:648-670`），但 concrete
    /// widget 是 widget 链尾，子节点仍是独立布局根。
    BorderBoxTail,
}

pub(crate) struct Native {
    pub tag: &'static str,
    pub kind: NativeKind,
    /// 组件是否暴露 `padding` 属性：`wxapplib/src/skyline/src/components/wx-custom-scroll-view/{index,list-builder,grid-builder}.ts`
    pub padding_attr: bool,
}

impl NativeKind {
    /// 这个组件连 yoga 节点都没有：CSS 布局属性整体失效。
    pub fn ignores_layout(&self) -> bool {
        matches!(self, Self::NoCssBox { .. })
    }
}

pub(crate) const NATIVES: &[Native] = &[
    Native {
        tag: "list-view",
        kind: NativeKind::NoCssBox { background: true },
        padding_attr: true,
    },
    Native {
        tag: "grid-view",
        kind: NativeKind::NoCssBox { background: true },
        padding_attr: true,
    },
    Native {
        tag: "list-builder",
        kind: NativeKind::NoCssBox { background: true },
        padding_attr: true,
    },
    Native {
        tag: "grid-builder",
        kind: NativeKind::NoCssBox { background: true },
        padding_attr: true,
    },
    Native {
        tag: "sticky-section",
        kind: NativeKind::NoCssBox { background: false },
        padding_attr: true,
    },
    Native {
        tag: "sticky-header",
        kind: NativeKind::NoCssBox { background: false },
        padding_attr: true,
    },
    Native {
        tag: "nested-scroll-header",
        kind: NativeKind::NoCssBox { background: false },
        padding_attr: false,
    },
    Native {
        tag: "nested-scroll-body",
        kind: NativeKind::NoCssBox { background: false },
        padding_attr: false,
    },
    Native {
        tag: "scroll-view",
        kind: NativeKind::BorderBoxTail,
        padding_attr: true,
    },
    Native {
        tag: "swiper",
        kind: NativeKind::BorderBoxTail,
        padding_attr: false,
    },
    Native {
        tag: "draggable-sheet",
        kind: NativeKind::BorderBoxTail,
        padding_attr: false,
    },
    Native {
        tag: "picker-view-column",
        kind: NativeKind::BorderBoxTail,
        padding_attr: false,
    },
];
