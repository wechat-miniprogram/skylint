//! 工程视图：把项目目录变成一个「可被规则消费」的结构。
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

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::input::js::JsDoc;
use crate::input::wxml;
use crate::input::wxss::{self, ParseError, WxssDoc};
use crate::{Diagnostic, Severity};

mod fs;
mod paths;
mod units;

use self::fs::LocalFs;
use self::paths::{posix_dirname, posix_join};
use self::units::{Exts, resolve_unit};

/// 一个编译单元（页面或自定义组件）的四件套，值为项目相对 posix 路径。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnitFiles {
    pub json: Option<String>,
    pub wxml: Option<String>,
    pub wxss: Option<String>,
    pub js: Option<String>,
}

/// 一个单元（页面 / 组件）在样式图里的身份。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
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
    /// app.json 原始对象。
    pub app_json: Map<String, Value>,
    /// app.json 原文，配置类规则定位用；读不到时为空串。
    pub app_json_source: String,
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
    /// 模板不属于任何单元：它的样式作用域不可知，但单文件结构规则（不支持组件、
    /// 内联文本等）仍应检查它。
    pub template_files: Vec<String>,
    /// 建图过程中发现的非阻断问题（如页面声明了但 wxml 缺失）。
    pub warnings: Vec<Diagnostic>,
}

impl Project {
    /// `root` 是项目根（含 `project.config.json` 的目录）。
    pub fn load(root: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let root = root.into();
        let (config, config_source) = read_config(&root, "project.config.json");
        let (private_config, private_config_source) =
            read_config(&root, "project.private.config.json");
        let miniprogram_root = config
            .get("miniprogramRoot")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim_matches('/')
            .to_string();

        let fs = LocalFs::new(root.clone());
        let app_source = fs
            .read_file(&miniprogram_root, "app.json")
            .ok_or_else(|| anyhow::anyhow!("Couldn't found the 'app.json' file"))?;
        let app_value: Map<String, Value> =
            serde_json::from_str(&app_source).map_err(|err| anyhow::anyhow!("app.json: {err}"))?;
        let exts = Exts::from_config(&config);
        let app_json = Value::Object(app_value.clone());
        let app_routes: AppRoutes = serde_json::from_value(app_json.clone())
            .map_err(|err| anyhow::anyhow!("app.json: {err}"))?;
        let routes = app_routes.routes();

        let mut warnings = Vec::new();

        // 页面 json 只解析一次：既用于 skyline 判定，也作为组件闭包的入口。
        let page_json: Vec<(String, Option<Map<String, Value>>)> = routes
            .iter()
            .map(|route| {
                let json = read_unit_json(&fs, &miniprogram_root, route, &mut warnings);
                (route.clone(), json)
            })
            .collect();
        // 原文单独留一份：`Value` 丢了 span，配置类规则要指到用户写的那一行。
        let page_json_source: BTreeMap<String, String> = page_json
            .iter()
            .filter(|(_, json)| json.is_some())
            .map(|(route, _)| {
                let rel = format!("{route}.json");
                (
                    route.clone(),
                    fs.read_file(&miniprogram_root, &rel).unwrap_or_default(),
                )
            })
            .collect();

        let pages: Vec<PageUnit> = page_json
            .iter()
            .map(|(route, json)| {
                let empty = Map::new();
                let page = json.as_ref().unwrap_or(&empty);
                PageUnit {
                    route: route.clone(),
                    is_skyline: is_skyline_page(&app_json, page),
                    opted_out: page.get("renderer").and_then(Value::as_str) == Some("webview"),
                    files: resolve_unit(&exts, &fs, &miniprogram_root, route),
                }
            })
            .collect();

        let component_graph = collect_components(
            &fs,
            &miniprogram_root,
            &app_value,
            &page_json,
            &app_seeds(&fs, &miniprogram_root, &app_value),
            &mut warnings,
        );
        let components: Vec<ComponentUnit> = component_graph
            .paths
            .into_iter()
            .map(|path| ComponentUnit {
                files: resolve_unit(&exts, &fs, &miniprogram_root, &path),
                path,
            })
            .collect();

        let app_declared = using_component_names(&app_value);
        let mut declared_components: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut declare = |wxml: Option<&str>, names: &BTreeSet<String>| {
            if let Some(wxml) = wxml {
                declared_components
                    .entry(wxml.to_string())
                    .or_default()
                    .extend(names.iter().cloned());
            }
        };
        for page in &pages {
            let mut names = app_declared.clone();
            if let Some(json) = page_json
                .iter()
                .find(|(route, _)| route == &page.route)
                .and_then(|(_, json)| json.as_ref())
            {
                names.extend(using_component_names(json));
            }
            declare(page.files.wxml.as_deref(), &names);
        }
        for component in &components {
            let mut names = app_declared.clone();
            if let Some(own) = component_graph.declared.get(&component.path) {
                names.extend(own.iter().cloned());
            }
            declare(component.files.wxml.as_deref(), &names);
        }

        let app_style = resolve_unit(&exts, &fs, &miniprogram_root, "app").wxss;
        let styles = collect_styles(
            &root,
            &miniprogram_root,
            app_style.as_deref(),
            &pages,
            &components,
            &mut warnings,
        );
        let scripts = collect_scripts(&fs, &miniprogram_root, &exts, &pages, &components);
        let template_files = collect_template_files(&fs, &miniprogram_root, &pages, &components);

        Ok(Self {
            root,
            miniprogram_root,
            app_json: app_value,
            app_json_source: app_source,
            project_config: config.as_object().cloned().unwrap_or_default(),
            project_config_source: config_source,
            project_private_config: private_config.as_object().cloned().unwrap_or_default(),
            project_private_config_source: private_config_source,
            pages,
            components,
            deps: component_graph.deps,
            page_json: page_json
                .into_iter()
                .filter_map(|(route, json)| Some((route, json?)))
                .collect(),
            page_json_source,
            declared_components,
            styles,
            scripts,
            template_files,
            warnings,
        })
    }

    /// 这份 WXML 所属单元声明的组件标签名（含 app 级）；模板文件没有归属，返回空。
    pub fn declared_components(&self, wxml_path: &str) -> BTreeSet<String> {
        self.declared_components
            .get(wxml_path)
            .cloned()
            .unwrap_or_default()
    }

    /// 某个页面（含 app 级声明）**传递**用到的组件。
    pub fn page_components(&self, route: &str) -> Vec<&str> {
        self.reachable(&[route, "app.json"])
            .into_iter()
            .filter_map(|unit| {
                self.components
                    .iter()
                    .find(|c| c.path == unit)
                    .map(|c| c.path.as_str())
            })
            .collect()
    }

    /// 传递用到某个组件的页面。
    pub fn component_pages(&self, path: &str) -> Vec<&str> {
        self.pages
            .iter()
            .filter(|page| self.page_components(&page.route).contains(&path))
            .map(|page| page.route.as_str())
            .collect()
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
    pub fn renders_as_block(&self, scope: &Scope) -> bool {
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

    /// app 级 `defaultDisplayBlock` 是否开启。运行时两种写法都认，见 [`renderer_flag`]。
    pub fn app_display_block(&self) -> bool {
        self.app_json
            .get("rendererOptions")
            .and_then(Value::as_object)
            .map(|options| renderer_flag(options, "defaultDisplayBlock"))
            .unwrap_or(false)
    }

    /// app 级 `defaultContentBox` 是否开启。运行时只读 `rendererOptions.skyline` 这一层
    /// （`runtime/index.ts:145`），写在顶层的同名单键不生效，所以这里也只认嵌套写法。
    pub fn app_content_box(&self) -> bool {
        self.app_json
            .get("rendererOptions")
            .and_then(|options| options.get("skyline"))
            .and_then(|skyline| skyline.get("defaultContentBox"))
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
            // 单文件默认按 Skyline 工程算：用户想看的就是「这文件在 Skyline 下怎样」。
            app_json: Map::from_iter([("renderer".to_string(), Value::String("skyline".into()))]),
            app_json_source: String::new(),
            project_config: Map::new(),
            project_config_source: String::new(),
            project_private_config: Map::new(),
            project_private_config_source: String::new(),
            pages: Vec::new(),
            declared_components: BTreeMap::new(),
            components: Vec::new(),
            deps: BTreeMap::new(),
            page_json: BTreeMap::new(),
            page_json_source: BTreeMap::new(),
            styles: Vec::new(),
            scripts: Vec::new(),
            template_files: Vec::new(),
            warnings: Vec::new(),
        };

        match ext {
            "wxml" => project.pages.push(PageUnit {
                route: name.trim_end_matches(".wxml").to_string(),
                is_skyline: true,
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

/// 组件闭包。
///
/// 从页面与 app.json 的 `usingComponents` 出发做 BFS，外加 `app_seeds` 提供的
/// 非声明式入口。npm 裸包名按 `miniprogram_npm` 回溯解析——词法拼接会把这些值
/// 直接扔进引用方目录，静默丢掉组件。
struct ComponentGraph {
    /// 组件路径，排序去重。
    paths: Vec<String>,
    /// 单元 → 它直接引用的组件。
    deps: BTreeMap<String, Vec<String>>,
    /// 组件路径 → 它自己声明的组件标签名。
    declared: BTreeMap<String, BTreeSet<String>>,
}

fn collect_components(
    fs: &LocalFs,
    root: &str,
    app_json: &Map<String, Value>,
    page_json: &[(String, Option<Map<String, Value>>)],
    seeds: &[String],
    warnings: &mut Vec<Diagnostic>,
) -> ComponentGraph {
    // 入口：页面的组件引用（页面本身不是组件，不入 comps）。
    let mut queue: Vec<String> = Vec::new();
    let mut deps: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for (route, json) in page_json {
        let Some(json) = json else { continue };
        let targets = routes_of(fs, root, route, json);
        queue.extend(targets.iter().cloned());
        deps.insert(route.clone(), targets);
    }
    // app 级 usingComponents 以主包根为基准。
    let app_targets: Vec<String> = using_components(app_json)
        .iter()
        .filter_map(|target| resolve_component_route(fs, root, "app.json", target))
        .collect();
    queue.extend(app_targets.iter().cloned());
    deps.insert("app.json".to_string(), app_targets);
    // 非声明式入口（custom-tab-bar / app-bar / agent.skills）不在任何 usingComponents 里。
    queue.extend(seeds.iter().cloned());

    let mut comps: Vec<String> = Vec::new();
    let mut declared: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    while let Some(unit) = queue.pop() {
        if comps.contains(&unit) {
            continue;
        }
        // 缺 json 的组件不在这里报：构建阶段的组件路径校验已经会拦。
        let Some(json) = read_unit_json(fs, root, &unit, warnings) else {
            continue;
        };
        comps.push(unit.clone());
        declared.insert(unit.clone(), using_component_names(&json));
        let targets = routes_of(fs, root, &unit, &json);
        queue.extend(targets.iter().cloned());
        deps.insert(unit, targets);
    }
    comps.sort();
    ComponentGraph {
        paths: comps,
        deps,
        declared,
    }
}

/// 一个单元声明的组件引用解析后得到的根相对路径。
fn routes_of(fs: &LocalFs, root: &str, unit: &str, json: &Map<String, Value>) -> Vec<String> {
    using_components(json)
        .iter()
        .filter_map(|target| resolve_component_route(fs, root, unit, target))
        .collect()
}

/// app.json 里本项目真正消费的部分：页面清单与分包。
///
/// 用类型而不是 `Value::get` 读，是为了让「类型写错了」在加载阶段就失败：
/// `"pages": "pages/a"` 这种手误会静默变成 0 个页面，于是所有规则一条都不跑。
#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct AppRoutes {
    pages: Vec<String>,
    #[serde(rename = "subPackages", alias = "subpackages")]
    sub_packages: Vec<SubPackage>,
}

#[derive(Debug, serde::Deserialize)]
struct SubPackage {
    root: String,
    pages: Vec<String>,
}

impl AppRoutes {
    /// 分包页在调用方看来已带 root 前缀；root 接受 `pkg` / `pkg/` / `/pkg` 三种写法。
    fn routes(&self) -> Vec<String> {
        let mut routes = self.pages.clone();
        for sub in &self.sub_packages {
            let root = sub.root.trim_matches('/');
            for page in &sub.pages {
                routes.push(if root.is_empty() {
                    page.clone()
                } else {
                    format!("{root}/{page}")
                });
            }
        }
        routes
    }
}

/// 页面级 `renderer` 优先，缺省回落 app.json。
fn is_skyline_page(app: &Value, page: &Map<String, Value>) -> bool {
    match page.get("renderer").and_then(Value::as_str) {
        Some("skyline") => true,
        Some(_) => false,
        None => app.get("renderer").and_then(Value::as_str) == Some("skyline"),
    }
}

/// 非声明式组件入口：`custom-tab-bar` 与 `app-bar`。
///
/// 这两个入口不在任何 `usingComponents` 里，但会被编译器当作组件加载；漏掉它们，
/// 闭包会变小，跨文件规则（如标签选择器隔离）跟着漏报。
fn app_seeds(fs: &LocalFs, root: &str, app: &Map<String, Value>) -> Vec<String> {
    let mut seeds = Vec::new();
    let custom_tab_bar = app
        .get("tabBar")
        .and_then(|tab| tab.get("custom"))
        .and_then(Value::as_bool)
        == Some(true);
    if custom_tab_bar && fs.exists(root, "custom-tab-bar/index.json") {
        seeds.push("custom-tab-bar/index".to_string());
    }
    if app.get("appBar").is_some() && fs.exists(root, "app-bar/index.json") {
        seeds.push("app-bar/index".to_string());
    }
    seeds
}

/// `usingComponents` 的**标签名**（key），小写化。
fn using_component_names(json: &Map<String, Value>) -> BTreeSet<String> {
    json.get("usingComponents")
        .and_then(Value::as_object)
        .map(|map| map.keys().map(|name| name.to_ascii_lowercase()).collect())
        .unwrap_or_default()
}

fn using_components(json: &Map<String, Value>) -> Vec<String> {
    json.get("usingComponents")
        .and_then(Value::as_object)
        .map(|m| {
            m.values()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// `usingComponents` 的 target → 根相对路径。三种写法的优先级与编译器一致：
/// 绝对路径 → 相对路径 → npm 裸包名（`miniprogram_npm` 回溯）。
fn resolve_component_route(
    fs: &LocalFs,
    root: &str,
    from_unit: &str,
    target: &str,
) -> Option<String> {
    if target.starts_with("plugin://") {
        return None;
    }
    if let Some(rooted) = target.strip_prefix('/') {
        return existing_unit(fs, root, rooted);
    }

    let dir = posix_dirname(from_unit);
    let joined = posix_join(&dir, target);
    if let Some(route) = existing_unit(fs, root, &joined) {
        return Some(route);
    }

    resolve_npm_unit(fs, root, &dir, target)
}

/// npm 裸包名：`<祖先目录>/miniprogram_npm/<name>`（含 `index` 变体）逐级向上。
///
/// 与 `checkComponentPath` 的搜索序列一致：先当前目录直连，再逐层向上找
/// `miniprogram_npm`，最后回主包根。命中要求 json 与 wxml 同时存在——只有 json
/// 的目录是构建产物碎片，不是能渲染的组件。
fn resolve_npm_unit(fs: &LocalFs, root: &str, dir: &str, name: &str) -> Option<String> {
    let mut dirs: Vec<&str> = dir
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect();
    loop {
        let prefix = dirs.join("/");
        let base = if prefix.is_empty() {
            format!("miniprogram_npm/{name}")
        } else {
            format!("{prefix}/miniprogram_npm/{name}")
        };
        for candidate in [base.clone(), format!("{base}/index")] {
            if fs.exists(root, &format!("{candidate}.json"))
                && fs.exists(root, &format!("{candidate}.wxml"))
            {
                return Some(candidate);
            }
        }
        dirs.pop()?;
    }
}

/// 目标路径本身或它的 `index` 存在时返回根相对路径（与 checkComponentPath 的
/// `isValid` / `isValid(value + "/index")` 同序）。
fn existing_unit(fs: &LocalFs, root: &str, base: &str) -> Option<String> {
    let base = base.trim_matches('/');
    let has = |p: &str| fs.exists(root, &format!("{p}.json"));
    if has(base) {
        return Some(base.to_string());
    }
    let indexed = format!("{base}/index");
    has(&indexed).then_some(indexed)
}

/// 四件套按目标后缀向 resolver 查询实际源文件（`.ts`/`.less`/`.scss` 会被解析成
/// 真实源，而不是假定 `.js`/`.wxss`）。
/// 样式图：每个 wxss 被哪些单元引入，`@import` 的传递闭包在此展开。
///
/// 只解析一次（`WxssDoc` 连同源码存在 `Project.styles` 里），规则侧不再重复读盘。
/// 读失败 / 解析失败的文件直接跳过——单文件坏掉不该拖垮整轮检查。
fn collect_styles(
    root: &Path,
    miniprogram_root: &str,
    app_style: Option<&str>,
    pages: &[PageUnit],
    components: &[ComponentUnit],
    warnings: &mut Vec<Diagnostic>,
) -> Vec<StyleSheet> {
    let mut scopes: BTreeMap<String, Vec<Scope>> = BTreeMap::new();
    let mut docs: BTreeMap<String, WxssDoc> = BTreeMap::new();

    let mut entries: Vec<(String, Scope)> = Vec::new();
    // 全局样式同样要过后缀优先级解析：工程可能只有 app.less / app.scss，没有 app.wxss。
    if let Some(path) = app_style {
        entries.push((path.to_string(), Scope::Global));
    }
    entries.extend(pages.iter().filter_map(|page| {
        page.files
            .wxss
            .clone()
            .map(|p| (p, Scope::Page(page.route.clone())))
    }));
    entries.extend(components.iter().filter_map(|c| {
        c.files
            .wxss
            .clone()
            .map(|p| (p, Scope::Component(c.path.clone())))
    }));

    let mut queue: Vec<(String, Scope)> = entries;
    while let Some((path, scope)) = queue.pop() {
        let seen = scopes.entry(path.clone()).or_default();
        if seen.contains(&scope) {
            continue;
        }
        seen.push(scope.clone());
        if docs.contains_key(&path) {
            continue;
        }
        let abs = root.join(miniprogram_root).join(&path);
        let Ok(source) = std::fs::read_to_string(&abs) else {
            continue;
        };
        let doc = match wxss::parse(&path, &source) {
            Ok(doc) => doc,
            // 空文件不算问题；解析不了的必须说出来——否则这份样式（以及它的
            // `@import` 子图）会在没有任何提示的情况下不被检查。
            Err(ParseError::Empty) => continue,
            Err(ParseError::Syntax(detail)) => {
                warnings.push(
                    Diagnostic::new(
                        "skylint/unparsed-style",
                        Severity::Warning,
                        format!("样式文件无法解析，其中的规则未做任何检查：{detail}"),
                        path.clone(),
                        1,
                        1,
                    )
                    .help("修正语法错误后重新运行；这份文件的 @import 也不会被展开"),
                );
                continue;
            }
        };
        // WXSS 的 @import 只认相对路径与根路径（`/a.wxss`）；.less/.scss 的变量式导入
        // 在这里解析不到，跳过即可——它们本来也不在 wxss 图里。
        for import in &doc.imports {
            if let Some(target) =
                resolve_style_import(root, miniprogram_root, &path, &import.target)
            {
                queue.push((target, scope.clone()));
            }
        }
        docs.insert(path, doc);
    }

    scopes
        .into_iter()
        .filter_map(|(path, scopes)| {
            let doc = docs.remove(&path)?;
            let mut scopes = scopes;
            scopes.sort();
            Some(StyleSheet { doc, scopes })
        })
        .collect()
}

/// 脚本图：`app.js` + 每个页面 / 组件的 js + 其余全部 `.js` / `.ts` 源文件。
///
/// 单元脚本的路径按后缀优先级解析（因此 typescript 插件的 `.ts` 源会命中），
/// 不在这里重新拼后缀；同一文件被多个页面的组件闭包重复引用时按 path 去重。
/// 其余文件由 [`LocalFs::list_files`] 全域枚举（已剪掉 `node_modules` 与点目录），
/// 排除 `.d.ts` 与 `miniprogram_npm` 编译产物后记为 [`Scope::Loose`]。
/// 解析失败（语法错误 / 空文件）静默跳过——JS 语法由构建阶段拦，linter 不另造诊断。
fn collect_scripts(
    fs: &LocalFs,
    miniprogram_root: &str,
    exts: &Exts,
    pages: &[PageUnit],
    components: &[ComponentUnit],
) -> Vec<JsFile> {
    let mut entries: BTreeMap<String, Scope> = BTreeMap::new();
    if let Some(path) = resolve_unit(exts, fs, miniprogram_root, "app").js {
        entries.insert(path, Scope::Global);
    }
    for page in pages {
        if let Some(path) = &page.files.js {
            entries
                .entry(path.clone())
                .or_insert_with(|| Scope::Page(page.route.clone()));
        }
    }
    for component in components {
        if let Some(path) = &component.files.js {
            entries
                .entry(path.clone())
                .or_insert_with(|| Scope::Component(component.path.clone()));
        }
    }

    // `list_files` 的返回值带 miniprogramRoot 前缀，脚本路径不带。
    let prefix = if miniprogram_root.is_empty() {
        String::new()
    } else {
        format!("{miniprogram_root}/")
    };
    for path in fs.list_files(miniprogram_root) {
        let rel = path.strip_prefix(&prefix).unwrap_or(&path);
        if !is_source_script(rel) {
            continue;
        }
        entries
            .entry(rel.to_string())
            .or_insert_with(|| Scope::Loose(rel.to_string()));
    }

    entries
        .into_iter()
        .filter_map(|(path, scope)| {
            let source = fs.read_file(miniprogram_root, &path)?;
            let doc = JsDoc::parse(&path, &source).ok()?;
            Some(JsFile { path, scope, doc })
        })
        .collect()
}

/// 可以进入脚本图的源文件：`.js` / `.ts`，去掉类型声明与 npm 编译产物。
fn is_source_script(path: &str) -> bool {
    let ext = path.rsplit_once('.').map(|(_, ext)| ext);
    matches!(ext, Some("js" | "ts"))
        && !path.ends_with(".d.ts")
        && !path.split('/').any(|segment| segment == "miniprogram_npm")
}

/// 模板闭包：从所有单元的 WXML 出发，沿 `<import>` / `<include>` 递归收集。
///
/// 只保留 `fs.exists` 校验通过的路径；`src` 由编译器剥掉了 `.wxml` 后缀，这里按
/// 「无扩展名就补 `.wxml`」还原。模板可以再引用模板，闭包按 visited 去重。
fn collect_template_files(
    fs: &LocalFs,
    miniprogram_root: &str,
    pages: &[PageUnit],
    components: &[ComponentUnit],
) -> Vec<String> {
    let mut queue: Vec<String> = pages
        .iter()
        .filter_map(|page| page.files.wxml.clone())
        .chain(
            components
                .iter()
                .filter_map(|component| component.files.wxml.clone()),
        )
        .collect();
    let mut seen: HashSet<String> = queue.iter().cloned().collect();
    let mut out = Vec::new();

    while let Some(path) = queue.pop() {
        let source = fs.read_file(miniprogram_root, &path).unwrap_or_default();
        let Ok(doc) = wxml::parse(&path, &source) else {
            continue;
        };
        for src in doc.template_refs() {
            let Some(target) = resolve_template_ref(fs, miniprogram_root, &path, src) else {
                continue;
            };
            if seen.insert(target.clone()) {
                out.push(target.clone());
                queue.push(target);
            }
        }
    }

    out.sort();
    out
}

/// `<import src>` / `<include src>` 的值 → 项目相对路径（必须是真实存在的文件）。
fn resolve_template_ref(
    fs: &LocalFs,
    miniprogram_root: &str,
    from: &str,
    src: &str,
) -> Option<String> {
    let src = src.trim();
    if src.is_empty() {
        return None;
    }
    let joined = match src.strip_prefix('/') {
        Some(rooted) => posix_join("", rooted),
        None => posix_join(&posix_dirname(from), src),
    };
    let name = joined.rsplit('/').next().unwrap_or(&joined);
    let candidate = if name.contains('.') {
        joined
    } else {
        format!("{joined}.wxml")
    };
    fs.exists(miniprogram_root, &candidate).then_some(candidate)
}

/// `@import` 目标 → 项目相对路径（必须是真实存在的文件）。
///
/// 预处理器里 `@import "base"` 可以省后缀、也可以指向 `_base.scss` 这类 partial，
/// 所以按「同名 + 同后缀 → 下划线 partial」的顺序试，取第一个存在的。
fn resolve_style_import(
    root: &Path,
    miniprogram_root: &str,
    from: &str,
    target: &str,
) -> Option<String> {
    let target = target.trim().trim_matches(['"', '\'']);
    if target.is_empty() || target.starts_with("http") {
        return None;
    }
    let joined = if target.starts_with('/') {
        target.trim_start_matches('/').to_string()
    } else {
        posix_join(&posix_dirname(from), target)
    };
    let dir = posix_dirname(&joined);
    let name = joined
        .rsplit_once('/')
        .map_or(joined.as_str(), |(_, name)| name);
    let ext = from.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("wxss");

    let candidates = if name.contains('.') {
        vec![joined]
    } else {
        vec![
            posix_join(&dir, &format!("{name}.{ext}")),
            posix_join(&dir, &format!("_{name}.{ext}")),
        ]
    };
    candidates.into_iter().find(|candidate| {
        root.join(miniprogram_root)
            .join(candidate.as_str())
            .is_file()
    })
}

/// 读页面/组件的原始 json。文件不存在返回 `None`（页面 json 本来就是可选的），
/// 解析失败记一条 warning 并返回 `None`。
fn read_unit_json(
    fs: &LocalFs,
    miniprogram_root: &str,
    unit: &str,
    warnings: &mut Vec<Diagnostic>,
) -> Option<Map<String, Value>> {
    let rel = format!("{unit}.json");
    let raw = fs.read_file(miniprogram_root, &rel)?;
    match serde_json::from_str::<Value>(&raw) {
        Ok(Value::Object(map)) => Some(map),
        Ok(_) => {
            warnings.push(
                Diagnostic::new(
                    "skylint/invalid-json",
                    Severity::Error,
                    "page/component json must be an object",
                    rel,
                    1,
                    1,
                )
                .help("检查文件根节点是否为 `{}`"),
            );
            None
        }
        Err(e) => {
            warnings.push(Diagnostic::new(
                "skylint/invalid-json",
                Severity::Error,
                e.to_string(),
                rel,
                e.line() as u32,
                e.column() as u32,
            ));
            None
        }
    }
}

/// project 根目录下的配置文件原文与解析结果。文件缺失 / 不是合法 JSON 时给空串与
/// `Null`——与页面 json 的宽容策略一致，缺文件不该升级成 linter 自己的错误。
fn read_config(root: &Path, name: &str) -> (Value, String) {
    let source = std::fs::read_to_string(root.join(name)).unwrap_or_default();
    let value = serde_json::from_str(&source).unwrap_or(Value::Null);
    (value, source)
}

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
