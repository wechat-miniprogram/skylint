//! `<share-element>` 的 `key` 必填且页面内唯一。
//!
//! 依据：`skyline-components/references/special/share-element.md:63` 的通用属性表
//! 「key | string | - | **必填**，页面内唯一标识」。
//!
//! 判定口径：
//! - 完全没有 `key` 属性才报缺失。`key="{{k}}"` 这类动态值不报——静态阶段无法确认，
//!   而文档的示例本身就是 `key="image-{{detail.id}}"`，报出来全是误报。
//! - 重复只在**同一个 WXML 文件内**、两个 key 都是静态值时判，报在第二个及之后的
//!   元素上。动态 key 无法参与比较。
//! - 同一 `wx:if` 的不同分支互斥，运行时不会同时渲染，因此不算重复。
//!
//! **已知边界：不做跨文件的「页面内唯一」。** `Project.deps` 里有 page → component
//! 的边，理论上能算出「这个页面最终渲染了哪些 wxml」。但 key 是运行期配对语义：
//! 同一个组件被列表渲染多次时 key 会重复，而实例数量在静态阶段不可知；反过来一个
//! 组件也可能根本没被任何页面使用。按 scope 做全局唯一性误报率远高于收益，因此
//! 只做文件内唯一性，跨文件重复留给运行期。

use crate::diag::{Diagnostic, Severity};
use crate::input::wxml::{Branch, WxmlDoc, WxmlElement};
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};

use super::{element_diagnostic, visit, walk};

pub const ID: &str = "skyline/share-element-key";

const TAG: &str = "share-element";

fn find_attr<'a>(
    element: &'a WxmlElement,
    name: &str,
) -> Option<&'a crate::input::wxml::WxmlAttribute> {
    element
        .attributes
        .iter()
        .find(|attr| attr.name.eq_ignore_ascii_case(name))
}

pub struct ShareElementKey;

impl Rule for ShareElementKey {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Error,
            description: "`<share-element>` 的 key 必填且同一文件内唯一",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit(project, check_file)
    }
}

fn check_file(doc: &WxmlDoc, out: &mut Vec<Diagnostic>) {
    // 已出现的 (key, 分支栈)，按文档顺序；比较时跳过互斥分支。
    // 模板定义体里的 `share-element` 只在实例化处出现，跨定义的重复不一定是错，跳过。
    let mut seen: Vec<(String, Vec<Branch>)> = Vec::new();
    walk(doc, |element, _ancestors| {
        if element.tag != TAG || element.in_template {
            return;
        }
        let key = match find_attr(element, "key") {
            None => {
                out.push(
                    element_diagnostic(
                        ID,
                        doc,
                        element,
                        doc.path(),
                        Severity::Error,
                        "`<share-element>` 未声明 `key`，共享元素动画靠 key 在两个页面之间配对，缺了不会生效",
                    )
                    .help("给每个 `<share-element>` 声明页面内唯一的 `key`，通常拼上数据 id"),
                );
                return;
            }
            Some(attr) if !attr.is_static => return,
            Some(attr) => attr.value.as_deref().unwrap_or("").trim(),
        };
        // 空 key（`key=""` 或裸属性）无法参与配对，静态阶段不判。
        if key.is_empty() {
            return;
        }
        let duplicates = seen
            .iter()
            .filter(|(other_key, other_branches)| {
                other_key.as_str() == key
                    && !other_branches.iter().any(|left| {
                        element
                            .branches
                            .iter()
                            .any(|right| left.group == right.group && left.arm != right.arm)
                    })
            })
            .count();
        if duplicates > 0 {
            out.push(
                element_diagnostic(
                    ID,
                    doc,
                    element,
                    doc.path(),
                    Severity::Error,
                    format!(
                        "`<share-element>` 的 `key` \"{key}\" 与前面的元素重复（该 key 已出现 {duplicates} 次），共享元素靠 key 配对，重复时无法确定动画配到哪一个节点"
                    ),
                )
                .help("每个 `<share-element>` 用页面内唯一的 `key`；列表里重复渲染时把数据 id 拼进 key"),
            );
        }
        seen.push((key.to_string(), element.branches.clone()));
    });
}

#[cfg(test)]
mod tests {
    use super::ShareElementKey;
    use crate::Severity;
    use crate::rules::testutil::wxml::{Fixture, check_rule, project};

    fn check(fixture: &Fixture) -> Vec<crate::Diagnostic> {
        check_rule(fixture, &ShareElementKey)
    }

    // ---- 缺失 ----

    #[test]
    fn missing_key_is_reported() {
        let diagnostics = check(&project(
            "share-missing",
            "<share-element><image/></share-element>",
        ));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert!(diagnostics[0].message.contains("未声明"));
    }

    #[test]
    fn every_missing_key_is_reported() {
        let diagnostics = check(&project(
            "share-missing-two",
            "<share-element><image/></share-element><share-element><image/></share-element>",
        ));
        assert_eq!(diagnostics.len(), 2);
    }

    // ---- 重复 ----

    #[test]
    fn second_same_key_is_reported() {
        let diagnostics = check(&project(
            "share-dup",
            "<share-element key=\"hero\"/>\n<share-element key=\"hero\"/>",
        ));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert_eq!(diagnostics[0].line, 2);
        assert!(diagnostics[0].message.contains("hero"));
        assert!(diagnostics[0].message.contains("重复"));
    }

    #[test]
    fn three_same_keys_report_each_extra() {
        let diagnostics = check(&project(
            "share-dup-three",
            concat!(
                r#"<share-element key="hero"/>"#,
                r#"<share-element key="hero"/>"#,
                r#"<share-element key="hero"/>"#,
            ),
        ));
        assert_eq!(diagnostics.len(), 2);
    }

    #[test]
    fn duplicate_across_nesting_levels_is_reported() {
        let diagnostics = check(&project(
            "share-dup-nested",
            concat!(
                r#"<share-element key="hero"/>"#,
                r#"<view><view><share-element key="hero"/></view></view>"#,
            ),
        ));
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn different_keys_are_allowed() {
        let diagnostics = check(&project(
            "share-distinct",
            concat!(
                r#"<share-element key="hero"/>"#,
                r#"<share-element key="cover"/>"#,
            ),
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    // ---- 动态 key ----

    #[test]
    fn dynamic_key_is_not_reported_as_missing() {
        let diagnostics = check(&project(
            "share-dynamic",
            r#"<share-element key="image-{{detail.id}}"><image/></share-element>"#,
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn two_identical_dynamic_keys_are_not_compared() {
        let diagnostics = check(&project(
            "share-dynamic-two",
            concat!(
                r#"<share-element key="{{k}}"/>"#,
                r#"<share-element key="{{k}}"/>"#,
            ),
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn empty_key_is_not_reported() {
        // 静态阶段无法判断 `key=""` 的实际含义，按「宁可漏报」跳过。
        let diagnostics = check(&project("share-empty", r#"<share-element key=""/>"#));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    // ---- 互斥分支 ----

    #[test]
    fn same_key_in_exclusive_branches_is_allowed() {
        let diagnostics = check(&project(
            "share-branches",
            concat!(
                r#"<block wx:if="{{a}}"><share-element key="hero"/></block>"#,
                r#"<block wx:else><share-element key="hero"/></block>"#,
            ),
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn same_key_outside_and_inside_a_branch_is_reported() {
        let diagnostics = check(&project(
            "share-branch-mixed",
            concat!(
                r#"<share-element key="hero"/>"#,
                r#"<block wx:if="{{a}}"><share-element key="hero"/></block>"#,
            ),
        ));
        assert_eq!(diagnostics.len(), 1);
    }

    // ---- 空文件 / 其他组件 ----

    #[test]
    fn no_share_element_is_allowed() {
        let diagnostics = check(&project("share-none", "<view><image/></view>"));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn key_on_other_components_is_ignored() {
        let diagnostics = check(&project(
            "share-other",
            concat!(
                r#"<view key="a"/>"#,
                r#"<view key="a"/>"#,
                r#"<swiper-item key="a"/>"#,
            ),
        ));
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    // ---- 跨文件：不做，确认没有意外命中 ----

    #[test]
    fn same_key_in_page_and_component_is_not_reported() {
        let fixture = Fixture::new("share-cross-file");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                r#"{"usingComponents":{"my-box":"/components/box/box"}}"#,
            )
            .write("pages/index/index.wxml", r#"<share-element key="hero"/>"#)
            .write("components/box/box.json", r#"{"component":true}"#)
            .write("components/box/box.wxml", r#"<share-element key="hero"/>"#);

        let diagnostics = check(&fixture);
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    // ---- 容错 ----

    #[test]
    fn broken_file_does_not_stop_other_files() {
        let fixture = Fixture::new("share-broken");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/bad/bad","pages/good/good"],"renderer":"skyline"}"#,
            )
            .write("pages/bad/bad.json", "{}")
            .write("pages/bad/bad.wxml", "<view>{{ a</view>")
            .write("pages/good/good.json", "{}")
            .write("pages/good/good.wxml", "<share-element/>");

        let diagnostics = check(&fixture);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].path, "pages/good/good.wxml");
    }
}
