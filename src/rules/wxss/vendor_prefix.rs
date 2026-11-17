//! 带 `-webkit-` 前缀的动画写法在 Skyline 下被丢弃，而 WebView 认它们。
//!
//! 两种形态都来自「构建期 autoprefix」（devtools 的 `postcss` / `minifyWXSS` 任一开启
//! 就会跑，且只在 prod 构建生效）：
//!
//! 1. `@-webkit-keyframes`：Skyline 的关键帧解析只认 `@keyframes`，带前缀的整块被丢；
//!    WebView（Chromium）会把 `animation: name` 别名到 `-webkit-keyframes`，所以只有
//!    当「这个名字没有无前缀孪生块」时才构成真实差异。
//! 2. `-webkit-animation*` / `-webkit-transition*` **属性名**：fp 的属性名闭集里没有
//!    任何厂商前缀，整条声明回滚。只有同一条规则里没有标准属性兜底时才构成差异
//!    （autoprefixer 正常会同时写两条）。
//!
//! 带前缀的 `transition` **值**（`transition: -webkit-transform …`）由
//! `skyline/transition-property` 负责，这里不重复报。

use std::collections::HashSet;

use crate::input::wxss::WxssDoc;
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

use super::keyframes;
use super::value_util::at_declaration;
use super::visit_declarations;

pub const ID: &str = "skyline/vendor-prefix";

/// 会被 autoprefixer 加前缀的动画/过渡属性（去掉 `-webkit-` 后的名字）。
const ANIMATION_FAMILY: &[&str] = &[
    "animation",
    "animation-name",
    "animation-duration",
    "animation-timing-function",
    "animation-delay",
    "animation-iteration-count",
    "animation-direction",
    "animation-fill-mode",
    "animation-play-state",
    "transition",
    "transition-property",
    "transition-duration",
    "transition-timing-function",
    "transition-delay",
];

pub struct VendorPrefix;

impl VendorPrefix {
    pub const ID: &'static str = ID;
}

impl Rule for VendorPrefix {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Warning,
            description: "带 `-webkit-` 前缀的关键帧块 / 动画属性在 Skyline 下被丢弃，WebView 认得",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        for sheet in &project.styles {
            let doc = &sheet.doc;
            orphan_prefixed_keyframes(doc, &mut out);
        }
        out.extend(visit_declarations(project, |sheet, rule, decl, out| {
            let name = decl.name.to_ascii_lowercase();
            let Some(base) = name.strip_prefix("-webkit-") else {
                return;
            };
            if !ANIMATION_FAMILY.contains(&base) {
                return;
            }
            // 同一条规则里已经写了标准属性：两端都能用，前缀那份只是冗余。
            if rule
                .declarations
                .iter()
                .any(|other| other.name.eq_ignore_ascii_case(base))
            {
                return;
            }
            out.push(at_declaration(
                ID,
                &sheet.doc,
                decl,
                Severity::Warning,
                format!(
                    "`{name}` 在 Skyline 下不被识别，整条声明会被丢弃；这条规则里没有标准的 `{base}` 兜底"
                ),
                format!("改成标准属性 `{base}: …`（同一条规则里两份都写时两端一致）"),
            ));
        }));
        out
    }
}

/// 只有带前缀、没有同名的无前缀块时才是差异。
fn orphan_prefixed_keyframes(doc: &WxssDoc, out: &mut Vec<Diagnostic>) {
    let found = keyframes::blocks(doc.source());
    let plain: HashSet<&str> = found
        .iter()
        .filter(|block| block.prefix.is_none())
        .map(|block| block.name)
        .collect();
    for block in found
        .iter()
        .filter(|block| block.prefix == Some("-webkit-") && !plain.contains(block.name))
    {
        let (line, column) = doc.position(block.offset);
        out.push(
            Diagnostic::new(
                ID,
                Severity::Warning,
                format!(
                    "`@-webkit-keyframes {}` 只在带前缀的形式里定义：WebView 认这个名字，Skyline 不解析带前缀的 at-rule，`animation: {}` 不会播放",
                    block.name, block.name
                ),
                doc.path(),
                line,
                column,
            )
            .help(format!(
                "再写一份无前缀的 `@keyframes {}`（前缀块留着不影响），或改用一个有无前缀定义的关键帧",
                block.name
            )),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::VendorPrefix;
    use crate::rules::Rule;
    use crate::rules::testutil::wxss::{messages, page};

    #[test]
    fn orphan_prefixed_keyframes_is_reported() {
        let (_f, project) = page(
            "prefix-keyframes",
            r#"<view class="a"/>"#,
            "@-webkit-keyframes fade { from { opacity: 0 } to { opacity: 1 } }\n.a { animation: fade .3s; }",
        );
        let diagnostics = VendorPrefix.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("@-webkit-keyframes fade"));
        assert_eq!(diagnostics[0].severity, crate::Severity::Warning);
    }

    #[test]
    fn paired_keyframes_are_not_reported() {
        let (_f, project) = page(
            "prefix-keyframes-pair",
            r#"<view class="a"/>"#,
            r#"
            @-webkit-keyframes fade { from { opacity: 0 } to { opacity: 1 } }
            @keyframes fade { from { opacity: 0 } to { opacity: 1 } }
            .a { animation: fade .3s; }
            "#,
        );
        assert_eq!(messages(&VendorPrefix, &project), Vec::<String>::new());
    }

    #[test]
    fn prefixed_animation_property_without_fallback_is_reported() {
        let (_f, project) = page(
            "prefix-animation-prop",
            r#"<view class="a"/>"#,
            ".a { -webkit-animation: fade .3s; }",
        );
        let diagnostics = VendorPrefix.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("-webkit-animation"));
    }

    #[test]
    fn prefixed_property_with_standard_sibling_is_not_reported() {
        let (_f, project) = page(
            "prefix-with-fallback",
            r#"<view class="a"/>"#,
            ".a { -webkit-animation: fade .3s; animation: fade .3s; }",
        );
        assert_eq!(messages(&VendorPrefix, &project), Vec::<String>::new());
    }

    /// `-moz-` / `-ms-` 前缀 WebView 也不认，两端都是死代码，不是迁移差异。
    #[test]
    fn non_webkit_prefixes_are_ignored() {
        let (_f, project) = page(
            "prefix-moz",
            r#"<view class="a"/>"#,
            "@-moz-keyframes fade { from { opacity: 0 } }\n.a { -moz-animation: fade .3s; }",
        );
        assert_eq!(messages(&VendorPrefix, &project), Vec::<String>::new());
    }
}
