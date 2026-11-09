//! 背景 / 遮罩的多值数量限制。
//!
//! 超限后不是「多出的层失效」，而是该长写的整条值回退：fp 解析到 >2 层时 `break` 出
//! switch，最终返回 `CSSUnsetValue`（`parser/mod.rs` 的 background / mask 解析路径），
//! 引擎再走 `ApplyInitial`——image 回到 `none`、position 回到 `0% 0%`、size 回到 `auto`
//! （`skyline/css/css_properties/background_*.cc`、`mask_*.cc` 的 `ApplyInitial`）。
//! `background` / `mask` 简写展开后只有 image 那条长写回退，位置 / 尺寸 / 平铺仍按原值生效。

use crate::project::Project;
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

use super::value_util::{at_value, is_dynamic, split_top_level};
use super::visit_declarations;

pub struct MultiValueLimit;

impl MultiValueLimit {
    pub const ID: &'static str = "skyline/multi-value-limit";
}

impl Rule for MultiValueLimit {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "background / mask 的多值数量超出 Skyline 支持范围",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit_declarations(project, |sheet, _rule, decl, out| {
            let doc = &sheet.doc;
            let Some(max) = value_limit(&decl.name) else {
                return;
            };
            let value = decl.value.trim();
            if value.is_empty() || is_dynamic(&decl.tokens) {
                return;
            }
            let count = split_top_level(&decl.tokens).len();
            if count <= max {
                return;
            }
            let base = decl.name.to_ascii_lowercase();
            let message = if matches!(base.as_str(), "background" | "mask") {
                // 简写会展开成多个长写，图片那条回退到 none，其它长写仍按解析结果生效。
                let image = if base == "mask" {
                    "遮罩图"
                } else {
                    "背景图"
                };
                format!(
                    "`{}` 在 Skyline 下最多支持 {max} 层，当前有 {count} 层；超限时{image}（`{base}-image`）整条回退到初始值 `none`，各层都不渲染；位置 / 尺寸 / 平铺仍按各自的值生效",
                    decl.name,
                )
            } else {
                format!(
                    "`{}` 在 Skyline 下最多支持 {max} 层，当前有 {count} 层；超限时引擎不会只丢掉多出的层，而是把整条值当成 unset 并回落到初始值（{}）",
                    decl.name,
                    fallback_value(&base)
                )
            };
            out.push(at_value(
                Self::ID,
                doc,
                decl,
                message,
                "将多值拆成多条声明，或只保留 Skyline 支持的层数",
            ));
        })
    }
}

/// 超限后整条值回退到的初始值（对应各长写属性的 `ApplyInitial`）。
fn fallback_value(name: &str) -> &'static str {
    match name.to_ascii_lowercase().as_str() {
        "background-image" | "mask-image" => "背景图 / 遮罩图为 `none`",
        "background-position"
        | "mask-position"
        | "background-position-x"
        | "mask-position-x"
        | "background-position-y"
        | "mask-position-y" => "位置为 `0% 0%`",
        "background-size" | "mask-size" => "尺寸为 `auto`",
        "background-repeat" | "mask-repeat" => "平铺方式为 `repeat`",
        _ => "初始值",
    }
}

fn value_limit(name: &str) -> Option<usize> {
    match name.to_ascii_lowercase().as_str() {
        // 每一层背景/遮罩各一份；引擎逐层应用，超过两层直接丢声明
        // （image_property_utils.h:390-403、:518-530、:655-666）。
        "background-image"
        | "mask-image"
        | "background-position"
        | "mask-position"
        | "background-position-x"
        | "mask-position-x"
        | "background-position-y"
        | "mask-position-y"
        | "background-repeat"
        | "mask-repeat"
        | "background-size"
        | "mask-size" => Some(2),
        // 简写里的层数同样是「层」：>2 时整条 `background-image` 变成 none
        // （image_property_utils.h:257-273 对 len>2 不返回值），各层全部不渲染。
        "background" | "mask" => Some(2),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;

    use super::MultiValueLimit;
    use crate::rules::testutil::wxss::{messages, page};

    #[test]
    fn shorthand_layers_are_limited_too() {
        let (_f, project) = page(
            "multi-shorthand",
            r#"<view class="a"/>"#,
            r#".a { background: url(a), url(b), url(c); }"#,
        );
        let diags = MultiValueLimit.check(&project);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert!(diags[0].message.contains("3 层"));
        // 简写只丢图片那条长写，其它长写不受影响；措辞不能说「只丢多出的层」。
        assert!(diags[0].message.contains("回退到初始值 `none`"));
        assert!(
            diags[0]
                .message
                .contains("位置 / 尺寸 / 平铺仍按各自的值生效")
        );
    }

    #[test]
    fn multi_value_reports_excess_values() {
        let (_f, project) = page(
            "multi-positive",
            r#"<view class="a"/><view class="b"/><view class="c"/><view class="d"/><view class="e"/><view class="f"/><view class="g"/><view class="h"/><view class="i"/>"#,
            r#"
            .a { background-image: url(a.png), url(b.png), url(c.png); }
            .b { background-position: 0 0, 10px 10px, 20px 20px; }
            .c { background-repeat: no-repeat, repeat, repeat-x; }
            .d { background-size: 10px 10px, 20px 20px, 30px 30px; }
            .e { mask-position: 0 0, 10px 10px, 20px 20px; }
            .f { mask-repeat: no-repeat, repeat, repeat-x; }
            .g { mask-size: 10px 10px, 20px 20px, 30px 30px; }
            .h { background-position-x: 0, 10px, 20px; }
            .i { mask-position-y: 0, 10px, 20px; }
            "#,
        );
        let diags = MultiValueLimit.check(&project);
        assert_eq!(diags.len(), 9);
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Warning));
        assert!(diags.iter().any(|d| d.message.contains("3 层")));
        assert!(
            diags
                .iter()
                .filter(|d| d.message.contains("background-image"))
                .all(|d| d.message.contains("`none`"))
        );
        assert!(
            diags
                .iter()
                .filter(|d| d.message.contains("background-position"))
                .all(|d| d.message.contains("`0% 0%`"))
        );
        assert!(
            diags
                .iter()
                .filter(|d| d.message.contains("background-size"))
                .all(|d| d.message.contains("`auto`"))
        );
    }

    #[test]
    fn multi_value_accepts_two_values_and_commas_inside_parens() {
        let (_f, project) = page(
            "multi-negative",
            r#"<view class="a"/><view class="b"/><view class="c"/><view class="d"/><view class="e"/><view class="f"/><view class="g"/><view class="h"/><view class="i"/><view class="j"/><view class="k"/><view class="l"/><view class="m"/><view class="n"/>"#,
            r#"
            .a { background-image: url(a.png), url(b.png); }
            .b { background-image: url("a,b.png"), url(c.png); }
            .c { background-position: 0 0, 10px 10px; }
            .d { background-repeat: no-repeat; }
            .e { background-size: 10px 10px; }
            .h { background-repeat: no-repeat, repeat; }
            .i { background-size: 10px 10px, 20px 20px; }
            .j { mask-position: 0 0, 10px 10px; }
            .k { mask-repeat: no-repeat, repeat; }
            .l { mask-size: 10px 10px, 20px 20px; }
            .m { background-position-x: 0, 10px; }
            .n { mask-position-y: 0, 10px; }
            .f { background-image: var(--imgs); }
            .g { background-image: linear-gradient(red, blue), url(a.png); }
            "#,
        );
        assert_eq!(messages(&MultiValueLimit, &project), Vec::<String>::new());
    }
}
