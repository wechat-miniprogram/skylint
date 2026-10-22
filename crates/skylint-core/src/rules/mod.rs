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

pub trait Rule {
    fn meta(&self) -> RuleMeta;

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
            Box::new(wxml::HorizontalScrollEnableFlex),
            Box::new(wxss::NoPseudo),
            Box::new(wxss::BoxSizing),
            Box::new(wxss::FlexDirection),
            Box::new(wxss::TextOverflowEllipsis),
            Box::new(wxss::TagSelectorIsolation),
            Box::new(wxss::UnsupportedProperty),
            Box::new(wxss::UnsupportedValue),
            Box::new(wxss::ValueDegraded),
            Box::new(wxss::GradientLimit),
            Box::new(wxss::MultiValueLimit),
            Box::new(wxss::FilterLimit),
            Box::new(wxss::TextDecorationLine),
            Box::new(wxss::TextDecorationTarget),
            Box::new(wxss::CalcAngle),
            Box::new(wxss::MaskImageGradient),
            Box::new(config::LibVersion),
            Box::new(config::RendererOptions),
        ])
    }

    pub fn from_rules(rules: Vec<Box<dyn Rule>>) -> Self {
        Self { rules }
    }

    pub fn metas(&self) -> Vec<RuleMeta> {
        self.rules.iter().map(|r| r.meta()).collect()
    }

    /// 规则文档的唯一来源：README 的规则表直接由它生成，不再手写。
    pub fn to_markdown(&self) -> String {
        let mut out = String::from("| 规则 | 级别 | 说明 |\n|---|---|---|\n");
        for meta in self.metas() {
            out.push_str(&format!(
                "| `{}` | {} | {} |\n",
                meta.id,
                meta.severity.as_str(),
                meta.description
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
}
