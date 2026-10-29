//! `skyline/transition-animation-conflict`：同一元素上，`transition` 与 `animation`
//! 动同一个属性时，Skyline 会让 transition 提前把元素推到终态，动画只跑开头几帧。
//!
//! 证据来自短剧插件 actionsheet 的真机现象（面板「走一点点就跳到终态」）：`.panel` 上写着
//! `transition: transform 0.3s`，`.panel.slide-up` 上是 `animation: slide-up …`，两者都动
//! `transform`。删掉 `transition` 后动画恢复正常。
//!
//! 判据：元素命中的 `transition`（或 `transition-property`）里列出的属性，与它命中的
//! `animation`（或 `animation-name`）对应的 `@keyframes` 里声明的属性有交集。
//! `transition-property: all` 视为与任何被动画的属性都冲突。
//!
//! 修法唯一：删掉那条 `transition`（显隐/位移已经由动画负责），所以可以入规则面。
//! 判不出来的情况一律放过：动态值、`var()`、找不到 `@keyframes` 定义、内联 `style`。

use std::collections::HashMap;

use crate::input::wxss::ValueToken;
use crate::matcher::Tree;
use crate::project::Project;
use crate::rules::Rule;
use crate::rules::RuleMeta;
use crate::rules::wxml::{
    ElementDoc, element_diagnostic, element_docs, resolved_property, walk_skyline,
};
use crate::{Diagnostic, Severity};

pub struct TransitionAnimationConflict;

/// `transition` 简写里不是属性名的关键字。
const TRANSITION_KEYWORDS: &[&str] = &[
    "ease",
    "linear",
    "ease-in",
    "ease-out",
    "ease-in-out",
    "step-start",
    "step-end",
    "normal",
    "allow-discrete",
    "infinite",
];

/// `animation` 简写里不是名字的关键字。
const ANIMATION_KEYWORDS: &[&str] = &[
    "ease",
    "linear",
    "ease-in",
    "ease-out",
    "ease-in-out",
    "step-start",
    "step-end",
    "normal",
    "reverse",
    "alternate",
    "alternate-reverse",
    "forwards",
    "backwards",
    "both",
    "running",
    "paused",
    "infinite",
    "none",
];

impl Rule for TransitionAnimationConflict {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "同一元素上 transition 与 animation 动同一个属性，Skyline 下动画只会跑开头几帧",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let keyframes = collect_keyframes(project);
        if keyframes.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for ElementDoc { doc, reach } in element_docs(project) {
            let tree = Tree::new(&doc);
            walk_skyline(&doc, |element, _| {
                let Some(transition) = transition_properties(project, &tree, element) else {
                    return;
                };
                let Some(names) = animation_names(project, &tree, element) else {
                    return;
                };
                let mut hits: Vec<(&str, &str)> = Vec::new();
                for name in &names {
                    let Some(animated) = keyframes.get(name) else {
                        continue;
                    };
                    for property in animated {
                        if transition
                            .iter()
                            .any(|item| item == "all" || item == property)
                            && !hits.iter().any(|(p, n)| p == property && n == name)
                        {
                            hits.push((property.as_str(), name.as_str()));
                        }
                    }
                }
                if hits.is_empty() {
                    return;
                }
                let (property, name) = hits[0];
                out.push(
                    element_diagnostic(
                        Self::ID,
                        &doc,
                        element,
                        doc.path(),
                        Severity::Warning,
                        Self::message(property, name),
                    )
                    .help(reach.help(Self::HELP)),
                );
            });
        }
        out
    }
}

impl TransitionAnimationConflict {
    pub const ID: &'static str = "skyline/transition-animation-conflict";

    const HELP: &'static str =
        "同一个属性只留一种机制：显隐/位移交给 animation 时删掉 transition（反过来就删 animation）";

    fn message(property: &str, name: &str) -> String {
        format!(
            "`transition` 与 `animation: {name}` 都在动 `{property}`：Skyline 下 transition 会先把元素推到终态，动画只跑开头几帧"
        )
    }
}

/// 全部 `@keyframes`：名字 → 它声明过的属性（小写）。
///
/// 输入层把关键帧块摊平成普通规则（选择器是 `from` / `to` / `50%`，`at_rules` 为空），
/// 拿不到所属动画名，所以这里直接扫源码：定位 `@keyframes` 的名字与其花括号范围。
fn collect_keyframes(project: &Project) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for sheet in &project.styles {
        for (name, body) in keyframe_blocks(sheet.doc.source()) {
            let entry = out.entry(name).or_default();
            for property in property_names(body) {
                if !entry.contains(&property) {
                    entry.push(property);
                }
            }
        }
    }
    out
}

/// 源码里每个 `@keyframes` 动画的名字，以及它花括号内的原文。
fn keyframe_blocks(source: &str) -> Vec<(String, &str)> {
    let mut out = Vec::new();
    let bytes = source.as_bytes();
    let mut cursor = 0;
    while let Some(found) = source[cursor..].find("@keyframes").or_else(|| {
        source[cursor..]
            .find("-webkit-keyframes")
            .map(|index| index.saturating_sub("-webkit".len()))
    }) {
        let at = cursor + found;
        let mut index = at + "@keyframes".len();
        while index < bytes.len() && (bytes[index] as char).is_whitespace() {
            index += 1;
        }
        let name_start = index;
        while index < bytes.len() && !matches!(bytes[index] as char, '{' | ' ' | '\n' | '\t' | '\r')
        {
            index += 1;
        }
        let name = source[name_start..index].trim().to_string();
        while index < bytes.len() && bytes[index] != b'{' {
            index += 1;
        }
        if index >= bytes.len() {
            break;
        }
        let body_start = index + 1;
        let mut depth = 1;
        index = body_start;
        while index < bytes.len() && depth > 0 {
            match bytes[index] {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            }
            index += 1;
        }
        let body_end = index.saturating_sub(1);
        if !name.is_empty() {
            out.push((name, &source[body_start..body_end]));
        }
        cursor = index;
    }
    out
}

/// 关键帧块里出现的属性名。括号内的内容（`cubic-bezier(...)`、`url(...)`）不算。
fn property_names(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let chars: Vec<char> = body.chars().collect();
    let mut depth = 0_usize;
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ':' if depth == 0 => {
                let mut end = index;
                while end > 0 && (chars[end - 1].is_ascii_alphanumeric() || chars[end - 1] == '-') {
                    end -= 1;
                }
                let name: String = chars[end..index].iter().collect();
                let name = name.trim().to_ascii_lowercase();
                if !name.is_empty() && !out.contains(&name) {
                    out.push(name);
                }
            }
            _ => {}
        }
        index += 1;
    }
    out
}

/// 元素生效的 `transition` 里列出的属性名；没有 transition 或判不出来时返回 `None`。
fn transition_properties(
    project: &Project,
    tree: &Tree<'_>,
    element: &crate::input::wxml::WxmlElement,
) -> Option<Vec<String>> {
    let longhand = resolved_property(project, tree, element, "transition-property");
    let resolved = match longhand {
        Some(resolved) => resolved,
        None => resolved_property(project, tree, element, "transition")?,
    };
    let declared = resolved
        .declaration
        .map(|declaration| declaration.tokens.as_slice());
    let idents = match declared {
        Some(tokens) => idents(tokens),
        None => idents_of_value(&resolved.value),
    };
    let properties: Vec<String> = idents
        .into_iter()
        .map(|ident| ident.to_ascii_lowercase())
        .filter(|ident| ident != "none" && !TRANSITION_KEYWORDS.contains(&ident.as_str()))
        .collect::<Vec<String>>();
    (!properties.is_empty()).then_some(properties)
}

/// 元素生效的 `animation` 引用的名字；没有 animation 或判不出来时返回 `None`。
fn animation_names(
    project: &Project,
    tree: &Tree<'_>,
    element: &crate::input::wxml::WxmlElement,
) -> Option<Vec<String>> {
    let longhand = resolved_property(project, tree, element, "animation-name");
    let resolved = match longhand {
        Some(resolved) => resolved,
        None => resolved_property(project, tree, element, "animation")?,
    };
    let declared = resolved
        .declaration
        .map(|declaration| declaration.tokens.as_slice());
    let idents = match declared {
        Some(tokens) => idents(tokens),
        None => idents_of_value(&resolved.value),
    };
    let names: Vec<String> = idents
        .into_iter()
        .map(|ident| ident.to_string())
        .filter(|ident| {
            let lower = ident.to_ascii_lowercase();
            lower != "none" && !ANIMATION_KEYWORDS.contains(&lower.as_str())
        })
        .collect();
    (!names.is_empty()).then_some(names)
}

fn idents(tokens: &[ValueToken]) -> Vec<&str> {
    tokens
        .iter()
        .filter_map(|token| match token {
            ValueToken::Ident(ident) => Some(ident.as_str()),
            _ => None,
        })
        .collect()
}

/// 内联 `style` 只有原文，按空白切分近似取标识符。
fn idents_of_value(value: &str) -> Vec<&str> {
    value
        .split(|c: char| c.is_ascii_whitespace() || c == ',')
        .filter(|part| {
            !part.is_empty()
                && !part.starts_with("var(")
                && !part.starts_with("calc(")
                && part
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '-')
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::Diagnostic;
    use crate::rules::Rule;
    use crate::rules::testutil::wxss::Fixture;

    use super::TransitionAnimationConflict;

    const SKYLINE_APP: &str = r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#;

    fn check(name: &str, wxml: &str, wxss: &str) -> Vec<Diagnostic> {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write("app.json", SKYLINE_APP)
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", wxml)
            .write("pages/index/index.wxss", wxss);
        let project = fixture.project();
        TransitionAnimationConflict.check(&project)
    }

    const PANEL: &str = r#"<view class="panel slide-up"/>"#;

    #[test]
    fn same_property_on_both_mechanisms_is_reported() {
        let diagnostics = check(
            "anim-conflict-basic",
            PANEL,
            "@keyframes slide-up { from { transform: translate3d(0, 100%, 0); } \
             to { transform: translate3d(0, 0, 0); } }\n\
             .panel { transition: transform 0.3s; }\n\
             .slide-up { animation: slide-up ease 0.3s forwards; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("`transform`"));
        assert!(diagnostics[0].message.contains("slide-up"));
        assert_eq!(diagnostics[0].line, 1);
    }

    #[test]
    fn different_properties_are_not_reported() {
        let diagnostics = check(
            "anim-conflict-different",
            PANEL,
            "@keyframes slide-up { from { transform: translate3d(0, 100%, 0); } \
             to { transform: translate3d(0, 0, 0); } }\n\
             .panel { transition: opacity 0.3s; }\n\
             .slide-up { animation: slide-up ease 0.3s forwards; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn transition_all_is_reported() {
        let diagnostics = check(
            "anim-conflict-all",
            PANEL,
            "@keyframes slide-up { to { transform: translate3d(0, 0, 0); } }\n\
             .panel { transition: all 0.3s; }\n\
             .slide-up { animation: slide-up 0.3s; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    #[test]
    fn transition_without_animation_is_not_reported() {
        let diagnostics = check(
            "anim-conflict-no-anim",
            r#"<view class="panel"/>"#,
            ".panel { transition: transform 0.3s; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn unknown_animation_name_is_not_reported() {
        // `@keyframes` 在别的文件/动态值里定义时判不出来，宁可漏报。
        let diagnostics = check(
            "anim-conflict-unknown",
            PANEL,
            ".panel { transition: transform 0.3s; }\n.slide-up { animation: from-js 0.3s; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }
}
