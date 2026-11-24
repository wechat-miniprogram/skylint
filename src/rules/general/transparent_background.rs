//! `general/transparent-background`：背景色绑定在默认为空的组件属性上，调用处没传 →
//! 这里一定透明。
//!
//! 典型形态是导航容器：组件 wxml 里 `<view class="nav-bar" style="background: {{background}}">`，
//! `background` 属性默认空串，调用方 `<nav-bar title="…">` 不传时 `background: ''`
//! 是无效值，导航区透出页面底色。WebView 时代这块被原生导航条盖着，迁到
//! `navigationStyle: custom` 之后才露出来，所以这条规则不限定渲染器。
//!
//! 只有四个前提同时成立才报（缺一不报）：
//!
//! 1. 属性确实声明在组件的 `properties` 里——`style="background: {{x}}"` 的 `x` 也可能
//!    是 `data` 字段或运行时算出来的值，调用方根本不该传；
//! 2. 该属性的默认值为空（`value` 缺省、`''`、`null`），默认给 `'#fff'` 时不传也有背景；
//! 3. 使用处确实没给值：属性缺失、或静态传了空串 `background=""` 才算；动态表达式
//!    （`{{x}}`、可能为空的三元）静态判不了，不报；
//! 4. 绑定背景的元素没有来自样式表的 `background` / `background-color`——空串声明会被
//!    丢弃，class 上的背景照样生效。
//!
//! 四个前提全部静态可证，所以文案是确定的「这里一定透明」，不再是「可能少传了值」。

use std::collections::{BTreeMap, BTreeSet};

use crate::diag::{Diagnostic, Severity};
use crate::input::js::PropertyDefault;
use crate::input::wxml::{WxmlDoc, WxmlElement};
use crate::matcher::Tree;
use crate::project::Project;
use crate::rules::wxml::{
    cascade_entries_with, element_diagnostic, for_each_element, load_doc, walk,
};
use crate::rules::{Rule, RuleMeta, Target, VisualCase, VisualComponent, VisualExpect};

pub const ID: &str = "general/transparent-background";

pub struct TransparentBackground;

/// 视觉夹具：导航容器把背景绑在默认空的属性上。
///
/// 组件自己的 wxss 不给 `.nav` 写背景——这正是「绑定元素没有 class 背景」的前提。
static VISUAL_COMPONENTS: &[VisualComponent] = &[VisualComponent {
    name: "nav",
    wxml: r#"<view class="nav" style="height: 88rpx; background: {{background}}"><slot/></view>"#,
    wxss: ".nav { height: 88rpx; }\n",
    js: Some("Component({ properties: { background: { type: String, value: '' } } });\n"),
}];

/// 阳性：调用处不传，导航区透出页面底色；阴性：显式传 `#fff`，导航区不透明。
///
/// 这条规则与渲染器无关（空串声明在两端都会被丢弃），两个用例都期望两端一致；
/// 「是否真的透明」由截图的实际颜色读数证明，不是靠两端差异。
static VISUAL_CASES: &[VisualCase] = &[
    VisualCase {
        wxml: r#"<view class="page">
  <nav><view class="title">不传 background</view></nav>
  <view class="body">导航区应透出页面底色</view>
</view>"#,
        wxss: "page { background: #d9e8ff; }\n.page { background: #d9e8ff; }\n.body { height: 240px; background: #ffd9d9; }\n",
        js: None,
        expect: VisualExpect::Same,
        note: "组件默认空、调用处不传：`background: ''` 是无效声明，导航区透出页面底色 #d9e8ff，两端一致",
        target: "",
    },
    VisualCase {
        wxml: r##"<view class="page">
  <nav background="#fff"><view class="title">传 background="#fff"</view></nav>
  <view class="body">导航区应不透明</view>
</view>"##,
        wxss: "page { background: #d9e8ff; }\n.page { background: #d9e8ff; }\n.body { height: 240px; background: #ffd9d9; }\n",
        js: None,
        expect: VisualExpect::Same,
        note: "阴性对照：显式传 `background=\"#fff\"` 后导航区不再透出页面底色，两端一致",
        target: "",
    },
];

impl Rule for TransparentBackground {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Warning,
            description: "组件把背景色绑在默认为空的属性上，调用处没传时这里一定透明",
        }
    }

    fn target(&self) -> Target {
        Target::Both
    }

    fn visual_cases(&self) -> &'static [VisualCase] {
        VISUAL_CASES
    }

    fn visual_components(&self) -> &'static [VisualComponent] {
        VISUAL_COMPONENTS
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let bound = transparent_properties(project);
        if bound.is_empty() {
            return Vec::new();
        }

        let mut out = Vec::new();
        for_each_element(project, self.target(), |site| {
            let Some(targets) = site.project.tag_targets.get(site.doc.path()) else {
                return;
            };
            let Some(component) = targets.get(&site.element.tag.to_ascii_lowercase()) else {
                return;
            };
            let Some(properties) = bound.get(component) else {
                return;
            };
            // 每个使用元素只报一条：同一组件把背景绑在两个属性上时也不重复。
            let missing: Vec<&str> = properties
                .iter()
                .map(String::as_str)
                .filter(|property| usage(site.element, property) == Usage::Empty)
                .collect();
            if missing.is_empty() {
                return;
            }
            let list = missing
                .iter()
                .map(|property| format!("`{property}`"))
                .collect::<Vec<_>>()
                .join("、");
            out.push(
                element_diagnostic(
                    ID,
                    site.doc,
                    site.element,
                    site.doc.path(),
                    Severity::Warning,
                    format!(
                        "`<{}>` 没传 {list}：组件里该属性的默认值为空，绑定元素也没有 class 背景，这里一定透明（会露出页面底色）",
                        site.element.tag
                    ),
                )
                .help(
                    "调用处显式传底色（如 `background=\"#fff\"`，或页面实际底色）；或在组件的 `properties` 里给一个非空默认值（`value: '#fff'`）",
                ),
            );
        });
        out
    }
}

/// 组件的使用处给某个属性传了什么。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Usage {
    /// 没传，或静态传了空串：运行时取组件默认值（已证明为空）。
    Empty,
    /// 传了非空静态值，或传的是动态表达式（静态判不了，一律不报）。
    Passed,
}

fn usage(element: &WxmlElement, property: &str) -> Usage {
    let Some(attribute) = element
        .attributes
        .iter()
        .find(|attribute| attribute.name == property)
    else {
        return Usage::Empty;
    };
    if !attribute.is_static {
        return Usage::Passed;
    }
    match attribute.value.as_deref() {
        Some(value) if !value.is_empty() => Usage::Passed,
        _ => Usage::Empty,
    }
}

/// 满足全部前置条件的「组件 → 背景属性」：组件路径 → 属性名集合。
fn transparent_properties(project: &Project) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for component in &project.components {
        let wxml = format!("{}.wxml", component.path);
        let Some(doc) = load_doc(project, &wxml) else {
            continue;
        };
        // 前提 1、2：属性声明在 `properties` 里，且默认值可证为空。
        let Some(properties) = declared_properties(project, &wxml) else {
            continue;
        };
        if properties.is_empty() {
            continue;
        }
        let tree = Tree::new(&doc);
        let mut bound = BTreeSet::new();
        walk(&doc, |element, _| {
            for property in bound_properties(&doc, element) {
                if !matches!(properties.get(&property), Some(PropertyDefault::Empty)) {
                    continue;
                }
                // 前提 4：绑定元素没有来自样式表的背景。
                if has_sheet_background(project, &tree, element) {
                    continue;
                }
                bound.insert(property);
            }
        });
        if !bound.is_empty() {
            out.insert(component.path.clone(), bound);
        }
    }
    out
}

/// 组件脚本里 `properties` 的声明与默认值形态。
fn declared_properties(
    project: &Project,
    wxml_path: &str,
) -> Option<BTreeMap<String, PropertyDefault>> {
    let scope = project.scope_of(wxml_path)?;
    let script = project.scripts.iter().find(|file| file.scope == scope)?;
    Some(script.doc.declared_properties())
}

/// 元素上所有绑定了背景色的属性名（`background: {{x}}` / `background-color: {{x}}`）。
fn bound_properties(doc: &WxmlDoc, element: &WxmlElement) -> Vec<String> {
    let mut out = Vec::new();
    for attribute in &element.attributes {
        if !attribute.name.eq_ignore_ascii_case("style") {
            continue;
        }
        let style = match attribute.value.as_deref() {
            Some(value) => value,
            // 动态属性（`style="… background: {{x}}"`）`value` 是 None，回源码取原文。
            None => raw_value(doc.slice(attribute.start, attribute.end)),
        };
        out.extend(bound_property(style));
    }
    out
}

/// 元素的背景色是否由样式表（class / 标签选择器）给出。
///
/// 空串的内联声明会被 CSS 丢弃，样式表里的背景照样生效；判不了（选择器超出受限文法、
/// 动态 class）时按「有背景」处理——这条规则宁可漏报。
fn has_sheet_background(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> bool {
    if !class_is_static(element) {
        return true;
    }
    ["background", "background-color"]
        .into_iter()
        .any(
            |property| match cascade_entries_with(project, tree, element, property) {
                None => true,
                Some(entries) => entries.iter().any(|entry| !entry.inline),
            },
        )
}

fn class_is_static(element: &WxmlElement) -> bool {
    element
        .attributes
        .iter()
        .filter(|attribute| attribute.name == "class")
        .all(|attribute| attribute.is_static)
}

/// 从属性的源码原文（`style="…"`）里取出取值部分。
fn raw_value(attribute: &str) -> &str {
    attribute
        .split_once('=')
        .map(|(_, value)| value.trim().trim_matches(['"', '\'']))
        .unwrap_or("")
}

/// 从一条 `style` 里取出 `background: {{prop}}` / `background-color: {{prop}}` 的属性名。
fn bound_property(style: &str) -> Vec<String> {
    let mut out = Vec::new();
    for declaration in style.split(';') {
        let Some((name, value)) = declaration.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        if name != "background" && name != "background-color" {
            continue;
        }
        let Some(inner) = value.trim().strip_prefix("{{") else {
            continue;
        };
        let Some(inner) = inner.strip_suffix("}}") else {
            continue;
        };
        let inner = inner.trim();
        if !inner.is_empty()
            && inner
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '_' | '$'))
        {
            out.push(inner.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{ID, TransparentBackground, bound_property};
    use crate::rules::testutil::wxml::{Fixture, check_rule};

    fn counts(fixture: &Fixture) -> usize {
        check_rule(fixture, &TransparentBackground)
            .iter()
            .filter(|diagnostic| diagnostic.rule == ID)
            .count()
    }

    fn fixture(name: &str, call: &str) -> Fixture {
        fixture_with(
            name,
            call,
            "Component({ properties: { background: { type: String, value: '' } } });",
            r#"<view class="nav" style="height: 88rpx; background: {{background}}"><slot/></view>"#,
            "",
        )
    }

    fn fixture_with(
        name: &str,
        call: &str,
        component_js: &str,
        component_wxml: &str,
        component_wxss: &str,
    ) -> Fixture {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                r#"{"navigationStyle":"custom","usingComponents":{"nav-bar":"/components/nav/index"}}"#,
            )
            .write("pages/index/index.wxml", call)
            .write("components/nav/index.json", r#"{"component":true}"#)
            .write("components/nav/index.wxml", component_wxml)
            .write("components/nav/index.js", component_js);
        if !component_wxss.is_empty() {
            fixture.write("components/nav/index.wxss", component_wxss);
        }
        fixture
    }

    #[test]
    fn omitted_background_is_reported() {
        let fixture = fixture("nav-omitted", r#"<nav-bar title="标题"/>"#);
        assert_eq!(counts(&fixture), 1);
    }

    #[test]
    fn passed_background_is_skipped() {
        let fixture = fixture(
            "nav-passed",
            r##"<nav-bar title="标题" background="#fff"/>"##,
        );
        assert_eq!(counts(&fixture), 0);
    }

    #[test]
    fn static_empty_background_is_reported() {
        let fixture = fixture("nav-empty", r#"<nav-bar title="标题" background=""/>"#);
        assert_eq!(counts(&fixture), 1);
    }

    #[test]
    fn dynamic_background_is_skipped() {
        let fixture = fixture(
            "nav-dynamic",
            r#"<nav-bar title="标题" background="{{theme}}"/>"#,
        );
        assert_eq!(counts(&fixture), 0);
    }

    #[test]
    fn possibly_empty_ternary_is_skipped() {
        let fixture = fixture(
            "nav-ternary",
            r#"<nav-bar title="标题" background="{{ flag ? '#fff' : '' }}"/>"#,
        );
        assert_eq!(counts(&fixture), 0);
    }

    #[test]
    fn non_empty_default_is_skipped() {
        let fixture = fixture_with(
            "nav-default",
            r#"<nav-bar title="标题"/>"#,
            r#"Component({ properties: { background: { type: String, value: '#fff' } } });"#,
            r#"<view class="nav" style="background: {{background}}"><slot/></view>"#,
            "",
        );
        assert_eq!(counts(&fixture), 0);
    }

    #[test]
    fn null_default_is_reported() {
        let fixture = fixture_with(
            "nav-null",
            r#"<nav-bar title="标题"/>"#,
            r#"Component({ properties: { background: { type: String, value: null } } });"#,
            r#"<view class="nav" style="background: {{background}}"><slot/></view>"#,
            "",
        );
        assert_eq!(counts(&fixture), 1);
    }

    #[test]
    fn property_missing_from_properties_is_skipped() {
        let fixture = fixture_with(
            "nav-data",
            r#"<nav-bar title="标题"/>"#,
            "Component({ data: { background: '' } });",
            r#"<view class="nav" style="background: {{background}}"><slot/></view>"#,
            "",
        );
        assert_eq!(counts(&fixture), 0);
    }

    #[test]
    fn set_data_written_property_is_skipped() {
        let fixture = fixture_with(
            "nav-setdata",
            r#"<nav-bar title="标题"/>"#,
            "Component({ properties: { background: { type: String, value: '' } }, lifetimes: { attached() { this.setData({ background: '#fff' }) } } });",
            r#"<view class="nav" style="background: {{background}}"><slot/></view>"#,
            "",
        );
        assert_eq!(counts(&fixture), 0);
    }

    #[test]
    fn class_background_is_skipped() {
        let fixture = fixture_with(
            "nav-class",
            r#"<nav-bar title="标题"/>"#,
            "Component({ properties: { background: { type: String, value: '' } } });",
            r#"<view class="nav" style="background: {{background}}"><slot/></view>"#,
            ".nav { background: #fff; }\n",
        );
        assert_eq!(counts(&fixture), 0);
    }

    #[test]
    fn class_background_color_is_skipped() {
        let fixture = fixture_with(
            "nav-class-color",
            r#"<nav-bar title="标题"/>"#,
            "Component({ properties: { background: { type: String, value: '' } } });",
            r#"<view class="nav" style="background: {{background}}"><slot/></view>"#,
            ".nav { background-color: #fff; }\n",
        );
        assert_eq!(counts(&fixture), 0);
    }

    #[test]
    fn two_bound_properties_report_once() {
        let fixture = fixture_with(
            "nav-two",
            r#"<nav-bar title="标题"/>"#,
            "Component({ properties: { background: { type: String, value: '' }, tint: { type: String, value: '' } } });",
            r#"<view class="nav" style="background: {{background}}; background-color: {{tint}}"><slot/></view>"#,
            "",
        );
        let diagnostics = check_rule(&fixture, &TransparentBackground);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(diagnostics[0].message.contains("`background`"));
        assert!(diagnostics[0].message.contains("`tint`"));
    }

    #[test]
    fn plain_element_is_skipped() {
        let fixture = fixture("nav-plain", r#"<view class="x"/>"#);
        assert_eq!(counts(&fixture), 0);
    }

    /// 视觉用例的阳性/阴性页就是这份夹具的最小复现：组件与页面源码都取自用例本身。
    #[test]
    fn visual_cases_trigger_the_rule() {
        let component = &super::VISUAL_COMPONENTS[0];
        let [positive, negative] = super::VISUAL_CASES else {
            panic!("预期两条视觉用例");
        };
        for (case, expected) in [(positive, 1), (negative, 0)] {
            let fixture = Fixture::new("nav-visual");
            fixture
                .write("project.config.json", r#"{"miniprogramRoot":""}"#)
                .write(
                    "app.json",
                    r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
                )
                .write(
                    "pages/index/index.json",
                    r#"{"usingComponents":{"nav":"/components/nav/index"}}"#,
                )
                .write("pages/index/index.wxml", case.wxml)
                .write("pages/index/index.wxss", case.wxss)
                .write("components/nav/index.json", r#"{"component":true}"#)
                .write("components/nav/index.wxml", component.wxml)
                .write("components/nav/index.wxss", component.wxss)
                .write("components/nav/index.js", component.js.unwrap_or_default());
            assert_eq!(counts(&fixture), expected, "{}", case.note);
        }
    }

    #[test]
    fn parses_background_binding_only() {
        assert_eq!(
            bound_property("height: 88rpx; background: {{background}}"),
            vec!["background".to_string()]
        );
        assert_eq!(
            bound_property("background-color:{{ color }}"),
            vec!["color".to_string()]
        );
        assert!(bound_property("background: #fff").is_empty());
        assert!(bound_property("background: rgba(0,0,0,.5)").is_empty());
        assert!(bound_property("color: {{textColor}}").is_empty());
    }
}
