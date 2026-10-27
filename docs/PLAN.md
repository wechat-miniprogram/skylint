# skylint 重做计划

> 状态标记：`[x]` 已完成 / `[ ]` 未开始 / `[?]` 待验证。
> 本文件是计划与进度的唯一来源；规则清单以 `src/rules/` 下的规则注册表
> 为准（生成到 `docs/RULES.md`，不在这里手写）。

## 0. 前提

| # | 前提 | 影响 |
|---|---|---|
| P1 | skylint 是独立仓库（`github.com/wechat-miniprogram/skylint`），Rust 重写，目标可对外开源 | 不做 `minidev` 的子命令，不把 Skyline 语义放进 minidev |
| P2 | ~~minidev 通过内部 remote 取源码~~ → **自有加载器**（2026-10-24） | 公开仓库零内网依赖，`cargo build` 直接可用 |
| P3 | **minidev 提供事实，skylint 提供判断** | Skyline 支持表、规则、诊断策略、报告永远留在 skylint |

P3 是硬边界。把「Skyline 支持哪些属性值」放进 minidev，skylint 开源就没有意义了——
那本来就是 minidev 的编译语义。所以 minidev 侧的改动只能是**与 Skyline 无关的通用正确性**。

## 1. 仓库与依赖

```
skylint/
├── Cargo.toml        # 单个 crate（lib + bin），无任何内网依赖
├── src/              # 库：工程加载器、输入解析、规则、报告
│   └── main.rs       # 薄 CLI（clap）
├── tests/            # 集成测试（工程视图、注册表与文档一致性）
├── examples/
│   └── rules_doc.rs  # 生成 docs/RULES.md
└── docs/
```

只有一个 crate：库与 CLI 同包（`cargo build` 出 `skylint`，`cargo test` 一次跑完），
维护脚本放 `examples/`；发版走 tag 触发的 GitHub Actions（`.github/workflows/release.yml`）。

依赖形态（**已落地**）：不依赖 minidev。工程加载器自建，见 §3。

历史：曾用 `common` / `diag` / `compiler-graph` 三个内网 crate + `.cargo/config.toml` 的
`[patch.crates-io]` 指向本地 checkout，代价是外部用户 clone 后无法 `cargo build`。
实测「只加 feature 门控」并不能解除——Cargo 连 `optional = true` 的依赖也会去索引解析，
内网 crate 不存在就直接失败。于是按「加载器自有」重写，三个依赖全部删除。

## 2. minidev 侧改造（已废弃）

这一段曾记录为复用 minidev 工程图而在 `skylint-p0` 分支上做的改造（关掉 `reqwest`、
分包前缀按路径段匹配、`is_skyline_page` 提取、撤销的 `page_comps` 等）。加载器自建后
这些改动不再被 skylint 依赖，正文删除；要考据看 `git log -- docs/PLAN.md`。

唯一仍成立的结论：组件闭包不能靠词法拼接，npm 裸包名必须做 `miniprogram_npm` 回溯
——现在由 `src/project.rs` 的 `resolve_npm_unit` 实现。

## 3. 结构

```
src/
├── diag.rs        # Diagnostic（rule/severity/path/line/col/end/help），1-based
├── project.rs     # Project + load/single_file 编排；styles/scripts/templates 收集
├── project/fs.rs      # 文件视图：递归枚举、相对 miniprogramRoot 的读写
├── project/paths.rs   # posix 路径工具（normalize / join / dirname）
├── project/units.rs   # 后缀优先级（wxss←less/sass/scss、js←ts）→ 单元四件套
├── report.rs      # text / json 两种渲染
└── （待建）input/{wxml,wxss,json}.rs、effective.rs、rules/
```

`project` 模块只取**事实**（路由、四件套实际源文件、组件闭包、是否 Skyline 页面），
永远读**原始** JSON——编译路径会注入 `disableScroll`/`navigationStyle` 并摊平全局
`usingComponents`，那会抹掉「用户没写」和「继承来的」的区别。

加载器口径（与 miniprogram-ci 对齐，改这里必须先量四工程诊断）：

- 文件枚举：递归、跳过 `node_modules` 与点目录、反斜杠归一；`list_files` 带
  `miniprogramRoot` 前缀；
- 后缀优先级由 `setting.useCompilerPlugins` 决定：`scss > sass > wxss`、`ts > js`；
- 组件闭包：`usingComponents` BFS，npm 裸包名按 `<祖先目录>/miniprogram_npm/<name>`
  逐级回溯（要求 json + wxml 同时存在），外加 `custom-tab-bar` / `app-bar` 入口；
- 页面：`pages` + `subPackages`（兼容全小写 `subpackages`），分包 root 接受
  `pkg` / `pkg/` / `/pkg`；
- 单文件模式：`Project::single_file` 把孤立文件包成最小工程，CLI 传文件即走这条。

改加载器后的验收口径：拿解耦前最后一个 commit 的二进制做基线，四工程诊断按 9 字段
（severity / rule / path / line / column / end_line / end_column / message / help）逐条 diff，
再加 summary 里的 pages / skylinePages / components。步骤：

```bash
git worktree add /tmp/base <解耦前的 commit>
# 旧版靠 .cargo/config.toml 的 [patch] 指向 minidev，从 git 历史取回该文件并改路径
cd /tmp/base && cargo build --release
# 两个二进制 各跑四工程:  skylint <project> --json
```

diff 必须逐字段 identical（不是总数相等）。合成工程还要盖住四工程没有的形态：
npm 裸包名（含 `@scope/`）、`componentGenerics` 默认值、`appBar`、`custom-tab-bar`、
`subpackages` 全小写写法。

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
| 0.5 | CLI：`--json`（默认 text）、退出码（0 无 error / 1 有 error / 2 加载失败）、`--ignore`、inline disable；text 用 miette 图形报告（源码片段 + 行列定位，TTY 自动上色） | `[x]` |
| 0.6 | 6 条规则验证管道：`unsupported-component`、`scroll-view-type`、`no-pseudo`、`box-sizing`、`no-inline-text`、`flex-direction` | `[x]` |
| 0.7 | 规则文档从代码生成（`docs/RULES.md`） | `[x]` |

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

`[x]` 完成：`scroll-view` 家族的 17 条结构约束（数据驱动，
`rules/wxml/data/scroll_family.txt`）+ 横向滚动 `enable-flex` 检查 + 组件属性取值表
（`skyline/component-attribute`）。组件白名单已在 P0 落地。

约束表用「关系」表达结构，Rust 侧只做分派：`direct` / `ancestor`（挂载位置）、
`first-child`（`sticky-header` 必须是 `sticky-section` 第一个元素子节点）、
`unique`（一个 `sticky-section` 只能有一个 `sticky-header`，一个 nested `scroll-view`
只能有一个 `nested-scroll-body`）、`renders-first`（header/body 只渲染第一个直接子节点，warning）、
`requires-child-tag`（`type="nested"` 的 `scroll-view` 必须有 `nested-scroll-body`）、
`requires-attr`（`list-builder` 的 `child-height`）、
`allowed-values`（`list-builder.type` / `grid-view.type`）、
`within:<容器>`（内层 `scroll-view` 的 `associative-container`）。

仍然**没做**的（文档含糊或自相矛盾，做了就是误报）：
- `sticky-header` 必须在 `sticky-section` 内——`sticky.md` NEVER 2 要求，但
  `scroll-view.md:54` 把 `sticky-header` 列为 custom 模式的受支持子组件，生产工程
  `miniprogram-skit_1` 也在 `<nested-scroll-header>` 内直接用。两者冲突，整体撤销；
  只保留「**在** `sticky-section` 内时必须是第一个子节点」这条条件约束。
- `within:nested-scroll-body` 的作用域取 body 而不是整个 nested scroll-view：
  `nested-scroll.md:210-224` 的示例里内层 scroll-view 在 body 内，而 header 里的
  scroll-view 不参与嵌套滚动（`skit_1` 的 `rank.wxml:26-41` 就是这种横向 tab 列表）。
- 需要知道组件内部渲染树（`slot` 落点）的约束。

**已知误报（待修）**：`unique` / `before` 依赖摊平后的兄弟列表，看不到 `wx:if`/`wx:else`
的分支互斥——`<block wx:if><nested-scroll-body/></block><block wx:else><nested-scroll-body/></block>`
会被算成两个 body。四个真实工程都没有这种写法。修法是在 `input/wxml.rs` 的摊平阶段给
每个节点带上「条件组 + 分支」标识，让计数/顺序约束跳过互斥分支；这属于解析层的改动，
单独一批做，不混在规则里打补丁。

### P3 跨文件语义（差异化最大）

`[x]` 完成（2 条规则落地 + 2 条撤销，撤销都有取证）。前置基建：**样式图**（`Project.styles`）。

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

- `skyline/attribute-selector`：属性选择器在 Skyline 下不参与匹配，两种后果分开判。
  引擎入集按 id → tag → class 分桶，主体三者都空且不是 `:host` 时直接返回
  （`style/rule_set.cc:106-160`），匹配也只查这三类桶（`selector_matcher.cc:456-500`）：
  主体只有属性（`[aria-checked]`）时整条声明从不生效；主体另有标签/class 时条件被
  丢掉，命中范围放宽。两种都只在真的能观察到差异时报——逐元素比对两个方言，要求存在
  「WebView 命中而 Skyline 不命中」或反向的节点。工程里没写过该属性时两端都不命中，
  所有 `.a` 都带该属性时放宽也没有后果，都不报。四个非插件工程只有 businessview 命中
  1 条（weui 的 `.weui-agree__checkbox-check[aria-checked='true'] + …`：WebView 靠
  `aria-checked` 判定，Skyline 丢掉条件后勾选样式恒亮），其余三个工程 0 条。

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

两条原本排在这一阶段的候选，取证后**撤销**：

- **`scroll-view` 直接子节点 margin**（旧项目叫 `scroll-view-margin`，Info 级，建议「给子节点套一层
  view」）。知识库 `changelog.md` 的 1.0.0（2023-05-11）写着「scroll-view 组件直接子节点支持
  CSS margin」，旧项目也在 2024-05-31 的 `feat: remove scroll-view-margin rule` 里删掉了。
  对着现代基线实现只会产生误报。
- **类名可达性**（wxss 里定义的 class 没有任何 wxml 使用）。这不是 Skyline 语义差异——死样式
  在 WebView 里同样是死的，属于通用死代码检查，越过了「只报 WebView 能跑、Skyline 不能或
  不一样」的规则边界；而且按 `weui.wxss` 这类被 `@import` 进来的公共样式算，噪声会非常大。

### P3.5 知识库全量对照（171 条）

`[x]` 已对照并补齐本轮能做的部分。

按「给定输入文件即可判定真假」的口径把知识库全量枚举了一遍（运行期渲染、手势实测、
性能建议一律不收），得 171 条约束，逐条带证据位置。当时的覆盖情况：完全覆盖约 55 条、
部分覆盖约 15 条、**未覆盖约 100 条**——未覆盖集中在组件属性/结构（约 45）、配置（约 23）、
JS/worklet（约 21）、WXSS 值语义（约 12）。所以 P4（JS）不是下一步的最优选择：组件与配置
这两块单文件即可判定、误报低、条数多。

**审计表本身有错，实现前必须回文档/源码复核。** 实测发现的错误：swiper 的 `layout-type`
是 `normal` 而不是 `default`；`transformer-type` 是 `scaleAndFade/accordion/threeD/zoomIn/
zoomOut/deepthPage`（不是 `scaleTinder/...`）；`indicator-type` 是 16 个动画值、没有
`dots`/`never`；`draggable-sheet` 根本没有 `size/min-size/max-size`（是
`initial-child-size/min-child-size/max-child-size`）；`display: inline` 其实**支持**
（`display.cc:74-78` 映射进 `fp::DisplayType`），旧项目那条 display-inline 规则本来就是错的。

本轮落地：
- `skyline/component-attribute`：一张数据表覆盖 24 个「组件.属性」的枚举白名单 /
  WebView-only 取值 / 不支持的属性 / 数值范围（`rules/data/component_attributes.txt`）。
- `skyline/scroll-view-family`：从 5 条挂载约束扩到 17 条结构约束。
- `skyline/text-decoration-target`：`text-decoration*` 只对 `<text>`/`<span>`/`<input>` 生效。
  依据不是文档而是引擎：`style_builder.cc:1298-1316`（TextSpan）、`:1486-1494`（Input），
  且 `text_decoration_line_` 在 `ComputedStyle` 的 `non_inherited_data_`（`computed_style.h:831`、
  `:1734`）——**不继承**，所以容器里有 `<text>` 也不能豁免（与 `text-overflow` 相反）。

**代价最大的一处修正：`flex-direction` 一直在误报。** 这条规则的前提是「Skyline 默认
`flex-direction: column`，WebView 是 `row`」，但
`rendererOptions.skyline.defaultDisplayBlock: true` 时 Skyline 的默认值对齐 WebView
（`flex.md:16`、`app-config.md:67`；运行时见 `wxapplib/src/skyline/src/runtime/index.ts:139-141`
与页面级 `:424-431`）。四个真实工程**全部**开了这个开关，实测一条规则贡献了 424 条误报：

| 工程 | 修前 | 修后 |
|---|---|---|
| miniprogram-novel | 95 | 16 |
| businessview | 878 | 622 |
| miniprogram-skit_1 | 102 | 18 |
| weapp-novel-plugin_1 | 7 | 3 |

`Project::renders_as_block(scope)` 负责解析 app 级 + 页面级配置（页面级优先；组件按「用到
它的页面」判定；`app.wxss` 要求每页都开），规则按样式表的 scope 决定是否整份跳过。
**结论：任何以「Skyline 默认值」为前提的规则都必须先读 `rendererOptions`。**
`box-sizing` 只看「值是否受支持」，不受 `defaultContentBox` 影响；
`tag-selector-isolation` 早已读了 `tagNameStyleIsolation`。

**插件工程（已落地）**：`project.config.json` 同时声明 `miniprogramRoot` 与
`pluginRoot` 时，宿主示例小程序（`miniprogramRoot/app.json`）与插件
（`pluginRoot/plugin.json`）是**两套独立视图**，`Project::load_views` 各加载一个：

| | 宿主视图 | 插件视图 |
|---|---|---|
| 页面清单 | `app.json.pages` | `plugin.json.pages`（「页面名 → 路径」的 map，兼容数组） |
| app 级配置 | `app.json` | `plugin.json`（`renderer` / `rendererOptions`） |
| 组件入口 | `usingComponents` + `custom-tab-bar` / `app-bar` | 页面 `usingComponents` + `publicComponents` |
| 全局样式 | `app.wxss` | 无 |
| 报告路径 | 相对 `miniprogramRoot`（不变） | 前缀 `pluginRoot/` |

视图内的路径相对自己的根（`Project::abs` 用 `miniprogram_root` 字段定位），输出时才由
`Project::report_prefix` 拼成项目相对路径——宿主路径因此完全不变（四个真实工程逐字段
0 diff），插件页与插件组件也各自有独立的 `scope` 归属，不需要改规则的跨文件判定。
`plugin_view` 标记该视图整块是插件代码，`rules/js` 的 `in_plugin` 据此判定。

`pluginRoot` 存在但 `plugin.json` 缺失/读不了时没有插件视图——插件清单本身有问题由
开发者工具拦，linter 不另造诊断。

### P4 JS（worklet / API 禁令 / 路由）

`[x]` 已落地，前端是 `oxc_parser` 0.155，不是 swc。

选 oxc 的理由：`oxc_allocator` 0.155 已经在依赖树里（`oxc-css-parser` 用的就是它），同一版本线
复用一套 allocator，CSS 与 JS 共用一个前端家族。代价是 +24 个包（`cargo tree` 408 → 520 行）。
原计划「与 minidev 的 swc 版本对齐」不成立——skylint 是独立仓库，不需要和 minidev 共用 AST 类型。

`input/js.rs` 是 owned 模型（源码 String + 四个聚合查询），刻意不自引用 `Allocator`：
`functions()`（名字/指令序言/函数体 span）、`calls()`（callee 点号链、`worklet_scope` 取最近外层
函数、实参对象静态键与键位置、内联函数指令状态、裸标识符名）、`object_fns()`（对象字面量里
值是函数或裸标识符的属性 + 键位置）、`this_data_destructures()`（解构点 + 是否在 worklet 内）。
**出了 `parse` 就没有 AST**，所以「在不在 worklet 内」「这个实参是不是内联函数」必须遍历时定死，
不能由规则侧事后补算——这是这四个查询存在而不做通用 AST 暴露的原因。

规则（5 条）：`unsupported-api`（数据表 `call:` / `page-option:` / `plugin-call:` 三种前缀，
`this.animate`、`Page.onPageScroll`、插件内 `wx.router.*`）、`worklet-api-scope`
（`scrollViewContext.scrollTo` 只能在 worklet 内、worklet 内不能调 `wx.*`、插件内不支持）、
`scroll-context-ref`（用 `scrollViewContext.scrollTo` 前必须有 `.ref()`）、
`draggable-sheet-scroll-to`（`size` 与 `pixels` 同时传）、`worklet-directive`（`worklet:` 绑定 /
`applyAnimatedStyle` 的 updater / 路由 builder 返回的 `handle*Animation`）、
`worklet-data-freeze`（worklet 内解构 `this.data`）、`worklet-compile-setting`（用 worklet 但没开
`setting.compileWorklet`）。

**路由那一处我判错过，记在这里。** 最初让 #153 判 `addRouteBuilder(type, builder)` 的 builder
本身要有 `'worklet'`——依据是从文档 grep 到 `'worklet'` 命中。实际 `router-api.md:65` 的硬约束是
builder **返回对象**里的 `handlePrimaryAnimation` / `handleSecondaryAnimation` /
`handlePreviousPageAnimation`，官方示例里 builder 本身没有指令。按原口径会把所有照官方示例写的
工程判成 error。**grep 到某个词只能说明「这里有」，不能说明「必须在这里」。**

已知盲区：`const h = method((e) => { 'worklet' ... })` 这种包装写法，箭头是 `method(...)` 的实参，
输入层刻意不给被包装的箭头命名（否则 `const f = wrap(() => {})` 里的箭头会误叫 `f`），所以
`worklet-directive` 按名字查不到 → 不报。方向是漏报，但要覆盖需要「变量名 → 初始化表达式内唯一
函数」的绑定分析。

### P5 全量审计与修复

`[x]` 37 条规则逐条审计（WXSS / WXML / JS+配置 三条并行线 + 共用层扫描），结论与修复记录如下。

**架构是对的，问题在同一条判断被复制多份、每份只修一部分**：9 条 P0 缺陷里有 6 条是这个原因
（文件集有 4 份拷贝、互斥判定有 3 份、「值是不是动态的」有 3 套口径、`ends_with` 有 5 处）。
修复分四个提交（`c54643d` / `bdfb041` / `6ff4600` / `b102fbc`）：

P0（会误报或系统性漏报）：
- `scroll-view` 的 `type="default"` 是引擎合法值（`scroll_view_shadow_node.cc:20-23` → `kFlex`），
  从 error 降为 warning（没有按需渲染），并把枚举收进数据表；
- `box-sizing` 对 `$var` / `@var` / `#{}` 误报 error —— 改用共用的 `is_dynamic`；
- `<text space/>` 这种**无值属性**在 `[unsupported]` 下零诊断（`value==None` 直接 continue）；
- `refresher-default-style="none"` 少了 `refresher-enabled` 前置（默认 false，根本没有刷新区）；
- `exactly:1` 按摊平结果计数，`wx:if`/`wx:else` 分支子节点数不等时误报 —— 改成「能同时渲染
  的子元素最大数」；
- `no-inline-text` 自己拼文件列表，漏掉页面级 `renderer: "webview"` 的 opt-out；
- `worklet-directive` 的路由判定是文件级闸门（文件里任意对象上的同名键都判）→ 改为只认
  `addRouteBuilder` 的 builder 函数体里的对象；
- 内联 `<template name>` 的定义体在 `globals.sub_templates` 里，8 条 WXML 规则全看不见 → 输入层
  摊平并打 `in_template` 标记，依赖祖先/子节点计数的规则对这类元素跳过；
- `tag-selector-isolation` 只对比「组件 ↔ 用到它的页面」，组件 ↔ 同一页面的兄弟组件整类漏报。

P1（口径不一致 / 边界错位）：`no_pseudo` 的伪类白名单按引擎纠正——原来只白名单
`:not`/`:only-child`/`:empty`/`:host`，而 `fp::PseudoClassesType` 与
`css_parser_impl.h:271-329` 实际支持 `:first-child`/`:last-child`/`:nth-child()`，单冒号的
`:before`/`:after` 也被解析成伪元素并生效（`selector.rs:345-375`）。修正后四工程
21/630/21/5 → **16/302/10/4**，去掉的 328 条 businessview 命中全是 `:first-child` 一类误报；
仍报的 `:active`/`:nth-last-child`/`:checked`/`:hover` 等确实不在引擎的映射表里。
其他：跨 scope 的 `text-overflow` / `text-decoration` 判定（共享样式在
A 页失效、B 页正常时要报）、`no_pseudo` 的转义冒号（`.hover\:bg-red`）、`transition-behavior`
关键字、简写里的初始值（`background: scroll`、`mask: border-box`）、`filter`/`mask-image` 的函数
白名单与渐变异形、`radial-gradient` 的长度型前言、`flex-flow` 与「最后一条 display 生效」、
`route-type` 的动态注册（注册表不可知时整体不判）与接收者白名单、`worklet-api-scope` 的链尾
边界与插件重复报、`project.private.config.json` 参与配置判定、页面级 `rendererOptions` 只读
运行时那 6 个键。

共用层收敛（这轮做了什么）：`rules/wxml/branch.rs`（互斥 / 同时渲染计数）、
`rules/js/callee.rs`（`Exact`/`Suffix`/`Qualified`/`Namespace`/`Method` 五种形态，替掉 5 处
无边界 `ends_with`）、`value_util::text_is_dynamic`（单一动态值口径）、`Project::setting`
（private 覆盖 public）、`rules/data_table.rs`（数据表读行骨架，9 个解析器迁移；下一轮已整层删除，
数据改为 Rust const）、
`rules/testutil.rs`（三份 Fixture 合并，38 个测试文件改 import）、诊断构造收敛成
`span_diagnostic` / `element_diagnostic` / `at_value` / `at_declaration` / config `warn` 五族
（25 处手写 `Diagnostic::new` 替掉 13 处，剩 7 处因 `end` 缺失、级别不同或 help 类型不同而
保留——字段不等价的替换会把重构变成改行为）。

**解析层：伪类进了 AST 视图**（`SelectorPart::Pseudo { name, element, args }` +
`WxssSelector.pseudos` 带绝对 span）。`no_pseudo` 的 241 行引号/转义/属性选择器状态机整块删除；
报错位置从「选择器 offset + 合成文本内偏移」改成 oxc 的绝对 span，嵌套选择器（`&:active`）的
错行错列（`describe.scss:15:5`）修正到真实位置（`:14:10`，businessview 共 14 条位置修正）。
同一选择器上有多个不受支持的伪类时现在逐个报（businessview +2 条，`:focus` 与
`:not(:placeholder-shown)` 各一条）。风险更高的副作用是 `text-overflow` / `tag-selector` 不再因
`:first-child` 这类无参伪类退化成 Unknown——四个工程实测无新增，因为没有这类写法。

**数据源：9 张 txt 表全部换成 Rust const**。删掉 `rules/data_table.rs`（读行骨架）与
`include_str!` + 每表一份 `parse`，数据按**解析后的语义**直接写成 `const`：约束是 enum
（`Constraint::Enum/WebviewOnly/Unsupported/Number`、`Relation::Direct/Ancestor/RequiresChildTag/…`）、
`[requires]` 表达式是 `Trigger`/`Requirement` 变体、组件表是两份共享 const（`is_builtin` 与
`unsupported_replacement` 同一份数据）。证据（`文件:行号`）逐条保留为条目的行尾注释。
顺带消掉三处同一事实的重复（`components.txt` 曾被两份手写解析各读一遍）。验收：四工程诊断
逐字段 identical（16 / 304 / 10 / 4），数据条目数与取值由脚本对旧 txt 逐表核对（9 张全部相同）。

**明确不做（附理由）**：
- 简写展开（`border: 1px double red` 这类）：子属性的失效是否导致整条声明被丢弃，取决于引擎
  对各简写的分解实现，没有证据前报出来就是猜；`unsupported-value` 数据表头已写明边界。
- `scroll-context-ref` 的文件级启发：改成「同一函数内要有 `.ref`」会在「onReady 里取 ref、
  手势里 scrollTo」这种常见写法上误报，静态数据流不在工具边界内。

### P6 适配历史审计（161 个渲染相关提交）

沿 `skylinenovel` 的演进历史逐提交审计（7 条只读线并行，结论与每条的 diff 原文落在
`.tmp/hist/`）。最大的发现不是缺规则，而是**匹配器从不求值 `renderer`**：工程里 286 处
动态 class（33 处直接含 renderer），`has_class` 对非静态 class 一律返回 false，于是
`.novel .container …`（根节点 `class="novel {{PLATFORM}}-font"`）整条判不命中，
该页 272/396 条规则失效——**恰好在专门为迁移改过的元素上失明**。
修复分两步。第一步取「确定 token」口径：插值之外的直属文本词一定出现在运行时 class
列表里，`a-{{b}}` 整词丢弃；判不出的动态属性置 `Tree.unresolved` 让 `match_one` 返回
`Unknown` 而不是 `No`。第二步改成读表达式 AST：glass-easel 的 `Attribute.value` 本来就
是 `Value::Dynamic { expression }`，之前只留原文、把 AST 扔掉，才不得不用「按 `{{` 切文本」
的启发。现在 `input/expr.rs` 把表达式拆成静态文本 + 动态段，由字面量 / 三元 / 与或 / `??`
/ `+` 拼接构成的枚举出有限候选集（上限 16 个，超限退化成「任意」），判据从「词是不是写死
的」变成「这个词在候选里吗」：不在候选里 → 确定不命中。`class="btn-{{type}}"` 也不再被
误算成有个 `btn-` 类（插值可能是 `"a foo"`，拼出来 `btn-a foo`）。

已落地（六工程 9 字段 diff 逐条核过）：

| 项 | 规则 | 六工程新增 |
|---|---|---|
| 动态 class 的候选词元（表达式 AST） | 全部 class 匹配型规则 | plugin_1 +9 / plugin_2 +12 / skit-player +4，另 −6 条 `no-inline-text` 误报 |
| 动态属性值的候选集（`mode="{{a ? 'x' : 'y'}}"`） | `component-attribute`、`[attr=v]` 匹配 | 0（六个工程的动态取值候选都合法；fixture 覆盖全候选非法 / 混合 / 动态触发） |
| `<text overflow="ellipsis">` 缺 `max-lines` | `component-attribute` | plugin_1/2 各 +31 |
| 标准 `line-clamp`（引擎只认私有 `_wx_line_clamp`） | `unsupported-property` | 0（条目补齐） |
| 媒体查询条件被丢弃 | `media-query` | businessview +11、plugin_1/2 各 +4、skit-player +2 |
| 插件视图的配置规则（含 `plugin.json` 路径） | 配置族 | skit-player +6 |
| `renderer` 取值 / 版本区间校验、`ohosVersion*` 键 | `renderer-options` | 0（项目本身写对） |
| `<text>` 里的块级子节点 | `component-children` | skit_1 +1、plugin_1/2 各 +2 |
| `position: fixed` 遮罩被提升成独立层 | `fixed-layer` | plugin_1 +1（真问题） |

### P7 用当前规则复扫迁移前基线（`0404c098`）

`.tmp/pre-skyline` 保留着 `ba303412^` 的编译产物，用今天的二进制复扫，看「一次跑完能不能
给全清单」：

| 规则 | 早期扫描 | 现在 | 说明 |
|---|---|---|---|
| flex-direction | 97 | 97 | 全部由 `defaultDisplayBlock` 一处配置引起 |
| unsupported-property | 15 | 15 | |
| unsupported-value | 3 | 3 | |
| legacy-flexbox | — | 12 | 新规则：老式 `-webkit-box` 弹性盒按规则块报一次 |
| component-attribute | 8 | 23 | +15 `<text overflow="ellipsis">` 缺 `max-lines` |
| text-overflow-ellipsis | 7 | 16 | +9 动态 class 修好后能看到祖先链 |
| scroll-view-type | 12 | 12 | |
| no-pseudo | 12 | 12 | |
| media-query | — | 5 | |
| value-degraded | 5 | 5 | |
| no-inline-text | 9 | 3 | −6 误报：祖先 class 动态时匹配不到 `display:flex` |
| component-children | 2 | 2 | |
| default-layout-drift | — | 2 | 未声明 renderer 时也报（本轮改） |
| **合计** | **170** | **207** | |

只补那行 `rendererOptions` 配置（`/tmp/pre-cfg`）后剩 **108** 条，即真实代码清单是 108 条，
97 + 2 条是同一处工程配置。

本轮复扫顺带查出两处「上一批以为做了、实际没落地」的问题，已修：

1. 老式 `-webkit-box` 弹性盒从来没被覆盖（`git log -S "webkit-box-orient"` 在源码里查不到任何
   提交，只有 PLAN 文档提过）。依据：fp 的 `display` 取值解析只认
   none/block/inline/inline-block/flex/grid/flow-root/inline-flex
   （`float-pigment-css-0.7.0/src/property.rs:193-200`），`-webkit-box` 解析失败即整条丢弃；
   `-webkit-box-*` 都不在 `css/css_property_names.h` 的 194 个属性名里。落地成
   `skyline/legacy-flexbox`，按规则块报一次。
2. `default-layout-drift` 原来只在「存在 Skyline 页面」时报，而**未声明 renderer** 的工程
   （正是准备迁移的工程）按 Skyline 口径检查时反而不报，导致 97 条 flex-direction 逐条
   出现在报告里、却没有任何一处说「这一片由一行配置解决」。现在未声明 renderer 也报，
   `flex-direction` 的 help 也先给整工程解法。

仍在盲区（不在这一轮）：

- `-webkit-box-flex` / `-webkit-box-align` / `-webkit-box-pack` 等在 `display` 已生效为
  flex 的块里同样是死声明，但静态判不出它是不是无害兜底（autoprefixer 栈、或父级才是
  老式容器），按宁可漏报跳过。判据细节写进 `legacy_flexbox.rs` 模块注释。
- C3（`flex:1` 的 `scroll-view` 塌高）、C4（`<text>` 的 `max-width: calc()`）还缺引擎证据，
  调研中。
- JS 逻辑重写、WXML 重排、`wx:if` 里的 `renderer` 条件求值：静态判定的边界外。

### P8 `fixed` / 死选择器 / `vertical-align` 长度值

三条按「误报优于漏报」落地的判定：

| 规则 | 判据 | 六工程新增 |
|---|---|---|
| `skyline/fixed-layer`（重写） | `position: fixed` 且解析不出整数 `z-index`（无声明 / `auto` / 变量 / 插值） | 18（biz 4 / p1 7 / p2 6 / player 1） |
| `skyline/dead-selector`（新） | 主体带 class + 每个 class 名在文档里确定出现 + 全作用域求值皆 `No` | 9（skit_1 2 / p1 2 / p2 2 / player 3） |
| `skyline/unsupported-value`（扩） | 只接受关键字的属性写了长度值（当前 `vertical-align`） | 0（六个工程里没有消费中的写法） |
| `skyline/scroll-view-height`（新） | `scroll-view` + `flex-grow > 0` + 自己没有确定高度 + 父是纵向 flex 容器且同样没有确定高度 | 8（p1 3 / p2 3 / player 2） |
| `skyline/implicit-color`（新，Info） | 省略颜色的 `border*` / `text-decoration` / 阴影 + 元素有效 `color` 非黑 | 8（biz 5 / player 3） |

两条判据的取舍值得记下来：

1. **fixed 不再做层比较。** 旧实现要求 fixed 铺满视口、且同父下存在 z-index 更高的静态可见兄弟，
   实测六工程 0 命中——等于没覆盖。现在只看「有没有 z-index」。代价是离屏挂载的画布
   （`position: fixed; top: -9999px`）也会报，这是「误报优于漏报」明知的代价。
2. **死选择器必须先过两道门，否则全是噪音。** 只看「0 命中」时 businessview 多出 **879** 条，
   其中 606 条来自 vendored `weui.wxss`（类名在工程里根本不存在 = 未使用 CSS，#1443 排除），
   其余是 `@keyframes` 的 `from`/`to`（不是选择器）与 `hover-class` 运行时类。加门后降到 9 条，
   逐条核实全为真：`.heart.skyline`（修饰类挂错节点，正是历史 `101872e4` 那类）、
   `.black.color`（LESS 自嵌套产出不可能命中的复合）、weui 里 `.menu`/`.simple` 分组写的
   防御性选择器（其中一组永不命中）。
   代价：**含 `<slot>` 的组件里带组合符的选择器一律判不出**（投影内容可任意嵌套），这是保守
   方向，写进 `dead_selector.rs` 模块注释。
3. **`vertical-align` 的长度值在 fp 层就死了**：fp 的 `<length>` 解析接受百分比，但
   `vertical-align` 的值语法只有 6 个关键字分支（`float-pigment-css-0.7.0/src/property.rs:426-433`），
   解析失败后整条属性被回滚（`parser/mod.rs:1768-1781`）；引擎 `ParseValue` 也只映射
   baseline/top/middle/bottom。`text-top`/`text-bottom` 在 fp 语法里有、引擎不认，两边都已报。

4. **D1（`flex:1` 的 scroll-view 塔高）不能靠 fp 布局引擎判。** fp-layout 是「纯算法 +
   纯 trait」，没有 `overflow` 字段，scroll-view 与普通节点在布局上同构；塔高发生在
   Skyline 的 widget/measure 层（`enableScrollViewAutoSize` 关时它不是 measurable 节点，
   `scroll_view_shadow_node.cc:1098`）。又是负向结论：静态树只有 1 个 item、文本高度未知，
   真跑一次布局会给出「看起来精确的错误答案」。落地改为「样式事实 + 父主轴不确定性」：
   `scroll-view` + `flex-grow > 0` + 自己没有确定高度 + 父是纵向 flex 容器且同样无确定高度。
   不带「父主轴不定」这条的话，命中集合就是「所有 `flex:1` 的 scroll-view」。

5. **`min-width: 0` / `min-height: 0` 在 Skyline 是 no-op。** Skyline 的 `min-*` 初值是
   `auto`，但 fp 的 `DefLength::Auto` 解析成 `OptionNum::none()`，`maybe_max` 对 None
   直接返回自身，所以「`min-width:auto` ≡ `min-width:0`」（`layout/src/types.rs:222,418`、
   `unit.rs:503`）。WebView 上「flex 子项需要 `min-width:0` 才能收缩」的经验不能直搬，
   写这条规则会全是误报。历史提交 `7a79e81d` 里真正起作用的是 `<text overflow=max-lines>`。

6. **feature toggle 一类的做法**：`space` / `decode` 不是实验开关（`feature_toggle.cc:45-46`
   建时无条件置 true），不必报；真正开关控制、且静态可判的是 `current_color`——省略颜色的
   `border` / `text-decoration` / 阴影在开关关闭（默认）时取黑色，而 WebView 取元素 `color`。
   规则只报「省略颜色 + 元素有效 color 非黑」，显式 `currentColor` 不受影响。开关状态运行时
   才可知，所以按 Info 报；其余 toggle（text layout 重构、`native_border`、`new_selection` 等）
   要么是 perf，要么没有 WebView 基线，不做规则。

评估后不做（附理由）：

- `skyline/fixed-layer` 的三条限制（详情见规则模块注释）：只判铺满视口的遮罩；只判同一
  父节点下的兄弟浮层；只判**静态可见**的被盖层（不在条件分支、自己与祖先都没有运行时改写
  的 `class`）。理由分别是：固定边条 / 底栏 / 离屏 canvas 同样被提升但重叠静态不可知；
  组件里的 fixed 提升到的是组件自己的 root container，页面浮层仍在它之上（历史 bug
  `8fc12686`）；浮层显隐几乎都由 class / `wx:if` 驱动，`showContents` 与 `showOption`
  互斥这类事实写在 JS 里——放开后六工程立刻出现「遮罩 vs 同容器面板」的误报。三条限制
  叠加后六工程 0 命中：规则守的是历史那四类形态（始终可见的顶栏被全屏 fixed 遮罩盖住），
  这类形态在现有六个工程里已经全部改成 absolute 了。
- `<span>` 被 WebView 页面复用：这是反向差异（WebView 坏、Skyline 好），与本工具
  #1443 的口径相反；且 WebView 对未知标签的处理未在源码里确证。
- `.a .a` 这类退化选择器（本意是同类却写成后代）：属于 #1443 排除的 dead-code 检查。
- 简写里含 `var()` 会被丢弃：已用 fp 源码否掉
  （`float-pigment-css-macro/src/style_syntax.rs:796-810` 为含 `var(` 的简写生成
  `var_in_shorthand`，运行时代入后重新解析），两个工程的 `rgba(var(--x), .5)` 至今正常显示。
- `no-inline-text` 在本工程（开了 `defaultDisplayBlock`）前提不成立：方向相反，
  `defaultDisplayBlock` 让容器变 block，而真差异正是块级容器。
- `display: -webkit-box` / `-webkit-box-orient` 做**逐条声明的表项**（而不是按块合并）：
  autoprefixer 的 `display:-webkit-box → -webkit-flex → flex` 兜底栈会让它命中 139 个块、
  其中 133 个最后一条 display 是 flex、根本没有差异；逐条报就是纯噪音。

### P9 模板常量环境与「硬换行省略号」

`renderer` 这类绑定挂在 `data` / `properties` 上，模板里的 `wx:if="{{renderer === 'skyline'}}"`
以前两条分支都当「可能渲染」。现在从数据来源注入一份常量环境（`Project.envs`，每个 WXML
一份），在 `input/expr.rs` 的推导链上做 JS 里无副作用的折叠，不可达分支直接从树里摘掉。

来源与保守方向：

| 来源 | 采纳条件 |
|---|---|
| 组件 `properties` 默认值 | 该键没在 `setData` / `this.data.x = ...` 里被写过（`data` 不收：初始化后会被反复改写） |
| 父级静态属性传值 | 同一组件**所有**使用处都传同一个静态值 |

缺省、动态传值、任意一处不一致 → 该键从环境消失。折叠错了会删掉可达分支，是最难发现的
漏报，所以默认方向偏保守。六工程 9 字段 diff：novel / businessview / skit_1 / skit-player
零变化；plugin_1、plugin_2 各 −5 +1，消失的 5 条全部核过为落在被摘掉的不可达分支里
（`official-heart-popup` 由 skylinenovel 静态传 `renderer="skyline"`，`heart-popup` 只在旧
novel 页用、默认 `webview`）。

`skyline/text-hard-break-ellipsis`（新，Info）：`<text overflow="ellipsis" max-lines>` 的静态文本
含 `\n` 时报。依据是 `show_hard_line_break_ellipsis` 默认关（`feature_toggle.cc:20-22`），
`widgets/text.cc:435-437` 只在开启时 `SetShowHardLineBreakEllipsis(true)`。六工程 0 命中——
347 处 `overflow="ellipsis"` 的 `<text>` 里没有静态换行，而真实工程里的硬换行基本来自数据
（`{{item.desc}}` 里的 `\n`），静态看不见。这条只当提示留着。

同一批被源码否掉的两条，不再做：

- **`word-break: break-all` + CJK/emoji**：旧路径 `MakeWordsBreakable`（`widgets/text_span.cc:23-35`）
  逐**字节**遍历，只对 `<= 0x7F` 的 ASCII 字节插 U+200B——多字节字符根本不会被插入断点，
  所以不存在「按 ASCII 逐字符切 CJK/emoji」这回事，与 WebView 的 `break-all` 无差异。
- **`mask-image` + `border-radius`**：开关关时遮罩不向外扩 0.5px（`paint/mask_image_painter.cc:46-56`），
  接缝只在亚像素边界可见。原本打算只在和 `border-radius` 同时出现时报，但 businessview
  239 个含 `mask-image` 的块里 **0 个**带 `border-radius`；不设这道门就等于报所有 `mask-image`，是纯噪音。

### P10 行内 run：兄弟文本级元素

`skyline/inline-run`（新，Info）：WebView 里 `<text>` 是 inline、`<image>` / `<span>` 是 inline-block，
block 容器里并列的文本级元素走同一个行内流；Skyline 里它们默认是 block，各自占一行。

判据：同一父节点下连续 ≥ 2 个文本级元素、没有裸文本（那一半归 `no-inline-text`）、
没显式写 `display`，且父级不是 flex/grid。三条闸门（全部基于实测误报添加）：

| 闸门 | 误报形态 |
|---|---|
| 父级 flex/grid | 两边都按 flex item 排，无差异（`recent-read.wxml:99` 的 `.oval-btn`） |
| 成员显式 `display` | 作者自己指定了行内/块级，两端一致 |
| 成员绝对定位 | `book.wxml:125` 的 `.finished-tag{position:absolute}` 两种渲染器都脱离文档流 |
| 成员有 `width` / 纵向 `margin` | `bookshelf.wxml:47` 的 `.title{width:100%}` + `.status{margin-top:2px}`：作者用纵向间距表达「各占一行」 |
| 同 `wx:if` 不同分支 | `comment-dialog` 的 like 图标三选一 |
| 元素在 `<template name>` 定义体内 | 见下 |

误报面：第一版 36 条 → 加闸后 10 条，逐条核过均为真：businessview 6（`auth.wxml:29` 的
「我已阅读并接受+链接」，`.comment-info text{margin-right:8px}` 这类横向间距、`-amount{margin-left:2px}`），
plugin 4（`comment-dialog` / `seg-comment-dialog` 的「未精选/置顶/作者赞过」三连，×2 组件 ×2 工程）。

**顺手发现的结构性缺口**：`display_values` 在 `<template name>` 定义体里一律返回空——
模板体实例化时才接上外层祖先，样式表里的后代选择器（本工程 LESS 嵌套几乎都是这种）在那里
匹配不到，于是模板体内的样式类判定全部失明。`inline_run` 直接跳过模板体；
`no-inline-text` 的「flex 容器跳过」闸门同样失效（可能在模板体里误报），待评估。

**D3 / P1-2 的结论修正**：源头上「TextShadowNode 的非文本子节点需要 `display:inline`」是不成立的
——`shadow_node.cc:205-209` 按**节点类型**分派，与 CSS `display` 无关；`inline-block` 只影响
WidgetSpan 内部排版。因此原先计划的「`<text>` 子节点 display 收窄」规则撤销。
文本里要放 `view` / 自定义组件时它们自动变原子 WidgetSpan；历史提交里的
`display:inline-block` 是给 WebView 侧用的（WebView 里 `<view>` 是块级），属于反向适配那一类。

### P11 反向：Skyline 代码退回 WebView

工具原本只有「WebView → Skyline」一个方向。存量按 Skyline 写的小程序跑在不支持 Skyline
的环境（PC 端等）时，**整页退回 WebView**、不报错不白屏（`skyline-overview/references/introduction/overview.md:108-110`；
能力位由宿主给，PC adapter 直接 `isLibSupportSkyline: false`），页面里所有 Skyline 专属能力
整体变成「未知能力」，逐个静默失效。

方向用 `Target` 表达（`Rule::target()`，默认 `Skyline`；`Both` 两侧都跑），CLI 是
`--target skyline|webview`（默认 `skyline`，正向行为不变）。两个方向不做同一套判据的复用：
同一条 `max-lines`，正向的修法是「补 `overflow`」、反向是「补 `-webkit-line-clamp`」，
合并会让 help 自相矛盾。

**52 → 53 条的归属**：43 条是纯正向（引擎能力边界，反向时 WebView 用浏览器实现，不存在同类问题）；
4 条双向（`dead-selector`、`renderer-options`、`lib-version`，加上 `component-attribute` 的部分判据）；
6 条反向。

已落的反向规则：

| 规则 | 判据 | WebView 下的实际行为 |
|---|---|---|
| `webview/text-truncation`（warning） | `<text>` 上的 `max-lines` / `overflow` | 静默忽略：`wx-text` 只声明 `selectable / user-select / decode / space`（`wxapplib/src/wx-component/src/wx-text/wx-text.js:131-173`），`overflow` 也不是 HTML 表现属性 |
| `webview/nested-scroll`（warning） | `scroll-view type="nested"` / `nested-scroll-header` / `nested-scroll-body` | `type` 从不被读（`wx-scroll-view.js:239`）；后两个标签**不在** WebView 的 `globalUsingComponents` 表里（`glass-easel-adapter/src/render/webview.ts:166-254`）→ 默认组件（只有默认 slot）+ inline 显示，头部不联动 |
| `webview/text-children`（error） | `<text>` 下的元素子节点（非嵌套 `<text>`） | `wx-text` 的 `TextPseudoSlot` 只放行文本节点、`__wxElement.is === 'wx-text'` 的子元素与文档片段（`wxapplib/src/wx-component/src/wx-text/wx-text.js:15-62`），`<view>` / `<image>` / `<span>` / 任何自定义组件直接消失、不报错 |
| `webview/worklet-binding`（error） | WXML 的 `worklet:` 前缀属性 | 模板编成 `wl()`，DOM wrapper 里它就是 `noop()`（`glass-easel/src/tmpl/proc_gen_wrapper_dom.ts:249-251`）——连 Skyline 的 `worklet-handler` behavior 都轮不到，回调不触发也不报错 |
| `webview/pinned-renderer`（warning） | 单元的 `renderer` 被固定成常量 | 判据不是属性名，而是正向折叠用的同一份事实：单元常量环境（`Project::env`）里 `renderer` 能定成 `skyline`（或定成 `webview` 但被 Skyline 页面用到）。环境只收 `properties` 默认值与父级静态传值，任何一处动态传 / 没传都会让它失效；因此命中就意味着「这个单元已把渲染后端当常量在用」——`wx:if` 相应分支在编译期就被折掉。`59a2d74f` 的修法（`data.renderer` 默认值 + `onLoad` 里 `setData({renderer: this.renderer})`、6 处 prop 改成 `{{renderer}}`）完成后两条路径都判不出常量 |
| `webview/tabbar-renderer`（error） | `app.json` 的 `tabBar.custom: true` + `tabBar.renderer: "skyline"` | tabBar 渲染器与页面分开取（`onAppRoute.js:672` 的 `WXConfig.tabBar.renderer \|\| res.renderer`），钉成 skyline 后回退时 `WebviewAdapterTabBar.getInstance` **直接 `throw`**（`render/webview.ts:1071-1075`），不是降级 |

消音机制：`WxmlElement::renderer_guarded()`——元素（或祖先）处在条件里引用 `renderer` 的分支里
就跳过。识别靠 `input/expr.rs::references` 走表达式 AST，能认出
`wx:if="{{renderer === 'skyline'}}"` / `wx:elif` / `{{!!renderer}}` 各种写法。
实测有效：`miniprogram-novel/pages/recent-read/recent-read.wxml:19` 的 `wx:elif="{{renderer === 'skyline'}}"`
下三个 `worklet:` 绑定被正确跳过，而同工程 `rank-center.wxml:189,339` 的无守卫写法照报。

明确不做的（源码否掉了「看起来该报」的候选）：

| 候选 | 否掉的理由 |
|---|---|
| `scroll-view type="list"` | WebView 不读 `type`，但**仍能滚动**（滚动由 `scroll-x/y` 的 CSS `overflow` 决定），只丢虚拟化 |
| `list-view` / `grid-view` / `sticky-*` / `snapshot` / 手势组件 | 它们**在** WebView 的 `globalUsingComponents` 表里（`render/webview.ts:166-254`），不是未知标签，各有一套 WebView 实现 |
| `grid-builder` 的 `slot:item` 内容丢失 | 确属丢失（未知标签只保留默认 slot），但两个参考工程都没用，等有真实使用再加 |
| `<text space>` / `decode` | WebView 真的支持（`_htmlDecode`），两端都有 |
| `<page-meta>` 全部属性 | 两端都支持；`screen-orientation` 两端都不存在（与 `page-orientation` 混淆） |
| `wx.worklet.shared` / `timing` / `applyAnimatedStyle` | WebView 有完整实现，不是降级：适配器按渲染器分派到 `createAnimatedStyleForWeb`（`glass-easel-adapter/src/subcontext/component.ts:675-704`），计算在逻辑线程 rAF 上跑、结果经 `node.setNodeStyle(..., ANIMATION_EXTRA)` 落到 DOM。注意 `hook/animatedStyle.ts:84` 的 `updateNodeStyle` 那条路径在 WebView 下确实是 `noop`（`worklet-module-web/index.ts:11-14`），但它只服务 `createAnimatedStyle`（原生路径），WebView 走不到。因此只有性能差异 |
| Skyline 页被注入的 `disableScroll: true` / `navigationStyle: custom` | 编译器对 Skyline 页总是注入这两项（`miniprogram-compiler-rs crates/graph/src/json/mutate.rs:470-475`），WebView 下仍然生效。但漏子不在源码可写的字段上：页面默认就是可滚动的，而「这个页面依赖页面级滚动」静态判不出，按现状不报 |
| `this.animate` / `wx.createAnimation` | 方向相反：它们是 **WebView-only**，恰是 worklet 的降级方案（正向 `unsupported-api` 已覆盖） |
| `fixed` 提升 / `defaultContentBox` | 反向时 WebView 是正常层叠 / 本来就是 content-box |
| CSS 能力差异（`position:sticky`、`clip-path`、`scroll-snap`、`object-fit`） | 差距几乎全是 WebView 更强，反向无需规则 |

待办（严重度高但两个参考工程零使用，未落）：`wx.router.getRouteContext` 直接解引用
（纯 WebView 下返回 `undefined` → TypeError 白屏）、`ScrollViewContext.triggerRefresh` 系列
（只 `publishSync('operateSkylineScrollView')`，WebView 不监听 → 纯 no-op）、
`NodesRef.ref()`（`res.ref = undefined` + warn）、`node()` 取 `snapshot` / `draggable-sheet` /
`open-container`（返回 fake 上下文）、`navigateTo({routeType})`（静默降级成默认转场）。

**顺手修正的一处旧文案**：`skyline/default-layout-drift` 原来说「position 默认 relative、text-align 默认 start 这两项没有开关能对齐」。`text-align` 两边初值都是 `start`（`skyline/css/css_properties/text_align.cc:44-47` 的 `ApplyInitial` 就是 `Start`），木有差异；只有 `position`（Skyline `Relative`、`position.cc:39-42` vs CSS `static`）成立。

实测命中（`--target webview`）：plugin_2 63、novel 19、skit_1 23、plugin_1 53、businessview 12。
正向方向逐字段 diff 五个工程**零变化**（170 / 23 / 27 / 326 / 175）。

`pinned-renderer` 与 `text-children` 的验收来自真实分支：`weapp-novel-plugin_1` 的 `feat-glass-easel-webview`
（`41c6ae58` / `c8944057` / `59a2d74f`）。拿 `8c8a9450`（分支前）与 `59a2d74f`（全部修完）两份源码对比，
同一个 `pages/skylinenovel` 的结果是：

| 状态 | `pinned-renderer` | `text-children` | `text-truncation` |
|---|---|---|---|
| 分支前 `8c8a9450` | 6（各自标出调用处的 `index.wxml` 行号） | 1 | 29 |
| 分支后 `59a2d74f` | 0 | 0 | 29 |

7 条正好对应那个分支的两处结构修复（6 处静态 `renderer` + 1 个 `<text>` 包组件），修完归零；
剩下 29 条 `text-truncation` 是分支没做的那部分（只说 `<text>` 在 WebView 不截断，不要求现在修）。
`miniprogram-skit_1` 的 `components/index-head/index-head.wxml:54`（`<text>` 包 `<comment-content>`）是第二个真实例。

**顺手修掉的两个方向判定错误**：

1. `skyline/inline-run` 与 `skyline/flex-direction` 原先标 `Both`，但它们的判据都是「Skyline 下怎么排」
   （inline run 在 WebView 里本来就是行内流），反向扫描只会制造噪音——已改回 `Skyline`。
2. `skyline/renderer-options` 把 `sdkVersionBegin/End`、`ios/android/ohosVersionBegin/End`、
   `disableABTest` 归为「只在 app.json 生效」，于是适配后的页面 json 被报了 6 条假错。
   官方文档写的是「app.json **或** page.json」（`skyline-overview/references/migration/release.md:105-145`），
   页面 schema（`miniprogram-compiler-rs crates/graph/src/json/json/schema_page.json`）也列了这些键，
   客户端选渲染器时读的就是页面配置——整个「按客户端版本只把这一页降回 WebView」的策略就靠它。
   已删掉 `Where` 维度与那条断言。

同一分支里另三件事**不做规则**：

| 分支里的修改 | 不做的理由 |
|---|---|
| `data.renderer` 从 `'skyline'` 改成 `'webview'` + `onLoad` 里 `setData({ renderer: this.renderer })` | 未适配状态下这个字面量是死值（同一 WXML 不用它分支），静态报就是噪音；而适配后它就变成 dirty 键，常量化自动失效，`webview` 分支也就不会被错误折叠 |
| 静态传 `renderer="skyline"` 给子组件 | 已落成 `webview/pinned-renderer`（按常量环境判，不按属性名） |
| `rendererOptions` 的 `iosVersionBegin/End` 收窄到 `15.255.254`、`glassEaselWebview: true` | 前者是「停掉 Skyline」的开关，`skyline/renderer-options` 已校验区间形态与合法性；后者是让 PC/老版本走 glass-easel+WebView 兼容模式，不开也不报错、只是回退到老 exparser 引擎，“该不该开”取决于发版策略，不是代码缺陷 |
| `.content { flex: 1; min-height: 0 }`（`half-screen-dialog.less`） | WebView（浏览器 flex）默认 `min-height: auto`，溢出子项不收缩；Skyline 的 `min-*` 初值解析成 `none`，不需要它。但「这个容器会溢出」静态判不出，做成规则就是给所有 flex 子项报一条 |

### 不做

autofix；14 条只能运行时判定的条目；`app.json` 自动改写。

### P12 评审收口：输入层与规则判据的一轮修正

对 `54a69f5..HEAD` 的 13 个提交做了四条 lane 评审（基础层 / 正向 WXSS / 反向 webview / 重复与死代码），
结论逐条回源码核对后落地。基线二进制是 `49b7c9c` 的 release 产物，六个参考工程正反两个方向逐字段 diff。

**输入层（会影响全部规则 → 只允许「修错」不允许「改口径」）**

| 问题 | 证据 | 修法 |
|---|---|---|
| `wx:if="false"` 被当成假 | 静态条件编成**字符串字面量**（`proc_gen/tag.rs:1016-1019` 的 `gen_lit_str`），`"false" ? 1 : 0` 为真；只有空值等价于没写属性（`parse/tag.rs:4386-4387`） | `condition_truthy` 只看 `!value.is_empty()` |
| `renderer_guarded` 是整条链的并集 | `wx:if="{{a}}"` + `wx:elif="{{renderer==='skyline'}}"` 时，前者两支都会渲染，却被一起消音 | 每个 arm 只按**自己**的条件标；`else` 仅在链上有 renderer 条件时标 |
| 内联 `<template name>` 定义体继承单元 env | 定义体的数据作用域是 `<template is data="…">` 新建的对象（`proc_gen/tag.rs:1275-1287`），看不到 data / properties | 定义体一律用空 env 解析（`load_doc_unfolded` 同源） |
| `this.data['x'] =` / `Object.assign(this.data, …)` 不置脏 | `js.rs` 只匹配 `this.data =` 与 `this.data.x =`，注释声称处理了前者 | 补 computed member（字面量键置脏、其余置 `dirty_all`）与 `Object.assign` |
| `+` 展平丢左结合 | `1 + 2 + 'px'` 得 `"12px"`，JS 是 `"3px"` | 常量拼接按 JS 语义：先求和纯数字前缀再逐段拼接 |

同时删掉四个零调用的 pub API（`Tree::doc` / `Tree::parseable` / `Subject::has_locator` / `Env::iter`），
把「page → 可达组件 → load_doc」三段逐字重复的扇出循环收成 `rules/wxml::page_wxml_groups`，
把「WXML 路径 → 单元」的两份查找收成 `Project::scope_of`，`inline_run` / `no_inline_text` 的互斥判定
合并到 `branch::exclusive_branches`。

**规则判据**

| 规则 | 原判据的问题 | 现在 |
|---|---|---|
| `webview/text-truncation` | ① 只要 `max-lines` / `overflow` 存在就报，`overflow="visible"`、`max-lines="0"` 两端一致；② 消音用 `selector.text.contains(class)` 子串匹配（`.btn` 命中 `.btn-primary`），且不看级联；③ 单行 `white-space: nowrap + overflow: hidden + text-overflow` 兜底不认 | ① 按取值筛；② 走 `resolved_property`（选择器匹配 + 后写覆盖）；③ 单行三件套与祖先的 `-webkit-line-clamp` 都算兜底 |
| `skyline/scroll-view-height` | ① 只看元素自身 `height`，`100vh > flex:1 > scroll-view` 这种标准布局被误报；② 内联 `style="height: {{panelHeight}}px"` 读不到值，把运行时高度读成没有 | ① 沿祖先上溯（父子项 `flex-grow > 0` + 列容器）；② 动态内联 `style` 一律按确定高度（宁可漏报） |
| `skyline/implicit-color` | `border: 0` / `border: none` / longhand 已给 `color` / `page { color }` / `var()` 都判错 | 跳过不可见声明与 longhand 覆盖；`page`/`:host` 基色参与；`var()`/`env()` 视为判不了 |
| `skyline/legacy-flexbox` | ① 取**最后一条** `display` 而不看能否解析（`flex` 后跟 `-webkit-box` 时按后者报）；② 文案与等级把多行省略惯用法说成「整块布局坏掉」 | ① 取最后一条**可解析**的 `display`；② 带 `-webkit-line-clamp` 时改讲「多行省略失效」并给 `<text max-lines>` 修法 |
| `skyline/dead-selector` | 逐文档独立求值，`app.wxss` 里 `.page-cell .cell-bd`（一半在页面、一半在组件）必判死 | 多文档时若没有任何一份同时含全部类名，则不判死 |
| `skyline/unsupported-value` | `vertical-align: 0` 漏报（`is_length("0")` 为假），而 fp 只有关键字分支 | 无单位零算 `<length>` |
| `webview/tabbar-renderer` | 只认精确小写 `"skyline"` | 只要不是 `"webview"` 都报（适配器就是按不等判断抛异常） |
| `skyline/lib-version` / `skyline/renderer-options` | 标 `Both` 但判据只对 Skyline 有意义，反向是噪音 | 改回 `Skyline`（同时修了 `Target::includes`：`Both` 作为检查方向时也要跑单方向规则） |

**明确不做的**

| 候选 | 理由 |
|---|---|
| `.write-comment.skyline` 这类「修饰类只在动态 class 里出现」的死选择器 | 它在 Skyline 那一支是活的；要报必须先按 target 求值，否则就是把渲染器差异当写错 |
| `hover-class` 上的复合选择器 | 悬停时才挂类，静态判不出它能否与其余条件同时成立——维持「不算确定存在」 |
| `fixed-layer` 的离屏画布豁免 | 用户口径是「有 fixed 无整数 z-index 就报」，离屏海报画布（`top: -9999px`）仍照报，属已接受的噪音 |

**验收**（六工程，两个方向逐字段 diff，`49b7c9c` → 本轮）

正向：novel 23 → 23、skit_1 27 → 27（仅文案）、businessview 326 → 326（+3 legacy、+2 `vertical-align: 0`、−5 `implicit-color` 误报）、
plugin_1 175 → 173、plugin_2 170 → 168（各 −2 条动态内联 `height` 的 scroll-view 误报）、skit-player 175 → 170（−3 implicit-color 误报、−2 scroll-view 误报）。

反向：novel 19 → 16、businessview 6 → 0、skit_1 23 → 15、plugin_1 56 → 25（+1 为 `book_name` 的 clamp 被 `.skyline` `unset` 掉的真实漏报）、
plugin_2 65 → 33、skit-player 3 → 3。每一条消失都回源码确认过兜底真实存在：
plugin_1/2 的 32/33 条落在 `cover` / `contents` / `recommend` 系列的 `-webkit-line-clamp` 或单行三件套上；
skit_1 的 8 条落在同组件 stylesheet 里定义的 `.ellipsis`（`slide-tips.less:1` 等）上；
唯一保留的 `logout-dialog.wxml:9` 同样写 `class="username ellipsis"`，但 `logout-dialog` 自己没有 `.ellipsis` 定义——组件样式隔离下它是真缺陷。

测试 710 + 30 + 3 全绿，clippy 0，`docs/RULES.md` 已重生成（53 条，error 15）。

## 5. 五个视图与规则族

| 视图 | 来源 | 服务哪族规则 |
|---|---|---|
| ProjectView | `AppConf` + `Resolver` + M6/M8 | 公共上下文 |
| WxmlView | `glass-easel-template-compiler::parse`（不经过 `compiler-wxml`，它在 API 边界丢弃 AST） | 结构类、跨文件类 |
| WxssStructure | `oxc-css-parser` 0.0.17（只保留摊平，见 P3） | 语义类、跨文件类 |
| WxssSemantics | float_pigment hook（feature-gated，默认不用） | 值支持类兜底 |
| JsonView | 原始 JSON + offset 索引 | 配置类 |
| JsView | `oxc_parser` 0.155（与 CSS 前端共用 `oxc_allocator` 版本线） | worklet / API 禁令 |

`JsView` 是 owned 模型：`Project.scripts` 持有**整个工程**的 `.js` / `.ts` 源文件与 `JsDoc`，
按 path 去重。单元脚本带 `Scope::Global` / `Page` / `Component`；其余（`utils/`、`monitor`、
`components-for-playlet/` 这类不在 app.json 闭包里的源码）记 `Scope::Loose`。文件枚举走
`ProjectFs::list_files`——那是 minidev 已有的事实接口，它自己剪掉 `node_modules` 与点目录；
我们在其上排除 `.d.ts` 与 `miniprogram_npm`（编译产物）。聚合统计（如「工程里有没有
worklet」）用一次过滤算出来，不做增量缓存。

`WxmlView` 的覆盖面同样是全域：单元 WXML 之外，`<import>` / `<include>` 引入的模板
（`Project.template_files`）也进图，走递归闭包、按路径去重。两处取舍要记住：

- 模板不属于任何单元，所以它没有 `Scope`，也因此**不受** `renderer: webview` 的页面级
  opt-out 保护——目前从所有单元 WXML 出发收集，被 webview 页面独占引用的模板仍会被检查。
  要收紧就得记录「模板 ← 哪些单元」的来源信息，眼下不值得。
- 编译器在解析时就把 `src` 的 `.wxml` 后缀剥掉了（`tag.rs:1391-1408`），所以还原路径要靠
  「无扩展名补 `.wxml`」，不能直接拿 `src` 拼。

这两处补盲区的动机是实测出来的：短剧工程 39 个 `.ts` 里只有 27 个属于单元，`utils/` 完全
不可见；`templates/pagination.wxml` 被 3 个页面 import 却不在任何单元里。补齐前后四个真实
工程的诊断**一条没变**（21 / 630 / 21 / 5），所以这是「消除未来工程的漏报源」，不是修现有
误报。

值支持类只读 `WxssSemantics`，语义/结构类只读 `WxssStructure`——两族不共享判定，
所以不会出现「同一行一条说解析失败、一条说不支持」，也不需要关联两遍解析结果。

`WxssStructure` 对规则层暴露的是**解析器原语，不是字符串**：`WxssDeclaration.tokens`
（`ValueToken`：Function/Dimension/Number/Percentage/Color/Url/Ident/Str/Comma/Dynamic/Other，
由 oxc 的 `ComponentValue` 递归转成 owned 值）与 `WxssSelector.parts`
（`SelectorPart`：Tag/Universal/Class/Id/Other，来自 `ComplexSelector`/`CompoundSelector`）。
规则层因此不再自己切值、扫选择器：`value_util` 只剩十几个基于词元的包装（`split_top_level`
按顶层 `Comma` 切、`split_tokens` 直接取词元、`top_level_calls` 取 `Function`），
选择器的形状分类（`rules/wxss/selector.rs`）只剩「是不是纯标签 / 纯 id」这一个用途，
「命中哪些节点」交给下面这一层。

### 选择器匹配（`src/matcher`）

跨文件规则要回答「这条声明落到哪些节点上」。解析与匹配分两层：解析仍用 oxc
（`input/wxss.rs`），匹配用 `selectors`（servo/stylo 抽出的引擎，MPL-2.0）。

分工是刻意的：CSS 文法（`an+b`、`:not()` 的复合参数、6 种属性运算符、`i`/`s` 标志）交给
引擎；我们只实现 `selectors::Element` 的 WXML 适配器（取父子兄弟、读属性）+ 一层分类。

**方言参数**是必需的，同一段选择器在两个渲染器下可匹配的范围不同：

| 构造 | WebView | Skyline |
|---|---|---|
| `[attr]` / `[attr=v]` | 按规范 | 条件被丢弃（`css/selector.h` 的 `CompoundSelector` 无 attribute 字段）；主体另有 id/标签/class 时等价于去掉该条件，主体只有属性时整条规则进不了规则集（`rule_set.cc:106-160`） |
| `:nth-child(an+b)` / `+` / `~` | 按规范 | 按规范（`selector_matcher.cc:126,292,311`） |
| `:nth-of-type` / `::selection` | 按规范 | 无对应 case，降级成「没有这个伪类/伪元素」 |
| `:active` / `:hover` / `:placeholder-shown` | 运行时状态 | 整条规则在解析阶段被丢弃（`parser/mod.rs:989-991`） |
| `::before` / `::after` | 文本节点，不是宿主 | 同左 |

三态（`Tri::Yes/No/Unknown`）由我们自己产生，引擎只返回 bool：动态属性（`class="{{x}}"`）、
运行时状态伪类、以及「选择器用到 `+`/`~`/`:nth-*` 但兄弟区含 `wx:if`/`wx:for`/模板片段」
三类提前判成 `Unknown`。动态属性只在候选集判不了时才算 `Unknown`：`[mode='cover']` 在
`mode="{{a ? 'cover' : 'fill'}}"` 上是 `Unknown`（两分支都可能），在 `[mode='x']` 上是
`No`（不在候选里），`[mode]` 是 `Yes`（存在性与取值无关）。Skyline 按运行时 shadow tree 计数兄弟（假分支不占位、`wx:for` 每份
是真兄弟），静态模板算不出序号，只能承认未知。

消费者与方言归属：`display_values`（`no-inline-text`、`horizontal-scroll-enable-flex`）
问「Skyline 算出什么 display」→ Skyline 方言；`broken_selectors`（`text-overflow`、
`text-decoration`）与 `tag-selector-isolation` 问「WebView 命中哪里」→ WebView 方言。

已知边界：选择器解析失败按 `Unknown` 处理（宁可漏报）；`wx:for` 的复制次数不建模；
组件边界不在这里判（由 scope 决定）。

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
| R1 | 加载器的语义正确性由我们自己负责（不再有编译器团队兜底） | 四工程 + 合成工程逐字段对拍；口径与复验步骤见 §3 |
| R2 | 改 minidev 破坏 parity | 行为变更（M4/V1）必须先有 fixture 证明「原行为是错的」 |
| R3 | 知识库自身矛盾（`form` / `calc` / `scroll-view type` / `display:inline`） | 实现前到 skyline 源码取实证 |
| R4 | 规则噪声 | negative fixture 下限；默认只开 error；inline disable |
| R5 | `glass-easel-template-compiler` 版本漂移（minidev 锁 1.2.0，crates.io 1.2.1） | 同一 version spec，让 cargo 合并成一份 AST |
| R6 | 缺 float_pigment 导致值判定漏报 | 一致性语料；实测漏报率不可接受再按 feature 引入 |
