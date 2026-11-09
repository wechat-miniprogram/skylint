# 架构：引擎与集成层

仓库是**一个 crate**（`skylint`），内部按「引擎 / 集成」分层：引擎回答「项目是什么、
哪里有问题」，集成层只负责把结论画出来或驱动 CLI。分层是模块边界，不是发布边界——
发布只发一个 `skylint`，避免两个 crate 各配一套 trusted publishing 的麻烦。

## 分层

| 层 | 路径 | 内容 |
|---|---|---|
| 引擎 | `src/project`、`src/input`、`src/matcher`、`src/wxml_graph.rs`、`src/rules`、`src/diag.rs`、`src/report.rs`、`src/suppress.rs` | 工程加载、模板/样式/脚本解析、选择器匹配、运行时父子关系、全部规则、诊断与报告数据、抑制 |
| 事实层 | `src/inspect.rs`、`src/migrate` | 「这个元素为什么这么判」以及迁移工具的 plan / job / baseline / ledger |
| 集成层 | `src/render.rs`、`src/main.rs` | 终端渲染（miette）、`skylint` 二进制（clap） |

依赖方向只有「集成 → 引擎」，引擎不引用 `render`，也不引用 clap。

### 为什么渲染独立成模块

`Report` 只是数据（`summary` + `diagnostics`，都实现了 `Serialize`）。终端上画成带源码
片段的块是**展示选择**：CLI 用 miette 的图形报告，迁移工具直接读结构体、MCP 工具返回
JSON。调用方不经过 `render` 就不会被终端渲染方式绑定。

`render::text(&report, source, style)` 接受一个「项目相对路径 → 源码」的闭包，所以调用方
自己决定从磁盘、从内存，还是从编辑器缓冲取。

## 作为库使用

```toml
[dependencies]
skylint = "0.2"
```

```rust
use std::collections::HashSet;

use skylint::{Project, Registry, Report};

let projects = Project::load_views(std::path::Path::new("."))?;
let views: Vec<&Project> = projects.iter().collect();
let diagnostics = Registry::builtin().run_views(&views, &HashSet::new());
let report = Report::from_views(&projects, diagnostics);
println!("{}", report.to_json());
```

要点：

- `Project::load_views` 对插件工程（`project.config.json` 同时有 `miniprogramRoot` 与
  `pluginRoot`）返回**两个视图**（宿主 + 插件），路径由各视图的 `report_prefix` 拼接；
- `Registry::builtin()` 拿到全部规则；`for_target(Target::Webview)` 换检查方向；
- `run_views` 的第二个参数是「忽略的规则 id」集合；
- 单文件视图用 `Project::single_file(path)`（不要求工程结构，跨文件规则也没东西可比）；
- 需要某一元素的计算值时不要自己重算级联，用 `skylint::rules::wxml::resolved_property`
  等函数——它们和规则用的是同一套实现。

## 集成层怎么加

新的集成（迁移流水线、MCP server 等）直接用库 API，不解析 CLI 输出：

0. **要用「元素为什么这么判」时，直接读 `inspect`**——`LoadedDoc::load` → `facts()`，
   能拿到级联溯源、逻辑祖先链、布局判定与单元配置，不要自己重算；
1. 用 `Project::load_views` / `single_file` 拿到视图；
2. 用 `Registry::for_target` + `run_views` 拿诊断，或直接 `Registry::builtin().run(&project, &ignored)`；
3. 需要「某个元素为什么这么判」时，用 `rules::wxml` 的取值函数（级联、祖先链、`wxml_graph` 的 slot 投影）；
4. 输出形态自己定（JSON / 作业单 / 终端文本都可以），不经过 `render`。

迁移工具（`skylint migration …`）是同一套库 API 的一等消费者：`migrate::plan` / `job` /
`verify` / `ledger` 都不依赖 CLI，编排层（`harness/`）只调命令。
