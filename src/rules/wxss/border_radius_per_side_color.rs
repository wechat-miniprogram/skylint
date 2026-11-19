//! 各边颜色不同的 border 会让 Skyline 丢掉同一块的 `border-radius`。
//!
//! WebView 下 `border-top-color` 之类只给某一条边换色，圆角照常绘制；Skyline 在
//! 检测到各边颜色不一致时不再走圆角路径，直接画成方角。静态盒子同样如此，与动画
//! 无关。常见的 loading 圆环（圆角 + 单边高亮色）因此变成方块。
//!
//! 判据只看同一块内的声明组合：有 `border-radius` / `border-*-radius`，且
//! - 有 `border-top-color` / `border-right-color` / `border-bottom-color` /
//!   `border-left-color`，或
//! - `border-color` 在顶层给了两个及以上值（各边颜色不同）。
//!
//! 只有一个 `border-color` 值、只写 `border` 简写、或两者分属不同规则块时都不报：
//! 这些写法下各边同色，圆角正常。
//!
//! 修法：各边统一颜色；需要高亮某一边时改用独立元素（或背景色块）叠在该边上。

use crate::input::wxss::{ValueToken, WxssDeclaration, WxssRule};
use crate::project::Project;
use crate::{Diagnostic, Severity};

use super::value_util::{at_value_with, is_dynamic};
use super::visit_declarations;
use crate::rules::{Rule, RuleMeta};

pub struct BorderRadiusPerSideColor;

impl BorderRadiusPerSideColor {
    pub const ID: &'static str = "skyline/border-radius-per-side-color";
}

const HELP: &str = "把各边 border 颜色统一后圆角即可正常绘制；需要高亮某一边时，用独立元素（或背景色块）叠在该边上，不要靠单边 border-color 区分";

impl Rule for BorderRadiusPerSideColor {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "各边颜色不同的 border 与 border-radius 同时出现时，Skyline 忽略圆角",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        visit_declarations(project, |sheet, rule, decl, out| {
            let doc = &sheet.doc;
            // 一条规则块只报一次：多个角各自写 `border-*-radius` 是同一种问题。
            let Some(radius) = rule.declarations.iter().find(|decl| is_radius(&decl.name)) else {
                return;
            };
            if !std::ptr::eq(radius, decl) {
                return;
            }
            // 变量 / 插值可能解析成 0，静态判不了。
            if is_dynamic(&radius.tokens) || radius.value.trim().is_empty() {
                return;
            }
            if radius
                .value
                .split_whitespace()
                .all(crate::rules::wxml::style::is_zero)
            {
                return;
            }
            let Some(color) = per_side_color(rule) else {
                return;
            };
            out.push(at_value_with(
                Self::ID,
                doc,
                radius,
                Severity::Warning,
                format!(
                    "同一块里各边 border 颜色不同（`{}: {}`）时，Skyline 会忽略这里的 `{}: {}`，圆角按直角绘制；按钮、loading 圆环等会变成方块",
                    color.name, color.value, radius.name, radius.value
                ),
                HELP,
            ));
        })
    }
}

/// `border-radius` 或四个角的 `border-*-radius`。
fn is_radius(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name == "border-radius" || (name.starts_with("border-") && name.ends_with("-radius"))
}

/// 块内按边指定颜色的声明；各边同色时不返回。
fn per_side_color(rule: &WxssRule) -> Option<&WxssDeclaration> {
    rule.declarations.iter().find(|decl| {
        if is_dynamic(&decl.tokens) || decl.value.trim().is_empty() {
            return false;
        }
        match decl.name.to_ascii_lowercase().as_str() {
            "border-top-color"
            | "border-right-color"
            | "border-bottom-color"
            | "border-left-color" => true,
            // 两个及以上顶层值 = 各边颜色不同；单值各边同色。
            "border-color" => {
                decl.tokens
                    .iter()
                    .filter(|token| !matches!(token, ValueToken::Comma))
                    .count()
                    >= 2
            }
            _ => false,
        }
    })
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testutil::wxss::{messages, page};

    use super::BorderRadiusPerSideColor;

    fn report(name: &str, wxss: &str) -> Vec<String> {
        let (_fixture, project) = page(name, r#"<view class="a"/><view class="b"/>"#, wxss);
        messages(&BorderRadiusPerSideColor, &project)
    }

    /// 简写给各边同色、单边再覆盖颜色：圆角被忽略。
    #[test]
    fn single_side_color_override_is_reported() {
        let out = report(
            "radius-side-color",
            ".a { border: 6rpx solid #f0f0f0; border-top-color: #ff5e00; border-radius: 50%; }",
        );
        assert_eq!(out.len(), 1, "{out:?}");
        assert!(out[0].contains("忽略"), "{out:?}");
        assert!(out[0].contains("border-top-color"), "{out:?}");
        assert!(
            out[0].contains("方角") || out[0].contains("直角"),
            "{out:?}"
        );
    }

    /// `border-color` 多值表示四边不同色。
    #[test]
    fn multi_value_border_color_is_reported() {
        let out = report(
            "radius-multi-color",
            ".a { border-color: #f00 #0f0 #00f #ff0; border-radius: 8rpx; }",
        );
        assert_eq!(out.len(), 1, "{out:?}");
        assert!(out[0].contains("border-color"), "{out:?}");
    }

    /// 没有圆角就没有可丢的东西。
    #[test]
    fn per_side_color_without_radius_is_silent() {
        assert_eq!(
            report(
                "radius-multi-color-no-radius",
                ".a { border-color: #f00 #0f0 #00f #ff0; }",
            ),
            Vec::<String>::new()
        );
    }

    /// 只改简写、各边同色：圆角正常。
    #[test]
    fn uniform_border_with_radius_is_silent() {
        assert_eq!(
            report(
                "radius-uniform",
                ".a { border: 6rpx solid #f0f0f0; border-radius: 50%; }",
            ),
            Vec::<String>::new()
        );
    }

    /// `border-color` 单值各边同色。
    #[test]
    fn single_value_border_color_is_silent() {
        assert_eq!(
            report(
                "radius-single-color",
                ".a { border-color: #f00; border-radius: 8rpx; }",
            ),
            Vec::<String>::new()
        );
    }

    /// 颜色和圆角不在同一块：两条规则各自生效，不被互相影响。
    #[test]
    fn declarations_in_different_blocks_are_silent() {
        assert_eq!(
            report(
                "radius-separate-blocks",
                ".a { border-top-color: #ff5e00; } .b { border-radius: 50%; }",
            ),
            Vec::<String>::new()
        );
    }

    /// 圆角写成 0 时报告没有意义。
    #[test]
    fn zero_radius_is_silent() {
        assert_eq!(
            report(
                "radius-zero",
                ".a { border-top-color: #ff5e00; border-radius: 0; }",
            ),
            Vec::<String>::new()
        );
    }

    /// 多个角写法只报一条，且定位在圆角声明上。
    #[test]
    fn corner_radii_report_once() {
        let (_fixture, project) = page(
            "radius-corners",
            r#"<view class="a"/>"#,
            ".a { border-top-color: #ff5e00; border-top-left-radius: 8rpx; border-top-right-radius: 8rpx; }",
        );
        let diags = BorderRadiusPerSideColor.check(&project);
        assert_eq!(diags.len(), 1, "{diags:#?}");
        assert_eq!(diags[0].line, 1);
        assert_eq!(diags[0].path, "pages/index/index.wxss");
    }

    /// 变量值静态判不了，不报。
    #[test]
    fn dynamic_values_are_silent() {
        assert_eq!(
            report(
                "radius-dynamic",
                ".a { border-top-color: var(--c); border-radius: 50%; }",
            ),
            Vec::<String>::new()
        );
    }
}
