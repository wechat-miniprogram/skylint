//! `~foo` / `^foo`：glass-easel 的**作用域类前缀**。
//!
//! 这不是写错，是 glass-easel 承认的写法：`class_list.ts:131-142` 的
//! `_$resolvePrefixes` 把 `~foo` 注册进**根作用域**、`^foo` 注册进**父级作用域**，
//! 老渲染器 exparser 同样实现（`global-console/src/lib/exparser.js:3008`）。用途是把
//! 组件内的节点绑到页面（或父级）的样式表上，绕开组件样式隔离——迁移工程用它把
//! 「只该由页面控制」的类从组件作用域里摘出去。
//!
//! 代价是**同一个类的组件内样式同时失效**：`~foo` 只登记根作用域，本单元样式表里的
//! `&{class}` 规则（作用域是本单元）匹配不到这个节点。实测 novel plugin 迁移时给
//! `ad-banner.wxml` 加上了 `~novel-bottom-banner-{{...}}`，而组件自己的
//! `ad-banner.less` 里`.novel-bottom-banner-N { background-color: … }` 还在——banner
//! 的背景色就这么丢了（页面侧那份只写子节点文字颜色）。
//!
//! 所以判据很窄：只在本单元样式表里确实声明了这个类时才报。

use std::collections::BTreeMap;

use crate::diag::{Diagnostic, Severity};
use crate::matcher;
use crate::project::{Project, Scope};
use crate::rules::{Rule, RuleMeta, Target};

use super::{span_diagnostic, visit, walk};

pub const ID: &str = "skyline/root-scope-class";

pub struct RootScopeClass;

impl RootScopeClass {
    pub const ID: &'static str = ID;
}

impl Rule for RootScopeClass {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Warning,
            description: "`~` / `^` 作用域类前缀会让本单元样式表里的同名规则失效",
        }
    }

    fn target(&self) -> Target {
        Target::Both
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit(project, |doc, out| {
            let Some(scope) = project.scope_of(doc.path()) else {
                return;
            };
            // 页面节点上 `~x` / `^x` 登记到的就是页面自己的 scope（`class_list.ts:107`
            // 的 `_$owner` 为空时 `rootScope = styleScope`），与不加前缀等价。
            if matches!(scope, crate::project::Scope::Page(_)) {
                return;
            }
            let local = local_classes(project, &scope);
            if local.is_empty() {
                return;
            }
            walk(doc, |element, _| {
                for attribute in &element.attributes {
                    if attribute.name != "class" {
                        continue;
                    }
                    let mut matched: BTreeMap<String, (String, char)> = BTreeMap::new();
                    for token in class_tokens(attribute) {
                        let Some((prefix, name)) = scope_token(&token) else {
                            continue;
                        };
                        let mut hits: Vec<(&String, &String)> = Vec::new();
                        if let Some((class, sheet)) = local.get_key_value(name) {
                            // 同一份样式表若也被页面（根作用域）引入，`~x`/`^x` 在那边就是普通类名，
                            // 规则照样命中——原文案「不再命中」不成立。
                            if declared_in_page_scope(project, name) {
                                continue;
                            }
                            hits.push((class, sheet));
                        } else if name.ends_with('-') || name.ends_with('_') {
                            // 半截词：`~banner-{{i}}` 对上 `banner-0` 这一族。只认数字后缀，
                            // 否则 `~novel-force-` 会连 `novel-force-container` 一起算进来。
                            hits.extend(local.iter().filter(|(class, _)| {
                                class.strip_prefix(name).is_some_and(|suffix| {
                                    !suffix.is_empty()
                                        && suffix.chars().all(|ch| ch.is_ascii_digit())
                                })
                            }));
                        }
                        for (class, sheet) in hits {
                            matched
                                .entry(class.clone())
                                .or_insert((sheet.clone(), prefix));
                        }
                    }
                    if matched.is_empty() {
                        continue;
                    }
                    let names = matched
                        .keys()
                        .map(|name| format!("`.{name}`"))
                        .collect::<Vec<_>>()
                        .join("、");
                    let scope_label = if matched.values().any(|(_, prefix)| *prefix == '~') {
                        "根"
                    } else {
                        "父级"
                    };
                    let sheet = matched
                        .values()
                        .map(|(sheet, _)| sheet.as_str())
                        .collect::<std::collections::BTreeSet<_>>()
                        .into_iter()
                        .collect::<Vec<_>>()
                        .join("、");
                    out.push(
                        span_diagnostic(
                            ID,
                            doc,
                            attribute.start,
                            attribute.end,
                            doc.path(),
                            Severity::Warning,
                            format!(
                                "`class` 里的作用域前缀把 {names} 绑到了{scope_label}作用域，`{sheet}` 里同名的规则不再命中这些节点，组件自身的样式（背景色等）会失效"
                            ),
                        )
                        .help(
                            "要两端都生效，就把组件内那份样式复制到页面（或父级组件）的样式表里；只想在某个渲染器下生效时用 `.xxx.skyline` / `.xxx.webview` 分支，别用作用域前缀",
                        ),
                    );
                }
            });
        })
    }
}

/// 本单元样式表里声明过的类名 → 声明它的样式表路径。
///
/// 全局样式表（`app.wxss` 一类）在所有作用域都生效，不受作用域前缀影响，不算。
fn local_classes(project: &Project, scope: &Scope) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for sheet in &project.styles {
        if !sheet.scopes.iter().any(|candidate| candidate == scope) {
            continue;
        }
        for rule in &sheet.doc.rules {
            for selector in &rule.selectors {
                for name in matcher::class_names(selector.text.trim()).unwrap_or_default() {
                    out.entry(name)
                        .or_insert_with(|| sheet.doc.path().to_string());
                }
            }
        }
    }
    out
}

/// 该类名是否被某个页面作用域的样式表声明过。
fn declared_in_page_scope(project: &Project, name: &str) -> bool {
    project.styles.iter().any(|sheet| {
        sheet
            .scopes
            .iter()
            .any(|scope| matches!(scope, crate::project::Scope::Page(_)))
            && sheet.doc.rules.iter().any(|rule| {
                rule.selectors.iter().any(|selector| {
                    matcher::class_names(selector.text.trim())
                        .unwrap_or_default()
                        .iter()
                        .any(|class| class == name)
                })
            })
    })
}

/// 属性里的类词元：动态 class 用候选集，静态 class 直接切分。
fn class_tokens(attribute: &crate::input::wxml::WxmlAttribute) -> Vec<String> {
    match &attribute.class_tokens {
        Some(tokens) => tokens
            .definite
            .iter()
            .chain(tokens.possible.iter())
            .chain(tokens.fragments.iter())
            .cloned()
            .collect(),
        None => attribute
            .value
            .iter()
            .flat_map(|value| value.split_ascii_whitespace())
            .map(str::to_owned)
            .collect(),
    }
}

/// `~foo` / `^^foo` → (`~` / `^`, "foo")。
fn scope_token(token: &str) -> Option<(char, &str)> {
    let prefix = token.chars().next().filter(|ch| matches!(ch, '~' | '^'))?;
    let name = token.trim_start_matches(['~', '^']);
    (!name.is_empty()).then_some((prefix, name))
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testutil::wxss::Fixture;

    use super::RootScopeClass;

    fn check(name: &str, wxml: &str, wxss: &str) -> Vec<crate::Diagnostic> {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", wxml)
            .write("pages/index/index.wxss", wxss)
            .write("pages/index/index.js", "Page({})");
        RootScopeClass.check(&fixture.project())
    }

    /// 页面节点上的 `~x` 登记到页面自己的 scope，与 `x` 等价。
    #[test]
    fn page_node_prefix_is_a_noop() {
        assert!(
            check(
                "root-scope-page",
                r#"<view class="cnt ~banner"/>"#,
                ".banner { background-color: #000; }",
            )
            .is_empty()
        );
    }

    fn component_check(name: &str, wxml: &str, wxss: &str) -> Vec<crate::Diagnostic> {
        let fixture = Fixture::new(name);
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
            .write("pages/index/index.wxml", "<c/>")
            .write("pages/index/index.js", "Page({})")
            .write("components/c/c.json", r#"{"component":true}"#)
            .write("components/c/c.wxml", wxml)
            .write("components/c/c.wxss", wxss)
            .write("components/c/c.js", "Component({})");
        RootScopeClass.check(&fixture.project())
    }

    #[test]
    fn root_scope_class_shadows_local_rule() {
        // 组件内的 `.banner` 规则不再命中 `~banner` 这个节点。
        let diags = component_check(
            "root-scope",
            r#"<view class="cnt ~banner"/>"#,
            ".banner { background-color: #000; }",
        );
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert!(diags[0].message.contains("`.banner`"), "{diags:#?}");
        assert_eq!(diags[0].rule, RootScopeClass::ID);
    }

    #[test]
    fn dynamic_class_candidate_is_checked_too() {
        let diags = component_check(
            "root-scope-dynamic",
            r#"<view class="cnt ~banner-{{i}}"/>"#,
            ".banner-1 { background-color: #000; }",
        );
        assert_eq!(diags.len(), 1, "{diags:#?}");
    }

    #[test]
    fn plain_class_is_fine() {
        assert!(
            check(
                "plain-class",
                r#"<view class="banner"/>"#,
                ".banner { color: red; }",
            )
            .is_empty()
        );
    }

    #[test]
    fn scope_class_without_local_rule_is_fine() {
        // 页面那份是唯一来源时，前缀就是它存在的原因。
        assert!(
            check(
                "root-scope-no-local",
                r#"<view class="cnt ~banner"/>"#,
                ".other { color: red; }",
            )
            .is_empty()
        );
    }
}
