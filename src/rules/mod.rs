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
pub mod general;
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

/// 规则级自定义组件夹具，生成器写成 `components/<name>/index.*`。
///
/// 由 [`Rule::visual_components`] 返回，生成器会把它挂到**该规则的每个**用例页面上。
/// 组件内部节点从页面层查不到（选择器不跨组件边界），所以需要观察组件内部时
/// 只能用 [`VisualMode::Page`] 的整页比对。
pub struct VisualComponent {
    /// 标签名，如 `"c"`；页面 WXML 用 `<c>` 引用。
    pub name: &'static str,
    pub wxml: &'static str,
    pub wxss: &'static str,
    /// 缺省写 `Component({})`。
    pub js: Option<&'static str>,
}

/// 规则级页面附属文件：生成器写到 `pages/<slug>/<path>`（相对页面目录）。
///
/// 需要真实文件的场景只用它：例如 `<wxs src="./probe.wxs">`，内联 `<wxs>` 不进
/// `project.scripts`，规则看不到。
pub struct VisualFile {
    pub path: &'static str,
    pub content: &'static str,
}

/// 采集脚本在页面渲染完成后、取 bbox 与截图之前注入的交互动作。
///
/// 用于差异必须靠输入才能触发的规则（WXS 事件处理器的第二参数、touchmove 冒泡）。
pub enum VisualAction {
    /// 点击选择器命中的元素（SDK 的 `Element.tap()`）。
    Tap(&'static str),
    /// 在元素 bbox 中心起算的滑动：选择器、CSS px 的 dx、dy。
    Swipe(&'static str, i32, i32),
    /// 等待指定毫秒数。
    Wait(u32),
}

/// 规则的两端渲染差异用例：一个小程序页面，证明这条规则描述的现象在 Skyline 与
/// WebView 里真的不同（或是阴性对照，证明两端确实一致）。
///
/// 用例只描述「页面内容 + 期望」，采集与判定由 `tools/visual/run.sh` 负责。
pub struct VisualCase {
    pub wxml: &'static str,
    pub wxss: &'static str,
    pub js: Option<&'static str>,
    pub expect: VisualExpect,
    /// 证明什么，一句话。
    pub note: &'static str,
    /// 页内关键元素选择器。`""` 表示整页比对；其它值按元素比对。
    pub target: &'static str,
}

/// 采集脚本的比对范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualMode {
    /// 取 `target` 选择器的 bbox，按 bbox 与元素区域像素判定。
    Element,
    /// 整页比对：bbox 视为整张截图，按整页像素差判定。
    Page,
}

impl VisualMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Element => "element",
            Self::Page => "page",
        }
    }
}

impl VisualCase {
    /// `target == ""` 走整页比对；其余按元素比对。
    ///
    /// 旧实现把空串隐式当成 `"#target"`；现在两者语义分开，元素比对必须显式写
    /// `"#target"`（或其它选择器）。
    pub fn mode(&self) -> VisualMode {
        if self.target.is_empty() {
            VisualMode::Page
        } else {
            VisualMode::Element
        }
    }

    /// 采集脚本要取 bbox 的选择器；整页模式没有选择器。
    pub fn selector(&self) -> Option<&'static str> {
        match self.mode() {
            VisualMode::Element => Some(self.target),
            VisualMode::Page => None,
        }
    }
}

/// 用例期望的比对结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualExpect {
    /// 两端该元素几何不同（bbox 任一边差 ≥4 设备 px）或元素区域内像素差 ≥1%。
    Differ,
    /// 两端该元素几何相同且区域内像素几乎一致。
    Same,
}

impl VisualExpect {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Differ => "differ",
            Self::Same => "same",
        }
    }
}

/// 规则的证据用例：一份完整最小工程，加一条「构建/运行 → 断言」的采集链。
///
/// 与 [`VisualCase`] 的分工：`VisualCase` 证明「同一份源码在两端渲染结果不同」；
/// `EvidenceCase` 证明「构建产物、构建日志或运行期取值符合/违反规则主张」。两端本来
/// 就不该有视觉差异的规则（构建期校验、运行期告警、产物改写）只能走这条。
///
/// 用例自带完整工程而不是复用视觉生成器：后者会强制写 `renderer` /
/// `componentFramework` / `lazyCodeLoading`，并对 `rendererOptions` 断言，而证据用例
/// 恰恰需要逐条控制这些键（缺一个键就是用例本身）。
pub struct EvidenceCase {
    /// 用例名；同一规则内唯一，同时是目录名（只允许 `[a-z0-9-]`）。
    pub name: &'static str,
    /// 完整最小工程文件，路径相对工程根。
    pub files: &'static [VisualFile],
    /// `project.config.json` 的内容；`None` 写默认的
    /// `{"miniprogramRoot":"","appid":"touristappid"}`。
    pub project_config: Option<&'static str>,
    pub run: EvidenceRun,
    pub expect: EvidenceExpect,
    /// 这条用例证明什么，一句话；报告里直接展示。
    pub note: &'static str,
}

/// 证据用例的采集方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceRun {
    /// 只构建：`sim build --project <case> --out <case>/dist`。
    Build,
    /// 构建后起 serve 并加载页面：收集 console，或执行 evaluate。
    Serve { page: &'static str },
}

impl EvidenceRun {
    /// `Serve` 要加载的页面路由；`Build` 没有页面。
    pub fn page(self) -> Option<&'static str> {
        match self {
            Self::Build => None,
            Self::Serve { page } => Some(page),
        }
    }
}

/// 证据用例的断言。
///
/// 所有 `must` / `must_not` 都是**字面子串**，不是正则：规则与文案要能逐字对照，
/// 正则会把「文案改了但断言还能过」变成静默失效。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceExpect {
    /// 构建成功（exit 0）且产出 `dist/`。
    BuildOk,
    /// 构建必须失败（exit ≠ 0），且 stderr 含全部片段。
    BuildFails {
        stderr_contains: &'static [&'static str],
    },
    /// 构建成功，且构建日志（stdout + stderr）含 `must`、不含 `must_not`。
    BuildLog {
        must: &'static [&'static str],
        must_not: &'static [&'static str],
    },
    /// 构建成功，且 `dist/<path>` 含 `must`、不含 `must_not`。
    Artifact {
        path: &'static str,
        must: &'static [&'static str],
        must_not: &'static [&'static str],
    },
    /// 页面加载后收集的 console 日志含 `must`、不含 `must_not`。
    Console {
        must: &'static [&'static str],
        must_not: &'static [&'static str],
    },
    /// 页面上下文 `evaluate(expr)` 的结果等于 `equals`。
    ///
    /// 字符串结果直接比较，其余按 `JSON.stringify` 比较（`true` / `3` / `{}`）。
    Runtime {
        expr: &'static str,
        equals: &'static str,
    },
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

    /// 这个元素在 `target` 方向的运行时是否会出现。
    ///
    /// `wx:if="{{renderer === 'webview'}}"` 的分支在 Skyline 下不渲染，反过来同理；
    /// 对它讲另一个方向的行为是必错的，所以由声明的方向统一决定，不留给规则自选。
    pub(crate) fn renders(self, element: &crate::input::wxml::WxmlElement) -> bool {
        match self {
            Self::Skyline => !element.webview_only(),
            Self::Webview => !element.skyline_only(),
            Self::Both => true,
        }
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

    /// 两端渲染差异用例；空表示还没补。
    ///
    /// 生成器 `examples/rule_visual_cases.rs` 与覆盖率门禁消费它。
    fn visual_cases(&self) -> &'static [VisualCase] {
        &[]
    }

    /// 构建产物/日志/运行期取值的证据用例；空表示还没补。
    ///
    /// 生成器 `examples/rule_evidence_cases.rs` 与 `tools/evidence/run.sh` 消费它。
    /// 两端没有视觉差异、只能产物或日志取证的规则用这条。
    fn evidence_cases(&self) -> &'static [EvidenceCase] {
        &[]
    }

    /// 本规则所有视觉用例共用的组件夹具；空表示页面不引用自定义组件。
    ///
    /// 刻意不放进 [`VisualCase`]：组件是规则级的，且给结构体加字段会让所有既有
    /// 用例字面量都要同步补齐。
    fn visual_components(&self) -> &'static [VisualComponent] {
        &[]
    }

    /// 本规则所有视觉用例共用的页面附属文件；空表示页面不引用额外文件。
    ///
    /// 与 [`Self::visual_components`] 同理，放在规则级：给 [`VisualCase`] 加字段会让
    /// 所有既有用例字面量都要同步补齐。
    fn visual_files(&self) -> &'static [VisualFile] {
        &[]
    }

    /// 本规则所有视觉用例渲染后要注入的动作，按声明顺序执行。默认不做任何输入。
    ///
    /// 只有单条用例需要不同注入点时才覆盖 [`Self::visual_case_actions`]。
    fn visual_actions(&self) -> &'static [VisualAction] {
        &[]
    }

    /// 单条视觉用例要注入的动作；默认沿用规则级的 [`Self::visual_actions`]。
    ///
    /// 同一规则的多条用例需要不同触点时才覆盖（例如阴性对照的滑动打在容器上、
    /// 阳性用例必须打在滚动内容上）。与组件夹具同理放在规则级：给 [`VisualCase`]
    /// 加字段会让所有既有用例字面量都要同步补齐。
    fn visual_case_actions(&self, index: usize) -> &'static [VisualAction] {
        let _ = index;
        self.visual_actions()
    }
}

/// 视觉用例生成器用的规则条目：元数据、用例、组件夹具、页面附属文件、
/// 逐用例的动作（与用例一一对应）。
pub type VisualEntry = (
    RuleMeta,
    &'static [VisualCase],
    &'static [VisualComponent],
    &'static [VisualFile],
    Vec<&'static [VisualAction]>,
);

/// 证据用例生成器用的规则条目：元数据与用例列表。
pub type EvidenceEntry = (RuleMeta, &'static [EvidenceCase]);

#[derive(Default)]
pub struct Registry {
    rules: Vec<Box<dyn Rule>>,
}

impl Registry {
    /// 全部内置规则。新增规则只改这里一处。
    pub fn builtin() -> Self {
        Self::from_rules(vec![
            Box::new(wxml::NestedTextBlock),
            Box::new(wxml::TextBoxProperty),
            Box::new(wxml::ImageBlockLayout),
            Box::new(general::TransparentBackground),
            Box::new(wxml::ComponentSupport),
            Box::new(wxml::ScrollGestureConflict),
            Box::new(wxml::ComponentAttribute),
            Box::new(wxml::ScrollViewFamily),
            Box::new(wxml::ComponentChildren),
            Box::new(wxml::ShareElementKey),
            // 已停用：模板数据作用域是框架语义，两个渲染器一致，不是迁移差异（源码保留）
            // Box::new(wxml::TemplateDataScope),
            Box::new(wxml::TextHardBreakEllipsis),
            Box::new(wxml::InlineRun),
            Box::new(js::PageScrollApi),
            Box::new(wxss::NoPseudo),
            Box::new(wxss::PseudoInlineLayout),
            Box::new(wxss::BoxSizing),
            Box::new(wxss::BorderRadiusPerSideColor),
            // FIXME: 待确认规则通用性
            // Box::new(wxss::ButtonAfterReset),
            Box::new(wxss::FlexDirection),
            Box::new(wxss::FixedLayer),
            Box::new(wxss::FixedOffsetMissing),
            Box::new(wxss::FixedZIndexInversion),
            Box::new(wxss::TextOnlyProperty),
            Box::new(wxss::SelectorSubset),
            // 已停用：死选择器是通用 lint，与本工具「WebView 正常 / Skyline 异常」的收录标准无关（源码保留）
            // Box::new(wxss::DeadSelector),
            Box::new(wxss::ScrollViewHeight),
            Box::new(wxss::NativeLayoutIgnored),
            Box::new(wxss::NativePadding),
            Box::new(wxss::ImplicitColor),
            Box::new(wxss::TagSelectorIsolation),
            Box::new(wxss::UnsupportedProperty),
            Box::new(wxss::UnsupportedValue),
            Box::new(wxss::ValueDegraded),
            Box::new(wxss::ShorthandPartial),
            Box::new(wxss::TransitionAnimationConflict),
            Box::new(wxss::TransitionProperty),
            Box::new(wxss::VendorPrefix),
            Box::new(wxss::GradientSupport),
            Box::new(wxss::LegacyFlexbox),
            Box::new(wxss::MediaQuery),
            Box::new(wxss::MultiValueLimit),
            Box::new(wxss::FilterLimit),
            Box::new(wxss::TextDecorationLine),
            Box::new(wxss::CalcMinMax),
            Box::new(js::UnsupportedApi),
            Box::new(js::WxsSyncQuery),
            Box::new(js::WorkletApiScope),
            Box::new(js::WorkletDirective),
            Box::new(js::WorkletDataFreeze),
            Box::new(js::WorkletCompileSetting),
            Box::new(js::ScrollContext),
            Box::new(js::DraggableSheetScrollTo),
            Box::new(js::RouteType),
            Box::new(js::DynamicStyleValue),
            Box::new(config::AutoprefixBuild),
            Box::new(config::ConfigIgnored),
            Box::new(config::DefaultLayoutDrift),
            Box::new(config::LibVersion),
            Box::new(config::RendererOptions),
            Box::new(wxml::TextTruncation),
            Box::new(wxml::NestedScroll),
            Box::new(wxml::WorkletBinding),
            Box::new(wxml::TextChildren),
            // 已停用：renderer 被钉成常量是构建期约定问题，不是渲染差异（源码保留）
            // Box::new(wxml::PinnedRenderer),
            Box::new(config::TabBarRenderer),
            Box::new(config::LazyCodeLoading),
            Box::new(config::ComponentFramework),
            Box::new(config::NavigationStyle),
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

    /// 有视觉用例的规则：id、用例列表、规则级组件夹具、页面附属文件与逐用例的动作。
    /// 生成器把它写成可运行的小程序工程。
    pub fn visual_cases(&self) -> Vec<VisualEntry> {
        self.rules
            .iter()
            .map(|rule| {
                let cases = rule.visual_cases();
                (
                    rule.meta(),
                    cases,
                    rule.visual_components(),
                    rule.visual_files(),
                    (0..cases.len())
                        .map(|index| rule.visual_case_actions(index))
                        .collect(),
                )
            })
            .filter(|(_, cases, _, _, _)| !cases.is_empty())
            .collect()
    }

    /// 有证据用例的规则：id 与用例列表。生成器把每条用例写成一个完整工程。
    pub fn evidence_cases(&self) -> Vec<EvidenceEntry> {
        self.rules
            .iter()
            .map(|rule| (rule.meta(), rule.evidence_cases()))
            .filter(|(_, cases)| !cases.is_empty())
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

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::path::{Component, Path};

    use super::Registry;

    /// 证据用例的静态约束：名字能当目录名、工程非空且没有越界路径、`project_config` 是
    /// JSON 对象、`Serve` 指定的页面 json 确实在 `files` 里。生成器写盘前会再校验一次，
    /// 这里提前在单测里失败。
    #[test]
    fn evidence_cases_are_well_formed() {
        let registry = Registry::builtin();
        let entries = registry.evidence_cases();
        assert!(!entries.is_empty(), "至少应有一组证据用例");

        for (meta, cases) in entries {
            let mut names = HashSet::new();
            for case in cases {
                assert!(
                    names.insert(case.name),
                    "规则 `{}` 的用例名重复：{}",
                    meta.id,
                    case.name
                );
                assert!(
                    !case.name.is_empty()
                        && case
                            .name
                            .chars()
                            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-'),
                    "规则 `{}` 的用例名必须是 [a-z0-9-]：{:?}",
                    meta.id,
                    case.name
                );
                assert!(
                    !case.note.trim().is_empty(),
                    "规则 `{}` 的 {} 缺 note",
                    meta.id,
                    case.name
                );
                assert!(
                    !case.files.is_empty(),
                    "规则 `{}` 的 {} 没有工程文件",
                    meta.id,
                    case.name
                );

                let mut paths = HashSet::new();
                for file in case.files {
                    let path = Path::new(file.path);
                    let safe = !file.path.is_empty()
                        && !path.is_absolute()
                        && path
                            .components()
                            .all(|part| matches!(part, Component::Normal(_)));
                    assert!(
                        safe,
                        "规则 `{}` 的 {} 含越界路径：{:?}",
                        meta.id, case.name, file.path
                    );
                    assert!(
                        paths.insert(file.path),
                        "规则 `{}` 的 {} 重复写了 {}：后写的会覆盖前面的",
                        meta.id,
                        case.name,
                        file.path
                    );
                }

                if let Some(text) = case.project_config {
                    let value: serde_json::Value =
                        serde_json::from_str(text).unwrap_or_else(|error| {
                            panic!(
                                "规则 `{}` 的 {} project_config 不是 JSON：{error}",
                                meta.id, case.name
                            )
                        });
                    assert!(
                        value.is_object(),
                        "规则 `{}` 的 {} project_config 必须是 JSON 对象",
                        meta.id,
                        case.name
                    );
                }

                if let Some(page) = case.run.page() {
                    assert!(
                        !page.starts_with('/') && !page.ends_with(".json"),
                        "规则 `{}` 的 {} 的 page 应写成 `pages/x/index`：{:?}",
                        meta.id,
                        case.name,
                        page
                    );
                    let page_json = format!("{page}.json");
                    assert!(
                        paths.contains(page_json.as_str()),
                        "规则 `{}` 的 {} 要加载 {page_json}，但 files 里没有",
                        meta.id,
                        case.name
                    );
                }
            }
        }
    }

    /// 视觉用例覆盖率门禁：每条规则（`Skyline` / `Webview` / `Both`）必须至少有一条
    /// [`super::VisualCase`] 或 [`super::EvidenceCase`]。
    ///
    /// 白名单给「差异无法用静态页面截图或产物/日志复现」的规则留出口，每条必须写清理由，
    /// 不允许无理由豁免。
    #[test]
    fn rules_have_cases() {
        /// (规则 id, 为什么没有任何自动用例)。
        /// 用例包含截图证据（`VisualCase`）与产物/日志证据（`EvidenceCase`）。
        const WHITELIST: &[(&str, &str)] = &[
            (
                "skyline/route-type",
                "差异只在跨页转场的过渡帧上，单页 reLaunch 表达不了；需要两页导航 + 过渡中取图",
            ),
            (
                "skyline/share-element-key",
                "共享元素转场同样跨页，且转场结束态两端一致，只有中间帧不同",
            ),
            (
                "skyline/draggable-sheet-scroll-to",
                "模拟器里 sheet 几何不随 scrollTo({size|pixels}) 变化，且传 pixels 会把 appservice 事件循环卡死",
            ),
            (
                "webview/tabbar-renderer",
                "存在 custom tabBar 时 automator 的 `App.callFunction` / `evaluate` 都不返回，Runtime 与 Console 两条取证通道都采不到样；非法 renderer 又直接让首页路由抛错，截图也归因不到",
            ),
        ];
        assert!(
            WHITELIST
                .iter()
                .all(|(_, reason)| !reason.trim().is_empty()),
            "白名单必须写清豁免理由"
        );

        let registry = Registry::builtin();
        let missing: Vec<&str> = registry
            .rules
            .iter()
            .filter(|rule| rule.visual_cases().is_empty() && rule.evidence_cases().is_empty())
            .map(|rule| rule.meta().id)
            .filter(|id| !WHITELIST.iter().any(|(allowed, _)| allowed == id))
            .collect();

        let stale: Vec<(&str, &str)> = WHITELIST
            .iter()
            .copied()
            .filter(|(id, _)| {
                !registry.rules.iter().any(|rule| {
                    rule.meta().id == *id
                        && rule.visual_cases().is_empty()
                        && rule.evidence_cases().is_empty()
                })
            })
            .collect();
        assert!(
            stale.is_empty(),
            "这些白名单条目已经有自动用例了，从白名单删掉（留假账比漏用例更难发现）：\n{stale:?}"
        );

        assert!(
            missing.is_empty(),
            "以下规则没有任何自动用例（截图或产物证据），补完后或写进白名单：\n{}",
            missing.join("\n")
        );
    }
}
