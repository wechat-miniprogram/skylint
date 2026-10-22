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

1. **规则不按 `is_skyline` 过滤页面，但按「显式 opt-out」过滤。** 原计划「只查 Skyline
   页面」是错的：迁移工具的受众是还没上 Skyline 的工程，纯 WebView 项目会一条都不报，
   用途落空。`is_skyline` 降级为工程视图的元数据。
   但页面 json 自己写了 `"renderer": "webview"` 是另一件事——那是「这页永远不迁」的明确
   决定（典型是含 `<web-view>` 的页面），对它讲 Skyline 的限制就是误报。所以
   `PageUnit.opted_out` 单独记一位：显式 webview 的页面整页跳过 WXML 规则；app 级 webview、
   页面没表态的不跳（那只是还没迁）。
   边界：组件 wxml 没有 renderer，一律检查（若某组件只被 opt-out 页面使用，仍会报——要精确
   得先有页→组件归属，见 P3）；wxss 无法按页归属（`@import` + `app.wxss` 共享），同样不跳。
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

`[~]` 进行中。前置基建已落地：**样式图**（`Project.styles`）。

`wxss` 不再只是「一个单元一个文件」：页面 / 组件 / `app.wxss` 是入口，`@import` 展开成
传递闭包，于是每条样式文件都带着「会被哪些页面/组件的 wxml 消费」——跨文件规则要的
就是这张表。`Project::scope_wxml(scope)` 给出该作用域的 wxml 文档，`Scope::Global`
覆盖全部页面与组件（页面按 opt-out 过滤）。

已完成的规则：
- `skyline/text-overflow-ellipsis`：`text-overflow: ellipsis` 在 Skyline 下只对 `<text>`
  生效。判定 = 选择器命中的节点及其后代里有没有 `<text>`（可继承）；选择器只认能静态
  判定的形状（tag / `.class` / `#id`），组合符、伪类、属性选择器、动态属性一律放过。
  逗号分隔的选择器各判各的。`.wxss` / `.less` / `.scss` 一视同仁——这三条规则早期按
  文件后缀过滤，等于在样式源全是预处理器的工程上从不生效（见下）。

样式图的覆盖面变化（真实工程实测）：`@import` 进来的样式文件以前完全没被检查过。
`businessview` 从 16 error / 91 warning 变成 50 error / 538 warning——新增的全部落在
`@import` 进来的 `weui.wxss` 等文件上，是覆盖率提升而不是新误报（最集中的一条是
`no-pseudo` 在 `weui.wxss` 上的 237 条，两份分包拷贝各一份）。

**预处理器：自己写了摊平层，不再吞文件。** 三个真实工程的样式源几乎都是 `.less` / `.scss`
（miniprogram-novel 27 less / 1 wxss，miniprogram-skit_1 67 less / 0 wxss，
businessview 82 scss / 13 wxss）。原先用 `cssparser`，它对嵌套不展开（实测
`.a { display:flex; .b { … } }` 只出 `.a` 一条规则），等于样式规则在这些工程上一条都跑不到。

结论是 `cssparser` 这条路走不通，不是因为「要不要解预处理器」，而是因为它的容忍模型跟
预处理器语法冲突：SASS 声明可以不带分号、LESS 用 `//` 注释，`cssparser` 遇到就整块丢弃，
再叠加不展开嵌套——两件事都必须解决才能用。

后来先换成自写扫描器，再换成 `oxc-css-parser`（0.0.17，精确锁版本）的 AST——词法与语法
交给成熟实现，`input/wxss.rs` 只保留「把嵌套摊平成规则」这一层。三套语法（CSS / SCSS /
LESS，外加缩进语法）都是同一个 `Syntax` 枚举，差异只剩摊平——`&` 原地替换、后代组合器
拼接、`@media` 保留父选择器上下文、`@mixin`/`@if` 透明、`.mixin(@x)` 识别为函数体、
`#{…}`/`@{…}` 插值原样保留。**仍然不做**变量求值/mixin 展开/`@extend`/条件求值，代价是
保守：值里含 `$`/`@`/`#{` 就不判；无选择器的函数体只做属性级检查，pairwise 规则
（`display:flex` 配 `flex-direction`）跳过。实测只有 0.4% 的声明值含变量/插值。

换 oxc 的依据是差分实验，不是「换个更正统的库」：拿 172 个真实预处理器文件
（skit_1 67 less + novel 27 less + businessview 78 scss）逐条比较「声明起点 + 属性名」，
自写扫描器与 oxc **170 个完全一致**，其余 2 个差异全落在 `@keyframes` 内部（按设计跳过）；
再拿 grass 编译 businessview 的 39 个不含 `@import` 的 scss 与自写摊平结果对照，
37 个逐条一致、2 个只差组合器两侧空格。这两个实验同时抓出了自写扫描器的两个静默漏判
（见下）。

解析失败现在会**冒出来**：`ParseError::Empty`（空文件/只有注释）静默跳过，
`ParseError::Syntax` 报 `skylint/unparsed-style` 告警——否则一份语法坏掉的样式（以及它的
`@import` 子图）会无声无息地不被检查。

覆盖面变化（真实工程）：miniprogram-novel 0e/11w → 0e/95w，miniprogram-skit_1 0e/3w →
1e/101w，businessview 50e/538w → 59e/766w。抽查确认新命中都是真的（`float`、
`mask-position`、`filter: drop-shadow`、嵌套块里的 `display:flex` 缺方向），
选择器摊平也正确（`.cnt .subscribe-list .subscribe-item:last-child`）。

自写扫描器被换掉前修掉的两个静默漏判（都会导致整块声明凭空消失、不报任何错）：
`url(https://…)` 里的 `//` 被当成 LESS 行注释（声明连同后续规则一起丢）；`}` 前省略
最后一个分号（SASS 常见写法）时那份待处理文本被直接丢弃。修完 businessview 766w → 811w。

顺带修掉一个预处理器独有的重复：`&:hover` 在裸块和 `@media` 块里各写一次时，摊平后是
两条同选择器规则，选择器类规则会重复报同一条。`Report::new` 在排序后 `dedup`——全字段
相同的诊断不携带信息（businessview 的 `no-pseudo` 499 → 488）。

- `skyline/tag-selector-isolation`：标签 / id 选择器在 Skyline 下不跨组件匹配。
  引擎证据在 `skyline/style/selector_matcher.cc`：页面/组件样式表走 `MatchScopedRules`，
  命中前要求 `node.StyleScopeId() == rule_scope_id`；`legacy` 只把**标签**选择器改查
  全局标签规则表，id 选择器那段没有 legacy 分支。`app.wxss` 走 `MatchGlobalRules`，
  本来就不分作用域，因此不受影响——这正是规则只在 page/component 样式表里判的原因。
  判定要用「页面用了哪些组件」，为此 `Project` 记下了 `deps`（单元 → 直接引用的组件），
  提供 `page_components()` / `component_pages()` 两个传递闭包。

**两条规则原先按 `.wxss` 后缀过滤，预处理器工程上一条都跑不到。** 去掉后实测：
businessview +10（`text-overflow-ellipsis` 2 条、`tag-selector-isolation` 8 条），其余三个
工程 0 变化。逐条核对为真：`b2c-bind/pages/bind-employee/bind-employee.scss:10` 的
`.ellipsis { text-overflow: ellipsis }` 作用在 `<view class="nickname ellipsis">` 上（Skyline
下不生效）；`phone-numbers/style/default-box-sizing.scss:2` 是 `page, view, text, image, …`
的标签选择器全局重置，命中组件内部节点（Skyline 下标签选择器不跨组件，组件内全部丢
`box-sizing`）。这正是老 WebView 工程迁移时最典型的坑，也说明「按后缀过滤」把规则的实际
覆盖率降到了 0。

去掉后缀过滤之前，非全局 wxss 里的纯标签选择器只有 `page {}`（页面自身节点，同作用域），
所以曾经四个工程都是 0 命中。规则仍然只在「确实命中了别的单元的节点」时开口，
所以不产生推测性告警。

剩余：`scroll-view` 直接子节点 margin、类名可达性。

### P4 JS（约 30 条）

worklet（`'worklet'` 声明、`runOnJS`、禁 `wx` API）、路由、`setData`。swc 版本与 minidev 对齐。

### 不做

autofix；14 条只能运行时判定的条目；`app.json` 自动改写。

## 5. 五个视图与规则族

| 视图 | 来源 | 服务哪族规则 |
|---|---|---|
| ProjectView | `AppConf` + `Resolver` + M6/M8 | 公共上下文 |
| WxmlView | `glass-easel-template-compiler::parse`（不经过 `compiler-wxml`，它在 API 边界丢弃 AST） | 结构类、跨文件类 |
| WxssStructure | `oxc-css-parser` 0.0.17（只保留摊平，见 P3） | 语义类、跨文件类 |
| WxssSemantics | float_pigment hook（feature-gated，默认不用） | 值支持类兜底 |
| JsonView | 原始 JSON + offset 索引 | 配置类 |

值支持类只读 `WxssSemantics`，语义/结构类只读 `WxssStructure`——两族不共享判定，
所以不会出现「同一行一条说解析失败、一条说不支持」，也不需要关联两遍解析结果。

`WxssStructure` 对规则层暴露的是**解析器原语，不是字符串**：`WxssDeclaration.tokens`
（`ValueToken`：Function/Dimension/Number/Percentage/Color/Url/Ident/Str/Comma/Dynamic/Other，
由 oxc 的 `ComponentValue` 递归转成 owned 值）与 `WxssSelector.parts`
（`SelectorPart`：Tag/Universal/Class/Id/Other，来自 `ComplexSelector`/`CompoundSelector`）。
规则层因此不再自己切值、扫选择器：`value_util` 只剩十几个基于词元的包装（`split_top_level`
按顶层 `Comma` 切、`split_tokens` 直接取词元、`top_level_calls` 取 `Function`），
`selector::SimpleSelector::parse` 只是对 `parts` 做一次折叠——组合符 / 伪类 / 属性选择器在
部件构造阶段就落成 `Other`，策略仍是「宁可漏报」。

这一层换来的是把 428 行手写状态机（`value_util` 283 + `selector` 145）删掉。代价是生产代码
净增约 225 行：AST → owned 词元的映射要自己写，而且**语义必须逐字保持**——验收口径是四个
真实工程的诊断逐字段完全一致（95 / 880 / 102 / 7，零差异）。两处刻意保留的旧口径：
`progid:` 开头的值（oxc 会解析成 `progid(...)` 函数）按整值原文当 `Other`；`is_dynamic` 的
`var(` / `env(` / `{{` / `$` / `#{` 仍在任意位置判定、`@` 仍只认值开头。唯一放宽的一处：
预处理器变量出现在函数参数里（`translate(@w, 0)`）现在也算动态——旧实现只看开头，
新实现看词元；方向是**少报**，不会多报。

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
