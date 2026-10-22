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
use crate::input::wxss::{self, WxssDoc};
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
    pub pages: Vec<PageUnit>,
    pub components: Vec<ComponentUnit>,
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
        let config = read_project_config(&root);
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

        let components: Vec<ComponentUnit> = collect_components(
            &fs,
            &miniprogram_root,
            &app.app_value,
            &page_json,
            &app.comps,
            &mut warnings,
        )
        .into_iter()
        .map(|path| ComponentUnit {
            files: resolve_unit_files(&resolver, &path),
            path,
        })
        .collect();

        let styles = collect_styles(&root, &miniprogram_root, &pages, &components);

        Ok(Self {
            root,
            miniprogram_root,
            app_json: app.app_value.clone(),
            pages,
            components,
            styles,
            warnings,
        })
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
) -> Vec<String> {
    // 入口：页面的组件引用（页面本身不是组件，不入 comps）。
    let mut queue: Vec<String> = page_json
        .iter()
        .flat_map(|(route, json)| match json {
            Some(json) => routes_of(fs, root, route, json),
            None => Vec::new(),
        })
        .collect();
    // app 级 usingComponents 以主包根为基准。
    queue.extend(
        using_components(app_json)
            .iter()
            .filter_map(|target| resolve_component_route(fs, root, "app.json", target)),
    );
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
        queue.extend(routes_of(fs, root, &unit, &json));
    }
    comps
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
        let Ok(doc) = wxss::parse(&path, &source) else {
            continue;
        };
        // WXSS 的 @import 只认相对路径与根路径（`/a.wxss`）；.less/.scss 的变量式导入
        // 在这里解析不到，跳过即可——它们本来也不在 wxss 图里。
        for import in &doc.imports {
            if let Some(target) = resolve_style_import(&path, &import.target) {
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

/// `@import "a.wxss"` → 项目相对路径（相对引用方目录，`/` 开头按主包根）。
fn resolve_style_import(from: &str, target: &str) -> Option<String> {
    let target = target.trim().trim_matches(['"', '\'']);
    if target.is_empty() || target.starts_with("http") {
        return None;
    }
    let joined = if target.starts_with('/') {
        target.trim_start_matches('/').to_string()
    } else {
        posix_join(&posix_dirname(from), target)
    };
    Some(posix_join("", &joined))
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

fn read_project_config(root: &Path) -> Value {
    std::fs::read_to_string(root.join("project.config.json"))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or(Value::Null)
}

/// minidev 的错误链里可能挂着结构化诊断，优先取它（带行列），否则用 Display。
fn describe(err: &anyhow::Error) -> String {
    err.chain()
        .find_map(|e| e.downcast_ref::<compiler_diag::Diagnostic>())
        .map(|d| from_compiler_diag(d).message)
        .unwrap_or_else(|| err.to_string())
}
