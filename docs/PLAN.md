# skylint 重做计划

> 状态标记：`[x]` 已完成 / `[ ]` 未开始 / `[?]` 待验证。
> 本文件是计划与进度的唯一来源；规则清单由 `crates/skylint-core/src/rules/data/*.toml`
> 生成，不在这里手写。

## 0. 前提

| # | 前提 | 影响 |
|---|---|---|
| P1 | skylint 是独立仓库（`github.com/wechat-miniprogram/skylint`），Rust 重写，目标可对外开源 | 不做 `minidev` 的子命令，不把 Skyline 语义放进 minidev |
| P2 | minidev 通过内部 remote 取源码，后期再决定是否开源 | 依赖面收窄到 3 个 crate；所有接触点收敛在一个模块 |
| P3 | **minidev 提供事实，skylint 提供判断** | Skyline 支持表、规则、诊断策略、报告永远留在 skylint |

P3 是硬边界。把「Skyline 支持哪些属性值」放进 minidev，skylint 开源就没有意义了——
那本来就是 minidev 的编译语义。所以 minidev 侧的改动只能是**与 Skyline 无关的通用正确性**。

## 1. 仓库与依赖

```
skylint/
├── Cargo.toml                   # workspace；minidev crate 以普通版本依赖声明
├── .cargo/config.toml.example   # [patch.crates-io] → 本机 minidev checkout
├── crates/skylint-core/         # 全部判断逻辑
└── crates/skylint-cli/          # 薄 CLI（clap）
```

依赖形态（**已落地**）：公开 `Cargo.toml` 只写 `compiler-common = "0.10"` 这类普通版本依赖，
开发期由 `.cargo/config.toml`（gitignore）的 `[patch.crates-io]` 指向本地 minidev。
好处：

- 公开仓库不出现内网域名；
- 不需要 cargo 的 git-source 替换（实测 `[patch]` 对 git 源仍会先 fetch 原地址，
  `[source]` 替换 git→git 不被支持）；
- minidev 发布到 crates.io 后，这里一行都不用改。

代价：外部用户 clone 后无法 `cargo build`，只能装预编译二进制。等 minidev 开源/发布后解除。

## 2. minidev 侧改造

分支：`skylint-p0`（从 `main` 0fc280f 切出）。

### 2.1 已落地

| ID | 内容 | 位置 |
|---|---|---|
| M1 `[x]` | `jsonschema` 关闭默认 feature，去掉 `reqwest`/`rustls`/`tokio`（约 360 个包） | `crates/graph/Cargo.toml` |
| M2 `[x]` | 分包前缀按路径段边界匹配（`pkg` 不再命中 `pkg2/`）。含 `conf.rs` 两处 + `mutate.rs` 两处 | `conf.rs` `in_package` / `mutate.rs` |
| M3 `[x]` | 删除 9 份死副本 `src/json/schema_*.json`（`include_str!` 实际用 `json/json/`） | `crates/graph/src/json/` |
| M6 `[x]` | `resolve_component_path2` → `pub fn resolve_component_path`，可被外部调用 | `mutate.rs` / `lib.rs` |
| M8 `[x]` | 提取 `pub fn is_skyline_page(app, page_json)`，编译器与 lint 共用同一定义 | `mutate.rs` / `lib.rs` |

验证：`cargo test -p compiler-graph` 70 passed；`cargo tree -p compiler-graph | grep -c reqwest` = 0。

### 2.2 已结（含撤销项）

| ID | 结论 | 说明 |
|---|---|---|
| M4 `[x]` | 补上 `checkComponentFramework` | 只校验 `all_pages` 里的页面：skyline 页面必须 `componentFramework == "glass-easel"`（页面级优先、缺省回落 app.json）。组件沿用 app 级声明。与 `check_renderer` 共用 `is_skyline_page`。lint 因此不需要这条规则 |
| M7 `[—]` | **撤销** | 原计划给 `AppConf` 加 `page_comps`/`comp_children`。但 `collect_page_deps` 用 `resolve_path` 做词法拼接，npm 裸包名解析不出来；在 minidev 里补一份 npm 不完整的映射，比 skylint 自建闭包更糟。skylint 的 `collect_components` 已经按 `resolve_component_path` 的语义自己走 BFS，页→组件边也在那一处一并记录 |
| M9 `[—]` | **撤销** | 约定的是「lint 用 `compiler-diag` 的 DTO」，实际 skylint 有自己的 `Diagnostic`，只从 `compiler_diag::Diagnostic` 里取 `message` 贴到加载失败信息上；`Severity::Info`、`end_line`/`end_column` 没有任何消费者 |
| M5 `[—]` | **不是缺陷** | `on_file_change` 的注释写明 TS 侧 `onFileChange` 为空实现、conf 靠 `getConf` 时全量重建，minidev 同步如此。常驻进程读的是重建后的 conf，不会陈旧 |

### 2.3 待验证

| ID | 问题 | 已知证据 |
|---|---|---|
| **V1** `[x]` | **已确认**：`collect_comps` 把 `usingComponents` 的值纯词法拼到引用方目录（`conf.rs:185-282`），`my-lib/button` 这类 npm 裸名 stat 失败后被静默丢弃——`miniprogram_npm` 回溯只存在于 `check_component_path`。**已解在 lint 侧**：自建闭包，种子取 `AppConf.comps`（兜住 appBar / custom-tab-bar / agent.skills 等非声明式入口）+ 页面与 app 的 `usingComponents`，npm 分支交给编译器的 `resolve_component_path`。`collect_comps` 不动 | 2026-10-22 补充取证：①`json/generate.rs:229` 的组件 json 产物**只**由 `conf.comps` 驱动，所以 `comps` 漏掉的组件在产物里就没有 json——而页面 json 里的引用已被 `check_component_path` 改写成 `../../miniprogram_npm/...`，即产物会留下悬空引用；②但 3 个真实工程（miniprogram-novel / businessview / miniprogram-skit_1，共 110 个 json）**没有一个**用 npm 裸名，全部写相对路径进 `miniprogram_npm/`；③TS 侧 `loadComp` 是否做同样回溯无从查证（仓库里没有 TS 源），无法排除「TS 也漏、devtools 靠别处兜底」。因此仍不改 `collect_comps`：改了会动产物字节，语料 golden 可能失败，而收益在真实工程上尚未被观测到。若将来真要修，验法是造一个 `miniprogram_npm/comp/index.{json,wxml}` + `usingComponents:{"comp":"comp"}` 的 fixture，断言产物里出现该 json |

### 2.4 明确不改

`collect_comps` 的解析语义（parity 区）；不加 WXSS 解析器/规则表/报告器；不加 `minidev lint` 子命令。

## 3. 结构

```
crates/skylint-core/src/
├── diag.rs        # Diagnostic（rule/severity/path/line/col/end/help），1-based
├── project.rs     # ProjectView + minidev_adapter（唯一接触面）
├── report.rs      # text / json 两种渲染
└── （待建）input/{wxml,wxss,json}.rs、effective.rs、rules/
```

`project.rs` 是与 minidev 的唯一接触点：只取**事实**（路由、四件套实际源文件、组件闭包、
是否 Skyline 页面），永远读**原始** JSON——编译路径会注入 `disableScroll`/`navigationStyle`
并摊平全局 `usingComponents`，那会抹掉「用户没写」和「继承来的」的区别。

## 4. 阶段

### P0 地基

两条实测校准（改了计划原本的假设）：

1. **规则不按 `is_skyline` 过滤页面。** 原计划「只查 Skyline 页面」是错的：迁移工具的受众
   是还没上 Skyline 的工程，纯 WebView 项目会一条都不报，用途落空。`is_skyline` 降级为
   工程视图的元数据。
2. **`no-inline-text` 只看「文本 + 元素混排」。** 原实现把「元素里有非空白文本子节点」全部
   报出，在两个生产 Skyline 工程上各报 122 / 196 条；抽查发现 `<view>{{text}}</view>` 在
   Skyline 上渲染正常（组件被 skyline 页使用），知识库的唯一反例也是混排
   `<view>Hello <view>World</view></view>`。收窄后降到 2 / 0。


| # | 工作 | 状态 |
|---|---|---|
| 0.1 | V1 验证（npm 组件闭包） | `[x]` 结论见 2.3 |
| 0.2 | minidev 低风险批 M1/M2/M3/M6/M8 | `[x]` |
| 0.3 | minidev M4（M7/M9 已撤销，见 2.2） | `[x]` |
| 0.4 | `ProjectView` + `Project::load` + 报告管道 + 单文件失败不终止 | `[x]` 真实工程验证：31 页混合项目（3 skyline）与独立脚本逐页对照一致 |
| 0.5 | CLI：`--format text/json`、退出码（0 无 error / 1 有 error / 2 加载失败）、`--ignore`、inline disable；text 用 miette 图形报告（源码片段 + 行列定位，TTY 自动上色） | `[x]` |
| 0.6 | 6 条规则验证管道：`unsupported-component`、`scroll-view-type`、`no-pseudo`、`box-sizing`、`no-inline-text`、`flex-direction` | `[x]` |
| 0.7 | 规则文档从代码生成（`skylint --rules`） | `[x]` |

### P1 WXSS 字典（ROI 最高）

`[x]` 部分完成：属性/值支持表 47 条数据（20 个不支持的属性 + 27 个不支持的值），
由两条数据驱动规则消费（`unsupported-property` / `unsupported-value`）。
另外 6 条「值结构」规则落在 `shape.rs`：渐变限制、多值数量、filter 函数、
text-decoration-line 单值、calc 角度、mask-image 渐变。

规则数（14）远小于数据条数（47）——这正是「字典层用数据、不用规则」的目的：
加一条支持表条目不需要写代码。

未做：选择器白名单（tag/id 跨组件隔离，属 P3）；简写展开（`background: url() no-repeat`
里的子属性不判定，整值匹配）；级联。

出口判据「一致性语料」已用**定向核对**代替全量转写：47 条数据逐条对着
`skyline/css/css_properties/*.cc` 的 `Parse` 与 `css_parser_impl.h` 的属性映射表核了一遍。

结论（详见各自 commit）：
- **删了 3 条错的**：`direction`、`overflow-x`、`overflow-y` 引擎都接受并生效
  （`direction` 真的应用到 Text；`overflow-x/y` 的 visible/hidden 会折叠进 `overflow`）。
  文档「不支持」的原文只对 `auto`/`scroll` 成立——已改成按值收录。
- **补了 4 条漏的**：`mask-position`、`display: inline-grid`、`overflow-x/y: auto|scroll`、
  `mask-repeat: space|round`。
- **修了 1 条建议**：`contain` 的替代方案 `-wx-contain` 本身也是 `kInvalid`，
  文档推荐的替代品同样不可用。

局限：引擎接受 ≠ 能渲染（`skyline/css` 之上还有一层取舍在私有 fork 里），
所以这套核对只能证伪「不支持」，不能证明「支持」。

### P2 WXML 结构

`[x]` 部分完成：`scroll-view` 家族的 5 条挂载约束（数据驱动，
`rules/wxml/data/scroll_family.txt`）+ 横向滚动 `enable-flex` 检查。
组件白名单已在 P0 落地。

刻意**没做**的（文档含糊或自相矛盾，做了就是误报）：
- `sticky-header` 必须在 `sticky-section` 内——`sticky.md` NEVER 2 要求，但
  `scroll-view.md:54` 把 `sticky-header` 列为 custom 模式的受支持子组件，生产工程
  `miniprogram-skit_1` 也在 `<nested-scroll-header>` 内直接用。两者冲突，整体撤销。
- 「必须有且只有一个」这类计数约束：`wx:if`/`wx:else` 摊平后会变成两个兄弟节点，
  静态计数必然误报。
- 需要知道组件内部渲染树（`slot` 落点）的约束。

### P3 跨文件语义（差异化最大）

`text-overflow:ellipsis` × `<text>`、`scroll-view` 直接子节点 margin、标签选择器隔离、
类名可达性。页→组件归属由 skylint 自建闭包提供（原 M7 已撤销）。

### P4 JS（约 30 条）

worklet（`'worklet'` 声明、`runOnJS`、禁 `wx` API）、路由、`setData`。swc 版本与 minidev 对齐。

### 不做

autofix；14 条只能运行时判定的条目；`app.json` 自动改写。

## 5. 五个视图与规则族

| 视图 | 来源 | 服务哪族规则 |
|---|---|---|
| ProjectView | `AppConf` + `Resolver` + M6/M8 | 公共上下文 |
| WxmlView | `glass-easel-template-compiler::parse`（不经过 `compiler-wxml`，它在 API 边界丢弃 AST） | 结构类、跨文件类 |
| WxssStructure | `cssparser`（已在依赖树里） | 语义类、跨文件类 |
| WxssSemantics | float_pigment hook（feature-gated，默认不用） | 值支持类兜底 |
| JsonView | 原始 JSON + offset 索引 | 配置类 |

值支持类只读 `WxssSemantics`，语义/结构类只读 `WxssStructure`——两族不共享判定，
所以不会出现「同一行一条说解析失败、一条说不支持」，也不需要关联两遍解析结果。

### 为什么默认不用 float_pigment

1. 解析后内省被关闭：`PropertyMeta::property()` 带 `#[cfg(test)]`，只能在解析期 hook 里看；
2. 它不是 oracle：Skyline 的 `skyline/css` 在它之上还有 130 个 `Parse` 二次白名单；
3. 「与运行时同源」不成立：运行时锁的是私有 fork（`DEPS` 指向 `prebuilt/float-pigment@4bc10b7`）。

它只省掉「值解析成类型」一步，省不掉「哪些值支持」，却带进版本漂移。

## 6. 验证

- **规则级**：每条规则 `positive/` + `negative/` fixture，**negative 数量 ≥ positive**；
- **引擎级**：从 `skyline/css/css_properties/*.cc` 的 130 个 `Parse` switch 整理
  `(属性, 值, 支持?)` 一致性语料（约 300-500 条），既是 P1 的数据源也是回归门禁。
  skyline cli 拿不到二进制（GN + 私有 fork），所以只能用源码当规格；
- **工程级**：纯 webview 老项目 / 混合 / 已迁移，三个真实工程；已迁移项目应接近 0 error。

## 7. 风险

| # | 风险 | 缓解 |
|---|---|---|
| R1 | 公开仓库依赖未开源的 minidev，外部不可构建 | 二进制为分发主通道；`[patch.crates-io]` 单点切换 |
| R2 | 改 minidev 破坏 parity | 行为变更（M4/V1）必须先有 fixture 证明「原行为是错的」 |
| R3 | 知识库自身矛盾（`form` / `calc` / `scroll-view type` / `display:inline`） | 实现前到 skyline 源码取实证 |
| R4 | 规则噪声 | negative fixture 下限；默认只开 error；inline disable |
| R5 | `glass-easel-template-compiler` 版本漂移（minidev 锁 1.2.0，crates.io 1.2.1） | 同一 version spec，让 cargo 合并成一份 AST |
| R6 | 缺 float_pigment 导致值判定漏报 | 一致性语料；实测漏报率不可接受再按 feature 引入 |
