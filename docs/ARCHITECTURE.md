# 架构：engine 与集成层

仓库是一个 workspace，按「引擎 / 集成」分层。目的是让「只想要 linter」的用法保持简单，
同时让后续的迁移工具、MCP server 这类集成不必把 CLI 的依赖带进依赖图。

## 分层

| crate | 路径 | 内容 | 关键依赖 |
|---|---|---|---|
| `skylint-core` | `crates/core` | 工程加载（`project`）、模板/样式/脚本输入（`input`）、选择器匹配（`matcher`）、运行时父子关系（`wxml_graph`）、全部规则（`rules`）、诊断与报告数据（`diag` / `report`）、抑制（`suppress`） | 解析器与选择器（oxc、glass-easel、selectors），**没有** clap / miette |
| `skylint` | 仓库根 | 引擎的再导出、终端渲染（`render`，miette）、`skylint` 二进制（clap） | `skylint-core`、clap、miette |

依赖方向只有 `skylint → skylint-core`，核心不反向依赖集成。

### 为什么渲染不放在 core

`Report` 只是数据（`summary` + `diagnostics`，都实现了 `Serialize`）。终端上画成带源码
片段的块是**展示选择**：CLI 用 miette 的图形报告，迁移工具直接读结构体、MCP 工具返回
JSON。放在 core 会让所有消费者都被绑上一个终端渲染库。

`skylint` 里 `render::text(&report, source, style)` 就是这层展示实现；它接受一个
「项目相对路径 → 源码」的闭包，所以调用方自己决定从磁盘、从内存，还是从编辑器缓冲取。

## 作为库使用

```toml
[dependencies]
skylint-core = "0.1"
```

```rust
use std::collections::HashSet;

use skylint_core::{Project, Registry, Report};

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
- 需要某一元素的计算值时不要自己重算级联，用 `skylint_core::rules::wxml::resolved_property`
  等函数——它们和规则用的是同一套实现。

### 兼容面

`use skylint::…` 仍然可用：根 crate 把 `diag` / `input` / `matcher` / `project` / `report` /
`rules` / `suppress` / `wxml_graph` 与常用类型原样再导出。唯一的破坏性变化是
`Report::to_text` 与 `report::Style` 搬到了 `skylint::render`（`Style` 同名，
`to_text` 变成 `render::text(&report, …)`），因为它依赖 miette。

## 集成层怎么加

新的集成（迁移流水线、MCP server 等）只依赖 `skylint-core`：

0. **要用「元素为什么这么判」时，直接读 `inspect`**——`LoadedDoc::load` → `facts()`，
   能拿到级联溯源、逻辑祖先链、布局判定与单元配置，不要自己重算；

1. 用 `Project::load_views` / `single_file` 拿到视图；
2. 用 `Registry::for_target` + `run_views` 拿诊断，或直接 `Registry::builtin().run(&project, &ignored)`；
3. 需要「某个元素为什么这么判」时，用 `rules::wxml` 的取值函数（级联、祖先链、`wxml_graph` 的 slot 投影）；
4. 输出形态自己定（JSON / 作业单 / 终端文本都可以），不经过 core。

这样 CLI 与其它集成之间没有任何代码共享，只有 `skylint-core` 这一个共同底座。
