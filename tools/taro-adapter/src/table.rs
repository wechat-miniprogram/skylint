//! Taro 组件名 → WXML 标签、JSX 属性名 → WXML 属性名。
//!
//! 这两张表只覆盖「规则会读的语义」，不是 Taro 组件表的完整复制：
//! - 标签映射决定结构规则能不能把元素认成内置组件；
//! - 属性名映射决定三态 flag / 属性白名单看得到什么；
//! - [`DEFAULTS`] 只补运行时一定会出现、且规则会据此下结论的默认值。

/// `@tarojs/components` 导出名 → WXML 标签。
///
/// 列表对照 Taro 4.x 的 `packages/taro-components/types` 与
/// `packages/taro-platform-weapp/src/components.ts`。没列到的名字按自定义组件处理。
pub const TAGS: &[(&str, &str)] = &[
    ("Ad", "ad"),
    ("Audio", "audio"),
    ("Block", "block"),
    ("Button", "button"),
    ("Camera", "camera"),
    ("Canvas", "canvas"),
    ("Checkbox", "checkbox"),
    ("CheckboxGroup", "checkbox-group"),
    ("CoverImage", "cover-image"),
    ("CoverView", "cover-view"),
    ("DraggableSheet", "draggable-sheet"),
    ("Editor", "editor"),
    ("Form", "form"),
    ("FunctionalPageNavigator", "functional-page-navigator"),
    ("GridView", "grid-view"),
    ("Icon", "icon"),
    ("Image", "image"),
    ("Input", "input"),
    ("KeyboardAccessory", "keyboard-accessory"),
    ("Label", "label"),
    ("ListView", "list-view"),
    ("LivePlayer", "live-player"),
    ("LivePusher", "live-pusher"),
    ("Map", "map"),
    ("MatchMedia", "match-media"),
    ("MovableArea", "movable-area"),
    ("MovableView", "movable-view"),
    ("NavigationBar", "navigation-bar"),
    ("Navigator", "navigator"),
    ("OfficialAccount", "official-account"),
    ("OpenData", "open-data"),
    ("PageContainer", "page-container"),
    ("PageMeta", "page-meta"),
    ("Picker", "picker"),
    ("PickerView", "picker-view"),
    ("PickerViewColumn", "picker-view-column"),
    ("Progress", "progress"),
    ("Radio", "radio"),
    ("RadioGroup", "radio-group"),
    ("RichText", "rich-text"),
    ("RootPortal", "root-portal"),
    ("ScrollView", "scroll-view"),
    ("ShareElement", "share-element"),
    ("Slider", "slider"),
    ("Slot", "slot"),
    ("Snapshot", "snapshot"),
    ("Span", "span"),
    ("StickyHeader", "sticky-header"),
    ("StickySection", "sticky-section"),
    ("Swiper", "swiper"),
    ("SwiperItem", "swiper-item"),
    ("Switch", "switch"),
    ("Tabbar", "tabbar"),
    ("TabbarItem", "tabbar-item"),
    ("Text", "text"),
    ("Textarea", "textarea"),
    ("Video", "video"),
    ("View", "view"),
    ("WebView", "web-view"),
    ("TapGestureHandler", "tap-gesture-handler"),
    ("DoubleTapGestureHandler", "double-tap-gesture-handler"),
    ("ForceTouchGestureHandler", "force-touch-gesture-handler"),
    ("HorizontalDragGestureHandler", "horizontal-drag-gesture-handler"),
    ("LongPressGestureHandler", "long-press-gesture-handler"),
    ("PanGestureHandler", "pan-gesture-handler"),
    ("PinchGestureHandler", "pinch-gesture-handler"),
    ("RotationGestureHandler", "rotation-gesture-handler"),
    ("VerticalDragGestureHandler", "vertical-drag-gesture-handler"),
];

/// 组件 → 运行时一定会写进 WXML 的默认属性（`base.wxml` 里是
/// `type="{{i.p37||'list'}}"` 这种形式）。只收规则会据此下结论的项：
/// `scroll-view` 没写 `type` 时 Taro 运行时按 `list` 渲染，若合成树不补，
/// `skyline/scroll-view-type` 会对每个 scroll-view 误报。
pub const DEFAULTS: &[(&str, &[(&str, &str)])] =
    &[("scroll-view", &[("type", "list")])];

/// JSX 属性名（camelCase）→ WXML 属性名（kebab-case）。
///
/// 绝大多数是机械转换；这里列的是转换规则之外的特例。
pub const RENAMES: &[(&str, &str)] = &[
    ("className", "class"),
    ("hoverClass", "hover-class"),
    ("hoverStartTime", "hover-start-time"),
    ("hoverStayTime", "hover-stay-time"),
    ("hoverStopPropagation", "hover-stop-propagation"),
    ("scrollX", "scroll-x"),
    ("scrollY", "scroll-y"),
    ("scrollTop", "scroll-top"),
    ("scrollLeft", "scroll-left"),
    ("scrollIntoView", "scroll-into-view"),
    ("scrollWithAnimation", "scroll-with-animation"),
    ("scrollAnchoring", "scroll-anchoring"),
    ("enableBackToTop", "enable-back-to-top"),
    ("enableFlex", "enable-flex"),
    ("enablePassive", "enable-passive"),
    ("maxLines", "max-lines"),
    ("pagingEnabled", "paging-enabled"),
    ("refresherEnabled", "refresher-enabled"),
    ("refresherTriggered", "refresher-triggered"),
    ("showScrollbar", "show-scrollbar"),
    ("fastDeceleration", "fast-deceleration"),
    ("associativeContainer", "associative-container"),
    ("cacheExtent", "cache-extent"),
    ("minDragDistance", "min-drag-distance"),
    ("nativeView", "native-view"),
];

pub fn tag_for(name: &str) -> Option<&'static str> {
    TAGS
        .iter()
        .find(|(component, _)| *component == name)
        .map(|(_, tag)| *tag)
}

pub fn defaults_for(tag: &str) -> &'static [(&'static str, &'static str)] {
    DEFAULTS
        .iter()
        .find(|(name, _)| *name == tag)
        .map(|(_, attrs)| *attrs)
        .unwrap_or(&[])
}

/// 属性名 / 事件名共用的大小写转换：`hoverClass` → `hover-class`。
pub fn kebab(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (index, ch) in name.char_indices() {
        if ch.is_ascii_uppercase() {
            if index > 0 {
                out.push('-');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// JSX 事件属性 → WXML 事件名。
///
/// 对照 Taro 编译模式插件的 `identify_jsx_event_key`
/// （`crates/swc_plugin_compile_mode/src/utils/mod.rs`）：去掉 `on` 前缀后全小写，
/// `click` 是唯一特例（内置组件上对应 `tap`）；`XxxWorklet` 不以 `on` 开头时按
/// kebab 写进 `worklet:` 命名空间。
pub fn event_name(jsx: &str) -> Option<String> {
    if let Some(base) = jsx.strip_suffix("Worklet") {
        return Some(match base.strip_prefix("on") {
            Some(rest) => format!("worklet:{}", rest.to_lowercase()),
            None => format!("worklet:{}", kebab(base)),
        });
    }
    if !(jsx.len() > 2 && jsx.starts_with("on") && jsx.as_bytes()[2].is_ascii_uppercase()) {
        return None;
    }
    let lower = jsx[2..].to_lowercase();
    let event = if lower == "click" { "tap" } else { lower.as_str() };
    Some(format!("bind{event}"))
}

/// WXML 属性名。
pub fn attr_name(jsx: &str) -> Option<String> {
    if let Some((_, renamed)) = RENAMES.iter().find(|(from, _)| *from == jsx) {
        return Some((*renamed).to_string());
    }
    // `compileMode` 是 Taro 的编译开关，不进入 WXML。
    if jsx == "compileMode" {
        return None;
    }
    Some(kebab(jsx))
}
