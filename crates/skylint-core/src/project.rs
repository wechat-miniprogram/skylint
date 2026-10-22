//! 工程视图：把项目目录变成一个「可被规则消费」的结构。
//!
//! 这里是与 minidev 的唯一接触面。用途是文件与配置的**事实**（页面清单、每页
//! 四件套的实际源文件、组件闭包、是否 Skyline 页面），不含任何 Skyline 语义判断。
//!
//! 两个刻意的设计：
//! - 位置只用「项目相对 posix 路径」表达，避免下游每条规则自己拼绝对路径；
//! - 永远读**原始** JSON。minidev 的编译路径会往页面 json 里注入
//!   `disableScroll`/`navigationStyle`、并把全局 `usingComponents` 摊进页面，
//!   那会抹掉「用户没写」和「继承来的」这个区别——而这恰是迁移检查要判断的。

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use compiler_common::pathutil::{posix_dirname, posix_join};
use compiler_common::{LocalFs, ProjectFs};
use compiler_graph::{AppConf, ResolveExtConf, Resolver, is_skyline_page, resolve_component_path};
use serde_json::{Map, Value};

use crate::diag::from_compiler_diag;
use crate::input::wxss::{self, ParseError, WxssDoc};
use crate::{Diagnostic, Severity};

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
}

/// 一份样式文件，以及它被哪些单元引入（含 `@import` 传递闭包）。
#[derive(Debug)]
pub struct StyleSheet {
    pub doc: WxssDoc,
    /// 去重后的引入方；同一文件被页面和组件同时引入时会同时出现。
    pub scopes: Vec<Scope>,
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
    pub pages: Vec<PageUnit>,
    pub components: Vec<ComponentUnit>,
    /// 单元 → 它**直接**引用的组件。页面用 route 作 key，组件用自身路径。
    ///
    /// 组件闭包本身是全局扁平表，看不出「这个页面到底用了哪些组件」；跨文件规则
    /// （标签选择器跨组件、类名可达性）必须知道归属，所以边也一并记下来。
    pub deps: BTreeMap<String, Vec<String>>,
    /// 页面 json 原始对象（按 route），只读不改。配置类规则要判断「用户写了什么」。
    pub page_json: BTreeMap<String, Map<String, Value>>,
    /// 页面 json 原文（与 `page_json` 同 key），用于把诊断落到具体行列。
    pub page_json_source: BTreeMap<String, String>,
    /// 样式文件 → 引入方（`app.wxss` 记为 [`Scope::Global`]），按路径排序。
    ///
    /// 跨文件规则（`text-overflow` 要落到具体 `<text>` 上、标签选择器隔离要判断
    /// 能不能跨组件）需要「这份 wxss 的样式会作用到哪些 wxml」，这个表就是答案。
    /// `@import` 的传递闭包在这里展开，所以被 `@import` 进来的样式文件也会被检查。
    pub styles: Vec<StyleSheet>,
    /// 建图过程中发现的非阻断问题（如页面声明了但 wxml 缺失）。
    pub warnings: Vec<Diagnostic>,
}

impl Project {
    /// `root` 是项目根（含 `project.config.json` 的目录）。
    pub fn load(root: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let root = root.into();
        let (config, config_source) = read_project_config(&root);
        let miniprogram_root = config
            .get("miniprogramRoot")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim_matches('/')
            .to_string();

        let fs = LocalFs {
            project_path: root.clone(),
        };
        let app =
            AppConf::build(&fs, &miniprogram_root).map_err(|e| anyhow::anyhow!(describe(&e)))?;
        let resolver = Resolver::new(&fs, &miniprogram_root, ext_conf(&config));

        let mut warnings = Vec::new();
        let app_value = Value::Object(app.app_value.clone());

        // 页面 json 只解析一次：既用于 skyline 判定，也作为组件闭包的入口。
        let page_json: Vec<(String, Option<Map<String, Value>>)> = app
            .pages
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
                    is_skyline: is_skyline_page(&app_value, page),
                    opted_out: page.get("renderer").and_then(Value::as_str) == Some("webview"),
                    files: resolve_unit_files(&resolver, route),
                }
            })
            .collect();

        let (component_paths, deps) = collect_components(
            &fs,
            &miniprogram_root,
            &app.app_value,
            &page_json,
            &app.comps,
            &mut warnings,
        );
        let components: Vec<ComponentUnit> = component_paths
            .into_iter()
            .map(|path| ComponentUnit {
                files: resolve_unit_files(&resolver, &path),
                path,
            })
            .collect();

        let styles = collect_styles(&root, &miniprogram_root, &pages, &components, &mut warnings);
        let app_json_source = fs
            .read_file(&miniprogram_root, "app.json")
            .unwrap_or_default();

        Ok(Self {
            root,
            miniprogram_root,
            app_json: app.app_value.clone(),
            app_json_source,
            project_config: config.as_object().cloned().unwrap_or_default(),
            project_config_source: config_source,
            pages,
            components,
            deps,
            page_json: page_json
                .into_iter()
                .filter_map(|(route, json)| Some((route, json?)))
                .collect(),
            page_json_source,
            styles,
            warnings,
        })
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
            // 全局样式表对所有页面生效：只有每一页都开着才成立。
            Scope::Global => {
                app && self
                    .pages
                    .iter()
                    .all(|page| self.page_display_block(&page.route).is_none())
            }
            Scope::Page(route) => self.page_display_block(route).unwrap_or(app),
            // 组件没有自己的页面配置，按「用到它的页面」判断；没人用时按 app 级。
            Scope::Component(path) => {
                let pages = self.component_pages(path);
                pages.is_empty()
                    || pages
                        .iter()
                        .all(|route| self.page_display_block(route).unwrap_or(app))
            }
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
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    /// 页面级覆盖；`None` 表示这一页没写，回落到 app 级。运行时只认
    /// `rendererOptions.skyline.*` 这一层（`skyline.js:428-431`）。
    fn page_display_block(&self, route: &str) -> Option<bool> {
        self.page_json
            .get(route)
            .and_then(|json| json.get("rendererOptions"))
            .and_then(|options| options.get("skyline"))
            .and_then(|skyline| skyline.get("defaultDisplayBlock"))
            .and_then(Value::as_bool)
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
}

/// 组件闭包。
///
/// `AppConf::comps` 只做词法解析（`usingComponents` 的值直接拼到引用方目录下），
/// 因此 `my-lib/button` 这类 npm 裸包名会被静默丢掉——编译器在
/// `check_component_path` 里另有一套 `miniprogram_npm` 回溯，但闭包不走那条路。
/// 这里以 `comps` 为种子（它已覆盖 appBar / custom-tab-bar / agent.skills 等
/// 额外入口），再按编译器的解析语义补齐 npm 分支。
fn collect_components(
    fs: &dyn ProjectFs,
    root: &str,
    app_json: &Map<String, Value>,
    page_json: &[(String, Option<Map<String, Value>>)],
    seeds: &[String],
    warnings: &mut Vec<Diagnostic>,
) -> (Vec<String>, BTreeMap<String, Vec<String>>) {
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
    // AppConf.comps 兜住 appBar / custom-tab-bar / agent.skills 这类非声明式入口。
    queue.extend(seeds.iter().cloned());

    let mut comps: Vec<String> = Vec::new();
    while let Some(unit) = queue.pop() {
        if comps.contains(&unit) {
            continue;
        }
        // 缺 json 的组件不在这里报：`minidev build` 的 checkComponentPath 已经会拦。
        let Some(json) = read_unit_json(fs, root, &unit, warnings) else {
            continue;
        };
        comps.push(unit.clone());
        let targets = routes_of(fs, root, &unit, &json);
        queue.extend(targets.iter().cloned());
        deps.insert(unit, targets);
    }
    comps.sort();
    (comps, deps)
}

/// 一个单元声明的组件引用解析后得到的根相对路径。
fn routes_of(fs: &dyn ProjectFs, root: &str, unit: &str, json: &Map<String, Value>) -> Vec<String> {
    using_components(json)
        .iter()
        .filter_map(|target| resolve_component_route(fs, root, unit, target))
        .collect()
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
/// 绝对路径 → 相对路径 → npm / extendedLib 裸名（交给编译器的解析器）。
fn resolve_component_route(
    fs: &dyn ProjectFs,
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

    let (_, rel) = resolve_component_path(fs, root, &dir, target);
    (!rel.is_empty()).then(|| posix_join(&dir, &rel))
}

/// 目标路径本身或它的 `index` 存在时返回根相对路径（与 checkComponentPath 的
/// `isValid` / `isValid(value + "/index")` 同序）。
fn existing_unit(fs: &dyn ProjectFs, root: &str, base: &str) -> Option<String> {
    let base = base.trim_matches('/');
    let has = |p: &str| fs.stat(root, &format!("{p}.json")).is_some();
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
    pages: &[PageUnit],
    components: &[ComponentUnit],
    warnings: &mut Vec<Diagnostic>,
) -> Vec<StyleSheet> {
    let mut scopes: BTreeMap<String, Vec<Scope>> = BTreeMap::new();
    let mut docs: BTreeMap<String, WxssDoc> = BTreeMap::new();

    let mut entries: Vec<(String, Scope)> = Vec::new();
    if root.join(miniprogram_root).join("app.wxss").is_file() {
        entries.push((posix_join(miniprogram_root, "app.wxss"), Scope::Global));
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

fn resolve_unit_files(resolver: &Resolver, unit: &str) -> UnitFiles {
    let at = |ext: &str| {
        resolver
            .resolve_info_map
            .get(&format!("{unit}.{ext}"))
            .map(|info| info.source.clone())
    };
    UnitFiles {
        json: at("json"),
        wxml: at("wxml"),
        wxss: at("wxss"),
        js: at("js"),
    }
}

/// 读页面/组件的原始 json。文件不存在返回 `None`（页面 json 本来就是可选的），
/// 解析失败记一条 warning 并返回 `None`。
fn read_unit_json(
    fs: &dyn ProjectFs,
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

fn ext_conf(config: &Value) -> ResolveExtConf {
    let mut conf = ResolveExtConf::default();
    let Some(plugins) = config
        .pointer("/setting/useCompilerPlugins")
        .and_then(Value::as_array)
    else {
        return conf;
    };
    for plugin in plugins.iter().filter_map(Value::as_str) {
        match plugin {
            "typescript" => conf.register("js", &["ts"]),
            "less" => conf.register("wxss", &["less"]),
            "sass" => conf.register("wxss", &["sass", "scss"]),
            _ => {}
        }
    }
    conf
}

/// project.config.json 的原文与解析结果。文件缺失 / 不是合法 JSON 时给空串与
/// `Null`——与页面 json 的宽容策略一致，缺文件不该升级成 linter 自己的错误。
fn read_project_config(root: &Path) -> (Value, String) {
    let source = std::fs::read_to_string(root.join("project.config.json")).unwrap_or_default();
    let value = serde_json::from_str(&source).unwrap_or(Value::Null);
    (value, source)
}

/// minidev 的错误链里可能挂着结构化诊断，优先取它（带行列），否则用 Display。
fn describe(err: &anyhow::Error) -> String {
    err.chain()
        .find_map(|e| e.downcast_ref::<compiler_diag::Diagnostic>())
        .map(|d| from_compiler_diag(d).message)
        .unwrap_or_else(|| err.to_string())
}

/// `rendererOptions` 里的布尔开关。
///
/// 运行时两种写法都认（`wxapplib/src/skyline/src/runtime/index.ts:139-141`）：
/// `rendererOptions.skyline.<key>` 与 app 级的 `rendererOptions.<key>`。
fn renderer_flag(options: &Map<String, Value>, key: &str) -> bool {
    options
        .get("skyline")
        .and_then(|skyline| skyline.get(key))
        .and_then(Value::as_bool)
        .or_else(|| options.get(key).and_then(Value::as_bool))
        .unwrap_or(false)
}
