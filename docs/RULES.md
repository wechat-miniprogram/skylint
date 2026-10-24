# 规则一览

> 本文件由 `Registry::rules_document()` 生成，不要手工编辑。
> 改了规则的 id / 级别 / 说明后，跑 `cargo run --example rules_doc` 重写。

共 39 条：error 11 条、warning 27 条、info 1 条。

| 规则 | 级别 | 说明 |
|---|---|---|
| `skyline/no-inline-text` | warning | Skyline 下文本与元素混排会变成独立 flex item，需用 `<text>` 包裹 |
| `skyline/unsupported-component` | error | Skyline 下不可用的组件（如 web-view / movable-view / editor） |
| `skyline/scroll-view-type` | warning | `<scroll-view>` 必须显式指定合法 type，否则性能退化 |
| `skyline/component-attribute` | warning | Skyline 会静默忽略的组件属性取值与组合（枚举白名单 / 不支持的属性 / 数值范围 / 跨属性条件） |
| `skyline/scroll-view-family` | error | scroll-view 家族组件的结构约束（挂载位置 / 顺序 / 唯一性 / 必需属性 / 关联容器） |
| `skyline/component-children` | error | 组件对直接子节点的结构约束（只允许某类子节点 / 必须唯一子节点 / 条件必需的子节点） |
| `skyline/share-element-key` | error | `<share-element>` 的 key 必填且同一文件内唯一 |
| `skyline/horizontal-scroll-enable-flex` | warning | 横向滚动的 `<scroll-view>` 需开启 `enable-flex` 以兼容 WebView |
| `skyline/no-pseudo` | warning | Skyline 不支持该伪类/伪元素，选择器不会命中 |
| `skyline/border-none-width` | warning | border-style 为 none 时 Skyline 仍按 border-width 参与布局，元素比 WebView 多占宽度 |
| `skyline/box-sizing` | error | box-sizing 的值在 Skyline 下不受支持，声明会被忽略 |
| `skyline/flex-direction` | warning | display: flex 未声明 flex-direction，迁移后默认方向会从 row 变为 column |
| `skyline/text-overflow-ellipsis` | warning | text-overflow: ellipsis 在 Skyline 下只对 `<text>` 生效 |
| `skyline/tag-selector-isolation` | warning | 标签 / id 选择器在 Skyline 下不跨组件匹配，WebView 上生效的样式会静默失效 |
| `skyline/unsupported-property` | error | 声明了 Skyline 完全不支持的属性，该属性不会生效 |
| `skyline/unsupported-value` | error | 属性在 Skyline 下可用，但该值不支持，声明会被静默忽略 |
| `skyline/value-degraded` | warning | 属性在 Skyline 下可用，但该值的表现与 WebView 不同（被降级 / 映射 / 无效果） |
| `skyline/shorthand-partial` | warning | 简写属性在 Skyline 下部分生效：值里属于不受支持子属性的那一段被丢弃 |
| `skyline/transition-property` | warning | transition 里的属性名不在 Skyline 的可动画属性白名单内，该段过渡不生效 |
| `skyline/gradient-limit` | warning | 渐变的形状 / 尺寸 / 颜色停止位置超出 Skyline 支持范围 |
| `skyline/multi-value-limit` | warning | background / mask 的多值数量超出 Skyline 支持范围 |
| `skyline/filter-limit` | warning | filter / backdrop-filter 使用了不支持的函数或组合 |
| `skyline/text-decoration-line` | warning | text-decoration-line 只支持单值，多值组合不生效 |
| `skyline/text-decoration-target` | warning | text-decoration 在 Skyline 下只对 `<text>` / `<span>` / `<input>` / `<navigator>` 生效 |
| `skyline/calc-angle` | warning | calc() 不支持角度计算，表达式中的角度单位不生效 |
| `skyline/calc-min-max` | warning | min() / max() / clamp() 在 Skyline 下求值为 0，整个表达式失效 |
| `skyline/gradient-unsupported` | warning | repeating 渐变不被 Skyline 解析，mask 也不支持 conic 渐变 |
| `skyline/unsupported-api` | warning | Skyline 不支持的 JS API（如 this.animate、插件内的 wx.router.*） |
| `skyline/worklet-api-scope` | error | scrollViewContext.scrollTo 必须在 worklet 内调用，worklet 内不能调用 wx.* |
| `skyline/worklet-directive` | error | worklet: 绑定、applyAnimatedStyle 与路由动画处理函数缺少 'worklet' 指令 |
| `skyline/worklet-data-freeze` | error | worklet 内解构 this.data 会冻结 this.data，导致 setData 失效 |
| `skyline/worklet-compile-setting` | warning | 工程使用了 worklet，但 project.config.json 没有开启 setting.compileWorklet |
| `skyline/scroll-context-ref` | warning | 用 scrollViewContext.scrollTo 前没有通过 NodesRef.ref 拿到 scroll-view 引用 |
| `skyline/draggable-sheet-scroll-to` | warning | DraggableSheetContext.scrollTo 同时传 size 与 pixels 时 pixels 被忽略 |
| `skyline/route-type` | warning | routeType 必须是 wx.router.addRouteBuilder 注册过的类型或 wx:// 预设 |
| `skyline/config-ignored` | warning | 配置写在不被读取的位置或类型上，静默无效 |
| `skyline/default-layout-drift` | info | app.json 未开启 defaultDisplayBlock / defaultContentBox，Skyline 的默认布局与盒模型仍与 WebView 不同 |
| `skyline/lib-version` | error | 项目配置的 libVersion 低于所用 Skyline 特性的最低要求 |
| `skyline/renderer-options` | warning | rendererOptions 的写法或类型不会被运行时读取，配置静默失效 |
