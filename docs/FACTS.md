# Facts：迁移作业单需要哪些中间结果

这份文档回答一个问题：**引擎里已经算出来的东西，够不够支撑「把一个元素为什么这么判」讲清楚**——
迁移工具（生成作业单、让 agent 改代码）要靠这些事实，而不是只靠一句 lint message。

结论写在最前面，后面的分层盘点是证据。

## 落地状态（2026-11）

| 阶段 | 状态 | 落下的是什么 |
|---|---|---|
| P0 暴露 | **已完成** | `skylint_core::inspect`：`LoadedDoc` → `Facts`（`elements` / `element_at` / `ancestors` / `cascade` / `resolved` / `layout` / `unit` / `reach`），返回拥有所有权的快照 |
| P1 级联准确化 | **已完成** | `cascade_entries` 按 `(important, specificity, 出现顺序)` 排序，对齐引擎的 `CascadePriority`；内联 `style` 参与排序（`!important` 也解析）；`CascadeView` 带 `selector` / `stylesheet` / `line` / `column` / `at_rules` / `overridden`（升序里前面的就是被覆盖的） |
| P2 图回链 | **已完成** | `ancestors` 走 `WxmlGraph`（跨 slot / `virtualHost`）；`usages()` 给出「这份组件被哪些使用点引用」，`usages_recursive(depth)` 再往上爬父组件；`component_options` 进 `UnitFacts` |
| P3 布局事实 | **部分** | `LayoutFacts` 已有 `display` / `flex_direction` / `out_of_flow` / `width_follows_content` / `block_rendered` / `definite_height`，以及截断三件套（`white_space` / `clips_overflow` / `webview_truncates`，复用 `webview_truncates`）|
| P4 脚本事实 | **部分** | `handlers(element)`：事件属性 → 事件名 + 静态处理函数名 + 脚本里的位置（含 `worklet` 标记），并顺着方法表跳转：`Component({ methods })` → 属性值 → 同文件函数 / `require`·`import` 的模块导出 / `this.x = m.f().bind(this)` / 链式 API 的 `.methods({..})` 与 `.init(cb)` 返回值。动态绑定标 `dynamic`；静态写了但找不到函数时只给名字、不给位置。内联 `<wxs>` 仍未入脚本图 |
| M1 迁移接口 | **已完成** | `skylint_core::migrate`：`plan()` / `job()` / `facts_doc()` / `element_detail()`，契约见 `docs/MIGRATE.md`；CLI 对应 `skylint plan` / `job` / `facts` |

两处对 `inspect` 的修正（M1 顺带发现）：

- `Facts::element_at(line, column)` 以前按「开始标签正好落在该位置」比对，而 `WxmlElement` 的位置来自编译器、是 0-based 行，和诊断的 1-based 行差一 —— 它当时没有任何调用者，所以没人发现。现在按**区间包含**取最深元素，行列都换算成 0-based 再比。
- `Facts::ancestors()` 对**组件**返回的是**多个使用点的并集**，不是单条父链（`nodes` 把每个使用点的路径依次接在一起）。名字叫 ancestors 容易让人当成链，已写进类型注释；迁移的稳定 id 因此不把祖先链算进去。

判别力：`cascade_tests.rs` 的 6 条用例里 3 条（specificity / important 压 specificity /
样式表 important 压内联）在去掉排序后立即变红，说明它们真的在守新口径。

六个参考工程按 9 字段逐位 diff：**0 增 0 减**——真实工程里没有「同属性跨 specificity
互相覆盖」的写法，所以这次是「把近似换成准确」而不是「修一批误报」。

> 插件视图的 `rendererOptions` 已修：插件页跑在宿主的小程序实例里，`rendererOptions`
> 是宿主 app 级开关，所以 `Loaded::plugin()` 现在把宿主 `app.json` 的 `rendererOptions`
> 带进插件视图（`Project.renderer_options`），三个访问器都读它而不是 `plugin.json`。
> 判别用例 `plugin_view_inherits_host_renderer_options`；六个参考工程诊断 0 增 0 减
> （没有哪条规则的结论正好卡在这个分支上）。

### handler 解析链（2026-11 补）

WXML 上的 `bind:tap="onTap"` 是按**属性键**找处理函数的，所以解析从「单元方法表」出发，
而不是从「函数名」出发：

1. `this.onTap = ...`（运行时覆盖对象方法，优先命）；
2. `Page({..})` / `Component({..})` 顶层与 `methods` 里的同名键；
3. 键的值：函数字面量 → 自身；裸标识符 → 同文件函数或 `require`/`import` 的模块导出；
   `m.fn`（含 `.bind(this)`）→ 模块 `m` 的导出 `fn`；
4. 兜底：同文件有同名函数（`onTap() {}` 简写）。

跨模块只走**单元脚本的 `require` / `import`**，说明符限相对路径与根路径（`./x`、`/x`），
候选后缀按打包器顺序试 `原样 → .js → .ts → /index.js → /index.ts`；裸模块名（`lodash`）
属于 `node_modules`，不进静态检查面。跳数上限 8，`(文件, 名字)` 去重防环。

六工程实测（事件绑定总数 → 能定位到函数体）：businessview 293→289、novel 166→165、
skit_1 167→167、plugin_1 354→329、plugin_2 413→386、skit-player 282→241
（改动前分别是 286 / 165 / 164 / 325 / 382 / 241）。剩下的空档分三类：

- **编译产物里的 async 包装**：`var onConfirm = method(_asyncToGenerator(_regenerator.mark(function _callee(){…})))`
  ——函数体在，但名字绑在 var 上、中间还隔着多层 transpiler 包装，不做模式匹配；
- **真·解不开**：`bind:tap="onReceive"` 而组件只有 `onReceiveFlower`（businessview
  `receive-flower.wxml:40`）、`handleRefund` 全仓无定义——这类正是 facts 应输出的
  「静态写了但找不到」，不是解析能力不足；
- **WXS**：仍未进脚本图（P4 剩余）。

## 结论

解析层（WXML / WXSS / JS）**已经够用**：原始结构、偏移、分支、slot、at-rule、词元都保留了，
不需要为了 facts 去改解析器。缺口全在**求值层**与**可见性**：

| 层 | 现状 | 判断 |
|---|---|---|
| 工程与配置 | `Project` 上有 renderer、`defaultDisplayBlock`、`defaultContentBox`、`scrollViewAutoSize`、libVersion | 够 |
| 单元（页面/组件） | `page_json` / `component_json` + `style_isolation()`；`multipleSlots` / `virtualHost` 只在 `wxml_graph` 内部解析 | **够但需暴露**；`componentGenerics` / `genericsImplementation` 未建模 |
| 文档（WXML） | `WxmlDoc`：`nodes`、`branches`（含 renderer arm）、`template`、`unknown_children`、`is_slot`、`slot_name`、`template_calls` | 够 |
| 元素 | `WxmlElement` + `renderer_arm` / `webview_only` / `skyline_only` | 够 |
| 级联 | `resolved_property` / `matched_declarations` 只按「样式表顺序 → 规则顺序」取最后一条 | **最大缺口**：没有 specificity、没有 `!important`、没有 at-rule 条件、内联与样式表没有统一排序 |
| 选择器匹配 | `Tri`（Yes/No/Unknown）+ 解析缓存 | **有缺口**：不记录「命中哪些规则」「为何 Unknown」 |
| 图关系 | `WxmlGraph::ancestors` / `render_docs`；slot 投影与 `virtualHost` 已建模 | **有缺口**：没有「元素 → 投影到哪个使用点」的回链，且对集成不可见 |
| 布局推导 | `flex_container` / `is_out_of_flow` / `width_follows_content` 在 `rules/wxml`；`definite_height` 在 `scroll_view_height` 内部 | **有缺口**：`definite_height` 这类中间结论不可复用 |
| 脚本 | `Scripts`：文件 → 单元、`calls()`、worklet 作用域 | **有缺口**：handler 名 → 函数体的解析在 `worklet-directive` 里自己实现；内联 `<wxs>` 不入脚本图 |

一句话：**facts 不是「加一个结构体」，而是把现在分散在规则里的求值结果提升成一层可复用的、
带溯源的计算**。核心工作量在级联（specificity / important）与布局推导两处。

## 分层盘点

### 1. 工程与配置

已有的访问器（`project.rs`）：

| 事实 | 位置 |
|---|---|
| 是否有 Skyline 页、页面 opt-out | `pages[].is_skyline` / `opted_out` |
| `defaultDisplayBlock`（app 级） | `app_display_block()` `:389` |
| `defaultContentBox` | `app_content_box()` `:399` |
| `enableScrollViewAutoSize` | `scroll_view_auto_size()` `:409` |
| 页面级覆盖（displayBlock） | `page_display_block()` `:423`（**私有**，facts 需暴露） |
| 组件 `styleIsolation` | `style_isolation(path)` `:236` |
| 环境常量（WXML 条件求值用） | `env(path)` `:213` |
| 路径换算 | `abs()` `:448`、`report_prefix` |

缺口：

- **没有页面级 `defaultContentBox`** 的读取（`app_content_box` 只读 app 级），而 `scroll-view-height`
  的判定要按页算；
- `project.config.json` 的 `setting`（`postcss` / `minifyWXSS`）只有 `autoprefix-build` 在用，没有
  统一成「构建配置事实」。

### 2. 单元（页面 / 组件）

`component_json` / `page_json` 原样保留（`:150-154`），`style_isolation` 已解析。缺口：

- `wxml_graph.rs:100-103` 里的 `ComponentOptions { multiple_slots, virtual_host }` 是私有结构，
  从组件 json 读（`:688-690`）之后不外露——facts 需要它来解释「slot 投影为什么这样走」；
- `componentGenerics` / `genericsImplementation` 完全没建模（`project.rs` 只读
  `componentGenerics.*.default` 参与依赖闭包），而它在插件工程里决定了整棵组件树的渲染位置
  （见 `docs/COVERAGE.md` 的「已知边界」）。

### 3. 文档（WXML）

`WxmlDoc`（`input/wxml.rs:44-62`）保留：`nodes`、`template_refs`、`includes`、`imports`、
`wxs_refs`、`template_calls`、`fragment`。私有的三项（`includes` / `imports` / `wxs_refs`）
如果需要「这个元素来自哪次 include」就得暴露，目前 facts 不需要。

`WxmlElement`（`:72-104`）携带：`tag`、四组偏移、`attributes`、`children`、`branches`、
`template`、`unknown_children`、`is_slot`、`slot_name`。这已经足以解释
「这个元素在哪个渲染器分支下」「它是不是模板体 / slot / 未知区域」。

### 4. 级联（最大缺口）

现有实现（`rules/wxml/mod.rs:284-354`）：

```rust
matched_declarations(project, tree, element, property) -> Vec<(sheet, declaration)>
resolved_property(...) -> Option<Resolved { value, sheet, declaration }>
```

两者都是「遍历所有覆盖该文档的样式表 → 命中该元素的规则 → 该属性声明」，**按遍历顺序取最后一个**，
内联 `style` 单独覆盖一切。代码注释自己写明：

> 不计算 specificity：按样式表顺序与规则顺序取最后一条（`visit` 侧同口径）；跨优先级冲突时静态判不了。

这带来三个具体问题（都已在真实工程里出现过）：

1. `.panel.slide-up { transition: none }` 写在 `.panel { transition: transform }` 之前——按顺序取最后
   一条会得出「有过渡」，引擎按 specificity 选 `none`；
2. `!important` 完全没参与比较（`WxssDeclaration.important` 字段有，但级联不用它）；
3. 命中多条互相矛盾的声明时，facts 只能给出「最后一条」，给不出「还有哪几条、为什么被覆盖」。

要补的：

- `selectors` 已经把选择器解析成 `SelectorList<SelImpl>`（`matcher/mod.rs:310` 有解析缓存），
  `Selector::specificity()` 现成可用 → 加 `Tree::selector_specificity(text) -> Option<u64>`；
- 级联按 `(important, specificity, 样式表顺序, 规则顺序, 声明顺序)` 排序，与
  `style/style_resolver.cc:28-35` 的 `CompareRules` 对齐；
- at-rule 条件：`WxssRule.at_rules` 已有上下文，但级联不看它（例如 `@media` 里的声明现在也参与
  「最后一条」）。facts 至少要标出 `conditions: Vec<AtRule>`；
- 溯源结构：`CascadeEntry { declaration, sheet, specificity, important, matched_selector, at_rules, status: Winner | Overridden | Discarded }`。

⚠️ 这一项会**改变现有诊断**（近似口径变准），必须先冻结基线、逐条解释 diff，再落地。

### 5. 选择器匹配

`Tree::matches(element, selector, dialect) -> Tri`（`:385`）与 `any_match`（`:396`），解析结果有缓存
（`:310`）。缺口：

- 没有「这个元素命中了哪几条规则」的聚合视图；每个规则自己遍历一遍样式表；
- `Unknown` 的原因没有落地（是动态 class、运行时伪类、还是兄弟区含 `wx:if`）——facts 要能回答
  「为什么判不了」，否则 agent 会把它当成「没问题」；
- 两套方言（`Dialect::Skyline` / `Webview`）在调用点各自选，facts 必须记录「这个结论用的是哪套」。

### 6. 图关系

`wxml_graph.rs` 已经能算：运行时父边、`<slot>` 投影、`virtualHost` 抹掉宿主、`render_docs`
（页面 → 实际渲染到的文档）。`WxmlGraph::ancestors()` 返回 `AncestorChain { nodes, open }`，
`open` 表示链尾停在解析不出的组件上（必须放弃判定）。

缺口：

- 没有**回链**：给定一个组件模板里的元素，不知道「它被哪些使用点投影进来」；
- 没有「文档可达性」的公开入口（`element_docs` 里的 `Reach` 是 `pub(crate)`）；
- 这些结构对集成完全不可见。

### 7. 布局推导

分散在三处，而且都是 `pub(crate)`：

| 结论 | 位置 |
|---|---|
| 是否 flex 容器 / 主轴方向 | `flex_container` `:451`，`resolved_property(…, "flex-direction")` |
| 是否脱离文档流 | `is_out_of_flow` `:408` |
| 宽度是否由内容决定 | `width_follows_content` `:422` |
| `scroll-view` 是否有确定高度 | `rules/wxss/scroll_view_height.rs` 的 `definite_height`（私有） |
| 是否按块级渲染（宿主 tag 覆写） | `Project::renders_as_block()` `:356`（已公开） |

其中 `definite_height` 正是迁移作业单最需要的一条（「为什么这个 scroll-view 塌成 0」），
现在无法从外部拿到。

### 8. 脚本

`Project.scripts` 覆盖 `.js` / `.ts` / `.wxs`（WXS 按引入单元挂 scope），`JsDoc::calls()` 给出调用点。
缺口：

- **handler 名 → 函数体**的解析在 `worklet-directive` 内部实现（`named_has_worklet`），facts 需要把它
  提升成公共查询（`wxml` 里的 `bind:x="onFoo"` → `js` 里的 `onFoo` 定义 → 是否 `'worklet'`）；
- 内联 `<wxs module="m">…</wxs>` 没有进入脚本图（`input/wxml.rs` 只收 `GlobalRef`）；
- 没有「这个 WXML 文件用了哪些 API」的聚合（`unsupported-api` 自己扫 callee）。

## 覆盖矩阵（规则族 → 需要的 facts）

| 规则族 | 需要的事实 | 现在够吗 |
|---|---|---|
| 值/属性支持面（`unsupported-value`、`shorthand-partial`、`gradient-*`…） | 声明原文 + 词元 + at-rule 上下文 | 够（`WxssDeclaration.tokens`、`at_rules`） |
| 布局/盒模型（`native-padding`、`scroll-view-*`、`fixed-*`） | 元素级联值 + 祖先链 + 组件档位表 + `definite_height` | 部分：级联是近似、`definite_height` 私有 |
| 选择器/作用域（`dead-selector`、`tag-selector-isolation`…） | 匹配集合 + specificity + 文档作用域 + 图可达 | 缺匹配集合与 specificity |
| 文本/行内（`no-inline-text`、`inline-run`、`text-overflow-*`） | 元素 + 子节点 + 逻辑祖先 + 级联值 + 块级判定 | 缺块级判定（分散在规则里） |
| API/worklet/JS | 调用点 + handler→函数 + 单元渲染器 | 缺 handler 解析与内联 wxs |
| 配置/组件结构 | 单元 json + 页面/插件视图 + 图结构 | 够（`component_json` 原样保留） |
| 迁移相关（z-index、autoprefix、vendor-prefix） | 层叠键（需要祖先层叠上下文）+ 构建配置 | 部分：`fixed-layer` 自己算包含块 |

## facts API 设计

原则：**按需计算 + memo**，不预生成全量的「每个元素 × 全属性」表（60 条规则、上千元素、
每个属性都要遍历样式表，预生成会慢一个数量级）。

```rust
// skylint_core::inspect
pub struct Facts<'a> {
    project: &'a Project,
    doc: &'a WxmlDoc,
    tree: Tree<'a>,
    cache: RefCell<HashMap<(usize, &'static str), Option<Cascade>>>,  // (element ptr, property)
}

impl<'a> Facts<'a> {
    pub fn for_doc(project: &'a Project, path: &str) -> Option<Self>;
    pub fn element(&self, tag_start: u32) -> Option<ElementRef<'a>>;
    pub fn ancestors(&self, element: &WxmlElement) -> AncestorChain;          // 复用 WxmlGraph
    pub fn cascade(&self, element: &WxmlElement, property: &str) -> Option<Cascade>;
    pub fn matched_rules(&self, element: &WxmlElement) -> Vec<MatchedRule>;    // 含 specificity / Tri
    pub fn layout(&self, element: &WxmlElement) -> LayoutFacts;               // flex / out-of-flow / block
    pub fn unit(&self) -> UnitFacts;                                          // renderer / styleIsolation / …
}

pub struct Cascade {
    pub winner: CascadeEntry,          // value + 来源 + specificity + important + at_rules
    pub overridden: Vec<CascadeEntry>, // 按优先级降序
    pub discarded: Vec<CascadeEntry>,  // 被 at-rule 条件 / 非法值丢弃
}

pub struct LayoutFacts {
    pub display: Option<String>,
    pub flex_direction: Option<String>,
    pub out_of_flow: bool,
    pub width_follows_content: bool,
    pub block_rendered: bool,
    pub definite_height: bool,        // 从 scroll_view_height 提升
}
```

字段来源全部复用现有实现，不自造第二套：

| facts 字段 | 来源 |
|---|---|
| `cascade` | `matched_declarations` + 新增 specificity/important 排序 |
| `ancestors` | `WxmlGraph::ancestors`（含 slot / virtualHost） |
| `layout.display/flex_direction/out_of_flow` | `resolved_property` 系列 |
| `layout.definite_height` | 从 `scroll_view_height` 提升（同一份实现，两处调用） |
| `layout.block_rendered` | `Project::renders_as_block()`（`:356`，已公开） |
| `unit` | `Project::{style_isolation, page/component_json, pages}` + `wxml_graph` 的 `ComponentOptions` |

## 性能

- 缓存键 `(元素指针, 属性)`；`Tree` 已经缓存选择器解析（`matcher/mod.rs:310`），所以一次
  `cascade()` 就是「遍历覆盖该文档的样式表 × 命中判断」，与现有单条规则一次查询同量级；
- 同一元素连续查 10 个属性的成本 ≈ 现在 10 条规则各查一次，不会更差；
- 逆序风险：如果 facts 被「对每个元素算全属性」，就是 O(元素 × 属性 × 规则)；文档里明确
  **禁止**这种用法，API 只提供按需查询。

## 分阶段落地

| 阶段 | 内容 | 验收 |
|---|---|---|
| **P0 暴露** | `inspect` 模块 + `Facts`（级联仍为「顺序近似」但标 `approximate: true`）+ 祖先链 + 单元事实 | 用短剧插件的 `comment-dialog` 生成一份事实快照；与人工读代码的结论一致 |
| **P1 级联准确化** | specificity + `!important` + at-rule 条件 + `overridden` 列表 | 六工程逐字段 diff 全部解释；新增/消失的诊断都能归因到「近似 → 准确」 |
| **P2 图回链** | 元素 → 使用点 / slot；暴露 `Reach`；`multipleSlots` / `virtualHost` 进 `UnitFacts` | 短剧插件的 `rank-tag`（宿主 + slot）能完整讲清祖先链 |
| **P3 布局事实** | 提升 `definite_height`；复用 `renders_as_block` / flex axis | `select-drama-dialog` 的 scroll-view 塌陷能用 facts 解释 |
| **P4 脚本事实** | handler → 函数、内联 wxs 入图、单元 API 清单 | `worklet-directive` 的 9 处绑定能通过 facts 判定 |

P0 不改变任何诊断；P1 会改（近似 → 准确），必须单独一个提交 + 冻结基线。

## 风险

1. **语义漂移**：P1 会让一部分「按顺序取最后一条」的结论翻转。必须像之前改 matcher 大小写那样，
   冻结基线二进制、逐条解释 diff，不能只看总数。
2. **API 冻结**：facts 一旦被迁移工具依赖，字段就要当公开契约维护。建议 P0 只放「稳定、低争议」
   的字段（元素、祖先、单元、resolved 值），带 `approximate` 标记的级联先不进契约。
3. **成本失控**：见上一节，禁止全量预计算；压力测试用 businessview（最大工程）。
4. **不要重复实现**：facts 必须复用 `rules/wxml` 的实现。任何「facts 里再写一遍级联」都是
   两块逻辑漂移的开始——这正是 P1 要改 `resolved_property` 本身、而不是在 facts 里另算的原因。
