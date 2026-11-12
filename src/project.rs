//! 工程视图：把项目目录变成一个「可被规则消费」的结构。
//!
//! 本文件是数据结构（[`Project`] 与页面 / 组件 / 样式表的描述）与查询侧访问器；
//! 工程发现与配置读取（目录扫描、JSON 解析、单元解析、样式与环境闭包）在各子模块：
//! `load.rs`（主流程）、`units.rs`（单元解析）、`fs.rs`（大小写不敏感的文件系统）、
//! `paths.rs`（posix 路径工具）。
//!
//! 只依赖文件与配置的**事实**（页面清单、每页四件套的实际源文件、组件闭包、是否
//! Skyline 页面），不含任何 Skyline 语义判断。加载器是自有的：同一套代码在内部与
//! 公开构建里行为一致，不依赖非公开 crate。
//!
//! 两个刻意的设计：
//! - 位置只用「项目相对 posix 路径」表达，避免下游每条规则自己拼绝对路径；
//! - 永远读**原始** JSON。编译路径会往页面 json 里注入 `disableScroll`/`navigationStyle`、
//!   并把全局 `usingComponents` 摊进页面，那会抹掉「用户没写」和「继承来的」这个区别
//!   ——而这恰是迁移检查要判断的。

use std::cell::{OnceCell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use serde_json::{Map, Value};

use crate::input::env::Env;
use crate::input::js::JsDoc;
use crate::input::wxml;
use crate::input::wxml::WxmlDoc;
use crate::input::wxss::{self, ParseError, WxssDoc};
use crate::wxml_graph::WxmlGraph;
use crate::{Diagnostic, Severity};

mod fs;
mod load;
pub(crate) mod paths;
mod units;

use self::load::Loaded;
use self::paths::{posix_dirname, posix_join};

/// 一个编译单元（页面或自定义组件）的四件套，值为项目相对 posix 路径。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnitFiles {
    pub json: Option<String>,
    pub wxml: Option<String>,
    pub wxss: Option<String>,
    pub js: Option<String>,
}

/// 一个单元（页面 / 组件）在样式图里的身份。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    /// `app.wxss`：对全部页面与组件生效。
    Global,
    Page(String),
    Component(String),
    /// 不属于任何页面 / 组件的脚本（`utils/`、`monitor` 等），承载其项目相对路径。
    ///
    /// 只出现在 [`Project::scripts`] 里，样式图永远不会产生它。
    Loose(String),
}

/// 一份样式文件，以及它被哪些单元引入（含 `@import` 传递闭包）。
#[derive(Debug)]
pub struct StyleSheet {
    pub doc: WxssDoc,
    /// 去重后的引入方；同一文件被页面和组件同时引入时会同时出现。
    pub scopes: Vec<Scope>,
}

/// 一份脚本文件，以及它所属的单元。
#[derive(Debug)]
pub struct JsFile {
    pub path: String,
    pub scope: Scope,
    pub doc: JsDoc,
}

#[derive(Debug, Clone)]
pub struct PageUnit {
    /// 页面路由（app.json 里的原始写法，分包页已带 root 前缀）。
    pub route: String,
    /// 该页是否走 Skyline 渲染（页面级 `renderer` 优先，缺省回落 app.json）。
    pub is_skyline: bool,
    /// 页面 json 自己写了 `renderer`（任何取值）。
    pub declares_renderer: bool,
    /// 页面 json 显式声明了 `"renderer": "webview"`。
    ///
    /// 和 `!is_skyline` 不是一回事：这里是「这页永远不迁 Skyline」的明确决定
    /// （典型是含 `<web-view>` 的页面），规则整页跳过；而 app 级 webview、页面自己
    /// 没表态，只是「还没迁」——那正是要检查的对象。
    pub opted_out: bool,
    pub files: UnitFiles,
}

#[derive(Debug, Clone)]
pub struct ComponentUnit {
    pub path: String,
    pub files: UnitFiles,
}

/// 项目根 + miniprogramRoot 的装配结果。
pub struct Project {
    /// 项目绝对根。
    pub root: PathBuf,
    /// 主包根（项目相对，可为空串）。
    pub miniprogram_root: String,
    /// 由单个文件包出来的临时工程（`skylint page.wxml`）：没有真实的配置可读，
    /// 配置类规则整体不参与，否则会对着不存在的 app.json 报噪音。
    pub single_file: bool,
    /// 这个视图整个是插件代码（`pluginRoot` 下那一套）。
    pub plugin_view: bool,
    /// 报告里给本视图所有诊断路径加的前缀。宿主视图为空串，插件视图是 `pluginRoot/`：
    /// 视图内的路径相对自己的根（`abs` 靠它定位），输出时才拼成项目相对路径。
    pub report_prefix: String,
    /// 插件代码根（项目相对，可为空串）。
    ///
    /// `compileType: plugin` 的工程里插件代码与宿主示例小程序是两块目录，插件限制
    /// （不支持 `wx.router`、`scrollViewContext`）只对这里面的代码成立。
    pub plugin_root: Option<String>,
    /// app.json 原始对象。
    pub app_json: Map<String, Value>,
    /// app.json 原文，配置类规则定位用；读不到时为空串。
    pub app_json_source: String,
    /// 该视图的 app 级配置文件（`app.json` / 插件视图的 `plugin.json`），
    /// 诊断路径相对视图根，前缀由报告层补。
    pub app_json_path: String,
    /// 生效的 `rendererOptions`。宿主视图取自己的 `app.json`；插件视图继承宿主的——
    /// 插件页跑在宿主的小程序实例里，`rendererOptions` 是宿主 app 级开关
    /// （`wxapplib/src/skyline/src/runtime/index.ts:124-151`），插件自己的 `plugin.json`
    /// 并不是它的读者。
    pub renderer_options: Map<String, Value>,
    /// project.config.json 原始对象（缺失 / 解析失败为空）。
    pub project_config: Map<String, Value>,
    /// project.config.json 原文，定位 `libVersion` 这类值时用；读不到时为空串。
    pub project_config_source: String,
    /// project.private.config.json 原始对象（开发者工具写入的个人配置，常被 gitignore）。
    pub project_private_config: Map<String, Value>,
    /// project.private.config.json 原文，定位时用；读不到时为空串。
    pub project_private_config_source: String,
    pub pages: Vec<PageUnit>,
    pub components: Vec<ComponentUnit>,
    /// 单元 → 它**直接**引用的组件。页面用 route 作 key，组件用自身路径。
    ///
    /// 组件闭包本身是全局扁平表，看不出「这个页面到底用了哪些组件」；跨文件规则
    /// （标签选择器跨组件、类名可达性）必须知道归属，所以边也一并记下来。
    pub deps: BTreeMap<String, Vec<String>>,
    /// 页面 json 原始对象（按 route），只读不改。配置类规则要判断「用户写了什么」。
    pub page_json: BTreeMap<String, Map<String, Value>>,
    /// 单元 WXML → 该单元（含 app 级）在 `usingComponents` 里声明的标签名。
    ///
    /// 声明了同名标签时它会遮住内置组件，规则不能把 `<navigation-bar>` 这类
    /// 自定义组件当成不支持的内置组件。模板文件不属于任何单元，不在这张表里。
    pub declared_components: BTreeMap<String, BTreeSet<String>>,
    /// 单元 WXML → 标签名 → 组件路径（根相对，不带扩展名）。
    ///
    /// [`Self::declared_components`] 只回答「这个标签是不是自定义组件」；跨组件边界
    /// 的关系（`<slot>` 投影、组件内部结构）必须知道标签指向哪份组件 WXML。
    pub tag_targets: BTreeMap<String, BTreeMap<String, String>>,
    /// 组件 json 原始对象（按组件路径）。
    ///
    /// 组件 json 不是只管 `usingComponents`：`styleIsolation` / `addGlobalClass` 决定
    /// 样式能不能跨组件边界作用，`disableScroll` 这类页面级开关写在这里也没有读者。
    pub component_json: BTreeMap<String, Map<String, Value>>,
    /// 组件 json 原文（与 `component_json` 同 key）。
    pub component_json_source: BTreeMap<String, String>,
    /// 页面 json 原文（与 `page_json` 同 key），用于把诊断落到具体行列。
    pub page_json_source: BTreeMap<String, String>,
    /// 样式文件 → 引入方（`app.wxss` 记为 [`Scope::Global`]），按路径排序。
    ///
    /// 跨文件规则（`text-overflow` 要落到具体 `<text>` 上、标签选择器隔离要判断
    /// 能不能跨组件）需要「这份 wxss 的样式会作用到哪些 wxml」，这个表就是答案。
    /// `@import` 的传递闭包在这里展开，所以被 `@import` 进来的样式文件也会被检查。
    pub styles: Vec<StyleSheet>,
    /// 脚本文件 → 所属单元（`app.js` 记为 [`Scope::Global`]），按路径排序。
    ///
    /// 覆盖面是**整个工程**：页面 / 组件的 js 来自 `UnitFiles.js`（已按 typescript
    /// 插件解析到 `.ts` 源），同一文件被多个页面引用时只留一份；其余 `.js` / `.ts`
    /// 源文件一律记为 [`Scope::Loose`]。因此 `utils/` 里的实现也会被 JS 规则看到。
    /// 解析失败静默跳过：JS 语法错误由构建阶段拦截，linter 不另造诊断。
    pub scripts: Vec<JsFile>,
    /// 被 `<import>` / `<include>` 引入的模板（项目相对，去重排序）。
    ///
    /// 模板没有自己的四件套，但单文件结构规则（不支持组件、内联文本等）仍应检查它。
    pub template_files: Vec<String>,
    /// 模板文件 → 实际引用它的单元（`<import>` / `<include>` 传递闭包）。
    ///
    /// key 与 [`Self::template_files`] 同源；同一模板被多个单元引用时同时出现。
    /// 诊断归属靠它把模板上的命中落到具体单元，而不是伪单元 `unit:shared`。
    pub template_scopes: BTreeMap<String, Vec<Scope>>,
    /// WXML 路径 → 该文件作用域里能确定取值的绑定。
    ///
    /// 来源只有两处静态可证的东西：单元脚本的 `data` / `properties` 字面量（没有被
    /// 动态 `setData` 改写），以及父级以静态属性传进组件的值。`wx:if` 与 `class`
    /// 绑定靠它折叠——`{{renderer === 'skyline'}}` 在 `renderer` 已知时只有一支可达。
    pub envs: BTreeMap<String, Env>,
    /// 建图过程中发现的非阻断问题（如页面声明了但 wxml 缺失）。
    pub warnings: Vec<Diagnostic>,
    /// 页面 ⇄ 组件闭包。跨文件规则会逐元素问「这个页面用了哪些组件 / 这个组件被
    /// 哪些页面用到」，现算一次要遍历全部单元（本项目上就退化成平方级），必须缓存。
    closure: OnceCell<Closure>,
    /// [`Self::renders_as_block`] 的结果。纯函数，元素级规则会对同一 scope 反复询问。
    block_cache: RefCell<HashMap<Scope, bool>>,
    /// 渲染树（`<slot>` 投影 + 组件边界）。建一次要遍历全部文档，
    /// 规则与 Facts 都会问它，绝不能每次重算。
    graph: OnceCell<WxmlGraph>,
    /// 文档路径 → 覆盖它的样式表下标（[`Self::covering_sheets`]）。
    ///
    /// 级联按 (元素, 属性) 反复问，而覆盖关系只取决于文档；每次重算就是几千次
    /// `scope_wxml`（线性扫页面 / 组件），模板文档纳入求值后成了主要开销。
    cover_cache: RefCell<HashMap<String, Rc<Vec<usize>>>>,
    /// 解析过的 WXML 文档（key 的第二项：是否用空环境解析）。
    ///
    /// 规则是「每个文件 × 每条规则」地扫，不缓存的话同一份文档会被 glass-easel
    /// 编译器重复解析几十次——大工程上这是首要不耗时项。
    doc_cache: DocCache,
}

/// 解析缓存：key 的第二项是「是否用空环境解析」。
type DocCache = RefCell<HashMap<(String, bool), Option<Rc<WxmlDoc>>>>;

/// 组件闭包的正反向表，首次查询时建一次。
#[derive(Default)]
struct Closure {
    page_components: HashMap<String, Vec<String>>,
    component_pages: HashMap<String, Vec<String>>,
}

impl Project {
    /// `root` 是项目根（含 `project.config.json` 的目录）。
    pub fn load(root: impl Into<PathBuf>) -> anyhow::Result<Self> {
        Loaded::read(root.into()).host()
    }

    /// 宿主视图 + 插件视图。
    ///
    /// `project.config.json` 同时声明 `miniprogramRoot` 与 `pluginRoot` 时，两边是两套
    /// 独立的页面与组件：宿主示例小程序在 `miniprogramRoot`，插件代码在 `pluginRoot`，
    /// 插件页由 `pluginRoot/plugin.json` 的 `pages` 描述（宿主 `app.json` 里看不到）。
    /// 插件视图的路径以 `pluginRoot/` 为前缀输出（[`Project::report_prefix`]），
    /// 与宿主路径不会混淆。
    ///
    /// 没有 `pluginRoot` / `plugin.json` 的工程只有宿主视图。
    pub fn load_views(root: impl Into<PathBuf>) -> anyhow::Result<Vec<Self>> {
        let loaded = Loaded::read(root.into());
        let host = loaded.host()?;
        Ok(std::iter::once(host).chain(loaded.plugin()).collect())
    }

    /// 按视图相对路径取脚本。
    pub(crate) fn script(&self, path: &str) -> Option<&JsFile> {
        self.scripts.iter().find(|file| file.path == path)
    }

    /// 把 `require` / `import` 的说明符解析成工程内的脚本。
    ///
    /// 只认相对路径（`./x`）与根路径（`/x`）；裸模块名（`lodash`、`@scope/pkg`）
    /// 属于 `node_modules`，不在静态检查面内，返回 `None`。候选后缀按打包器的
    /// 常规顺序试：原样 → `.js` → `.ts` → `/index.js` → `/index.ts`。
    pub(crate) fn require_target(&self, importer: &str, specifier: &str) -> Option<&JsFile> {
        let base = if let Some(rest) = specifier.strip_prefix('/') {
            posix_join("", rest)
        } else if specifier.starts_with('.') {
            posix_join(&posix_dirname(importer), specifier)
        } else {
            return None;
        };
        [
            base.clone(),
            format!("{base}.js"),
            format!("{base}.ts"),
            format!("{base}/index.js"),
            format!("{base}/index.ts"),
        ]
        .iter()
        .find_map(|candidate| self.script(candidate))
    }

    /// 单元 id（页面路由或组件路径）→ 它的 WXML 路径。
    pub(crate) fn unit_wxml(&self, unit: &str) -> Option<&str> {
        self.pages
            .iter()
            .find(|page| page.route == unit)
            .and_then(|page| page.files.wxml.as_deref())
            .or_else(|| {
                self.components
                    .iter()
                    .find(|component| component.path == unit)
                    .and_then(|component| component.files.wxml.as_deref())
            })
    }

    /// [`Scope`] 所在单元的 WXML 路径，**包含**显式 opt-out 的页面。
    ///
    /// 与 [`Self::scope_wxml`] 的差别只有 opt-out 那一处：那个口径问「这份样式会不会被
    /// Skyline 用到」，这个口径问「这份文档属于谁」——模板可达性要把 opt-out 页面引用的
    /// 模板标成 WebviewOnly，而不是当成没人引用而丢掉。
    pub(crate) fn scope_wxml_any(&self, scope: &Scope) -> Option<&str> {
        match scope {
            Scope::Page(route) => self.unit_wxml(route),
            Scope::Component(path) => self.unit_wxml(path),
            Scope::Global | Scope::Loose(_) => None,
        }
    }

    /// `<import>` 进来的模板定义文档 → 引用它的单元 id。
    pub(crate) fn template_references(&self) -> impl Iterator<Item = (&str, &str)> {
        self.template_scopes.iter().flat_map(|(template, scopes)| {
            scopes.iter().filter_map(move |scope| match scope {
                Scope::Page(route) => Some((template.as_str(), route.as_str())),
                Scope::Component(path) => Some((template.as_str(), path.as_str())),
                Scope::Global | Scope::Loose(_) => None,
            })
        })
    }

    /// 这份 WXML 是否就是一个显式 `renderer: "webview"` 的页面。
    ///
    /// 这类页面是「不迁 Skyline」的明确决定，Skyline 方向对它们报限制是误报；
    /// 反方向（WebView 降级）仍然要检查。
    pub(crate) fn is_opted_out_page(&self, wxml_path: &str) -> bool {
        self.pages
            .iter()
            .any(|page| page.opted_out && page.files.wxml.as_deref() == Some(wxml_path))
    }

    /// 这份 WXML 所属单元声明的组件标签名（含 app 级）；模板文件没有归属，返回空。
    pub fn declared_components(&self, wxml_path: &str) -> BTreeSet<String> {
        self.declared_components
            .get(wxml_path)
            .cloned()
            .unwrap_or_default()
    }

    /// 这份 WXML 的常量环境；没有可折叠的绑定时返回 `None`。
    pub fn env(&self, wxml_path: &str) -> Option<&Env> {
        self.envs.get(wxml_path).filter(|env| !env.is_empty())
    }

    /// 某个页面（含 app 级声明）**传递**用到的组件。
    pub fn page_components(&self, route: &str) -> Vec<&str> {
        self.closure()
            .page_components
            .get(route)
            .map(|components| components.iter().map(String::as_str).collect())
            .unwrap_or_default()
    }

    /// 页面 ⇄ 组件闭包，首次查询时建一次：O(页面数 × 闭包大小)。
    fn closure(&self) -> &Closure {
        self.closure.get_or_init(|| {
            let mut closure = Closure::default();
            for page in &self.pages {
                let components: Vec<String> = self
                    .reachable(&[page.route.as_str(), "app.json"])
                    .into_iter()
                    .filter(|unit| {
                        self.components
                            .iter()
                            .any(|component| &component.path == unit)
                    })
                    .collect();
                for component in &components {
                    closure
                        .component_pages
                        .entry(component.clone())
                        .or_default()
                        .push(page.route.clone());
                }
                closure
                    .page_components
                    .insert(page.route.clone(), components);
            }
            closure
        })
    }

    /// 传递用到某个组件的页面。
    /// 组件的样式隔离级别（`styleIsolation`，`addGlobalClass` 是它的旧写法）。
    ///
    /// 归一规则见 `wxapplib/src/glass-easel-adapter/src/space.ts:610-640`：没写
    /// `styleIsolation` 时 `addGlobalClass: true` 等价于 `apply-shared`，否则
    /// `isolated`；`page-*` 前缀的三个值分别等同去掉前缀的三个值。
    pub fn style_isolation(&self, path: &str) -> StyleIsolation {
        let json = self.component_json.get(path);
        match json
            .and_then(|json| json.get("styleIsolation"))
            .and_then(Value::as_str)
        {
            Some("shared" | "page-shared") => StyleIsolation::Shared,
            Some("apply-shared" | "page-apply-shared") => StyleIsolation::ApplyShared,
            Some("isolated" | "page-isolated") => StyleIsolation::Isolated,
            _ if json
                .and_then(|json| json.get("addGlobalClass"))
                .is_some_and(truthy) =>
            {
                StyleIsolation::ApplyShared
            }
            _ => StyleIsolation::Isolated,
        }
    }

    pub fn component_pages(&self, path: &str) -> Vec<&str> {
        self.closure()
            .component_pages
            .get(path)
            .map(|pages| pages.iter().map(String::as_str).collect())
            .unwrap_or_default()
    }

    /// 生效的 `setting.<key>`：`project.private.config.json` 覆盖
    /// `project.config.json`（开发者工具按这个顺序合并），两处都没有时 `None`。
    pub fn setting(&self, key: &str) -> Option<&Value> {
        self.project_private_config
            .get("setting")
            .and_then(|setting| setting.get(key))
            .or_else(|| {
                self.project_config
                    .get("setting")
                    .and_then(|setting| setting.get(key))
            })
    }

    /// 生效的 `libVersion`：`project.private.config.json` 覆盖 `project.config.json`
    /// （开发者工具按这个顺序合并），两处都没有时 `None`。
    ///
    /// 返回 `(值, 命中的文件名, 该文件的原文)`，诊断直接落到实际生效的那份文件上。
    pub fn effective_lib_version(&self) -> Option<(&str, &'static str, &str)> {
        let (value, path, source) = match self.project_private_config.get("libVersion") {
            Some(value) => (
                value,
                "project.private.config.json",
                self.project_private_config_source.as_str(),
            ),
            None => (
                self.project_config.get("libVersion")?,
                "project.config.json",
                self.project_config_source.as_str(),
            ),
        };
        value.as_str().map(|raw| (raw, path, source))
    }

    /// 从若干单元出发，沿组件边可达的单元（不含起点本身）。
    fn reachable(&self, start: &[&str]) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut queue: Vec<String> = start.iter().map(|s| s.to_string()).collect();
        for unit in &queue {
            out.push(unit.clone());
        }
        while let Some(unit) = queue.pop() {
            for target in self.deps.get(&unit).into_iter().flatten() {
                if out.contains(target) {
                    continue;
                }
                out.push(target.clone());
                queue.push(target.clone());
            }
        }
        out.retain(|unit| !start.contains(&unit.as_str()));
        out
    }

    /// 某份样式的全部引入方对应的 WXML（按路径去重）。规则拿它做选择器匹配；
    /// 拿不到（页面被 opt-out、wxml 缺失、解析失败）就跳过该单元。
    /// 这份样式表是否作用于 `path` 这份文档：**单元作用域**口径。
    ///
    /// `Global` 的样式表对所有文档生效；其余按 `scope` 对应的单元 WXML 判等。独立
    /// 模板文件与 `<include>` 片段不是单元，这里恒为假（它们用 [`Self::covering_sheets`]）。
    pub(crate) fn sheet_covers(&self, sheet: &StyleSheet, path: &str) -> bool {
        sheet.scopes.iter().any(|scope| match scope {
            Scope::Global => true,
            _ => self.scope_wxml(scope) == Some(path),
        })
    }

    /// 覆盖 `path` 这份文档的样式表下标，按文档缓存。
    ///
    /// 单元（页面 / 组件，含写在其内的内联 `<template>`）按 [`Self::sheet_covers`]；
    /// 独立模板文件与 `<include>` 片段按实际 `<import>` / `<include>` 它的单元
    /// （[`Self::template_scopes`]）与 `Global` 的并集；查不到就是空集（判不了）。
    pub(crate) fn covering_sheets(&self, path: &str) -> Rc<Vec<usize>> {
        if let Some(hit) = self.cover_cache.borrow().get(path) {
            return hit.clone();
        }
        let is_unit = self.scope_of(path).is_some();
        let template_scopes = self.template_scopes.get(path);
        let indices: Vec<usize> = self
            .styles
            .iter()
            .enumerate()
            .filter(|(_, sheet)| {
                if is_unit {
                    return self.sheet_covers(sheet, path);
                }
                let Some(scopes) = template_scopes else {
                    return false;
                };
                sheet
                    .scopes
                    .iter()
                    .any(|scope| *scope == Scope::Global || scopes.contains(scope))
            })
            .map(|(index, _)| index)
            .collect();
        let value = Rc::new(indices);
        self.cover_cache
            .borrow_mut()
            .insert(path.to_string(), value.clone());
        value
    }

    pub fn scope_wxml(&self, scope: &Scope) -> Option<&str> {
        match scope {
            Scope::Global => None,
            Scope::Page(route) => self
                .pages
                .iter()
                .find(|page| &page.route == route && !page.opted_out)
                .and_then(|page| page.files.wxml.as_deref()),
            Scope::Component(path) => self
                .components
                .iter()
                .find(|component| &component.path == path)
                .and_then(|component| component.files.wxml.as_deref()),
            // 非单元脚本没有对应的 WXML。
            Scope::Loose(_) => None,
        }
    }

    /// 这个作用域下元素默认是否按 Block 布局渲染。
    ///
    /// `rendererOptions.skyline.defaultDisplayBlock: true` 时 Skyline 的默认值与 WebView
    /// 对齐（`display: block`、`flex-direction: row`、`align-items: normal`），所以「依赖
    /// Skyline 默认 flex/column」这类规则在这个作用域下不成立。页面级配置优先于 app 级。
    /// WXML 文件所属单元；被 `@import` / `@include` 的模板片段不属于任何单元。
    pub fn scope_of(&self, wxml: &str) -> Option<Scope> {
        if let Some(page) = self
            .pages
            .iter()
            .find(|page| page.files.wxml.as_deref() == Some(wxml))
        {
            return Some(Scope::Page(page.route.clone()));
        }
        self.components
            .iter()
            .find(|component| component.files.wxml.as_deref() == Some(wxml))
            .map(|component| Scope::Component(component.path.clone()))
    }

    /// 解析并缓存一份 WXML 文档；`unfolded` 用空环境保留全部分支。
    pub(crate) fn cached_doc(&self, path: &str, unfolded: bool) -> Option<Rc<WxmlDoc>> {
        if let Some(hit) = self.doc_cache.borrow().get(&(path.to_string(), unfolded)) {
            return hit.clone();
        }
        let doc = (|| {
            let source = std::fs::read_to_string(self.abs(path)).ok()?;
            let empty = Env::default();
            let env = if unfolded {
                &empty
            } else {
                self.env(path).unwrap_or(&empty)
            };
            let mut doc = wxml::parse(path, &source, env).ok()?;
            doc.fragment = self.template_files.iter().any(|file| file == path);
            Some(Rc::new(doc))
        })();
        self.doc_cache
            .borrow_mut()
            .insert((path.to_string(), unfolded), doc.clone());
        doc
    }

    /// 渲染树，首次查询时建一次。
    pub(crate) fn graph(&self) -> &WxmlGraph {
        self.graph.get_or_init(|| WxmlGraph::new(self))
    }

    pub fn renders_as_block(&self, scope: &Scope) -> bool {
        if let Some(hit) = self.block_cache.borrow().get(scope) {
            return *hit;
        }
        let value = self.compute_renders_as_block(scope);
        self.block_cache.borrow_mut().insert(scope.clone(), value);
        value
    }

    fn compute_renders_as_block(&self, scope: &Scope) -> bool {
        let app = self.app_display_block();
        match scope {
            // 全局样式表对所有页面生效：每一页的有效值都是 block 才成立。
            // 页面显式开 / 关优先于 app 级，所以不能用「页面没表态」来判断。
            Scope::Global => self
                .pages
                .iter()
                .all(|page| self.page_display_block(&page.route).unwrap_or(app)),
            Scope::Page(route) => self.page_display_block(route).unwrap_or(app),
            // 组件没有自己的页面配置，按「用到它的页面」判断；没人用时按 app 级。
            Scope::Component(path) => {
                let pages = self.component_pages(path);
                pages.is_empty()
                    || pages
                        .iter()
                        .all(|route| self.page_display_block(route).unwrap_or(app))
            }
            // 非单元脚本不会走到这里；保守地按 app 级配置回答。
            Scope::Loose(_) => app,
        }
    }

    /// 工程里是否出现过任何 `renderer` 声明（app/plugin.json 或任一页面 json）。
    ///
    /// 没有声明时规则按「未迁移、将被迁到 Skyline」检查，这正是迁移前该做的事，
    /// 但读者容易把它误读成当前渲染器下的 bug，所以报告里要说明这个前提。
    pub fn declares_renderer(&self) -> bool {
        self.app_json.contains_key("renderer")
            || self.pages.iter().any(|page| page.declares_renderer)
    }

    /// app 级 `defaultDisplayBlock` 是否开启。运行时两种写法都认，见 [`renderer_flag`]。
    pub fn app_display_block(&self) -> bool {
        renderer_flag(&self.renderer_options, "defaultDisplayBlock")
    }

    /// app 级 `defaultContentBox` 是否开启。运行时只读 `rendererOptions.skyline` 这一层
    /// （`runtime/index.ts:145`），写在顶层的同名单键不生效，所以这里也只认嵌套写法。
    pub fn app_content_box(&self) -> bool {
        self.renderer_options
            .get("skyline")
            .and_then(|skyline| skyline.get("defaultContentBox"))
            .is_some_and(truthy)
    }

    /// app 级 `rendererOptions.skyline.enableScrollViewAutoSize` 是否开启。
    /// 开启后 scroll-view 变成 measurable 节点，尺寸来自内容而不是 flex 分配。
    ///
    /// 运行时只读嵌套这一层（`wxapplib/src/skyline/src/runtime/index.ts:148`：
    /// `rendererOptions?.skyline?.enableScrollViewAutoSize || false`）。写在
    /// `rendererOptions` 顶层的同名单键**不生效**——不像 `defaultDisplayBlock`
    /// （`renderer_flag` 那种「顶层 OR 嵌套」），所以这里不能走 [`renderer_flag`]。
    pub fn scroll_view_auto_size(&self) -> bool {
        self.renderer_options
            .get("skyline")
            .and_then(|skyline| skyline.get("enableScrollViewAutoSize"))
            .is_some_and(truthy)
    }

    /// app.json 顶层的开关（JS 真值语义，如 `convertRpxToVw`）。
    pub fn app_flag(&self, key: &str) -> bool {
        self.app_json.get(key).is_some_and(truthy)
    }

    /// 页面级覆盖；`None` 表示这一页没写，回落到 app 级。运行时只认
    /// `rendererOptions.skyline.*` 这一层（`skyline.js:428-431`）。
    fn page_display_block(&self, route: &str) -> Option<bool> {
        self.page_json
            .get(route)
            .and_then(|json| json.get("rendererOptions"))
            .and_then(|options| options.get("skyline"))
            .and_then(|skyline| skyline.get("defaultDisplayBlock"))
            .map(truthy)
    }

    /// `Scope::Global` 覆盖的全部 WXML（页面 + 组件，页面按 opt-out 过滤）。
    pub fn global_wxml(&self) -> impl Iterator<Item = &str> {
        let mut seen = HashSet::new();
        self.pages
            .iter()
            .filter(|page| !page.opted_out)
            .filter_map(|page| page.files.wxml.as_deref())
            .chain(
                self.components
                    .iter()
                    .filter_map(|component| component.files.wxml.as_deref()),
            )
            .filter(move |path| seen.insert(*path))
    }

    /// 项目根下某个相对路径的绝对路径。
    pub fn abs(&self, rel: &str) -> PathBuf {
        self.root.join(self.miniprogram_root.as_str()).join(rel)
    }

    /// 单文件视图：把一个孤立文件包成最小工程。
    ///
    /// 给没准备整包工程的开发者试规则用（`skylint page.wxml`）。代价是跨文件判断
    /// 全部缺失：样式→节点、组件闭包、配置都是空的，只有单文件规则会出结果。
    pub fn single_file(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let source = std::fs::read_to_string(path)
            .map_err(|err| anyhow::anyhow!("{}: {err}", path.display()))?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        let ext = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default();

        let mut project = Self {
            root: path.parent().unwrap_or(Path::new(".")).to_path_buf(),
            miniprogram_root: String::new(),
            single_file: true,
            plugin_view: false,
            report_prefix: String::new(),
            plugin_root: None,
            // 单文件默认按 Skyline 工程算：用户想看的就是「这文件在 Skyline 下怎样」。
            app_json: Map::from_iter([("renderer".to_string(), Value::String("skyline".into()))]),
            component_json: BTreeMap::new(),
            component_json_source: BTreeMap::new(),
            app_json_source: String::new(),
            app_json_path: "app.json".to_string(),
            renderer_options: Map::new(),
            project_config: Map::new(),
            project_config_source: String::new(),
            project_private_config: Map::new(),
            project_private_config_source: String::new(),
            pages: Vec::new(),
            declared_components: BTreeMap::new(),
            tag_targets: BTreeMap::new(),
            envs: BTreeMap::new(),
            components: Vec::new(),
            deps: BTreeMap::new(),
            page_json: BTreeMap::new(),
            page_json_source: BTreeMap::new(),
            styles: Vec::new(),
            scripts: Vec::new(),
            template_files: Vec::new(),
            template_scopes: BTreeMap::new(),
            warnings: Vec::new(),
            closure: OnceCell::new(),
            block_cache: RefCell::new(HashMap::new()),
            graph: OnceCell::new(),
            cover_cache: RefCell::new(HashMap::new()),
            doc_cache: RefCell::new(HashMap::new()),
        };

        match ext {
            "wxml" => project.pages.push(PageUnit {
                route: name.trim_end_matches(".wxml").to_string(),
                is_skyline: true,
                declares_renderer: true,
                opted_out: false,
                files: UnitFiles {
                    wxml: Some(name),
                    ..UnitFiles::default()
                },
            }),
            "wxss" | "less" | "scss" | "sass" => match wxss::parse(&name, &source) {
                Ok(doc) => project.styles.push(StyleSheet {
                    doc,
                    scopes: vec![Scope::Global],
                }),
                Err(ParseError::Empty) => {}
                Err(ParseError::Syntax(detail)) => project.warnings.push(Diagnostic::new(
                    "skylint/unparsed-style",
                    Severity::Warning,
                    format!("样式文件无法解析，其中的规则未做任何检查：{detail}"),
                    name,
                    1,
                    1,
                )),
            },
            "js" | "ts" => {
                let Ok(doc) = JsDoc::parse(&name, &source) else {
                    anyhow::bail!("{name}: 脚本语法错误")
                };
                project.scripts.push(JsFile {
                    path: name.clone(),
                    scope: Scope::Loose(name),
                    doc,
                });
            }
            other => anyhow::bail!("不支持的文件类型：.{other}（支持 wxml / wxss / js / ts）"),
        }
        Ok(project)
    }
}

/// 组件的样式隔离级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleIsolation {
    /// 组件样式只作用于自己，外部样式也进不来。
    Isolated,
    /// 组件样式不外泄，但全局（页面 / app）的 class 样式能进来。
    ApplyShared,
    /// 双向共享：组件样式拿到页面那一份 scope，页面节点也会被命中。
    Shared,
}

/// 工程根下的共享上下文：配置只读一次，宿主与插件两个视图共用。
/// `rendererOptions` 里的布尔开关。
///
/// 运行时是 `rendererOptions.<key> || rendererOptions.skyline.<key> || false`
/// （`wxapplib/src/skyline/src/runtime/index.ts:141-144`）：**顶层优先**，且是 JS 的
/// `||` 真值语义（非空字符串、非零数字都算开启）。
fn renderer_flag(options: &Map<String, Value>, key: &str) -> bool {
    options.get(key).is_some_and(truthy)
        || options
            .get("skyline")
            .and_then(|skyline| skyline.get(key))
            .is_some_and(truthy)
}

/// JS 的真值语义（作为 `||` 的操作数时）。
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(number) => number.as_f64().is_some_and(|number| number != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::Project;
    use crate::rules::testutil::Fixture;

    fn load(name: &str, app_json: &str) -> Project {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write("app.json", app_json)
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", "<view/>");
        fixture.project()
    }

    #[test]
    fn scroll_view_auto_size_reads_the_skyline_layer() {
        let project = load(
            "project-scroll-auto-size-nested",
            r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"enableScrollViewAutoSize":true}}}"#,
        );
        assert!(project.scroll_view_auto_size());
    }

    /// 运行时只读 `rendererOptions.skyline.enableScrollViewAutoSize`
    /// （`runtime/index.ts:148`）；顶层的同名单键不生效，认了它会让
    /// `scroll-view-height` 整条规则静默（全工程漏报）。
    #[test]
    fn scroll_view_auto_size_ignores_the_top_level_key() {
        let project = load(
            "project-scroll-auto-size-top-level",
            r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"enableScrollViewAutoSize":true}}"#,
        );
        assert!(!project.scroll_view_auto_size());
    }
}
