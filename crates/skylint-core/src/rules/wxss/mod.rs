//! WXSS 语义类规则。
//!
//! 一条规则一个文件：手写判定的少量规则、纯数据驱动的属性/值支持表、以及
//! 「值的个数/结构」类限制各占一个文件。它们共用同一个 [`visit`] 与
//! [`value_util`] 的值分词工具。

pub mod box_sizing;
pub mod calc_angle;
pub mod dict_table;
pub mod filter_limit;
pub mod flex_direction;
pub mod gradient_limit;
pub mod mask_image_gradient;
pub mod multi_value_limit;
pub mod no_pseudo;
pub mod selector;
pub mod text_decoration_line;
pub mod text_overflow_ellipsis;
pub mod unsupported_property;
pub mod unsupported_value;
pub mod value_util;

pub use box_sizing::BoxSizing;
pub use calc_angle::CalcAngle;
pub use filter_limit::FilterLimit;
pub use flex_direction::FlexDirection;
pub use gradient_limit::GradientLimit;
pub use mask_image_gradient::MaskImageGradient;
pub use multi_value_limit::MultiValueLimit;
pub use no_pseudo::NoPseudo;
pub use text_decoration_line::TextDecorationLine;
pub use text_overflow_ellipsis::TextOverflowEllipsis;
pub use unsupported_property::UnsupportedProperty;
pub use unsupported_value::UnsupportedValue;

use crate::Diagnostic;
use crate::input::wxss::WxssDoc;
use crate::project::Project;

/// 遍历全部样式文件（页面 / 组件 / `app.wxss` / 被 `@import` 进来的）。
///
/// 数据来自 `Project.styles`：建图时已经解析过一遍，规则侧不重复读盘。不按
/// `is_skyline` 过滤：这是**迁移**检查工具，目标用户是还在 WebView 上的项目。
/// 若只查已迁移的页面，纯 WebView 工程会一条都不报，「迁移前拿到问题清单」的用途
/// 就落空了。
pub(crate) fn visit(
    project: &Project,
    mut f: impl FnMut(&WxssDoc, &mut Vec<Diagnostic>),
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for sheet in &project.styles {
        // 单文件读失败 / 解析失败 / 非 `.wxss` 源（`.less`、`.scss`）一律跳过，不终止扫描。
        if !sheet.doc.path().to_ascii_lowercase().ends_with(".wxss") {
            continue;
        }
        f(&sheet.doc, &mut out);
    }
    out
}

#[cfg(test)]
pub(crate) mod testutil {
    use std::fs;
    use std::path::PathBuf;

    use crate::project::Project;
    use crate::rules::Rule;

    pub(crate) struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        pub(crate) fn new(name: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("skylint-wxss-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            Self { root }
        }

        pub(crate) fn write(&self, rel: &str, content: &str) -> &Self {
            self.write_bytes(rel, content.as_bytes())
        }

        pub(crate) fn write_bytes(&self, rel: &str, content: &[u8]) -> &Self {
            let path = self.root.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
            self
        }

        pub(crate) fn project(&self) -> Project {
            Project::load(&self.root).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    /// 单页 Skyline 工程，页面 wxss 即 `wxss`。
    pub(crate) fn single(name: &str, wxss: &str) -> (Fixture, Project) {
        let f = Fixture::new(name);
        f.write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", "<view/>")
            .write("pages/index/index.wxss", wxss);
        let project = f.project();
        (f, project)
    }

    pub(crate) fn messages(rule: &dyn Rule, project: &Project) -> Vec<String> {
        rule.check(project).into_iter().map(|d| d.message).collect()
    }
}
