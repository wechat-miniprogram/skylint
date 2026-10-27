# 规则一览

> 本文件由 `Registry::rules_document()` 生成，不要手工编辑。
> 改了规则的 id / 级别 / 说明后，跑 `cargo run --example rules_doc` 重写。

共 53 条：error 15 条、warning 34 条、info 4 条。

方向列：`skyline` = WebView 迁 Skyline 的检查（默认）；`webview` = Skyline 代码在不支持 Skyline 的环境里退回 WebView 渲染时的降级；`both` = 两个方向都跑。

| 规则 | 级别 | 方向 | 说明 |
|---|---|---|---|
| `skyline/no-inline-text` | warning | skyline | Skyline 下文本与元素混排会变成独立 flex item，需用 `<text>` 包裹 |
| `skyline/unsupported-component` | error | skyline | Skyline 下不可用的组件（如 web-view / movable-view / editor） |
| `skyline/scroll-view-type` | warning | skyline | `<scroll-view>` 必须显式指定合法 type，否则性能退化 |
| `skyline/component-attribute` | warning | skyline | Skyline 会静默忽略的组件属性取值与组合（枚举白名单 / 不支持的属性 / 数值范围 / 跨属性条件） |
| `skyline/scroll-view-family` | error | skyline | scroll-view 家族组件的结构约束（挂载位置 / 顺序 / 唯一性 / 必需属性 / 关联容器） |
| `skyline/component-children` | error | skyline | 组件对直接子节点的结构约束（只允许某类子节点 / 必须唯一子节点 / 条件必需的子节点） |
| `skyline/share-element-key` | error | skyline | `<share-element>` 的 key 必填且同一文件内唯一 |
| `skyline/horizontal-scroll-enable-flex` | warning | skyline | 横向滚动的 `<scroll-view>` 需开启 `enable-flex` 以兼容 WebView |
| `skyline/text-hard-break-ellipsis` | info | skyline | 多行省略里含硬换行时，Skyline 默认不给末行省略号 |
| `skyline/inline-run` | info | skyline | WebView 下并排的文本级元素在 Skyline 下会各自换行 |
| `skyline/no-pseudo` | warning | skyline | Skyline 不支持该伪类/伪元素，选择器不会命中 |
| `skyline/box-sizing` | error | skyline | box-sizing 的值在 Skyline 下不受支持，声明会被忽略 |
| `skyline/flex-direction` | warning | skyline | display: flex 未声明 flex-direction，迁移后默认方向会从 row 变为 column |
| `skyline/fixed-layer` | warning | skyline | `position: fixed` 在 Skyline 下被提升成与页面根平级的独立层，没有有效 `z-index` 时先后不可控 |
| `skyline/text-overflow-ellipsis` | warning | skyline | text-overflow: ellipsis 在 Skyline 下只对 `<text>` 生效 |
| `skyline/attribute-selector` | warning | skyline | 属性选择器在 Skyline 下不参与匹配：主体只有属性时整条声明失效，否则命中范围被放宽 |
| `skyline/dead-selector` | warning | both | 样式表里的选择器在它作用到的所有 WXML 里都不命中任何节点，多半是类名 / 标签名写错 |
| `skyline/scroll-view-height` | warning | skyline | `flex: 1` 的 scroll-view 在主轴尺寸不定的列容器里没有高度可撑，Skyline 下会塌成 0 |
| `skyline/implicit-color` | info | skyline | 省略颜色时 Skyline 用黑色而不是元素自身的 color（受实验开关影响） |
| `skyline/tag-selector-isolation` | warning | skyline | 标签 / id 选择器在 Skyline 下不跨组件匹配，WebView 上生效的样式会静默失效 |
| `skyline/unsupported-property` | error | skyline | 声明了 Skyline 完全不支持的属性，该属性不会生效 |
| `skyline/unsupported-value` | error | skyline | 属性在 Skyline 下可用，但该值不支持，声明会被静默忽略 |
| `skyline/value-degraded` | warning | skyline | 属性在 Skyline 下可用，但该值的表现与 WebView 不同（被降级 / 映射 / 无效果） |
| `skyline/shorthand-partial` | warning | skyline | 简写属性在 Skyline 下部分生效：值里属于不受支持子属性的那一段被丢弃 |
| `skyline/transition-property` | warning | skyline | transition 里的属性名不在 Skyline 的可动画属性白名单内，该段过渡不生效 |
| `skyline/gradient-limit` | warning | skyline | 渐变的形状 / 尺寸 / 颜色停止位置超出 Skyline 支持范围 |
| `skyline/legacy-flexbox` | error | skyline | 规则块用了老式 `-webkit-box` 弹性盒，Skyline 不认识这些声明，整块布局会退回默认 display |
| `skyline/media-query` | warning | skyline | 媒体查询条件在 Skyline 下被丢弃，块内声明对全部设备生效 |
| `skyline/multi-value-limit` | warning | skyline | background / mask 的多值数量超出 Skyline 支持范围 |
| `skyline/filter-limit` | warning | skyline | filter / backdrop-filter 使用了不支持的函数或组合 |
| `skyline/text-decoration-line` | warning | skyline | text-decoration-line 只支持单值，多值组合不生效 |
| `skyline/text-decoration-target` | warning | skyline | text-decoration 在 Skyline 下只对 `<text>` / `<span>` / `<input>` / `<navigator>` 生效 |
| `skyline/calc-angle` | warning | skyline | calc() 不支持角度计算，表达式中的角度单位不生效 |
| `skyline/calc-min-max` | warning | skyline | min() / max() / clamp() 在 Skyline 下求值为 0，整个表达式失效 |
| `skyline/gradient-unsupported` | warning | skyline | repeating 渐变不被 Skyline 解析，mask 也不支持 conic 渐变 |
| `skyline/unsupported-api` | warning | skyline | Skyline 不支持的 JS API（如 this.animate、插件内的 wx.router.*） |
| `skyline/worklet-api-scope` | error | skyline | scrollViewContext.scrollTo 必须在 worklet 内调用，worklet 内不能调用 wx.* |
| `skyline/worklet-directive` | error | skyline | worklet: 绑定、applyAnimatedStyle 与路由动画处理函数缺少 'worklet' 指令 |
| `skyline/worklet-data-freeze` | error | skyline | worklet 内解构 this.data 会冻结 this.data，导致 setData 失效 |
| `skyline/worklet-compile-setting` | warning | skyline | 工程使用了 worklet，但 project.config.json 没有开启 setting.compileWorklet |
| `skyline/scroll-context-ref` | warning | skyline | 用 scrollViewContext.scrollTo 前没有通过 NodesRef.ref 拿到 scroll-view 引用 |
| `skyline/draggable-sheet-scroll-to` | warning | skyline | DraggableSheetContext.scrollTo 同时传 size 与 pixels 时 pixels 被忽略 |
| `skyline/route-type` | warning | skyline | routeType 必须是 wx.router.addRouteBuilder 注册过的类型或 wx:// 预设 |
| `skyline/config-ignored` | warning | skyline | 配置写在不被读取的位置或类型上，静默无效 |
| `skyline/default-layout-drift` | info | skyline | app.json 未开启 defaultDisplayBlock / defaultContentBox，Skyline 的默认布局与盒模型仍与 WebView 不同 |
| `skyline/lib-version` | error | skyline | 项目配置的 libVersion 低于所用 Skyline 特性的最低要求 |
| `skyline/renderer-options` | warning | skyline | rendererOptions 的写法或类型不会被运行时读取，配置静默失效 |
| `webview/text-truncation` | warning | webview | `<text>` 的 max-lines / overflow 在 WebView 下被忽略，截断与省略失效 |
| `webview/nested-scroll` | warning | webview | 嵌套滚动只在 Skyline 下生效，WebView 下头部不联动、常驻顶部 |
| `webview/worklet-binding` | error | webview | `worklet:` 事件绑定在 WebView 下不触发回调，且不报错 |
| `webview/text-children` | error | webview | `<text>` 里的元素子节点在 WebView 下被丢弃（只保留文本与嵌套 text） |
| `webview/pinned-renderer` | warning | webview | 单元的 renderer 被固定成常量，回退 WebView 后所有按它分叉的地方都判错 |
| `webview/tabbar-renderer` | error | webview | 自定义 tabBar 声明 renderer: "skyline"，退回 WebView 时抛异常 |
