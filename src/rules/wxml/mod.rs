//! WXML 规则。
//!
//! 一条规则一个文件：逐元素的局部判定（内联文本、组件白名单、scroll-view type）
//! 与父子结构约束（scroll-view 家族、enable-flex）各占一个文件。它们共用同一套
//! 「页面 + 组件 WXML 遍历」与解析辅助。
//!
//! 样式求值对两种文档统一：单元（页面 / 组件）按自己的作用域求值，独立模板文件与
//! `<include>` 片段按**引用它的单元**求值（[`cascade_entries`] 是唯一口径）。

pub(crate) mod ancestry;
pub mod branch;
pub(crate) mod cascade;
pub mod component_attribute;
pub mod component_children;
pub mod horizontal_scroll_enable_flex;
pub mod inline_run;
pub mod no_inline_text;
pub mod scroll_gesture_conflict;
pub mod scroll_view_family;
pub mod scroll_view_type;
pub mod share_element_key;
pub(crate) mod structure;
pub mod template_data_scope;
pub mod text_hard_break_ellipsis;
pub(crate) mod truncation;
pub mod unsupported_component;
pub mod webview_nested_scroll;
pub mod webview_pinned_renderer;
pub mod webview_text_children;
pub mod webview_text_truncation;
pub mod webview_worklet_binding;

pub use component_attribute::ComponentAttribute;
pub use component_children::ComponentChildren;
pub use horizontal_scroll_enable_flex::HorizontalScrollEnableFlex;
pub use inline_run::InlineRun;
pub use no_inline_text::NoInlineText;
pub use scroll_gesture_conflict::ScrollGestureConflict;
pub use scroll_view_family::ScrollViewFamily;
pub use scroll_view_type::ScrollViewType;
pub use share_element_key::ShareElementKey;
pub use template_data_scope::TemplateDataScope;
pub use text_hard_break_ellipsis::TextHardBreakEllipsis;
pub use unsupported_component::UnsupportedComponent;
pub use webview_nested_scroll::NestedScroll;
pub use webview_pinned_renderer::PinnedRenderer;
pub use webview_text_children::TextChildren;
pub use webview_text_truncation::TextTruncation;
pub use webview_worklet_binding::WorkletBinding;

// 级联是「元素 + 属性 → 取值」的唯一入口；规则模块只调它，不自己筛样式表。
pub(crate) use cascade::{
    Resolved, cascade_entries, cascade_entries_with, matched_declarations, resolved_property,
    resolved_property_with, resolved_property_with_dialect,
};

// 截断语义单独一个模块（[`truncation`]）：两条迁移规则与 `inspect` 的事实输出共用，
// 规则模块不再各自复制一份。
pub(crate) use truncation::{clips_overflow, max_lines};

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use glass_easel_template_compiler::parse::Position;

use crate::diag::{Diagnostic, Severity};
use crate::input::wxml::{WxmlDoc, WxmlElement, WxmlNode};
use crate::matcher::Tree;
use crate::project::{Project, Scope};

/// 页面与组件的 WXML + 被 import / include 的模板，按路径去重。
///
/// 页面 json 显式声明 `"renderer": "webview"` 的整页跳过：那是「这页不迁
/// Skyline」的明确决定（典型是含 `<web-view>` 的页面），对它讲 Skyline 的
/// 限制只是误报。app 级 webview / 页面未表态不跳——那只是还没迁，正是要检查的。
/// 组件的 wxml 没有 renderer 可言，一律检查；模板不属于任何单元，也一律检查。
pub(crate) fn wxml_files(project: &Project) -> impl Iterator<Item = &str> {
    let mut seen = HashSet::new();
    project
        .pages
        .iter()
        .filter(|page| !page.opted_out)
        .filter_map(|page| page.files.wxml.as_deref())
        .chain(
            project
                .components
                .iter()
                .filter_map(|component| component.files.wxml.as_deref()),
        )
        .chain(project.template_files.iter().map(String::as_str))
        .filter(move |path| seen.insert(*path))
}

/// 一份文档在 Skyline 下的可达性。
///
/// 一个组件可能被 Skyline 页面和 `renderer: "webview"` 页面同时引用——只要有一处能
/// 在 Skyline 下渲染到，它就按 [`Reach::Skyline`] 处理（诊断是全局的，不该因为
/// 某个 WebView 页面也用了它而降级）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reach {
    Skyline,
    /// 只被 `renderer: "webview"` 的页面（传递）引用，或它自己就是这类页面的内容。
    WebviewOnly,
}

impl Reach {
    /// 把可达性说明接到 `help` 后面；Skyline 可达时原样返回。
    pub(crate) fn help(self, base: &str) -> String {
        match self {
            Self::Skyline => base.to_string(),
            Self::WebviewOnly => format!(
                "{base}（这份 WXML 目前只在 WebView 下渲染：页面声明了 `renderer: \"webview\"` 或只被这类页面引用，迁移到 Skyline 后才全生效）"
            ),
        }
    }
}

/// 一份 WXML 文档与它在 Skyline 下的可达性。
pub(crate) struct ElementDoc {
    pub doc: Rc<WxmlDoc>,
    pub reach: Reach,
}

/// 元素级规则能看到的文档：全部页面、组件，与被 `<include>` / `<import>` 引用的模板。
///
/// 元素级规则曾经只走「页面 + 直接引用的组件」，于是只被 `renderer: "webview"` 页面引用的
/// 组件永远进不了检查，而同一份文件的声明级规则照报——同一个组件一半报一半不报。
/// 这里对所有文档求值，再用 [`Reach`] 把「WebView 専属」在 `help` 里说明。
///
/// 可达性按**渲染树**算（[`WxmlGraph::render_docs`] 沿真实标签用法展开），不是按
/// `usingComponents` 声明算：声明了但没人用的组件不该被算成 Skyline 可达。同一份
/// 组件被 Skyline 页面与 WebView 页面同时引用时按 Skyline 处理（只要有一处能渲染到）。
///
/// `<import>` 的模板定义体不进 `render_docs`（它只沿渲染树走），但定义体确实渲染在每个
/// `<template is>` 调用处，所以按 [`Project::template_scopes`] 补进来；只收被某个能渲染到
/// 的单元引用过的模板，死模板属于「未使用代码」，不在本工具范围。
pub(crate) fn element_docs(project: &Project) -> Vec<ElementDoc> {
    let graph = project.graph();

    let mut reach: HashMap<&str, Reach> = HashMap::new();
    for page in &project.pages {
        let Some(page_wxml) = page.files.wxml.as_deref() else {
            continue;
        };
        // opt-out 页面照样可能用到受检查的组件，只是标成「仅在 WebView 下渲染」。
        let page_reach = if page.opted_out {
            Reach::WebviewOnly
        } else {
            Reach::Skyline
        };
        for path in graph.render_docs(page_wxml) {
            let entry = reach.entry(path).or_insert(page_reach);
            if page_reach == Reach::Skyline {
                *entry = Reach::Skyline;
            }
        }
    }

    // 模板文件没有自己的作用域：`<include>` 的整段内容已由 `render_docs` 带上，
    // `<import>` 的定义体按引用它的单元取可达性。单元自己的 WXML 不在 `reach` 里
    // （未被任何页面渲染到）就不补，避免把死组件引用的模板算成检查对象。
    for (path, scopes) in &project.template_scopes {
        let mut template_reach = None;
        for scope in scopes {
            let Some(unit) = unit_wxml(project, scope) else {
                continue;
            };
            match reach.get(unit) {
                Some(Reach::Skyline) => {
                    template_reach = Some(Reach::Skyline);
                    break;
                }
                Some(Reach::WebviewOnly) => template_reach = Some(Reach::WebviewOnly),
                None => {}
            }
        }
        let Some(template_reach) = template_reach else {
            continue;
        };
        let entry = reach.entry(path.as_str()).or_insert(template_reach);
        if template_reach == Reach::Skyline {
            *entry = Reach::Skyline;
        }
    }

    // 顺序：页面自己的 WXML 按页面顺序在前，其余（组件、片段）按路径排序，保证输出稳定。
    let mut ordered: Vec<&str> = Vec::new();
    let mut seen = HashSet::new();
    for page in &project.pages {
        if let Some(path) = page.files.wxml.as_deref()
            && seen.insert(path)
        {
            ordered.push(path);
        }
    }
    let mut rest: Vec<&str> = reach
        .keys()
        .copied()
        .filter(|path| !seen.contains(path))
        .collect();
    rest.sort_unstable();
    ordered.extend(rest);

    ordered
        .into_iter()
        .filter_map(|path| {
            let doc = load_doc(project, path)?;
            let reach = *reach.get(path).unwrap_or(&Reach::WebviewOnly);
            Some(ElementDoc { doc, reach })
        })
        .collect()
}

/// 作用域对应的单元 WXML，**包含 opt-out 页面**：它们仍然会渲染（只是不迁 Skyline），
/// 由 [`Reach`] 统一标注，不能在这里提前丢掉。
fn unit_wxml<'a>(project: &'a Project, scope: &Scope) -> Option<&'a str> {
    match scope {
        Scope::Page(route) => project
            .pages
            .iter()
            .find(|page| &page.route == route)
            .and_then(|page| page.files.wxml.as_deref()),
        Scope::Component(path) => project
            .components
            .iter()
            .find(|unit| &unit.path == path)
            .and_then(|unit| unit.files.wxml.as_deref()),
        Scope::Global | Scope::Loose(_) => None,
    }
}

pub(crate) fn load_doc(project: &Project, path: &str) -> Option<Rc<WxmlDoc>> {
    project.cached_doc(path, false)
}

/// 用空常量环境解析：保留所有 `wx:if` 分支，用于「只数引用、不看可达」的检查。
///
/// 位置与折叠后的文档完全一致（折叠只丢分支，不移动元素）。
pub(crate) fn load_doc_unfolded(project: &Project, path: &str) -> Option<Rc<WxmlDoc>> {
    project.cached_doc(path, true)
}

/// 遍历全部待检查的 WXML 文件（页面/组件单元 + 被 import/include 的模板）。
/// 单个文件读失败或解析失败一律跳过，不终止扫描。
pub(crate) fn visit(
    project: &Project,
    mut f: impl FnMut(&WxmlDoc, &mut Vec<Diagnostic>),
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for path in wxml_files(project) {
        if let Some(doc) = load_doc(project, path) {
            f(&doc, &mut out);
        }
    }
    out
}

/// 深度优先遍历全部元素，并维护从根到父节点的祖先栈。
///
/// 不做任何过滤（包括 `in_template`）：是否跳过由调用方决定。
pub(crate) fn walk<'a>(
    doc: &'a WxmlDoc,
    mut visit: impl FnMut(&'a WxmlElement, &[&'a WxmlElement]),
) {
    fn recurse<'a>(
        nodes: &'a [WxmlNode],
        ancestors: &mut Vec<&'a WxmlElement>,
        visit: &mut impl FnMut(&'a WxmlElement, &[&'a WxmlElement]),
    ) {
        for node in nodes {
            let WxmlNode::Element(element) = node else {
                continue;
            };
            visit(element, ancestors);
            ancestors.push(element);
            recurse(&element.children, ancestors, visit);
            ancestors.pop();
        }
    }

    let mut ancestors = Vec::new();
    recurse(&doc.nodes, &mut ancestors, &mut visit);
}

/// Skyline 方向的遍历：跳过只会在 WebView 下渲染的元素。
///
/// `wx:if="{{renderer === 'skyline'}}"` 的 `wx:else` 分支只在 WebView 下成立，
/// 对它讲「Skyline 下会怎样」是误报。WebView 方向用 [`walk`]。
pub(crate) fn walk_skyline(doc: &WxmlDoc, mut visit: impl FnMut(&WxmlElement, &[&WxmlElement])) {
    walk(doc, |element, ancestors| {
        if !element.webview_only() {
            visit(element, ancestors);
        }
    });
}

/// 元素是否脱离文档流（`position: absolute | fixed`）。
pub(crate) fn is_out_of_flow(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> bool {
    resolved_property(project, tree, element, "position").is_some_and(|resolved| {
        matches!(
            resolved.value.trim().to_ascii_lowercase().as_str(),
            "fixed" | "absolute"
        )
    })
}

/// 元素是否自己声明了纵向占位：固定宽度或纵向 margin。
///
/// 作者这么写就已经表达了「各占一行」，并列关系不是无意中形成的。`auto` / `0` /
/// `none` 不算（那些不产生纵向块）。
pub(crate) fn stacks_vertically(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> bool {
    ["width", "margin-top", "margin-bottom"]
        .iter()
        .filter_map(|property| resolved_property(project, tree, element, property))
        .any(|resolved| {
            let value = resolved.value.trim();
            !matches!(value, "0" | "0px" | "0rpx" | "auto" | "none")
        })
}

/// 元素的 `width` 是否本来就由内容决定，`fit-content` 被丢弃也不改变渲染。
///
/// 两种情况：自身脱离文档流（`position: absolute | fixed`），或父级是行向 flex 容器
/// 且自己不参与拉伸——`flex-basis: auto` 下主轴尺寸就是内容尺寸，与 `fit-content`
/// 同解。写了 `flex` / `flex-grow` 就不算：那会让元素按剩余空间拉伸。
///
/// 方向缺省按引擎的 `ApplyInitial` 走：普通模式下是 `column`
/// （`skyline/css/css_properties/flex_direction.cc:39-48`），列向容器里 `width` 是
/// 交叉轴、`auto` 会被拉伸；开了 `defaultDisplayBlock` 才回到 `row`——与
/// `scroll_view_height::flex_axis` 同一口径。那个开关同时让没写 `display` 的父级
/// 变成 `block`（`flex_container` 返回 false），`width: auto` 撑满父级，同样不按
/// 内容收缩。
pub(crate) fn width_follows_content(
    project: &Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
) -> bool {
    if is_out_of_flow(project, tree, element) {
        return true;
    }
    let Some(parent) = tree.parent_of(element) else {
        return false;
    };
    if !flex_container(project, tree, parent) {
        return false;
    }
    let default_block = project.scope_of(tree.path()).map_or_else(
        || project.app_display_block(),
        |scope| project.renders_as_block(&scope),
    );
    let row = resolved_property(project, tree, parent, "flex-direction").map_or(
        default_block,
        |resolved| {
            matches!(
                resolved.value.trim().to_ascii_lowercase().as_str(),
                "row" | "row-reverse"
            )
        },
    );
    row && ["flex", "flex-grow"]
        .iter()
        .all(|name| resolved_property(project, tree, element, name).is_none())
}

/// 元素的 `display` 是否算成 flex 容器（取级联最后一个近似值）。
///
/// 与 [`display_values`] 的区别：这里要的是「最终算出哪个值」，同一元素命中
/// 多份互相矛盾的 `display` 时按样式表顺序取最后一个，让各规则口径一致。
pub(crate) fn flex_container(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> bool {
    resolved_property(project, tree, element, "display")
        .is_some_and(|resolved| is_flex_value(&resolved.value))
}

/// `display: flex` / `inline-flex`：文本与元素在 WebView 与 Skyline 下都是 flex item。
pub(crate) fn is_flex_value(value: &str) -> bool {
    let value = value.trim();
    value.eq_ignore_ascii_case("flex") || value.eq_ignore_ascii_case("inline-flex")
}

/// Skyline 组件支持表。
///
/// 来源：skyline-skills/skills/skyline-overview/references/introduction/component-support.md
/// 同步方式：按「完全支持的组件」「基本支持的组件」「Skyline 新增组件」更新本表，
/// 按「暂不支持的组件」更新 `UNSUPPORTED_COMPONENTS`；替代方案为空时填「该组件在 Skyline 下不可用」。
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

/// Skyline 下不可用的组件及替代方案。来源同 `SUPPORTED_COMPONENTS`。
const UNSUPPORTED_COMPONENTS: &[(&str, &str)] = &[
    ("web-view", r#"该页面配置 `"renderer": "webview"`"#),
    ("movable-area", "手势 + worklet 动画方案"),
    ("movable-view", "手势 + worklet 动画方案"),
    ("editor", "该组件在 Skyline 下不可用"),
    ("progress", "该组件在 Skyline 下不可用"),
    ("match-media", "该组件在 Skyline 下不可用"),
    // 知识库 component-support.md:63 把该组件拼成 `keyboard-accessary`（官方名为
    // keyboard-accessory，developers.weixin.qq.com/miniprogram/dev/component/keyboard-accessory.html），
    // 我们按官方名收录，否则真实使用永远命中不了。
    (
        "keyboard-accessory",
        "input 的 worklet:onkeyboardheightchange 回调",
    ),
    ("navigation-bar", "Skyline 只能用自定义导航"),
    ("xr-frame", "该组件在 Skyline 下不可用"),
];

/// 组件是否属于内置组件（用于区分自定义组件）。
pub(crate) fn is_builtin(tag: &str) -> bool {
    SUPPORTED_COMPONENTS.contains(&tag)
}

/// 黑名单命中才返回替代方案；白名单与未知自定义组件都不报。
pub(crate) fn unsupported_replacement(tag: &str) -> Option<&'static str> {
    if is_builtin(tag) {
        return None;
    }
    UNSUPPORTED_COMPONENTS
        .iter()
        .find(|(name, _)| *name == tag)
        .map(|(_, replacement)| *replacement)
}

/// 定位在 WXML 源文本任意一段区间上的诊断。
pub(crate) fn span_diagnostic(
    id: &'static str,
    doc: &WxmlDoc,
    start: Position,
    end: Position,
    path: &str,
    severity: Severity,
    message: impl Into<String>,
) -> Diagnostic {
    let (line, column) = doc.position(start);
    let (end_line, end_column) = doc.position(end);
    Diagnostic::new(id, severity, message, path, line, column).end(end_line, end_column)
}

/// 定位在元素标签起止位置上的诊断。
pub(crate) fn element_diagnostic(
    id: &'static str,
    doc: &WxmlDoc,
    element: &WxmlElement,
    path: &str,
    severity: Severity,
    message: impl Into<String>,
) -> Diagnostic {
    span_diagnostic(
        id,
        doc,
        element.tag_start,
        element.tag_end,
        path,
        severity,
        message,
    )
}

#[cfg(test)]
mod tests {
    use super::{Reach, element_docs, is_builtin};
    use crate::matcher::Tree;
    use crate::rules::testutil::Fixture;

    /// `width_follows_content` 看的是 fixture 里 `class="x"` 的那个元素。
    fn follows_content_in(fixture: Fixture) -> bool {
        let project = fixture.project();
        let path = project.pages[0].files.wxml.clone().unwrap();
        let doc = super::load_doc(&project, &path).unwrap();
        let tree = Tree::new(&doc);
        let element = doc
            .elements()
            .find(|element| {
                element.attributes.iter().any(|attribute| {
                    attribute.name == "class" && attribute.value.as_deref() == Some("x")
                })
            })
            .expect("fixture 里没有 class=x 的元素");
        super::width_follows_content(&project, &tree, element)
    }

    fn follows_content(name: &str, wxml: &str, wxss: &str) -> bool {
        follows_content_in(Fixture::page(name, wxml, wxss))
    }

    fn default_block_fixture(name: &str, wxml: &str, wxss: &str) -> Fixture {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"defaultDisplayBlock":true}}}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", wxml)
            .write("pages/index/index.wxss", wxss);
        fixture
    }

    /// Skyline 的 `flex-direction` 初始值是 `column`（`flex_direction.cc:39-48`）：
    /// `display: flex` 不带方向时 `width` 落在交叉轴、`auto` 会被拉伸，与
    /// `fit-content` 不同解，不能豁免。
    #[test]
    fn width_follows_content_defaults_to_column() {
        assert!(
            !follows_content(
                "wfc-default-column",
                r#"<view class="p"><view class="x"/></view>"#,
                ".p { display: flex }\n.x { width: fit-content }",
            ),
            "缺省方向是 column，应判为不按内容收缩"
        );
        assert!(
            !follows_content(
                "wfc-explicit-column",
                r#"<view class="p"><view class="x"/></view>"#,
                ".p { display: flex; flex-direction: column }\n.x { width: fit-content }",
            ),
            "显式 column 同样在交叉轴"
        );
        assert!(
            follows_content(
                "wfc-explicit-row",
                r#"<view class="p"><view class="x"/></view>"#,
                ".p { display: flex; flex-direction: row }\n.x { width: fit-content }",
            ),
            "行向主轴上的 auto 才等于内容宽"
        );
    }

    /// `defaultDisplayBlock: true` 时没写 `display` 的父级是 block，不是 flex：
    /// `width: auto` 撑满父级，同样不按内容收缩。
    #[test]
    fn width_follows_content_sees_a_block_parent_under_default_display_block() {
        assert!(
            !follows_content_in(default_block_fixture(
                "wfc-default-block",
                r#"<view class="p"><view class="x"/></view>"#,
                ".p { }\n.x { width: fit-content }",
            )),
            "block 父级下 width: auto 是拉伸"
        );
    }

    /// 同一个开关把缺省 `flex-direction` 拉回 row（`FlexDirection::ApplyInitial`）：
    /// 父级显式 `display: flex` 时仍是行向主轴，按内容收缩。
    #[test]
    fn width_follows_content_defaults_to_row_under_default_display_block() {
        assert!(
            follows_content_in(default_block_fixture(
                "wfc-default-block-row",
                r#"<view class="p"><view class="x"/></view>"#,
                ".p { display: flex }\n.x { width: fit-content }",
            )),
            "defaultDisplayBlock 下缺省方向是 row"
        );
    }

    /// 三种引用形态：只被 Skyline 页面用、只被 WebView 页面用、两边都用。
    #[test]
    fn component_reach_follows_the_pages_that_render_it() {
        let fixture = Fixture::new("reach-three-kinds");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/web/index","pages/sky/index"],"rendererOptions":{"skyline":{"defaultDisplayBlock":true}}}"#,
            )
            // Skyline 页（未表态 = 默认迁移目标）与显式 WebView 页。
            .write("pages/sky/index.json", "{}")
            .write(
                "pages/sky/index.wxml",
                r#"<sky-only/><both/><outer/>"#,
            )
            .write("pages/sky/index.js", "Page({});")
            .write("pages/web/index.json", r#"{"renderer":"webview"}"#)
            .write("pages/web/index.wxml", r#"<web-only/><both/>"#)
            .write("pages/web/index.js", "Page({});")
            .write(
                "pages/sky/index.json",
                r#"{"usingComponents":{"sky-only":"/components/sky-only/sky-only","both":"/components/both/both","outer":"/components/outer/outer"}}"#,
            )
            .write(
                "pages/web/index.json",
                r#"{"renderer":"webview","usingComponents":{"web-only":"/components/web-only/web-only","both":"/components/both/both"}}"#,
            );
        for name in ["sky-only", "both", "web-only", "outer", "inner"] {
            let json = if name == "outer" {
                r#"{"component":true,"usingComponents":{"inner":"/components/inner/inner"}}"#
            } else {
                r#"{"component":true}"#
            };
            fixture
                .write(&format!("components/{name}/{name}.json"), json)
                .write(&format!("components/{name}/{name}.wxml"), "<view/>")
                .write(&format!("components/{name}/{name}.js"), "Component({})");
        }

        let project = fixture.project();
        let reach: std::collections::HashMap<String, Reach> = element_docs(&project)
            .into_iter()
            .map(|doc| (doc.doc.path().to_string(), doc.reach))
            .collect();

        assert_eq!(reach["components/sky-only/sky-only.wxml"], Reach::Skyline);
        // 两边都被引用：只要有一处能在 Skyline 下渲染就算 Skyline。
        assert_eq!(reach["components/both/both.wxml"], Reach::Skyline);
        assert_eq!(
            reach["components/web-only/web-only.wxml"],
            Reach::WebviewOnly
        );
        // 传递引用：页面 → outer → inner。
        assert_eq!(reach["components/outer/outer.wxml"], Reach::Skyline);
        assert_eq!(reach["components/inner/inner.wxml"], Reach::Skyline);
        assert_eq!(reach["pages/web/index.wxml"], Reach::WebviewOnly);
    }

    /// `<import>` 的模板定义体不进 `render_docs`，但定义体确实在 `<template is>` 调用处
    /// 渲染：元素级规则必须看到它，可达性跟着引用它的单元走。
    #[test]
    fn element_docs_follow_imported_template_reach() {
        let fixture = Fixture::new("element-docs-import");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/sky/index","pages/web/index"]}"#,
            )
            .write("pages/sky/index.json", "{}")
            .write(
                "pages/sky/index.wxml",
                r#"<import src="../../templates/sky.wxml"/><template is="a"/>"#,
            )
            .write("pages/web/index.json", r#"{"renderer":"webview"}"#)
            .write(
                "pages/web/index.wxml",
                r#"<import src="../../templates/web.wxml"/><template is="b"/>"#,
            )
            .write(
                "templates/sky.wxml",
                r#"<template name="a"><view/></template>"#,
            )
            .write(
                "templates/web.wxml",
                r#"<template name="b"><view/></template>"#,
            );

        let project = fixture.project();
        let reach: std::collections::HashMap<String, Reach> = element_docs(&project)
            .into_iter()
            .map(|doc| (doc.doc.path().to_string(), doc.reach))
            .collect();

        assert_eq!(reach["templates/sky.wxml"], Reach::Skyline);
        assert_eq!(reach["templates/web.wxml"], Reach::WebviewOnly);
    }

    /// 引擎节点工厂注册的内置组件（shadow/node_factory.cc:329,331,342-343）
    /// 必须在白名单里，否则结构规则会把它们当成自定义组件边界而跳过内部约束。
    #[test]
    fn engine_builtin_components_are_known() {
        for tag in ["open-container", "list-builder", "grid-builder"] {
            assert!(is_builtin(tag), "{tag} should be builtin");
        }
    }
}
