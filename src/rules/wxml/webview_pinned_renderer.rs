//! 单元的 `renderer` 被固定成一个常量：WebView 下所有按它分叉的地方都判错。
//!
//! 判据不是「哪个属性叫 renderer」，而是正向折叠用的同一份事实：单元的常量环境
//! （[`Project::env`]）里 `renderer` 能不能定成具体值。环境只有三个来源
//! （`project::collect_envs`）：
//!
//! 1. 单元脚本 `data` / `properties` 里的字面量；
//! 2. 父级以静态属性传入的值（所有使用处都传同一个静态值才算）；
//! 3. 被 `setData` / `this.data.x = …` 写过就丢弃（`dirty_data`）。
//!
//! 只要环境里 `renderer` 定得住，这个单元的作者就已经把它当成常量在用：`wx:if` 相应
//! 分支在编译期就被折叠掉了，`class="x {{renderer}}"` 也只取到那一个值。真实的渲染
//! 后端是 `this.renderer`，回退到 WebView 时可能完全相反——所以「钉死成常量」本身就是
//! 缺陷，而不是某个属性名写错。
//!
//! 真实修复形态（`weapp-novel-plugin_1` 的 `feat-glass-easel-webview`）：
//! `data.renderer` 默认 `'webview'`、`onLoad` 里 `setData({ renderer: this.renderer })`
//! （这会让该键变成 dirty、环境里不留常量），父级把 6 处 `renderer="skyline"` 改成
//! `renderer="{{ renderer }}"`。修完两条路径都判不出常量，规则自然静默。

use crate::diag::Severity;
use crate::input::env::Lit;
use crate::input::wxml::WxmlElement;
use crate::project::{Project, Scope};
use crate::rules::{Rule, RuleMeta, Target};

use super::{element_diagnostic, load_doc, walk, wxml_files};

pub const ID: &str = "webview/pinned-renderer";

pub struct PinnedRenderer;

impl Rule for PinnedRenderer {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Warning,
            description: "单元的 renderer 被固定成常量，回退 WebView 后所有按它分叉的地方都判错",
        }
    }

    fn target(&self) -> Target {
        Target::Webview
    }

    fn check(&self, project: &Project) -> Vec<crate::Diagnostic> {
        let sites = static_renderer_sites(project);
        let mut out = Vec::new();
        for path in wxml_files(project) {
            let Some(env) = project.env(path) else {
                continue;
            };
            let Some(Lit::Str(pinned)) = env.get("renderer") else {
                continue;
            };
            let Some(doc) = load_doc(project, path) else {
                continue;
            };
            let mut mentions = Vec::new();
            walk(&doc, |element: &WxmlElement, _| {
                if element.mentions_renderer() {
                    mentions.push(element);
                }
            });
            let Some(first) = mentions.first() else {
                continue;
            };
            // 钉成 `webview` 的单元只需要在它被 Skyline 页面用到时才是缺陷（那时它渲染的
            // 是 WebView 分支）；钉成 `skyline` 的单元只要回退就会错。
            if pinned != "skyline" && !reached_from_skyline_page(project, path) {
                continue;
            }
            let own = own_script_pins(project, path);
            let source = if own {
                "单元的 `properties` 默认值"
            } else {
                "父级静态属性传入的值"
            };
            let callers = call_sites(project, path, &sites);
            let where_ = if callers.is_empty() {
                String::new()
            } else {
                format!("，调用处：{callers}")
            };
            out.push(
                element_diagnostic(
                    ID,
                    &doc,
                    first,
                    doc.path(),
                    Severity::Warning,
                    format!(
                        "本单元的 `renderer` 被固定成 `{pinned}`（{source}{where_}），共有 {} 处按它分叉；回退到 WebView 时这些分支会全部判错",
                        mentions.len()
                    ),
                )
                .help(format!(
                    "改由运行时决定：`onLoad` 里 `setData({{ renderer: this.renderer }})`，父级传参改成 `renderer=\"{{{{ renderer }}}}\"`（原值 `{pinned}` 不代表真实后端）"
                )),
            );
        }
        out
    }
}

/// 单元自己的脚本里是否有 `renderer` 的字面量（`properties` 默认值）。
///
/// 用来区分「常量来自本单元脚本」与「来自父级静态传值」——两者的修法不同。
fn own_script_pins(project: &Project, wxml_path: &str) -> bool {
    let scope = unit_scope(project, wxml_path);
    project.scripts.iter().any(|file| {
        Some(&file.scope) == scope.as_ref()
            && matches!(file.doc.constants().get("renderer"), Some(Lit::Str(_)))
    })
}

fn unit_scope(project: &Project, wxml_path: &str) -> Option<Scope> {
    if let Some(page) = project
        .pages
        .iter()
        .find(|page| page.files.wxml.as_deref() == Some(wxml_path))
    {
        return Some(Scope::Page(page.route.clone()));
    }
    project
        .components
        .iter()
        .find(|component| component.files.wxml.as_deref() == Some(wxml_path))
        .map(|component| Scope::Component(component.path.clone()))
}

/// 全工程静态写死 `renderer` 的属性位置，按「父 WXML 路径」归组，带标签名。
///
/// 只用来给「父级静态传值」补上调用处——判据本身是常量环境，这里仅是坐标。
fn static_renderer_sites(
    project: &Project,
) -> std::collections::BTreeMap<String, Vec<(String, u32)>> {
    let mut out: std::collections::BTreeMap<String, Vec<(String, u32)>> = Default::default();
    for path in wxml_files(project) {
        let Some(doc) = load_doc(project, path) else {
            continue;
        };
        walk(&doc, |element: &WxmlElement, _| {
            let Some(attribute) = super::structure::find_attr(element, "renderer") else {
                return;
            };
            if !attribute.is_static || attribute.value.is_none() {
                return;
            }
            let (line, _) = doc.position(element.start);
            out.entry(doc.path().to_string())
                .or_default()
                .push((element.tag.clone(), line));
        });
    }
    out
}

/// 传 `renderer` 给这个组件的调用处。
///
/// 一个页面可以给多个子组件各传一个静态值，那些子组件同样被钉死；但每条诊断只列
/// 「确实指向这个组件」的标签，所以需要把标签名解析回组件路径：用 `deps` 缩小到
/// 直接引用它的父单元，再用 `usingComponents` 里的路径尾段与组件的路径尾段对应。
/// 尾段不唯一时宁可少列，不猜。
fn call_sites(
    project: &Project,
    wxml_path: &str,
    sites: &std::collections::BTreeMap<String, Vec<(String, u32)>>,
) -> String {
    let Some(component) = project
        .components
        .iter()
        .find(|component| component.files.wxml.as_deref() == Some(wxml_path))
    else {
        return String::new();
    };
    let Some(basename) = component.path.rsplit('/').next() else {
        return String::new();
    };
    let mut found: Vec<String> = Vec::new();
    for (unit, deps) in &project.deps {
        if !deps.contains(&component.path) {
            continue;
        }
        let Some(parent) = unit_wxml(project, unit) else {
            continue;
        };
        let tags = declared_tags(project, unit, basename);
        for (tag, line) in sites.get(parent).into_iter().flatten() {
            if tags.contains(tag) {
                found.push(format!("{parent}:{line}"));
            }
        }
    }
    if found.is_empty() {
        return String::new();
    }
    let shown = found.iter().take(3).cloned().collect::<Vec<_>>().join("、");
    if found.len() > 3 {
        format!("{shown} 等 {} 处", found.len())
    } else {
        shown
    }
}

/// 单元 json 里 `usingComponents` 指向尾段为 `basename` 的标签名。
fn declared_tags(project: &Project, unit: &str, basename: &str) -> Vec<String> {
    let json = project
        .page_json
        .get(unit)
        .or_else(|| project.component_json.get(unit));
    let Some(using) = json
        .and_then(|json| json.get("usingComponents"))
        .and_then(serde_json::Value::as_object)
    else {
        return Vec::new();
    };
    using
        .iter()
        .filter(|(_, value)| {
            value
                .as_str()
                .and_then(|path| path.rsplit('/').next())
                .is_some_and(|tail| tail == basename)
        })
        .map(|(tag, _)| tag.to_ascii_lowercase())
        .collect()
}

/// 这个组件会不会被一个 Skyline 页面用到（含多级嵌套，`app.json` 声明的也算）。
fn reached_from_skyline_page(project: &Project, wxml_path: &str) -> bool {
    let Some(component) = project
        .components
        .iter()
        .find(|component| component.files.wxml.as_deref() == Some(wxml_path))
    else {
        return false;
    };
    let mut stack: Vec<&str> = project
        .pages
        .iter()
        .filter(|page| page.is_skyline)
        .map(|page| page.route.as_str())
        .collect();
    stack.push("app.json");
    let mut seen = std::collections::BTreeSet::new();
    while let Some(unit) = stack.pop() {
        if unit == component.path {
            return true;
        }
        if !seen.insert(unit) {
            continue;
        }
        if let Some(deps) = project.deps.get(unit) {
            stack.extend(deps.iter().map(String::as_str));
        }
    }
    false
}

/// `deps` 的 key（页面路由 / 组件路径）→ 它的 WXML 路径。
fn unit_wxml<'a>(project: &'a Project, unit: &str) -> Option<&'a str> {
    project
        .pages
        .iter()
        .find(|page| page.route == unit)
        .and_then(|page| page.files.wxml.as_deref())
        .or_else(|| {
            project
                .components
                .iter()
                .find(|component| component.path == unit)
                .and_then(|component| component.files.wxml.as_deref())
        })
}

#[cfg(test)]
mod tests {
    use crate::project::Project;
    use crate::rules::Rule;
    use crate::rules::testutil::Fixture;

    use super::PinnedRenderer;

    /// 页面 → 组件：父级以静态属性把 renderer 钉成常量。
    fn with_static_prop(name: &str, value: &str) -> (Fixture, Project) {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                r#"{"usingComponents":{"card":"/components/card/card"}}"#,
            )
            .write("pages/index/index.js", "Page({ data: {} })")
            .write(
                "pages/index/index.wxml",
                &format!("<view><card renderer=\"{value}\"/></view>"),
            )
            .write("components/card/card.json", r#"{"component":true}"#)
            .write("components/card/card.js", "Component({ methods: {} })")
            .write(
                "components/card/card.wxml",
                "<view><text wx:if=\"{{renderer === 'skyline'}}\">a</text></view>",
            );
        let project = fixture.project();
        (fixture, project)
    }

    #[test]
    fn static_prop_from_parent_is_reported_on_the_child() {
        let (_f, project) = with_static_prop("webview/pinned-prop", "skyline");
        let diagnostics = PinnedRenderer.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].path, "components/card/card.wxml");
        assert!(diagnostics[0].message.contains("父级静态属性传入的值"));
        assert!(diagnostics[0].message.contains("`skyline`"));
    }

    #[test]
    fn dynamic_prop_leaves_the_env_open() {
        let (_f, project) = with_static_prop("webview/pinned-prop-dynamic", "{{renderer}}");
        assert!(PinnedRenderer.check(&project).is_empty());
    }

    /// 同一个组件被两处使用、只有一处传静态值时，环境也判不出常量。
    #[test]
    fn mixed_usages_leave_the_env_open() {
        let fixture = Fixture::new("webview/pinned-mixed");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index","pages/other/other"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                r#"{"usingComponents":{"card":"/components/card/card"}}"#,
            )
            .write("pages/index/index.js", "Page({ data: {} })")
            .write(
                "pages/index/index.wxml",
                "<view><card renderer=\"skyline\"/></view>",
            )
            .write(
                "pages/other/other.json",
                r#"{"usingComponents":{"card":"/components/card/card"}}"#,
            )
            .write("pages/other/other.js", "Page({ data: {} })")
            .write(
                "pages/other/other.wxml",
                "<view><card renderer=\"{{renderer}}\"/></view>",
            )
            .write("components/card/card.json", r#"{"component":true}"#)
            .write("components/card/card.js", "Component({ methods: {} })")
            .write(
                "components/card/card.wxml",
                "<view><text wx:if=\"{{renderer === 'skyline'}}\">a</text></view>",
            );
        assert!(PinnedRenderer.check(&fixture.project()).is_empty());
    }

    #[test]
    fn set_data_clears_a_property_default() {
        let fixture = Fixture::new("webview/pinned-setData");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                r#"{"usingComponents":{"card":"/components/card/card"}}"#,
            )
            .write("pages/index/index.js", "Page({ data: {} })")
            .write("pages/index/index.wxml", "<view><card/></view>")
            .write("components/card/card.json", r#"{"component":true}"#)
            .write(
                "components/card/card.js",
                "Component({ properties: { renderer: { type: String, value: 'skyline' } }, lifetimes: { attached() { this.setData({ renderer: this.renderer }) } } })",
            )
            .write(
                "components/card/card.wxml",
                "<view><text wx:if=\"{{renderer === 'skyline'}}\">a</text></view>",
            );
        assert!(PinnedRenderer.check(&fixture.project()).is_empty());
    }

    #[test]
    fn pinned_but_unused_is_fine() {
        let fixture = Fixture::new("webview/pinned-unused");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                r#"{"usingComponents":{"card":"/components/card/card"}}"#,
            )
            .write("pages/index/index.js", "Page({ data: {} })")
            .write(
                "pages/index/index.wxml",
                "<view><card renderer=\"skyline\"/></view>",
            )
            .write("components/card/card.json", r#"{"component":true}"#)
            .write("components/card/card.js", "Component({ methods: {} })")
            .write("components/card/card.wxml", "<view><text>a</text></view>");
        assert!(PinnedRenderer.check(&fixture.project()).is_empty());
    }

    /// 只被 WebView 页面用到的组件把 `renderer` 默认成 `webview` 是对的，不报。
    #[test]
    fn webview_page_component_is_fine() {
        let fixture = Fixture::new("webview/pinned-webview-page");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/legacy/legacy"],"renderer":"webview"}"#,
            )
            .write(
                "pages/legacy/legacy.json",
                r#"{"usingComponents":{"card":"/components/card/card"}}"#,
            )
            .write("pages/legacy/legacy.js", "Page({ data: {} })")
            .write("pages/legacy/legacy.wxml", "<view><card/></view>")
            .write("components/card/card.json", r#"{"component":true}"#)
            .write(
                "components/card/card.js",
                "Component({ properties: { renderer: { type: String, value: 'webview' } } })",
            )
            .write(
                "components/card/card.wxml",
                "<view><text wx:if=\"{{renderer === 'webview'}}\">a</text></view>",
            );
        assert!(PinnedRenderer.check(&fixture.project()).is_empty());
    }

    #[test]
    fn class_binding_counts_as_usage() {
        let (_f, project) = with_static_prop("webview/pinned-class", "skyline");
        let diagnostic = &PinnedRenderer.check(&project)[0];
        assert!(diagnostic.message.contains("1 处") || diagnostic.message.contains("共有"));
    }
}
