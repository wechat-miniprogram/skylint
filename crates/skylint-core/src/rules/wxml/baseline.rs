//! 需要手写判定的 WXML 规则：内联文本、组件白名单、scroll-view type。

use std::collections::{HashMap, HashSet};
use std::fs;

use crate::diag::{Diagnostic, Severity};
use crate::input::wxml::{self, WxmlDoc, WxmlElement, WxmlNode};
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};

use super::{load_doc, wxml_files};

pub const ID: &str = "skyline/no-inline-text";

/// 文本与元素混排时唯一允许直接承载文本的标签。
///
/// 依据：`skyline-components` 的 MUST 规则「内联文本必须用 text 组件」与
/// 「text 组件内只能嵌套 text」；`span` 只是图文混排的内联容器，其中的文字
/// 示例仍然用 `<text>` 包裹，因此不在白名单内。
const TEXT_TAG: &str = "text";

pub struct NoInlineText;

impl Rule for NoInlineText {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Warning,
            description: "Skyline 下文本与元素混排会变成独立 flex item，需用 `<text>` 包裹",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let wxml_files = project
            .pages
            .iter()
            .filter_map(|page| page.files.wxml.as_deref())
            .chain(
                project
                    .components
                    .iter()
                    .filter_map(|component| component.files.wxml.as_deref()),
            );

        let mut seen = HashSet::new();
        let mut diagnostics = Vec::new();
        for path in wxml_files {
            if !seen.insert(path) {
                continue;
            }
            // 单文件读取/解析失败只跳过该单元，绝不终止整次扫描。
            let Ok(source) = fs::read_to_string(project.abs(path)) else {
                continue;
            };
            let Ok(doc) = wxml::parse(path, &source) else {
                continue;
            };
            collect(&doc, &doc.nodes, path, &mut diagnostics);
        }
        diagnostics
    }
}

fn collect(doc: &WxmlDoc, nodes: &[WxmlNode], path: &str, out: &mut Vec<Diagnostic>) {
    for node in nodes {
        let WxmlNode::Element(element) = node else {
            continue;
        };
        check_element(doc, element, path, out);
        collect(doc, &element.children, path, out);
    }
}

/// 只有「文本与元素混排」在 Skyline 下会静默错位：纯文本子节点仍按 inline 流渲染。
/// `<text>` 内部允许文本与 `<text>` 子节点混排，整棵子树豁免。
fn check_element(doc: &WxmlDoc, element: &WxmlElement, path: &str, out: &mut Vec<Diagnostic>) {
    if element.tag == TEXT_TAG {
        return;
    }
    let has_element = element
        .children
        .iter()
        .any(|child| matches!(child, WxmlNode::Element(_)));
    if !has_element {
        return;
    }
    for text in element.children.iter().filter_map(|child| match child {
        WxmlNode::Text(text) if text.is_meaningful() => Some(text),
        _ => None,
    }) {
        let (line, column) = doc.position(text.start);
        let (end_line, end_column) = doc.position(text.end);
        out.push(
            Diagnostic::new(
                ID,
                Severity::Warning,
                format!(
                    "`<{}>` 内文本与元素混排，Skyline 会把文本当成独立 flex item，导致错位",
                    element.tag
                ),
                path,
                line,
                column,
            )
            .end(end_line, end_column)
            .help("把文本包进 `<text>`，或把整段文字移入独立的 `<text>` 子节点"),
        );
    }
}

/// 组件支持表。数据文件是唯一依据，Rust 侧只做查表。
const COMPONENTS_DATA: &str = include_str!("../data/components.txt");

struct ComponentTable {
    supported: HashSet<&'static str>,
    unsupported: HashMap<&'static str, &'static str>,
}

impl ComponentTable {
    fn load() -> Self {
        let mut supported = HashSet::new();
        let mut unsupported = HashMap::new();
        let mut section = "";
        for line in COMPONENTS_DATA.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            match line {
                "[supported]" => {
                    section = "supported";
                    continue;
                }
                "[unsupported]" => {
                    section = "unsupported";
                    continue;
                }
                _ => {}
            }
            match section {
                "supported" => {
                    supported.insert(line);
                }
                "unsupported" => {
                    if let Some((name, replacement)) = line.split_once('|') {
                        unsupported.insert(name.trim(), replacement.trim());
                    }
                }
                _ => {}
            }
        }
        Self {
            supported,
            unsupported,
        }
    }

    /// 黑名单命中才返回替代方案；白名单与未知自定义组件都不报。
    fn replacement(&self, tag: &str) -> Option<&'static str> {
        if self.supported.contains(tag) {
            return None;
        }
        self.unsupported.get(tag).copied()
    }
}

pub struct UnsupportedComponent;

impl UnsupportedComponent {
    pub const ID: &'static str = "skyline/unsupported-component";
}

impl Rule for UnsupportedComponent {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Error,
            description: "Skyline 下不可用的组件（如 web-view / movable-view / editor）",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let table = ComponentTable::load();
        let mut diagnostics = Vec::new();
        for path in wxml_files(project) {
            let Some(doc) = load_doc(project, path) else {
                continue;
            };
            for element in doc.elements() {
                let tag = element.tag.to_ascii_lowercase();
                let Some(replacement) = table.replacement(&tag) else {
                    continue;
                };
                let (line, column) = doc.position(element.tag_start);
                let (end_line, end_column) = doc.position(element.tag_end);
                diagnostics.push(
                    Diagnostic::new(
                        Self::ID,
                        Severity::Error,
                        format!("Skyline 不支持组件 `<{}>`", element.tag),
                        path,
                        line,
                        column,
                    )
                    .end(end_line, end_column)
                    .help(replacement),
                );
            }
        }
        diagnostics
    }
}

pub struct ScrollViewType;

impl ScrollViewType {
    pub const ID: &'static str = "skyline/scroll-view-type";
}

/// `scroll-view` 在 Skyline 下的合法 `type`。
/// 依据 `scroll-view.md` 的 type 属性表：list / custom / nested。
const SCROLL_VIEW_TYPES: [&str; 3] = ["list", "custom", "nested"];

impl Rule for ScrollViewType {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "`<scroll-view>` 必须显式指定合法 type，否则性能退化",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        for path in wxml_files(project) {
            let Some(doc) = load_doc(project, path) else {
                continue;
            };
            for element in doc.elements() {
                if element.tag != "scroll-view" {
                    continue;
                }
                match element
                    .attributes
                    .iter()
                    .find(|attr| attr.name.eq_ignore_ascii_case("type"))
                {
                    None => {
                        let (line, column) = doc.position(element.tag_start);
                        let (end_line, end_column) = doc.position(element.tag_end);
                        diagnostics.push(
                            Diagnostic::new(
                                Self::ID,
                                Severity::Warning,
                                "`<scroll-view>` 缺少 type 属性，Skyline 下会退化为非按需渲染，性能下降",
                                path,
                                line,
                                column,
                            )
                            .end(end_line, end_column)
                            .help(
                                "显式指定 type：list（普通长列表）、custom（list-view/grid-view/sticky 布局）、nested（嵌套滚动）",
                            ),
                        );
                    }
                    // 动态值无法在静态阶段确定，跳过。
                    Some(attr) if !attr.is_static => {}
                    Some(attr) => {
                        let value = attr.value.as_deref().unwrap_or("").trim();
                        if SCROLL_VIEW_TYPES.contains(&value) {
                            continue;
                        }
                        let (line, column) = doc.position(attr.start);
                        let (end_line, end_column) = doc.position(attr.end);
                        diagnostics.push(
                            Diagnostic::new(
                                Self::ID,
                                Severity::Error,
                                format!("`<scroll-view type=\"{value}\">` 不是 Skyline 支持的滚动模式"),
                                path,
                                line,
                                column,
                            )
                            .end(end_line, end_column)
                            .help(
                                "合法值：list（普通长列表）、custom（list-view/grid-view/sticky 布局）、nested（嵌套滚动）",
                            ),
                        );
                    }
                }
            }
        }
        diagnostics
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use crate::project::Project;
    use crate::rules::Rule;

    use super::{NoInlineText, ScrollViewType, UnsupportedComponent};

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("skylint-wxml-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            Self { root }
        }

        fn write(&self, rel: &str, content: &str) -> &Self {
            let path = self.root.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
            self
        }

        fn path(&self) -> &Path {
            &self.root
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    /// 单页 Skyline 工程。
    fn project(name: &str, wxml: &str) -> Fixture {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", wxml);
        fixture
    }

    fn check(fixture: &Fixture) -> Vec<crate::Diagnostic> {
        let project = Project::load(fixture.path()).unwrap();
        NoInlineText.check(&project)
    }

    // ---- positive ----

    #[test]
    fn text_and_view_mixed() {
        let diagnostics = check(&project("mixed", "<view>Hello <view>World</view></view>"));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (1, 7));
        assert_eq!(diagnostics[0].end_column, 12);
    }

    #[test]
    fn interpolation_and_text_child_mixed() {
        let diagnostics = check(&project("interp", "<view>{{a}}<text>b</text></view>"));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (1, 7));
    }

    #[test]
    fn text_and_image_mixed() {
        let diagnostics = check(&project("image", "<view>文字<image/></view>"));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (1, 7));
    }

    #[test]
    fn block_text_and_element_mixed_reports_text_position() {
        let diagnostics = check(&project(
            "block",
            "<view>\n  <block wx:for=\"{{list}}\">{{item}}<image/></block>\n</view>",
        ));
        assert_eq!(diagnostics.len(), 1);
        // 报在文本节点上，不是 block 上。
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (2, 28));
    }

    #[test]
    fn every_text_child_is_reported_separately() {
        let diagnostics = check(&project("multiple", "<view>a<text>b</text>c</view>"));
        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].column, 7);
        assert_eq!(diagnostics[1].column, 22);
    }

    #[test]
    fn text_after_element_is_reported() {
        let diagnostics = check(&project("nested", "<view><view/>尾部</view>"));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].column, 14);
    }

    #[test]
    fn mixed_text_with_surrounding_whitespace() {
        let diagnostics = check(&project("ws", "<view>\n  hello\n  <view/>\n</view>"));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (2, 3));
    }

    #[test]
    fn text_mixed_inside_wx_if() {
        let diagnostics = check(&project("if", "<view wx:if=\"{{ok}}\">yes<view/></view>"));
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn emoji_before_text_uses_character_column() {
        let diagnostics = check(&project("emoji", "<view>😀hi<image/></view>"));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (1, 7));
        assert_eq!(diagnostics[0].end_column, 10);
    }

    // ---- negative ----

    #[test]
    fn whitespace_only_is_ignored() {
        let diagnostics = check(&project("blank", "<view>\n  <view/>\n</view>"));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn pure_static_text_is_allowed() {
        let diagnostics = check(&project("pure-static", "<view>静态文本</view>"));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn pure_interpolation_text_is_allowed() {
        let diagnostics = check(&project("pure-interp", "<view>{{a}}</view>"));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn multiple_pure_text_nodes_are_allowed() {
        let diagnostics = check(&project("pure-multi", "<view>a{{b}}c</view>"));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn text_inside_text_is_allowed() {
        let diagnostics = check(&project("text", "<text>hello</text>"));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn nested_text_is_allowed() {
        let diagnostics = check(&project("text-nested", "<text>a<text>b</text></text>"));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn text_wrapped_in_view_is_allowed() {
        let diagnostics = check(&project("wrapped", "<view><text>hello</text></view>"));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn comments_are_ignored() {
        let diagnostics = check(&project("comment", "<view><!-- hi --></view>"));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn text_with_comment_is_not_mixed() {
        let diagnostics = check(&project("text-comment", "<view>hi<!-- c --></view>"));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn only_elements_are_allowed() {
        let diagnostics = check(&project("only-elements", "<view><view/><image/></view>"));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn text_element_with_element_child_is_exempt() {
        // `<text>` 内只能嵌 `<text>` 是另一条规则（text-only-children），这里不报。
        let diagnostics = check(&project("text-exempt", "<text>a<image/></text>"));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn self_closing_tags_are_ignored() {
        let diagnostics = check(&project("self-closing", "<view><image/></view>"));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn pure_text_inside_structural_wrappers_is_allowed() {
        let diagnostics = check(&project(
            "wrapper-ok",
            "<view wx:if=\"{{ok}}\">yes</view>\n<block wx:for=\"{{list}}\">{{item}}</block>",
        ));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn span_still_requires_text() {
        let diagnostics = check(&project("span", "<span><text>a</text><image/></span>"));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn webview_page_is_checked_too() {
        // 迁移工具的受众就是还没上 Skyline 的工程，未迁移的页面不能跳过。
        let fixture = Fixture::new("webview");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"webview"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write(
                "pages/index/index.wxml",
                "<view>hello <view>world</view></view>",
            );
        assert_eq!(check(&fixture).len(), 1);
    }

    // ---- resilience ----

    #[test]
    fn broken_file_does_not_stop_other_files() {
        let fixture = Fixture::new("broken");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/bad/bad","pages/good/good"],"renderer":"skyline"}"#,
            )
            .write("pages/bad/bad.json", "{}")
            .write("pages/bad/bad.wxml", "<view>{{ a</view>")
            .write("pages/good/good.json", "{}")
            .write("pages/good/good.wxml", "<view>hello<image/></view>");

        let diagnostics = check(&fixture);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].path, "pages/good/good.wxml");
    }

    // ---- unsupported-component ----

    fn check_rule(fixture: &Fixture, rule: &dyn Rule) -> Vec<crate::Diagnostic> {
        let project = Project::load(fixture.path()).unwrap();
        rule.check(&project)
    }

    #[test]
    fn unsupported_components_are_reported() {
        let diagnostics = check_rule(
            &project(
                "unsupported",
                r#"<view>
  <web-view src="https://example.com"/>
  <movable-view/>
  <editor/>
  <progress/>
  <xr-frame/>
</view>"#,
            ),
            &UnsupportedComponent,
        );
        assert_eq!(diagnostics.len(), 5);
        assert!(
            diagnostics
                .iter()
                .all(|d| d.severity == crate::Severity::Error)
        );

        let web_view = diagnostics
            .iter()
            .find(|d| d.message.contains("web-view"))
            .unwrap();
        assert!(
            web_view
                .help
                .as_deref()
                .unwrap()
                .contains(r#""renderer": "webview""#)
        );

        let movable_view = diagnostics
            .iter()
            .find(|d| d.message.contains("movable-view"))
            .unwrap();
        assert!(movable_view.help.as_deref().unwrap().contains("worklet"));
    }

    #[test]
    fn every_unsupported_occurrence_is_reported() {
        let diagnostics = check_rule(
            &project("unsupported-twice", "<view><web-view/><web-view/></view>"),
            &UnsupportedComponent,
        );
        assert_eq!(diagnostics.len(), 2);
    }

    #[test]
    fn supported_components_are_allowed() {
        let diagnostics = check_rule(
            &project(
                "supported",
                "<view><form/><scroll-view/><navigator/><rich-text/><page-meta/><span/><sticky-header/><list-view/></view>",
            ),
            &UnsupportedComponent,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn unknown_custom_component_is_ignored() {
        let diagnostics = check_rule(
            &project("custom-component", "<my-widget/>"),
            &UnsupportedComponent,
        );
        assert!(diagnostics.is_empty());
    }

    // ---- scroll-view-type ----

    #[test]
    fn scroll_view_without_type_warns_on_tag() {
        let diagnostics = check_rule(
            &project("scroll-missing", "<scroll-view scroll-y></scroll-view>"),
            &ScrollViewType,
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, crate::Severity::Warning);
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (1, 2));
    }

    #[test]
    fn scroll_view_with_invalid_static_type_errors_on_attribute() {
        let diagnostics = check_rule(
            &project(
                "scroll-invalid",
                r#"<scroll-view type="page"></scroll-view>"#,
            ),
            &ScrollViewType,
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, crate::Severity::Error);
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (1, 14));
        assert!(diagnostics[0].help.as_deref().unwrap().contains("nested"));
    }

    #[test]
    fn valid_and_dynamic_scroll_view_types_are_allowed() {
        let diagnostics = check_rule(
            &project(
                "scroll-ok",
                "<view><scroll-view type=\"list\"/><scroll-view type=\"custom\"/><scroll-view type=\"nested\"/><scroll-view type=\"{{t}}\"/></view>",
            ),
            &ScrollViewType,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }
}
