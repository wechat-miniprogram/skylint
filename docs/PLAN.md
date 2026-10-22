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

### 2.2 待做

| ID | 内容 | 位置 | 风险 |
|---|---|---|---|
| M4 `[ ]` | 补 `checkComponentFramework`：skyline 页面/包必须 `componentFramework == "glass-easel"`（现在只有一句注释，函数不存在）。补上后 lint 不需要这条规则 | `json/mutate.rs` | 中 |
| M7 `[ ]` | `AppConf` 加 `page_comps` / `comp_children`（per-page 组件归属）。lint 的跨文件规则依赖它 | `conf.rs` | 低 |
| M9 `[ ]` | `compiler-diag`：`Severity` 加 `Info`，`Diagnostic` 加 `end_line`/`end_column` | `crates/diag/src/lib.rs` | 低 |
| M5 `[ ]` | `on_file_change` 不失效 `conf`，`app.json` 变更后页面集合陈旧（只影响常驻进程） | `basegraph.rs` | 中 |

### 2.3 待验证

| ID | 问题 | 已知证据 |
|---|---|---|
| **V1** `[x]` | **已确认**：`collect_comps` 把 `usingComponents` 的值纯词法拼到引用方目录（`conf.rs:185-282`），`my-lib/button` 这类 npm 裸名 stat 失败后被静默丢弃——`miniprogram_npm` 回溯只存在于 `check_component_path`。**已解在 lint 侧**：自建闭包，种子取 `AppConf.comps`（兜住 appBar / custom-tab-bar / agent.skills 等非声明式入口）+ 页面与 app 的 `usingComponents`，npm 分支交给编译器的 `resolve_component_path`。`collect_comps` 不动 | 剩余未决：minidev 自身的产物是否因此缺 npm 组件的 json/wxml。没有 devtools 作为 oracle 无法判定，且修它可能破坏 parity，故推迟——lint 的正确性已不依赖它 |

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

| # | 工作 | 状态 |
|---|---|---|
| 0.1 | V1 验证（npm 组件闭包） | `[x]` 结论见 2.3 |
| 0.2 | minidev 低风险批 M1/M2/M3/M6/M8 | `[x]` |
| 0.3 | minidev M4/M7/M9 | `[ ]` |
| 0.4 | `ProjectView` + `Project::load` + 报告管道 + 单文件失败不终止 | `[x]` 真实工程验证：31 页混合项目（3 skyline）与独立脚本逐页对照一致 |
| 0.5 | CLI：`--format text/json`、退出码（0 无 error / 1 有 error / 2 加载失败）、`--ignore`、inline disable | 部分（`--ignore` 与 disable 未做） |
| 0.6 | 6 条规则验证管道：`unsupported-component`、`scroll-view-type`、`no-pseudo`、`box-sizing`、`no-inline-text`、`flex-direction` | `[ ]` |
| 0.7 | 规则文档从 TOML 生成 | `[ ]` |

### P1 WXSS 字典（约 45 条，ROI 最高）

属性支持表、值支持表、选择器白名单，全部 TOML。出口判据：一致性语料全过。

### P2 WXML 结构（约 25 条）

`scroll-view` 家族（list/custom/nested、list-view、sticky-section、nested-scroll、
draggable-sheet）20+ 条父子约束；组件白名单扩展。

### P3 跨文件语义（差异化最大）

`text-overflow:ellipsis` × `<text>`、`scroll-view` 直接子节点 margin、标签选择器隔离、
类名可达性。依赖 M7。

### P4 JS（约 30 条）

worklet（`'worklet'` 声明、`runOnJS`、禁 `wx` API）、路由、`setData`。swc 版本与 minidev 对齐。

### 不做

autofix；14 条只能运行时判定的条目；`app.json` 自动改写。

## 5. 五个视图与规则族

| 视图 | 来源 | 服务哪族规则 |
|---|---|---|
| ProjectView | `AppConf` + `Resolver` + M6/M7/M8 | 公共上下文 |
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
