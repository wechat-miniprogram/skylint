# 规则一览

> 本文件由 `Registry::rules_document()` 生成，不要手工编辑。
> 改了规则的 id / 级别 / 说明后，跑 `cargo run --example rules_doc` 重写。

共 65 条：error 19 条、warning 43 条、info 3 条。

方向列：`skyline` = WebView 迁 Skyline 的检查（默认）；`webview` = Skyline 代码在不支持 Skyline 的环境里退回 WebView 渲染时的降级；`both` = 两个方向都跑。

| 规则 | 级别 | 方向 | 说明 |
|---|---|---|---|
| `skyline/nested-text-block` | warning | skyline | 嵌套 `<text>` 上的 `display: block` 只在 WebView 生效：Skyline 的文本内部只走 inline 流 |
| `skyline/text-box-property` | warning | skyline | 顶层 `<text>` / `<span>` 上写了宽高或垂直间距：Skyline 会按块级盒应用，WebView 把这些声明当 inline 忽略 |
| `skyline/image-block-layout` | warning | skyline | `<image>` 在 Skyline 是块级盒、WebView 是 inline-block：靠行内对齐居中、单侧 auto 外边距、`vertical-align`、祖先的 `text-indent` 在 Skyline 下都不再生效 |
| `general/transparent-background` | warning | both | 组件把背景色绑在默认为空的属性上，调用处没传时这里一定透明 |
| `skyline/component-support` | error | skyline | Skyline 下没有可用实现的组件：未注册标签按行内/无模板/块级 flex 三种退化方式渲染，黑名单组件按不可用/绘制丢失/整页 opt-out 分档 |
| `skyline/scroll-gesture-conflict` | warning | skyline | 滚动容器上的滑动会冒泡触发祖先的 touchmove 手势（拖拽收起等） |
| `skyline/component-attribute` | warning | skyline | Skyline 会静默忽略的组件属性取值与组合（枚举白名单 / 不支持的属性 / 数值范围 / 跨属性条件） |
| `skyline/scroll-view-family` | error | skyline | scroll-view 家族组件的结构约束（type 取值 / 挂载位置 / 顺序 / 唯一性 / 必需属性 / 关联容器） |
| `skyline/component-children` | error | skyline | 组件对直接子节点的结构约束（只允许某类子节点 / 必须唯一子节点 / 条件必需的子节点） |
| `skyline/share-element-key` | error | skyline | `<share-element>` 的 key 必填且同一文件内唯一 |
| `skyline/text-hard-break-ellipsis` | info | skyline | 多行省略里含硬换行时，Skyline 默认不给末行省略号 |
| `skyline/inline-run` | warning | skyline | Skyline 没有元素级行内格式化上下文：裸文本与元素混排、非文本元素上的 `display: inline` / `inline-block`、并列的文本级元素都会变成块级堆叠 |
| `skyline/page-scroll-api` | warning | skyline | 页面级滚动在 Skyline 下不存在：页面根默认不建滚动容器、页面级订阅只在 WebView 渲染层，`<page-meta>` 的滚动接线也被注释掉——整页改用 scroll-view |
| `skyline/no-pseudo` | warning | skyline | Skyline 不支持该伪类/伪元素：条件被丢弃（命中范围变大）或整条规则被跳过（声明不生效） |
| `skyline/pseudo-inline-layout` | warning | skyline | 伪元素与文本并排时 Skyline 下会换行：伪元素是块级盒，WebView 下是 inline |
| `skyline/box-sizing` | error | skyline | box-sizing 两端不一致：非法取值/initial 的回落值不同，或未开 defaultContentBox 时默认值不同 |
| `skyline/border-radius-per-side-color` | warning | skyline | 各边颜色不同的 border 与 border-radius 同时出现时，Skyline 忽略圆角 |
| `skyline/flex-direction` | warning | skyline | display: flex 未声明 flex-direction，迁移后默认方向会从 row 变为 column |
| `skyline/fixed-layer` | warning | skyline | `position: fixed` 在 Skyline 下被提到页面根，祖先的包含块不再约束它（WebView 下相对该祖先定位），两端位置不同 |
| `skyline/fixed-offset-missing` | warning | skyline | `position: fixed` 元素没写某轴偏移：Skyline 下按页面根起点定位，WebView 下按它在原容器里的位置 |
| `skyline/fixed-z-index-inversion` | warning | skyline | `position: fixed` 在 Skyline 下跳出祖先的层叠上下文，上下顺序可能与 WebView 相反 |
| `skyline/text-only-property` | warning | skyline | text-overflow / text-decoration* 只在文本节点（`<text>` / `<span>` / `<navigator>`）上生效、且不继承到后代：写在 `<view>` 等容器上会静默失效 |
| `skyline/selector-subset` | error | skyline | 选择器不在 Skyline 的选择器子集里：属性选择器被丢弃（整条失效或放宽命中），`*` 在编译期就被拒绝 |
| `skyline/scroll-view-height` | warning | skyline | scroll-view 没有确定高度，Skyline 下内容不会撑开它，会塌成 0 |
| `skyline/native-layout-ignored` | warning | skyline | 没有 CSS 盒的内置组件上，布局类声明在 Skyline 下不生效 |
| `skyline/native-padding` | warning | skyline | 内置组件上的 CSS padding / border-width / gap 在 Skyline 下不内缩子节点 |
| `skyline/implicit-color` | info | skyline | 省略颜色时 Skyline 用黑色而不是元素自身的 color（受实验开关影响） |
| `skyline/tag-selector-isolation` | warning | skyline | 标签 / id 选择器在 Skyline 下不跨组件匹配，WebView 上生效的样式会静默失效 |
| `skyline/unsupported-property` | error | skyline | 声明了 Skyline 完全不支持的属性，该属性不会生效 |
| `skyline/unsupported-value` | error | skyline | 属性在 Skyline 下可用，但该值不支持：按 unset 回落到初始值或父元素的值，或整条声明被丢弃 |
| `skyline/value-degraded` | warning | skyline | 属性在 Skyline 下可用，但该值的表现与 WebView 不同（被降级 / 映射 / 无效果） |
| `skyline/shorthand-partial` | warning | skyline | 简写属性在 Skyline 下部分生效：值里属于不受支持子属性的那一段被丢弃 |
| `skyline/transition-animation-conflict` | warning | skyline | 同一元素上 transition 与 animation 动同一个属性时，Skyline 的 transition 抢占该属性，animation 的位移不按动画推进（本用例停在过渡起点） |
| `skyline/transition-property` | warning | skyline | transition 里的属性名不受支持：闭集外（如 `visibility`、厂商前缀名）会让整条声明被丢弃；闭集内但引擎没有实现（如 `margin`、`background`）只丢该属性 |
| `skyline/vendor-prefix` | warning | skyline | 带 `-webkit-` 前缀的关键帧块 / 动画属性在 Skyline 下被丢弃，WebView 认得 |
| `skyline/gradient-support` | warning | skyline | 渐变函数的支持矩阵：函数名、属性家族与形状 / 尺寸 / 停止位子文法在 Skyline 下只实现了一部分 |
| `skyline/legacy-flexbox` | error | skyline | 规则块用了老式 `-webkit-box` 弹性盒，Skyline 不认识这些声明，整块布局会退回默认 display |
| `skyline/media-query` | warning | skyline | 媒体查询条件在 Skyline 下被丢弃，块内声明对全部设备生效 |
| `skyline/multi-value-limit` | warning | skyline | background / mask 的多值数量超出 Skyline 支持范围 |
| `skyline/filter-limit` | warning | skyline | filter / backdrop-filter 的多个函数组合在 Skyline 下只应用第一个，后续函数被丢弃 |
| `skyline/text-decoration-line` | warning | skyline | text-decoration-line 只支持单值；多值或特殊线型会让 line 回落到 none |
| `skyline/calc-min-max` | warning | skyline | min() / max() / clamp() 在 Skyline 下求值失败，所在声明整条失效（长度类属性回落 auto / 初始值） |
| `skyline/unsupported-api` | warning | skyline | Skyline 不支持的 JS API（如 this.animate、插件内的 wx.router.*） |
| `skyline/wxs-sync-query` | warning | skyline | WXS 里同步形式的 `getBoundingClientRect()` / `getComputedStyle()` 在 Skyline 下拿不到值 |
| `skyline/worklet-api-scope` | error | skyline | scrollViewContext.scrollTo 必须在 worklet 内调用，worklet 内不能调用 wx.* |
| `skyline/worklet-directive` | error | skyline | worklet: 绑定、applyAnimatedStyle 与路由动画处理函数缺少 'worklet' 指令（回调不会注册） |
| `skyline/worklet-data-freeze` | error | skyline | worklet 内把 this.data 赋给变量（含解构）会冻结 this.data，导致 setData 失效 |
| `skyline/worklet-compile-setting` | warning | skyline | 工程使用了 worklet，但 project.config.json 没有开启 setting.compileWorklet |
| `skyline/scroll-context` | warning | skyline | 调用 ScrollViewContext 前没有拿到 context 或节点引用（刷新缺 `enhanced` / refresher 属性，`scrollTo` 缺 ref） |
| `skyline/draggable-sheet-scroll-to` | warning | skyline | DraggableSheetContext.scrollTo 同时传 size 与 pixels 时 pixels 被忽略 |
| `skyline/route-type` | warning | skyline | routeType 必须是 wx.router.addRouteBuilder 注册过的类型或 wx:// 预设 |
| `skyline/dynamic-style-value` | warning | skyline | JS 字符串里的内联样式用了 Skyline 不支持的写法（前缀渐变、老式弹性盒、不可解析的 display、-webkit-line-clamp、会回落初始值的取值） |
| `skyline/autoprefix-build` | info | skyline | prod 构建的 autoprefixer 会给样式注入 `-webkit-` 前缀，Skyline 不认 |
| `skyline/config-ignored` | warning | skyline | 配置写在不被读取的位置或类型上，静默无效 |
| `skyline/default-layout-drift` | warning | skyline | app.json 未开启 defaultDisplayBlock / defaultContentBox，Skyline 的默认布局与盒模型仍与 WebView 不同 |
| `skyline/lib-version` | error | skyline | 项目配置的 libVersion 低于所用 Skyline 特性的最低要求 |
| `skyline/renderer-options` | warning | skyline | rendererOptions 的写法或类型不会被运行时读取，配置静默失效 |
| `webview/text-truncation` | warning | webview | `<text>` 的 max-lines / overflow 在 WebView 下被忽略，截断与省略失效 |
| `webview/nested-scroll` | warning | webview | 嵌套滚动只在 Skyline 下生效，WebView 下头部不联动、常驻顶部 |
| `webview/worklet-binding` | error | webview | `worklet:` 事件绑定在 WebView 下不触发回调，且不报错 |
| `webview/text-children` | error | webview | `<text>` 里的元素子节点在 WebView 下被丢弃（只保留文本与嵌套 text） |
| `webview/tabbar-renderer` | error | webview | 自定义 tabBar 声明 renderer: "skyline"，退回 WebView 时抛异常 |
| `skyline/lazy-code-loading` | error | skyline | 页面显式声明 Skyline 渲染时，app.json 必须设置 lazyCodeLoading: "requiredComponents" |
| `skyline/component-framework` | error | skyline | Skyline 页面的 componentFramework 解析结果必须是 glass-easel |
| `skyline/navigation-style` | error | skyline | Skyline 页面的 navigationStyle 必须解析为 custom，且不能同时写 navigationBar* 字段 |
