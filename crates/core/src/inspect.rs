//! 面向集成的「元素事实」查询。
//!
//! 规则层自己已经算出了这些东西（级联取值、逻辑祖先链、布局判定），但它们都是
//! `pub(crate)`、且散在各处。迁移工具 / MCP server 需要的是「把一个元素为什么这么判
//! 讲清楚」，所以这里提供一层**只读、按需计算**的稳定视图：
//!
//! ```ignore
//! let loaded = LoadedDoc::load(&project, "pages/index/index.wxml")?;
//! let facts = loaded.facts(&project);
//! for element in facts.elements() {
//!     let display = facts.resolved(element, "display");
//! }
//! ```
//!
//! 设计约束：
//!
//! - **不重复实现**：级联、祖先链、布局判定都调用规则层同一份实现（`rules::wxml`、
//!   `wxml_graph`、`scroll_view_height`），否则两套语义会漂移；
//! - **不预生成全量**：每个属性单独查，成本与「一条规则查一次」同级；不要写
//!   「对每个元素算全属性」的循环；
//! - 返回的都是**拥有所有权的快照**，不把内部借用漏出去。

use std::cell::OnceCell;

use crate::input::wxml::{WxmlDoc, WxmlElement};
use crate::matcher::Tree;
use crate::project::{Project, Scope, StyleIsolation};
use crate::rules::wxml::{self, Reach};
use crate::wxml_graph::WxmlGraph;

/// 一份已解析的 WXML：事实查询要在它上面建索引，所以由调用方持有、`Facts` 借用。
pub struct LoadedDoc {
    pub doc: WxmlDoc,
}

impl LoadedDoc {
    pub fn load(project: &Project, path: &str) -> Option<Self> {
        wxml::load_doc(project, path).map(|doc| Self { doc })
    }

    pub fn facts<'a>(&'a self, project: &'a Project) -> Facts<'a> {
        Facts::new(project, &self.doc)
    }
}

/// 一个元素在运行时（逻辑树，含 slot 投影）的祖先。
#[derive(Debug, Clone)]
pub struct AncestorView {
    pub doc: String,
    pub tag: String,
    pub line: u32,
    pub column: u32,
    /// 这个祖先是一个具名 `<slot name="x">` 时给出槽名。
    pub slot: Option<String>,
}

/// 祖先链。`open` 表示链尾落在静态不可知的边上（动态槽名、未渲染的组件…），
/// 依赖祖先的结论此时应当放弃。
#[derive(Debug, Clone, Default)]
pub struct Ancestors {
    pub nodes: Vec<AncestorView>,
    pub open: bool,
}

/// 一条参与级联的声明（快照）。数组按优先级升序，末位生效。
#[derive(Debug, Clone)]
pub struct CascadeView {
    pub value: String,
    pub important: bool,
    /// packed specificity（id/class/tag 各 10 位）；内联 `style` 为 `u32::MAX`。
    pub specificity: u32,
    pub inline: bool,
    /// 命中的选择器原文；内联为 `None`。
    pub selector: Option<String>,
    /// 声明来自哪份样式表（项目相对路径）。
    pub stylesheet: Option<String>,
    /// 声明在样式表里的位置（1-based）。
    pub line: Option<u32>,
    pub column: Option<u32>,
    /// 从外到内的 at-rule 上下文，例如 `["media (min-width: 600px)"]`。
    pub at_rules: Vec<String>,
}

/// 元素在 Skyline 下的布局判定（都是规则层已经算过的结论）。
#[derive(Debug, Clone)]
pub struct LayoutFacts {
    pub display: Option<String>,
    pub flex_direction: Option<String>,
    pub position: Option<String>,
    /// `position: absolute | fixed`。
    pub out_of_flow: bool,
    /// `width` 是否本来就由内容决定（`fit-content` 被丢弃也不改变渲染）。
    pub width_follows_content: bool,
    /// 这个单元是否按 `defaultDisplayBlock` 的块级默认值渲染。
    pub block_rendered: bool,
    /// `scroll-view` 之类的盒子是否拿到了确定高度（= 不会塌成 0）。
    pub definite_height: bool,
    /// 自身生效的 `white-space`（不含继承）。
    pub white_space: Option<String>,
    /// 自身是否在 WebView 里裁掉溢出（`hidden` / `clip` / `auto` / `scroll`）。
    pub clips_overflow: bool,
    /// 自身或任一祖先在 WebView 下本来就会截断（单行三件套或多行 clamp）。
    pub webview_truncates: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnitKind {
    Page {
        route: String,
    },
    Component,
    /// 被 `<import>` / `<include>` 引用的模板片段。
    Fragment,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Renderer {
    Skyline,
    Webview,
    /// 判定不了（片段、或没有任何页面渲染到它）。
    Unknown,
}

/// 一次组件使用点：哪个文档用哪个标签引用了当前这份组件。
#[derive(Debug, Clone)]
pub struct UsageView {
    pub doc: String,
    pub tag: String,
    pub line: u32,
    pub column: u32,
    /// 距离查询文档的跳数（直接使用点为 1）。
    pub depth: u32,
}

/// 元素上的一个事件绑定及其处理函数。
#[derive(Debug, Clone)]
pub struct HandlerView {
    /// 属性名原文（`bind:tap` / `catchtap` …）。
    pub attribute: String,
    /// 事件名（去掉 `bind` / `catch` / `capture-` / `mut-` 前缀）。
    pub event: String,
    /// 静态写的处理函数名；动态绑定或非裸标识符时为 `None`。
    pub name: Option<String>,
    /// 值不是静态裸标识符：`{{handler}}`、`this.fn`、表达式。
    pub dynamic: bool,
    /// 在单元脚本里找到的同名函数。
    pub function: Option<HandlerFunction>,
}

/// 事件处理函数在脚本里的位置。
#[derive(Debug, Clone)]
pub struct HandlerFunction {
    pub script: String,
    pub line: u32,
    pub column: u32,
    pub worklet: bool,
}

/// 事件绑定属性名 → 事件名。
///
/// 认 `bindtap` / `catchtap` / `bind:tap` / `catch:tap` / `capture-bind:tap` /
/// `capture-catch:tap` / `mut-bind:tap`；`worklet:` 是另一套语义，不算事件绑定。
fn event_name(attribute: &str) -> Option<&str> {
    let rest = attribute
        .strip_prefix("capture-")
        .or_else(|| attribute.strip_prefix("mut-"))
        .unwrap_or(attribute);
    ["bind:", "catch:", "bind", "catch"]
        .iter()
        .find_map(|prefix| rest.strip_prefix(prefix))
        .filter(|event| {
            !event.starts_with(':')
                && event
                    .chars()
                    .next()
                    .is_some_and(|first| first.is_ascii_alphabetic())
        })
}

/// 单元（页面 / 组件 / 片段）的配置事实。
#[derive(Debug, Clone)]
pub struct UnitFacts {
    pub path: String,
    pub kind: UnitKind,
    pub renderer: Renderer,
    pub reach: Reach,
    /// 组件才有；页面与片段为 `None`。
    pub style_isolation: Option<StyleIsolation>,
    pub multiple_slots: bool,
    pub virtual_host: bool,
    /// 该单元按 `rendererOptions.skyline.defaultDisplayBlock` 算出的块级默认值。
    pub default_display_block: bool,
    pub default_content_box: bool,
    pub scroll_view_auto_size: bool,
}

/// 一份文档的事实视图。构造是 O(元素数)，查询是按需的。
pub struct Facts<'a> {
    project: &'a Project,
    doc: &'a WxmlDoc,
    tree: Tree<'a>,
    elements: Vec<&'a WxmlElement>,
    graph: OnceCell<WxmlGraph>,
    scope: Option<Scope>,
}

impl<'a> Facts<'a> {
    fn new(project: &'a Project, doc: &'a WxmlDoc) -> Self {
        let elements = doc.elements().collect();
        Self {
            project,
            doc,
            tree: Tree::new(doc),
            elements,
            graph: OnceCell::new(),
            scope: project.scope_of(doc.path()).clone(),
        }
    }

    pub fn path(&self) -> &str {
        self.doc.path()
    }

    /// 文档里的全部元素（文档顺序）。
    pub fn elements(&self) -> &[&'a WxmlElement] {
        &self.elements
    }

    /// 按开始标签在源码里的位置（1-based 行、UTF-16 列）找元素。
    pub fn element_at(&self, line: u32, column: u32) -> Option<&'a WxmlElement> {
        self.elements
            .iter()
            .copied()
            .find(|element| element.tag_start.line == line && element.tag_start.utf16_col == column)
    }

    /// 运行时（逻辑树）祖先，近 → 远。
    pub fn ancestors(&self, element: &WxmlElement) -> Ancestors {
        let graph = self.graph.get_or_init(|| WxmlGraph::new(self.project));
        let chain = graph.ancestors(self.doc.path(), element);
        Ancestors {
            nodes: chain
                .nodes
                .into_iter()
                // `<slot>` 是投影占位，不是真实渲染的盒子：跳过它，
                // 「跨了组件边界」由相邻节点的 doc 变化体现。
                .filter(|(_, flat)| flat.tag != "slot")
                .map(|(doc, flat)| AncestorView {
                    doc: doc.to_string(),
                    tag: flat.tag.clone(),
                    line: flat.line,
                    column: flat.column,
                    slot: flat.slot_name().map(str::to_string),
                })
                .collect(),
            open: chain.open,
        }
    }

    /// 这份文档被哪些使用点引用（只在组件上非空）。
    ///
    /// 与 [`Facts::ancestors`] 正交：祖先回答「它上面是什么」，这里回答「它会被
    /// 哪些页面 / 父组件渲染出来」。只给直接使用点，多跳由调用方递归。
    pub fn usages(&self) -> Vec<UsageView> {
        self.usages_recursive(1)
    }

    /// 递归使用点：把「父组件被谁用」也展开，`depth` 是跳数。
    ///
    /// `max_depth` 至少按 1 算；环（A 用 B、B 又用 A）不会无限展开。
    pub fn usages_recursive(&self, max_depth: u32) -> Vec<UsageView> {
        let graph = self.graph.get_or_init(|| WxmlGraph::new(self.project));
        graph
            .component_usages_recursive(self.doc.path(), max_depth.max(1))
            .into_iter()
            .map(|hop| UsageView {
                doc: hop.site.doc,
                tag: hop.site.tag,
                line: hop.site.line,
                column: hop.site.column,
                depth: hop.depth,
            })
            .collect()
    }

    /// 元素上的全部事件绑定及其处理函数。
    ///
    /// 只回答「静态可确定的」：`{{handler}}` 这类动态绑定给出 `name: None`、
    /// `dynamic: true`；静态值但不是裸标识符（`this.onTap`）也算动态。
    pub fn handlers(&self, element: &WxmlElement) -> Vec<HandlerView> {
        let script = self
            .scope
            .as_ref()
            .and_then(|scope| crate::rules::js::scope_script(self.project, scope));
        element
            .attributes
            .iter()
            .filter_map(|attribute| {
                let event = event_name(&attribute.name)?;
                let name = attribute
                    .value
                    .as_deref()
                    .filter(|_| attribute.is_static)
                    .filter(|value| crate::rules::js::is_identifier(value))
                    .map(str::to_string);
                let function = script.zip(name.as_deref()).and_then(|(file, name)| {
                    let function = file
                        .doc
                        .functions()
                        .into_iter()
                        .find(|function| function.name == Some(name))?;
                    let (line, column) = file.doc.position(function.start as usize);
                    Some(HandlerFunction {
                        script: file.path.clone(),
                        line,
                        column,
                        worklet: function.has_worklet,
                    })
                });
                Some(HandlerView {
                    attribute: attribute.name.clone(),
                    event: event.to_string(),
                    dynamic: name.is_none(),
                    name,
                    function,
                })
            })
            .collect()
    }

    /// 某属性参与级联的全部声明，升序（末位生效）。
    pub fn cascade(&self, element: &WxmlElement, property: &str) -> Vec<CascadeView> {
        wxml::cascade_entries(self.project, &self.tree, element, property)
            .into_iter()
            .map(|entry| {
                let (stylesheet, line, column) = match (entry.sheet, entry.declaration) {
                    (Some(sheet), Some(declaration)) => {
                        let (line, column) = sheet.doc.position(declaration.offset);
                        (Some(sheet.doc.path().to_string()), Some(line), Some(column))
                    }
                    _ => (None, None, None),
                };
                CascadeView {
                    value: entry.value,
                    important: entry.important,
                    specificity: entry.specificity,
                    inline: entry.inline,
                    selector: entry.selector,
                    stylesheet,
                    line,
                    column,
                    at_rules: entry
                        .at_rules
                        .iter()
                        .map(|rule| format!("{} {}", rule.name, rule.prelude).trim().to_string())
                        .collect(),
                }
            })
            .collect()
    }

    /// 某属性的有效取值（级联赢家）。
    pub fn resolved(&self, element: &WxmlElement, property: &str) -> Option<CascadeView> {
        self.cascade(element, property).pop()
    }

    /// 布局判定。内部调用规则层同一份实现。
    pub fn layout(&self, element: &WxmlElement) -> LayoutFacts {
        let value = |property: &str| {
            self.resolved(element, property)
                .map(|entry| entry.value.trim().to_string())
        };
        let default_block = self.unit().default_display_block;
        let ancestors = self.lexical_ancestors(element);
        LayoutFacts {
            display: value("display"),
            flex_direction: value("flex-direction"),
            position: value("position"),
            out_of_flow: wxml::is_out_of_flow(self.project, &self.tree, element),
            width_follows_content: wxml::width_follows_content(self.project, &self.tree, element),
            block_rendered: default_block,
            definite_height: crate::rules::wxss::scroll_view_height::definite_height(
                self.project,
                &self.tree,
                element,
                default_block,
                0,
            ),
            white_space: value("white-space"),
            clips_overflow: wxml::clips_overflow(self.project, &self.tree, element),
            webview_truncates: wxml::webview_truncates(
                self.project,
                &self.tree,
                element,
                &ancestors,
            ),
        }
    }

    /// 本文件内的词法祖先，根在前。跨组件边界的不算（不同文件，级联也另外算）。
    fn lexical_ancestors(&self, element: &WxmlElement) -> Vec<&WxmlElement> {
        let mut chain = Vec::new();
        let mut cursor = self.tree.parent_of(element);
        while let Some(parent) = cursor {
            chain.push(parent);
            cursor = self.tree.parent_of(parent);
        }
        chain.reverse();
        chain
    }

    /// 这份文档在 Skyline 下的可达性。
    pub fn reach(&self) -> Reach {
        let graph = self.graph.get_or_init(|| WxmlGraph::new(self.project));
        for page in &self.project.pages {
            let Some(wxml) = page.files.wxml.as_deref() else {
                continue;
            };
            if page.opted_out {
                continue;
            }
            if graph
                .render_docs(wxml)
                .iter()
                .any(|doc| *doc == self.doc.path())
            {
                return Reach::Skyline;
            }
        }
        Reach::WebviewOnly
    }

    /// 单元级配置事实。
    pub fn unit(&self) -> UnitFacts {
        let path = self.doc.path();
        let kind = if let Some(page) = self
            .project
            .pages
            .iter()
            .find(|page| page.files.wxml.as_deref() == Some(path))
        {
            UnitKind::Page {
                route: page.route.clone(),
            }
        } else if self
            .project
            .components
            .iter()
            .any(|component| component.files.wxml.as_deref() == Some(path))
        {
            UnitKind::Component
        } else if self.project.template_files.iter().any(|file| file == path) {
            UnitKind::Fragment
        } else {
            UnitKind::Unknown
        };

        let reach = self.reach();
        let renderer = match &kind {
            UnitKind::Page { .. } => {
                let skyline = self
                    .project
                    .pages
                    .iter()
                    .any(|page| page.files.wxml.as_deref() == Some(path) && !page.opted_out);
                if skyline {
                    Renderer::Skyline
                } else {
                    Renderer::Webview
                }
            }
            UnitKind::Component => match reach {
                Reach::Skyline => Renderer::Skyline,
                Reach::WebviewOnly => Renderer::Webview,
            },
            _ => Renderer::Unknown,
        };

        let graph = self.graph.get_or_init(|| WxmlGraph::new(self.project));
        let (multiple_slots, virtual_host) = graph.component_options(path);
        let default_display_block = self.scope.clone().map_or_else(
            || self.project.app_display_block(),
            |scope| self.project.renders_as_block(&scope),
        );

        UnitFacts {
            path: path.to_string(),
            kind: kind.clone(),
            renderer,
            reach,
            style_isolation: matches!(kind, UnitKind::Component)
                .then(|| self.project.style_isolation(path)),
            multiple_slots,
            virtual_host,
            default_display_block,
            default_content_box: self.project.app_content_box(),
            scroll_view_auto_size: self.project.scroll_view_auto_size(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::StyleIsolation;
    use crate::rules::testutil::Fixture;
    use crate::rules::wxml::Reach;

    /// 级联里能拿到胜者、被覆盖的声明、选择器与来源。
    #[test]
    fn cascade_exposes_provenance() {
        let fixture = Fixture::page(
            "inspect-cascade",
            r#"<view class="a b"/>"#,
            ".a { color: red }\n.a.b { color: blue }",
        );
        let project = fixture.project();
        let path = project.pages[0].files.wxml.clone().unwrap();
        let loaded = LoadedDoc::load(&project, &path).unwrap();
        let facts = loaded.facts(&project);
        let element = facts.elements()[0];

        let all = facts.cascade(element, "color");
        assert_eq!(all.len(), 2, "{all:#?}");
        let winner = facts.resolved(element, "color").unwrap();
        assert_eq!(winner.value, "blue");
        assert_eq!(winner.selector.as_deref(), Some(".a.b"));
        assert_eq!(winner.stylesheet.as_deref(), Some("pages/index/index.wxss"));
        assert!(winner.specificity > all[0].specificity);
        assert!(!winner.inline);
    }

    /// 祖先链穿过 `<slot>` 投影：元素写在页面里，运行时父级是组件的 `<text>`。
    #[test]
    fn ancestors_cross_the_slot_projection() {
        let fixture = Fixture::new("inspect-slot");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                r#"{"usingComponents":{"c":"/components/c/c"}}"#,
            )
            .write("pages/index/index.wxml", r#"<c><view class="inner"/></c>"#)
            .write("components/c/c.json", r#"{"component":true}"#)
            .write(
                "components/c/c.wxml",
                r#"<text class="label"><slot/></text>"#,
            );

        let project = fixture.project();
        let path = project.pages[0].files.wxml.clone().unwrap();
        let loaded = LoadedDoc::load(&project, &path).unwrap();
        let facts = loaded.facts(&project);
        let inner = facts
            .elements()
            .iter()
            .copied()
            .find(|element| element.tag == "view")
            .unwrap();

        let ancestors = facts.ancestors(inner);
        let tags: Vec<&str> = ancestors
            .nodes
            .iter()
            .map(|node| node.tag.as_str())
            .collect();
        assert_eq!(tags, ["text", "c"], "{ancestors:#?}");
        assert_eq!(ancestors.nodes[1].doc, "pages/index/index.wxml");
    }

    /// 反链：组件模板的事实要知道它被谁用到。
    #[test]
    fn usages_point_back_to_the_host() {
        let fixture = Fixture::new("inspect-usages");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                r#"{"usingComponents":{"c":"/components/c/c"}}"#,
            )
            .write(
                "pages/index/index.wxml",
                r#"<c class="one"/><c class="two"/>"#,
            )
            .write("components/c/c.json", r#"{"component":true}"#)
            .write("components/c/c.wxml", r#"<view/>"#);

        let project = fixture.project();
        let loaded = LoadedDoc::load(&project, "components/c/c.wxml").unwrap();
        let facts = loaded.facts(&project);
        let usages = facts.usages();
        assert_eq!(usages.len(), 2, "{usages:#?}");
        assert!(
            usages
                .iter()
                .all(|site| site.doc == "pages/index/index.wxml")
        );
        assert!(usages.iter().all(|site| site.tag == "c"));

        // 页面本身没有使用点。
        let page = LoadedDoc::load(&project, "pages/index/index.wxml").unwrap();
        assert!(page.facts(&project).usages().is_empty());
    }

    /// 事件属性 → 处理函数：静态裸标识符能落到脚本位置，动态值给出 `dynamic`。
    #[test]
    fn handlers_resolve_to_functions() {
        let fixture = Fixture::page(
            "inspect-handlers",
            r#"<view bindtap="onTap" catch:touchstart="onTouch" bindlongpress="{{handler}}"/><view worklet:ongesture="onPan"/>"#,
            "",
        );
        // `Fixture::page` 不带脚本，这里补上再重新加载。
        fixture.write(
            "pages/index/index.js",
            "Page({ onTap() {}, onTouch: function () {} });",
        );
        let project = fixture.project();
        let loaded = LoadedDoc::load(&project, "pages/index/index.wxml").unwrap();
        let facts = loaded.facts(&project);
        let first = facts.handlers(facts.elements()[0]);

        assert_eq!(first.len(), 3, "{first:#?}");
        assert_eq!(first[0].event, "tap");
        assert_eq!(first[0].name.as_deref(), Some("onTap"));
        assert!(!first[0].dynamic);
        assert!(first[0].function.is_some(), "应能定位到 onTap");
        assert_eq!(first[1].event, "touchstart");
        assert_eq!(first[1].name.as_deref(), Some("onTouch"));
        assert_eq!(first[2].event, "longpress");
        assert!(first[2].dynamic);
        assert!(first[2].name.is_none());

        // `worklet:` 是手势回调，不是事件绑定。
        let second = facts.handlers(facts.elements()[1]);
        assert!(second.is_empty(), "{second:#?}");
    }

    /// 递归反链：内层组件 → 外层组件 → 页面，`depth` 是跳数。
    #[test]
    fn usages_recursive_climbs_parent_components() {
        let fixture = Fixture::new("inspect-usages-recursive");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                r#"{"usingComponents":{"outer":"/components/outer/outer"}}"#,
            )
            .write("pages/index/index.wxml", "<outer/>")
            .write(
                "components/outer/outer.json",
                r#"{"component":true,"usingComponents":{"inner":"/components/inner/inner"}}"#,
            )
            .write("components/outer/outer.wxml", "<inner/>")
            .write("components/inner/inner.json", r#"{"component":true}"#)
            .write("components/inner/inner.wxml", "<view/>");

        let project = fixture.project();
        let loaded = LoadedDoc::load(&project, "components/inner/inner.wxml").unwrap();
        let facts = loaded.facts(&project);
        let hops = facts.usages_recursive(3);
        let pairs: Vec<(&str, u32)> = hops
            .iter()
            .map(|hop| (hop.doc.as_str(), hop.depth))
            .collect();
        assert_eq!(
            pairs,
            [
                ("components/outer/outer.wxml", 1),
                ("pages/index/index.wxml", 2)
            ],
            "{hops:#?}"
        );
    }

    /// 截断事实：自身与祖先分开看，`webview_truncates` 会算上祖先。
    #[test]
    fn layout_facts_expose_webview_truncation() {
        let fixture = Fixture::page(
            "inspect-truncation",
            r#"<view class="clip"><text class="inner">x</text></view>"#,
            ".clip { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }",
        );
        let project = fixture.project();
        let path = project.pages[0].files.wxml.clone().unwrap();
        let loaded = LoadedDoc::load(&project, &path).unwrap();
        let facts = loaded.facts(&project);
        let [clip, inner] = facts.elements() else {
            panic!("应有两个元素")
        };

        let outer = facts.layout(clip);
        assert_eq!(outer.white_space.as_deref(), Some("nowrap"));
        assert!(outer.clips_overflow);
        assert!(outer.webview_truncates);

        let inner = facts.layout(inner);
        assert_eq!(inner.white_space, None, "white-space 不在解析值里继承");
        assert!(!inner.clips_overflow);
        assert!(inner.webview_truncates, "祖先的 nowrap 也算截断");
    }

    /// 单元事实：页面/组件、渲染器、组件 options。
    #[test]
    fn unit_facts_describe_the_component() {
        let fixture = Fixture::new("inspect-unit");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                r#"{"usingComponents":{"c":"/components/c/c"}}"#,
            )
            .write("pages/index/index.wxml", r#"<c/>"#)
            .write(
                "components/c/c.json",
                r#"{"component":true,"styleIsolation":"isolated"}"#,
            )
            .write("components/c/c.wxml", r#"<view/>"#)
            .write(
                "components/c/c.js",
                "Component({ options: { multipleSlots: true } });",
            );

        let project = fixture.project();
        let loaded = LoadedDoc::load(&project, "components/c/c.wxml").unwrap();
        let facts = loaded.facts(&project);
        let unit = facts.unit();
        assert_eq!(unit.kind, UnitKind::Component);
        assert_eq!(unit.renderer, Renderer::Skyline);
        assert_eq!(unit.reach, Reach::Skyline);
        assert_eq!(unit.style_isolation, Some(StyleIsolation::Isolated));
        assert!(unit.multiple_slots);
        assert!(!unit.virtual_host);
    }
}
