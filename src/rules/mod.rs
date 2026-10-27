//! 规则层。
//!
//! 一个 `Rule` 就是一个纯函数式的检查：拿到工程视图，返回诊断。规则之间不共享状态、
//! 不排序、不互相调用；需要跨规则的判定一律在规则内部显式做，不做优先级系统。
//!
//! 刻意不做的：
//! - 规则 DSL（结构类规则是图/树上的路径约束，DSL 会退化成查询语言）；
//! - 动态加载（插件机制在规则稳定性得到验证之前只是负担）；
//! - 自动修复（旧实现的 patch 层同起点 insert 顺序未定义，没有原文校验）。

pub mod config;
pub mod js;
#[cfg(test)]
pub(crate) mod testutil;
pub mod wxml;
pub mod wxss;

use std::collections::HashSet;

use crate::project::Project;
use crate::{Diagnostic, Severity};

/// 规则的静态描述。`id` 同时是用户关闭规则、生成文档、CI 过滤的键。
pub struct RuleMeta {
    pub id: &'static str,
    pub severity: Severity,
    /// 一句话说明「什么问题」。
    pub description: &'static str,
}

/// 检查方向。
///
/// - `Skyline`：WebView 代码迁到 Skyline 时哪里会坏（默认方向）。
/// - `Webview`：按 Skyline 写的代码跑在不支持 Skyline 的环境（PC 等）里，
///   退回 WebView 渲染后哪些能力会静默降级。
/// - `Both`：差异本身是双向的，两个方向都要跑。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Skyline,
    Webview,
    Both,
}

impl Target {
    /// 本方向是否要跑属于 `rule` 方向的规则。
    ///
    /// `Both` 作为检查方向表示「两个方向都跑」，不是一种规则归属。
    pub fn includes(self, rule: Target) -> bool {
        self == Target::Both || rule == Target::Both || rule == self
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Skyline => "skyline",
            Self::Webview => "webview",
            Self::Both => "both",
        }
    }
}

pub trait Rule {
    fn meta(&self) -> RuleMeta;

    /// 规则所属方向；反向（`webview`）与双向（`both`）规则要显式覆盖。
    fn target(&self) -> Target {
        Target::Skyline
    }

    /// 检查整个工程。返回的诊断按文件/位置自治，排序由 `Report` 负责。
    fn check(&self, project: &Project) -> Vec<Diagnostic>;
}

#[derive(Default)]
pub struct Registry {
    rules: Vec<Box<dyn Rule>>,
}

impl Registry {
    /// 全部内置规则。新增规则只改这里一处。
    pub fn builtin() -> Self {
        Self::from_rules(vec![
            Box::new(wxml::NoInlineText),
            Box::new(wxml::UnsupportedComponent),
            Box::new(wxml::ScrollViewType),
            Box::new(wxml::ComponentAttribute),
            Box::new(wxml::ScrollViewFamily),
            Box::new(wxml::ComponentChildren),
            Box::new(wxml::ShareElementKey),
            Box::new(wxml::HorizontalScrollEnableFlex),
            Box::new(wxml::TextHardBreakEllipsis),
            Box::new(wxml::InlineRun),
            Box::new(wxss::NoPseudo),
            Box::new(wxss::BoxSizing),
            Box::new(wxss::FlexDirection),
            Box::new(wxss::FixedLayer),
            Box::new(wxss::TextOverflowEllipsis),
            Box::new(wxss::AttributeSelector),
            Box::new(wxss::DeadSelector),
            Box::new(wxss::ScrollViewHeight),
            Box::new(wxss::ImplicitColor),
            Box::new(wxss::TagSelectorIsolation),
            Box::new(wxss::UnsupportedProperty),
            Box::new(wxss::UnsupportedValue),
            Box::new(wxss::ValueDegraded),
            Box::new(wxss::ShorthandPartial),
            Box::new(wxss::TransitionProperty),
            Box::new(wxss::GradientLimit),
            Box::new(wxss::LegacyFlexbox),
            Box::new(wxss::MediaQuery),
            Box::new(wxss::MultiValueLimit),
            Box::new(wxss::FilterLimit),
            Box::new(wxss::TextDecorationLine),
            Box::new(wxss::TextDecorationTarget),
            Box::new(wxss::CalcAngle),
            Box::new(wxss::CalcMinMax),
            Box::new(wxss::GradientUnsupported),
            Box::new(js::UnsupportedApi),
            Box::new(js::WorkletApiScope),
            Box::new(js::WorkletDirective),
            Box::new(js::WorkletDataFreeze),
            Box::new(js::WorkletCompileSetting),
            Box::new(js::ScrollContextRef),
            Box::new(js::DraggableSheetScrollTo),
            Box::new(js::RouteType),
            Box::new(config::ConfigIgnored),
            Box::new(config::DefaultLayoutDrift),
            Box::new(config::LibVersion),
            Box::new(config::RendererOptions),
            Box::new(wxml::TextTruncation),
            Box::new(wxml::NestedScroll),
            Box::new(wxml::WorkletBinding),
            Box::new(wxml::TextChildren),
            Box::new(wxml::PinnedRenderer),
            Box::new(config::TabBarRenderer),
        ])
    }

    pub fn from_rules(rules: Vec<Box<dyn Rule>>) -> Self {
        Self { rules }
    }

    /// 只保留属于该方向的规则；`--target` 通过这里生效。
    pub fn for_target(self, target: Target) -> Self {
        Self {
            rules: self
                .rules
                .into_iter()
                .filter(|rule| target.includes(rule.target()))
                .collect(),
        }
    }

    pub fn metas(&self) -> Vec<RuleMeta> {
        self.rules.iter().map(|r| r.meta()).collect()
    }

    /// 规则 + 方向；文档生成与方向不变量校验共用它。
    pub fn entries(&self) -> Vec<(RuleMeta, Target)> {
        self.rules
            .iter()
            .map(|rule| (rule.meta(), rule.target()))
            .collect()
    }

    /// 规则文档的唯一来源：`docs/RULES.md` 由它生成，不手写。
    ///
    /// 刷新方式：`cargo run --example rules_doc`。
    pub fn rules_document(&self) -> String {
        let rows = self.entries();
        let count = |severity: Severity| {
            rows.iter()
                .filter(|(meta, _)| meta.severity == severity)
                .count()
        };
        let mut out = String::from(
            "# 规则一览\n\n\
             > 本文件由 `Registry::rules_document()` 生成，不要手工编辑。\n\
             > 改了规则的 id / 级别 / 说明后，跑 `cargo run --example rules_doc` 重写。\n\n",
        );
        out.push_str(&format!(
            "共 {} 条：error {} 条、warning {} 条、info {} 条。\n\n\
             方向列：`skyline` = WebView 迁 Skyline 的检查（默认）；\
             `webview` = Skyline 代码在不支持 Skyline 的环境里退回 WebView 渲染时的降级；\
             `both` = 两个方向都跑。\n\n\
             | 规则 | 级别 | 方向 | 说明 |\n|---|---|---|---|\n",
            rows.len(),
            count(Severity::Error),
            count(Severity::Warning),
            count(Severity::Info),
        ));
        for (meta, target) in rows {
            out.push_str(&format!(
                "| `{}` | {} | {} | {} |\n",
                meta.id,
                meta.severity.as_str(),
                target.as_str(),
                escape_cell(meta.description)
            ));
        }
        out
    }

    /// `ignored` 里的 id 整条规则跳过。
    pub fn run(&self, project: &Project, ignored: &HashSet<String>) -> Vec<Diagnostic> {
        self.rules
            .iter()
            .filter(|rule| !ignored.contains(rule.meta().id))
            .flat_map(|rule| rule.check(project))
            .collect()
    }

    /// 多个视图（宿主 + 插件）依次检查，合并成一份结果。
    ///
    /// 视图内的诊断路径相对该视图自己的根，这里补上 [`Project::report_prefix`]
    /// 变成项目相对路径；工程视图产出的 warning 也一并计入。
    pub fn run_views(&self, projects: &[&Project], ignored: &HashSet<String>) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        for project in projects {
            let mut view = crate::suppress::apply(project, self.run(project, ignored));
            view.extend(project.warnings.iter().cloned());
            if !project.report_prefix.is_empty() {
                for diagnostic in &mut view {
                    diagnostic.path = format!("{}{}", project.report_prefix, diagnostic.path);
                }
            }
            diagnostics.extend(view);
        }
        diagnostics
    }
}

/// 表格单元里的 `|` 会截断单元格，裸写的 `<tag>` 会被 markdown 当成 HTML 标签吞掉——
/// 两者都按「是否在反引号内」分开处理：代码段里的 `<` 本来就是字面量，不该转义。
fn escape_cell(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_code = false;
    for ch in text.chars() {
        match ch {
            '`' => {
                in_code = !in_code;
                out.push(ch);
            }
            '|' => out.push_str("\\|"),
            '<' if !in_code => out.push_str("&lt;"),
            '>' if !in_code => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}
