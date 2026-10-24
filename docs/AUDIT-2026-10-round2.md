# 第二轮规则审计（2026-10-24）

8 条只读 lane，覆盖全部 40 条规则 + 输入层 + 抽象重复。每条结论都在源码里带 `文件:行号`，
关键项已由主会话回源码复核（下文标 ✅ 核）。

基线：四个参考工程 `miniprogram-novel` / `businessview` / `miniprogram-skit_1` /
`weapp-novel-plugin_1`，诊断 `19 / 299-304 / 9 / 4`。

---

## A. 误报（必须先处理）

### A1. `skyline/border-none-width` 整条规则错误 ✅核 — 删除
`style/computed_style.h:1119-1139` 的 `GetBorder*Width()` 在 `BorderStyle::kNone` 时直接返回
`Length::Fixed(0)`，`style/style_builder.cc:819-838` 交给 yoga 的就是这组已归零的值。
该早返回来自 `339d08ce4 fix: border none (#617)`（2022-06-29），任何在用版本都包含。
真实工程 7 条命中（novel 3 / businessview 1 / skit_1 3）全是误报。

### A2. `skyline/tag-selector-isolation` — businessview 8/8 误报 ✅核
`default-box-sizing.scss` 被页面**和组件自己**同时 `@import`（`navigation-bar.scss:1` 等），
样式在组件内部仍然生效，不存在隔离失效。
根因：`tag_selector_isolation.rs:71-124` 的 `foreign` 直接取 `page_components(route)`，从不排除
`sheet.scopes` 里已有的单元。修法：从 `foreign` 剔除本 sheet 的 scope。

### A3. `skyline/horizontal-scroll-enable-flex` — skit_1 2/2 误报 ✅核
命中 `.tab-list-cnt` 没有 `display: flex`（`calendar.less:30-36`）。组件自身的告警条件就是
`getComputedStyle(this.$$).display === 'flex'`（`wxapplib wx-scroll-view.js:544-546`）。
另外 `flag()` 把 `scroll-x="false"` 当假值，而 WXML 里非空字符串＝真
（`shadow/js_value.h:31-44` `GetLooseyBool`）。

### A4. 伪元素被当透明 → `text-overflow` / `text-decoration` 误判
`wxss/selector.rs:34-43` 把 `::before`/`::after` 当透明，于是拿宿主 `<view>` 判「能不能承载文本」。
伪元素本身是文本节点（`shadow/pseudo_node.h:57`、`style/style_builder_scope.cc:25-44`）。
`Match::Unknown` 还会一票否决整个 scope（`text_overflow_ellipsis.rs:73` 同时置 `matched` 与
`hostable`），businessview `request-send-msg` 因此漏报 3 条。

### A5. `skyline/route-type` — `redirectTo` / `reLaunch` 不该查 ✅核
运行时只在 navigateTo 分支消费 `routeType`：
`wxapplib src/sdk/src/appservice/app/onAppRoute.js:501`
`if (res.openType === 'navigateTo' && spaData.routeType)`。
`route_type.rs:29-33` 三个方法一视同仁 → 误报。

### A6. `animation-play-state` 的消息是错的 ✅核
引擎 `css/css_properties/animation_play_state.cc:51-86` 先 `Diag(...)` 但**照样返回有效值**；
`animation/css/css_animations.cc:415-416` 会读 `PlayStateList` 与 `kPaused` 比较。
即静态 `animation-play-state: paused` **生效**，只有运行中切换无效。
`context.Diag` 只有 `cli/wxss.cc` 消费，运行时没有读者 —— Diag ≠ 不支持。

### A7. `calc-angle` / `calc-min-max` 的常量折叠误报
fp 会把同单位/纯角度的 `calc()` 折叠成字面值
（`float-pigment-css/src/parser/property_value/calc.rs:187-225`、`:352-386`），
规则却按「Skyline 不算 calc」报。`min(1rem,2rem)`、`rotate(calc(45deg))` 都是误报。

### A8. `unsupported-api` / `is_plugin` 的工程级判定
`unsupported_api.rs:75-85` 遍历全部 `Project.scripts`，不看页面 `renderer` 与工程是否为 Skyline；
纯 WebView 工程的 `this.animate` 会全量报。`rules/js/mod.rs:36-47` 的 `is_plugin` 按整个工程判，
插件工程的**宿主页**用 `wx.router` 会被误报（限制只针对 `pluginRoot`）。

### A9. `scroll-context-ref` / `draggable-sheet-scroll-to` 的边界过宽
前者按「文件里任意 `.ref`」放行（`scroll_context_ref.rs:36-55`）；后者任意 `.scrollTo` 都查，
且只看 `size`/`pixels` 键存在与否（`draggable_sheet_scroll_to.rs:17,37-45`）。

### A10. `<import>` 文件的顶层节点被当页面查
`card.wxml` 顶层的 `<web-view>` 运行时不会渲染，却被 `unsupported-component` 报出（`c1g`）。
`import` 只暴露 `<template name>` 定义体，`include` 才整份内联。

### A11. `COVERAGE.md` 两处理由与实现矛盾
#128 说「已按页面 opt-out 跳过」，但 `lib_version.rs:87-88` 的 `|| app_json.renderer == "skyline"`
没有跳过；#146 引用的 minidev `SubPackage` 里根本没有 `renderer` 字段。

---

## B. 输入层盲区（系统性漏判，优先级最高）

| # | 缺口 | 证据 | 影响 |
|---|---|---|---|
| B1 | WXML 致命解析错误 → 整份文件静默跳过，无任何诊断 | `rules/wxml/mod.rs:64-67` `load_doc` 用 `.ok()?`；对比 WXSS 有 `skylint/unparsed-style` | 全部 WXML 规则 |
| B2 | WXSS / WXML / JSON 带 UTF-8 BOM → 0 条诊断 | `project/fs.rs:42-44`、`project.rs:436,1018` 未剥 `\u{feff}` | 全部 |
| B3 | `project.config.json` 语法错误被当 `Null` | `project.rs:1017-1021` `unwrap_or(Value::Null)` | `useCompilerPlugins` 丢失 → 整个 `.scss` 不进图 |
| B4 | `@keyframes` 整块被丢 | `input/wxss.rs:499-503` 当透明容器之外处理 | 全部 WXSS 声明规则 |
| B5 | `<include>` 不内联 | `project.rs:869-909`、`input/wxml.rs:363-375` | 结构规则双向错（漏报 + 「位于文件顶层」误报） |
| B6 | 标签名大小写保留 | `input/wxml.rs:316` `to_string()` | 6 条 tag 精确比较的规则（`<Scroll-View>` 等全漏） |
| B7 | 模板文件对 JS 规则不可见 | `rules/js/mod.rs:64-82` `unit_wxml` 不含模板 | `worklet-directive`、`worklet-compile-setting` |
| B8 | `@use` / `@forward` 不进样式图 | `input/wxss.rs:472-478` 只收 `@import` | 全部 WXSS 规则 |
| B9 | 模板名含点（`card.v2.wxml`）解析不到 | `project.rs:926-932` `name.contains('.')` 启发式 | 该模板上的全部规则 |
| B10 | `componentGenerics.*.default` 不在组件闭包 | `project.rs:562-567`、`:634-644` | 全部 WXML 规则 |
| B11 | `styleIsolation` 从不读取 | 全仓零命中；`project.rs:540-556` 丢弃组件 json | `tag-selector-isolation` |
| B12 | `<slot>` 被当普通子节点 | `structure.rs:463-560` | `component-children` / `scroll-view-family` |
| B13 | 单文件 WXML 模式伪造 app.json | `project.rs:448-452` | `default-layout-drift` 噪音 |
| B14 | `/deep/`、`>>>` 让整份 WXSS 解析失败 | `input/wxss.rs:224-255` | 全部 WXSS 规则 |
| B15 | JS callee 丢 TS 包装与别名 | `input/js.rs:445-483` | 6 条 JS 规则 |
| B16 | `disable-next-line` 只严格对下一行 | `suppress.rs:30-33` | 多行元素内部诊断无法抑制 |

---

## C. 规则漏判（表内缺口）

- `box-sizing`：`initial` / `unset` 在 `defaultContentBox` 未开时实为 border-box
  （`box_sizing.cc:35-41`），规则无条件放过。
- `flex-direction`：`initial`/`unset`、`flex-flow: wrap`、分组选择器只判首个、`-webkit-flex`。
- `gradient-limit`：fp 无 shape 默认 `Ellipse`（`gradient.rs:846-849`）而引擎只收 `Circle`
  （`image_property_utils.h:47`）；`mask-image` 整类跳过；linear 的 `rpx`/物理单位/`calc()`。
- `text-decoration-line`：`text-decoration` 简写会展开到 line（`style_property_shorthand.cc:213-223`）。
- `multi-value-limit`：只覆盖 5 个长写，`mask-*`、`background-position-x/y`、`background`/`mask` 简写全漏。
- `transition-property`：漏 `text-decoration-thickness`（`transition_property.cc:232-236`）。
- `unsupported-value`：漏多值形态（`overflow: auto hidden`、`border-style: solid double`）。
- `value-degraded`：`will-change: none` 误报；`animation-fill-mode: none/backwards` 与引擎映射冲突
  （`css_to_style_map.cc:97-115`），真机行为待验。
- `lib-version`：`convertRpxToVw: false` 仍抬 3.3.0（`lib_version.rs:114` 只看键存在）；
  app 级 skyline + 全页 webview 仍报 2.30.4。
- `default-layout-drift`：优先级写反（运行时是 `顶层 || 嵌套`，`runtime/index.ts:141-144`）；
  `as_bool` 只认 JSON 布尔，运行时是 `||` 真值语义。
- `renderer-options` / `config-ignored`：app 顶层 `disableScroll`、页面/组件 json 的
  `convertRpxToVw`、`window.rendererOptions` 等位置漏判。

---

## D. 新增规则候选

1. **`-webkit-line-clamp` / `display: -webkit-box`**：Skyline 完全无效（`css/css_parser_impl.h:47`
   `WxLineClamp → kInvalid`；fp `display` 无 `-webkit-box`）。真实使用 50 处 clamp / 158 处 webkit-box，
   替代是 `<text max-lines>`。**最高价值**。
2. `object-fit` / `object-position`：引擎未实现，图片会拉伸（novel 3 处）。
3. `radial-gradient` 隐式 ellipse、conic 停止点、linear 的 rpx/物理单位。
4. `flex-flow` 不带 `flex-direction`。
5. `text-decoration` 简写里的 line 值。
6. `route-type-ignored`（`redirectTo`/`reLaunch` 里的 `routeType` 不生效）。
7. `worklet-chaining-method`、`template-worklet-binding`、`worklet-derived-updater`。

---

## E. 抽象（4 项值得做）

1. `broken_selectors`：`text_overflow_ellipsis.rs:53-90` 与 `text_decoration_target.rs:65-100`
   是同一段判定，抽到 `wxss/mod.rs`。
2. `find_attr`：`structure.rs:121` 一份被抄到 `share_element_key.rs:30`、
   `horizontal_scroll_enable_flex.rs:67`、`component_attribute.rs:521`，另内联在 `scroll_view_type.rs:38`。
3. 配置诊断构造：`config_ignored.rs:110-129` 与 `renderer_options.rs:341-360` 逐字相同。
4. `at_value` 补 severity 变体，消掉 `box_sizing.rs:45-60`、`text_overflow_ellipsis.rs:92-103` 的手写。

---

## F. 建议执行顺序

- **P0 输入层静默**（B1/B2/B3/B4/B6）：会让任何工程出现「0 条诊断却有问题」的假安全。
- **P1 删误报**（A1/A2/A3/A5/A6/A7/A10）：直接减少真实工程噪音，改动小、可逐条验证。
- **P2 输入层语义**（B5/B7/B8/B9/B10/B11/B12/B13/B14/B15）：涉及解析层，按条独立验证。
- **P3 表内漏判**（C）：逐条改数据/判定，每条配正反例。
- **P4 新增规则**（D）：`-webkit-line-clamp` 先做。
- **P5 抽象**（E）：纯重构，最后做，验收口径是四工程 9 字段 0 diff。

验收口径（每批）：`cargo test` 全绿、`clippy --all-targets` 0 warning、`fmt --check` 干净、
四工程诊断逐字段 diff 且每条增减都能对应到某项改动。

---

## G. 落地记录（2026-10-24，P1/P4 第一批）

已改并提交：

| 项 | 提交 | 依据 |
|---|---|---|
| 删 `skyline/border-none-width` | `68e27d6` | `style/computed_style.h:1119-1139` 在 `BorderStyle::kNone` 时直接返回 0（`339d08ce4` 起） |
| `tag-selector-isolation` 减去 `sheet.scopes` | `68e27d6` | 同 sheet 被页面与组件 `@import` 时样式本来就生效 |
| `route-type` 只认 `navigateTo` | `68e27d6` | `onAppRoute.js:501` 只在 `openType === 'navigateTo'` 分支读 `routeType` |
| calc 只报非常量、`horizontal-scroll-enable-flex` 方向纠正、动态 class 不再否决 | `68e27d6` | 见各规则上注释 |
| `unsupported-component` 认 `usingComponents` 声明 | `cd43aad` | 声明同名标签时用的是自定义组件（businessview 8 条误报） |
| `in_template` 逐关系跳过、`Unique` 只数直接子节点、`Exactly` 算文本、互斥分支文本、span/navigator 承载文本、`nested-scroll-header` 是 sliver 槽、`type=""` 等同缺失 | `cd43aad` | 逐条见 `structure.rs` / `no_inline_text.rs` 注释 |
| `-webkit-line-clamp` / `object-fit` / `object-position` 入不支持表 | `5a2787a` | `css_parser_impl.h:47` kInvalid；`css/`、`style/` 零命中 |
| `mask-*` 与 `background-position-x/y` 多值上限、gradient circle 必填 / 只认 px / 覆盖 mask | `360b470` | `image_property_utils.h` 各分支 |
| `text-decoration` 简写展开到 line、`box-sizing` 全局关键字 | `ff5732d` | `style_property_shorthand.cc:213-223`、`box_sizing.cc:33-41` |
| WXML 解析失败报 `skylint/unparsed-template`、读文件去 BOM | `8e7d323` | `load_doc` 的 `.ok()?` 吞错；BOM 破坏 JSON/CSS 前缀 |

### 明确不改（证据不足或口径未定）

- `<block slot>` 摊平丢 slot（`RequiresChild` 误报）：要保留 slot 就得改摊平模型，影响所有结构规则。
- `<include>` 片段按独立文档检查：内联需要跨文件拼接，与「单文件可静态判定」的口径冲突。
- 大写标签（`<Scroll-View>`）：编译器把它当自定义组件，是否该与内置组件同判未定。
- 根级 / 模板根级裸文本：`walk` 只从元素开始，需要新遍历入口。
- `share-element` 的 `key=""`、模板内缺 key、`wx:for` + 静态 key：后者拿不到迭代次数。
- `styleIsolation`（shared / apply-shared / isolated）：加载器没保留组件 json 内容，要先把 `styleIsolation` 带进 `Project`。
- `<slot>` fallback 内容、`@use` / `@forward` 样式图、`componentGenerics.default` 闭包：需先确认 minidev / devtool 口径。
- `flex-flow` 不带 `flex-direction`、`flex-direction` 的 `initial`/分组选择器：反事实工程才有命中，留待下一批。

### 四工程当前基线

novel 6 / 16、businessview 44 / 247、skit_1 10 / 4、plugin_1 2 / 2（error / warning）。
相较本轮起点（16 / 304 / 10 / 4 全为 warning）：`border-none-width` 7 条误报消失、
businessview 8 条 `unsupported-component` 误报消失、skit_1 与 businessview 的
`-webkit-line-clamp` 与 novel 的 `object-fit` 作为真实漏报补出（共 22 条 error）。
