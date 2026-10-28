//! 模板体引用了某个字段，但**所有** `<template is="x">` 调用点都没传它。
//!
//! 内联模板的数据作用域是一个新建对象：`<template is>` 的 `data` 就是模板体的全部
//! 可见数据，看不到页面 / 组件的 `data` 与 `properties`（`proc_gen/tag.rs:1275-1287`）。
//! 漏传的键不会报错，模板体里对它的引用恒为 `undefined`，`wx:if` 直接不成立。
//!
//! 真实事故：`weapp-novel-plugin` 的 `b3ebf6ba` 把 `<template is>` 的 `data` 换成新版
//! 本、模板主体留旧版，模板体里 `isOfficial` 用了 17 次，调用点一个都没传——官方文末
//! 互动区与作者行整块不渲染，页面上没有任何报错。
//!
//! 为什么按「所有调用点都没传」而不是「这个调用点没传」：模板常被有选择地传入部分
//! 键，未传的键靠 `undefined` 走「关闭」分支（同一个 `Blocks` 模板在这类工程里有 8 个
//! 调用点故意不传 `isOfficial`，只有官方页那两处传）。逐调用点判会在真实工程上产生
//! 十来条噪音，而「一个都没传」几乎必然是漏改。
//!
//! 口径与边界：
//! - 只比较静态信息：`is` 必须是静态模板名，`data` 必须是对象字面量（含 spread 或写成
//!   变量时判不了，跳过）。
//! - 模板体的「需要」= 属性表达式、`wx:if` / `wx:elif` 条件、以及**文本插值**里引用的
//!   数据字段；`wx:for` 的 `item` / `index`、WXS 模块名是作用域引用，不在其中。
//! - 跨文件 `<import>` 的模板不做。

use std::collections::{BTreeMap, BTreeSet};

use crate::diag::{Diagnostic, Severity};
use crate::input::wxml::{WxmlDoc, WxmlNode};
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};

use super::visit;

pub const ID: &str = "skyline/template-data-scope";

pub struct TemplateDataScope;

impl Rule for TemplateDataScope {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Warning,
            description: "模板体引用的字段在它的所有 `<template is>` 调用点都没传，引用恒为 undefined",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit(project, check_file)
    }
}

/// 模板名 → 模板体引用到的字段（元素、文本插值、分支条件）。
fn collect_needed<'a>(nodes: &'a [WxmlNode], needed: &mut BTreeMap<&'a str, BTreeSet<String>>) {
    for node in nodes {
        match node {
            WxmlNode::Element(element) => {
                if let Some(name) = element.template.as_deref() {
                    let fields = needed.entry(name).or_default();
                    for attribute in &element.attributes {
                        if let Some(expression) = &attribute.expression {
                            fields.extend(crate::input::expr::data_fields(expression));
                        }
                    }
                    for branch in &element.branches {
                        fields.extend(branch.fields.iter().cloned());
                    }
                }
                collect_needed(&element.children, needed);
            }
            WxmlNode::Text(text) => {
                if let Some(name) = text.template.as_deref() {
                    needed.entry(name).or_default().extend(text.data_fields());
                }
            }
            WxmlNode::Comment => {}
        }
    }
}

fn check_file(doc: &WxmlDoc, out: &mut Vec<Diagnostic>) {
    // 模板名 → 模板体用到的字段。
    let mut needed: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    collect_needed(&doc.nodes, &mut needed);

    // 模板名 → 所有顶层调用点提供的键的并集；判不了 data 的调用点整体跳过该模板。
    let mut provided: BTreeMap<&str, Option<BTreeSet<&str>>> = BTreeMap::new();
    for call in &doc.template_calls {
        // 定义体内部的调用（递归模板）由外层调用负责，它的 data 是模板体内的表达式。
        if call.template.is_some() {
            continue;
        }
        let entry = provided
            .entry(call.name.as_str())
            .or_insert_with(|| Some(BTreeSet::new()));
        match (&call.data_keys, &mut *entry) {
            (Some(keys), Some(acc)) => acc.extend(keys.iter().map(String::as_str)),
            _ => *entry = None,
        }
    }

    for (name, fields) in &needed {
        let Some(Some(provided)) = provided.get(name) else {
            continue;
        };
        let missing: Vec<&str> = fields
            .iter()
            .map(String::as_str)
            .filter(|field| !provided.contains(field))
            .collect();
        let Some(first) = doc
            .template_calls
            .iter()
            .find(|call| call.template.is_none() && call.name == *name)
        else {
            continue;
        };
        if missing.is_empty() {
            continue;
        }
        let (line, column) = doc.position(first.start);
        out.push(
            Diagnostic::new(
                ID,
                Severity::Warning,
                format!(
                    "模板 `{name}` 的模板体引用了 {}，但它的 `<template is=\"{name}\">` 调用点都没有提供这些字段：模板作用域只看得到 `data`，这些引用恒为 `undefined`",
                    missing
                        .iter()
                        .map(|field| format!("`{field}`"))
                        .collect::<Vec<_>>()
                        .join("、")
                ),
                doc.path(),
                line,
                column,
            )
            .help("把缺的字段加进各调用点的 `data` 对象字面量，或从模板体里删掉这些引用"),
        );
    }
}

#[cfg(test)]
mod tests {
    use crate::rules::testutil::wxml::{check_rule, project};

    use super::TemplateDataScope;

    const DEF: &str = r#"<template name="Card">
  <view wx:if="{{official}}" class="{{officialName}}">{{title}}</view>
  <text>{{title}}</text>
</template>
"#;

    #[test]
    fn field_used_only_in_text_interpolation_is_reported() {
        // 文本里的插值同样吃模板作用域：`{{label}}` 只看得到调用点的 `data`。
        let diagnostics = check_rule(
            &project(
                "template-scope-text",
                r#"<template name="Card"><text>{{label}}</text></template><template is="Card" data="{{ {} }}"/>"#,
            ),
            &TemplateDataScope,
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(
            diagnostics[0].message.contains("`label`"),
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn field_missing_from_every_call_site_is_reported() {
        let diagnostics = check_rule(
            &project(
                "template-scope-missing",
                &format!("{DEF}<template is=\"Card\" data=\"{{{{ title }}}}\"/>"),
            ),
            &TemplateDataScope,
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(
            diagnostics[0].message.contains("`official`"),
            "{diagnostics:#?}"
        );
        assert!(
            diagnostics[0].message.contains("`officialName`"),
            "条件与属性里的引用都算：{diagnostics:#?}"
        );
        assert_eq!(diagnostics[0].severity, crate::Severity::Warning);
    }

    #[test]
    fn field_provided_by_any_call_site_is_not_reported() {
        // 模板常被有选择地传部分键，未传的靠 undefined 关闭分支。
        let diagnostics = check_rule(
            &project(
                "template-scope-partial",
                &format!(
                    "{DEF}<template is=\"Card\" data=\"{{{{ title }}}}\"/>
<template is=\"Card\" data=\"{{{{ title, official: true, officialName }}}}\"/>"
                ),
            ),
            &TemplateDataScope,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn unknown_data_is_skipped() {
        // `data` 写成变量 / 含 spread 时不知道提供了什么，不报。
        let diagnostics = check_rule(
            &project(
                "template-scope-unknown",
                &format!(
                    "{DEF}<template is=\"Card\" data=\"{{{{ item }}}}\"/>
<template is=\"Card\" data=\"{{{{ ...rest }}}}\"/>"
                ),
            ),
            &TemplateDataScope,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn dynamic_template_name_is_skipped() {
        let diagnostics = check_rule(
            &project(
                "template-scope-dynamic-name",
                &format!("{DEF}<template is=\"{{{{ name }}}}\" data=\"{{{{}}}}\"/>"),
            ),
            &TemplateDataScope,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn loop_variables_are_not_template_fields() {
        // `item` / `index` 是 `wx:for` 引入的作用域，不需要调用方提供。
        let diagnostics = check_rule(
            &project(
                "template-scope-loop",
                r#"<template name="List">
  <view wx:for="{{rows}}" wx:key="index">{{item.name}}-{{index}}</view>
</template>
<template is="List" data="{{ rows }}"/>"#,
            ),
            &TemplateDataScope,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn recursive_call_inside_the_body_needs_no_separate_report() {
        let diagnostics = check_rule(
            &project(
                "template-scope-recursive",
                r#"<template name="Node">
  <view>{{label}}</view>
  <template is="Node" data="{{label}}"/>
</template>
<template is="Node" data="{{label}}"/>"#,
            ),
            &TemplateDataScope,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }
}
