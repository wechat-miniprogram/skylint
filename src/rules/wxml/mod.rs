//! WXML 规则。
//!
//! 一条规则一个文件：逐元素的局部判定（内联文本、组件白名单、scroll-view type）
//! 与父子结构约束（scroll-view 家族、enable-flex）各占一个文件。它们共用同一套
//! 「页面 + 组件 WXML 遍历」与解析辅助。

pub mod branch;
pub mod component_attribute;
pub mod component_children;
pub mod horizontal_scroll_enable_flex;
pub mod inline_run;
pub mod no_inline_text;
pub mod root_scope_class;
pub mod scroll_view_family;
pub mod scroll_view_type;
pub mod share_element_key;
pub(crate) mod structure;
pub mod template_data_scope;
pub mod text_hard_break_ellipsis;
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
pub use root_scope_class::RootScopeClass;
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

use std::collections::HashSet;
use std::fs;

use glass_easel_template_compiler::parse::Position;

use crate::diag::{Diagnostic, Severity};
use crate::input::wxml::{self, WxmlDoc, WxmlElement, WxmlNode};
use crate::matcher::{Dialect, Tree, Tri};
use crate::project::{Project, Scope, StyleSheet};

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

/// 每个（未 opt-out 的）页面的 WXML 路径集合：首项是该页面自己，其后是它可达组件的 WXML。
///
/// 跨文件的元素级检查（fixed / scroll-view 高度 / 隐含颜色）都要这一份扇出；
/// 解析结果由调用方按路径缓存，多个页面共用组件时只解析一次。
pub(crate) fn page_wxml_groups(project: &Project) -> Vec<(&str, Vec<String>)> {
    project
        .pages
        .iter()
        .filter(|page| !page.opted_out)
        .filter_map(|page| {
            let page_wxml = page.files.wxml.as_deref()?;
            let mut paths = vec![page_wxml.to_string()];
            paths.extend(project.page_components(&page.route).into_iter().filter_map(
                |component| {
                    project
                        .components
                        .iter()
                        .find(|unit| unit.path == component)
                        .and_then(|unit| unit.files.wxml.clone())
                },
            ));
            Some((page_wxml, paths))
        })
        .collect()
}

/// 一份 WXML 文档与它在 Skyline 下的可达性。
pub(crate) struct ElementDoc {
    pub doc: WxmlDoc,
    pub reach: Reach,
}

/// 元素级规则能看到的文档：全部页面与组件的 WXML（不只非 opt-out 页面可达的那些）。
///
/// 元素级规则曾经只走 [`page_wxml_groups`]，于是只被 `renderer: "webview"` 页面引用的
/// 组件永远进不了检查，而同一份文件的声明级规则照报——同一个组件一半报一半不报。
/// 这里对所有文档求值，再用 [`Reach`] 把「WebView 専属」在 `help` 里说明。
pub(crate) fn element_docs(project: &Project) -> Vec<ElementDoc> {
    let skyline: HashSet<String> = page_wxml_groups(project)
        .into_iter()
        .flat_map(|(_, paths)| paths)
        .collect();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for page in &project.pages {
        let paths: Vec<String> = std::iter::once(page.files.wxml.iter().cloned())
            .flatten()
            .chain(
                project
                    .page_components(&page.route)
                    .into_iter()
                    .filter_map(|component| {
                        project
                            .components
                            .iter()
                            .find(|unit| unit.path == component)
                            .and_then(|unit| unit.files.wxml.clone())
                    }),
            )
            .collect();
        for path in paths {
            if !seen.insert(path.clone()) {
                continue;
            }
            let Some(doc) = load_doc(project, &path) else {
                continue;
            };
            let reach = if skyline.contains(&path) {
                Reach::Skyline
            } else {
                Reach::WebviewOnly
            };
            out.push(ElementDoc { doc, reach });
        }
    }
    out
}

/// 文档能否被按 Skyline 检查的页面渲染到。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reach {
    Skyline,
    /// 只被 `renderer: "webview"` 的页面引用（或它自己就是这类页面）。
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

pub(crate) fn load_doc(project: &Project, path: &str) -> Option<WxmlDoc> {
    let source = fs::read_to_string(project.abs(path)).ok()?;
    let empty = crate::input::env::Env::default();
    let mut doc = wxml::parse(path, &source, project.env(path).unwrap_or(&empty)).ok()?;
    doc.fragment = project.template_files.iter().any(|file| file == path);
    Some(doc)
}

/// 用空常量环境解析：保留所有 `wx:if` 分支，用于「只数引用、不看可达」的检查。
///
/// 位置与折叠后的文档完全一致（折叠只丢分支，不移动元素）。
pub(crate) fn load_doc_unfolded(project: &Project, path: &str) -> Option<WxmlDoc> {
    let source = fs::read_to_string(project.abs(path)).ok()?;
    let mut doc = wxml::parse(path, &source, &crate::input::env::Env::default()).ok()?;
    doc.fragment = project.template_files.iter().any(|file| file == path);
    Some(doc)
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

/// 元素自身在样式层里的全部 `display` 取值（内联 `style` 在前）。
///
/// 用 Skyline 方言求值：这里问的是「Skyline 下这个元素算出什么 display」，
/// 所以属性选择器按引擎的口径当成恒真（见 [`crate::matcher::Dialect`]）。
/// 同一元素命中多份互相矛盾的 `display` 时全部返回，由调用方决定怎么解释。
pub(crate) fn display_values(
    project: &Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(value) =
        structure::find_attr(element, "style").and_then(|attr| attr.value.as_deref())
    {
        out.extend(inline_displays(value));
    }
    out.extend(
        project
            .styles
            .iter()
            .filter(|sheet| sheet_covers(sheet, project, tree.path()))
            .flat_map(|sheet| &sheet.doc.rules)
            .filter(|rule| {
                rule.selectors.iter().any(|selector| {
                    tree.matches(element, selector.text.trim(), Dialect::Skyline) == Tri::Yes
                })
            })
            .filter_map(|rule| {
                rule.declarations
                    .iter()
                    .rev()
                    .find(|decl| decl.name.eq_ignore_ascii_case("display"))
                    .map(|decl| decl.value.trim().to_string())
            }),
    );
    out
}

/// 元素命中的某条属性声明（取级联的最后一个近似值）。
///
/// 不计算 specificity：按样式表顺序与规则顺序取最后一条（`visit` 侧同口径），
/// 同名规则里靠后的声明覆盖靠前的，与同优先级 CSS 一致；跨优先级冲突时静态判不了。
pub(crate) struct Resolved<'a> {
    pub value: String,
    /// 来自样式表时的文件与声明；来自内联 `style` 时都是 `None`。
    pub sheet: Option<&'a crate::project::StyleSheet>,
    pub declaration: Option<&'a crate::input::wxss::WxssDeclaration>,
}

/// 元素某个属性的有效取值：内联 `style` 最后生效（`!important` 不计）。
pub(crate) fn resolved_property<'a>(
    project: &'a Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
    property: &str,
) -> Option<Resolved<'a>> {
    let from_sheets = project
        .styles
        .iter()
        .filter(|sheet| sheet_covers(sheet, project, tree.path()))
        .flat_map(|sheet| {
            sheet
                .doc
                .rules
                .iter()
                .filter(|rule| {
                    rule.selectors.iter().any(|selector| {
                        tree.matches(element, selector.text.trim(), Dialect::Skyline) == Tri::Yes
                    })
                })
                .flat_map(move |rule| {
                    rule.declarations
                        .iter()
                        .filter(|decl| decl.name.eq_ignore_ascii_case(property))
                        .map(move |decl| (sheet, decl))
                })
        })
        .last()
        .map(|(sheet, declaration)| Resolved {
            value: declaration.value.trim().to_string(),
            sheet: Some(sheet),
            declaration: Some(declaration),
        });

    let inline = structure::find_attr(element, "style")
        .and_then(|attr| attr.value.as_deref())
        .and_then(|style| inline_value(style, property));
    match inline {
        Some(value) => Some(Resolved {
            value,
            sheet: None,
            declaration: None,
        }),
        None => from_sheets,
    }
}

/// WebView 侧本来就会截断吗：多行靠 `-webkit-line-clamp`，单行靠
/// `white-space: nowrap` + `overflow: hidden` + `text-overflow: ellipsis`。
///
/// 自身与任一祖先都算：祖先被 clamp 时内层文本同样被裁掉。用于「WebView 能截断、
/// Skyline 截不了」这类迁移差异的准入判断——WebView 也不截断时两端一致，不该报。
pub(crate) fn webview_truncates(
    project: &Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
    ancestors: &[&WxmlElement],
) -> bool {
    std::iter::once(element)
        .chain(ancestors.iter().rev().copied())
        .any(|candidate| {
            let clamped = resolved_property(project, tree, candidate, "-webkit-line-clamp")
                .is_some_and(|resolved| {
                    resolved
                        .value
                        .trim()
                        .parse::<u32>()
                        .is_ok_and(|lines| lines > 0)
                });
            clamped || single_line_ellipsis(project, tree, candidate)
        })
}

/// 元素的 `display` 是否算成 flex 容器（取级联最后一个近似值）。
///
/// 与 [`display_values`] 的区别：这里要的是「最终算出哪个值」，同一元素命中
/// 多份互相矛盾的 `display` 时按样式表顺序取最后一个，让各规则口径一致。
pub(crate) fn flex_container(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> bool {
    resolved_property(project, tree, element, "display")
        .is_some_and(|resolved| is_flex_value(&resolved.value))
}

/// 元素自己是否在 WebView 里裁掉溢出内容（`text-overflow` 生效的前提）。
///
/// `visible` / 缺省时不裁，`hidden` / `clip` / `auto` / `scroll` 都会裁。
pub(crate) fn clips_overflow(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> bool {
    let clips = |property: &str| {
        resolved_property(project, tree, element, property).is_some_and(|resolved| {
            matches!(
                resolved.value.trim().to_ascii_lowercase().as_str(),
                "hidden" | "clip" | "auto" | "scroll"
            )
        })
    };
    clips("overflow") || clips("overflow-x") || clips("overflow-y")
}

fn single_line_ellipsis(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> bool {
    let is = |property: &str, expected: &str| {
        resolved_property(project, tree, element, property)
            .is_some_and(|resolved| resolved.value.trim().eq_ignore_ascii_case(expected))
    };
    is("white-space", "nowrap")
        && clips_overflow(project, tree, element)
        && is("text-overflow", "ellipsis")
}

/// 自身或祖先最近的 `-webkit-line-clamp` 行数（多行截断的行数来源）。
pub(crate) fn clamp_lines(
    project: &Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
    ancestors: &[&WxmlElement],
) -> Option<u32> {
    std::iter::once(element)
        .chain(ancestors.iter().rev().copied())
        .find_map(|candidate| {
            resolved_property(project, tree, candidate, "-webkit-line-clamp")
                .and_then(|resolved| resolved.value.trim().parse::<u32>().ok())
                .filter(|lines| *lines > 0)
        })
}

/// 内联 `style` 里某个属性的最后一个取值。
fn inline_value(style: &str, property: &str) -> Option<String> {
    style
        .split(';')
        .filter_map(|declaration| {
            let (name, value) = declaration.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case(property)
                .then(|| value.trim().to_string())
        })
        .next_back()
}

fn inline_displays(style: &str) -> impl Iterator<Item = String> + '_ {
    style.split(';').filter_map(|declaration| {
        let (name, value) = declaration.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("display")
            .then(|| value.trim().to_string())
    })
}

/// 这份样式表是否作用于 `path` 这份 WXML 所属的单元。
pub(crate) fn sheet_covers(sheet: &StyleSheet, project: &Project, path: &str) -> bool {
    sheet.scopes.iter().any(|scope| match scope {
        Scope::Global => true,
        _ => project.scope_wxml(scope) == Some(path),
    })
}

/// `display: flex` / `inline-flex`：文本与元素在 WebView 与 Skyline 下都是 flex item。
pub(crate) fn is_flex_value(value: &str) -> bool {
    let value = value.trim();
    value.eq_ignore_ascii_case("flex") || value.eq_ignore_ascii_case("inline-flex")
}

/// Skyline 组件支持表。
///
/// 来源：/Volumes/2tb-ssd/nebula/Work/skyline-skills/skills/skyline-overview/references/introduction/component-support.md
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
    use super::is_builtin;

    /// 引擎节点工厂注册的内置组件（shadow/node_factory.cc:329,331,342-343）
    /// 必须在白名单里，否则结构规则会把它们当成自定义组件边界而跳过内部约束。
    #[test]
    fn engine_builtin_components_are_known() {
        for tag in ["open-container", "list-builder", "grid-builder"] {
            assert!(is_builtin(tag), "{tag} should be builtin");
        }
    }
}
