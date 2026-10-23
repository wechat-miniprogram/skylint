# skylint

Skyline 小程序迁移检查工具。

检查 WebView 小程序的代码在迁到 Skyline 渲染后会不会出问题：哪些属性/值不再生效、
哪些默认值变了、哪些组件的父子结构约束被违反。目标是让迁移从「上线后逐个页面肉眼比对」
变成「迁移前一次性拿到问题清单」。

## 状态

正在重写（Rust）。`2.0.0` 是重写后的第一个版本，规则命名与 CLI 参数都与 1.x 不兼容——
1.x 的规则名本身有若干不一致，兼容只会把问题一起继承下来。

进度与设计见 [docs/PLAN.md](docs/PLAN.md)。

当前可用：工程视图（路由 → 实际源文件、组件闭包、Skyline 页面判定、样式 `@import` 闭包、
脚本图）、37 条规则、终端报告（源码片段 + 行列定位）、json 报告、退出码。

## 输出

默认输出按条渲染成源码片段，定位到**行列**（`path:line:col` 在片段头里）：

```
skyline/unsupported-component

  × Skyline 不支持组件 `<navigation-bar>`
   ╭─[pages/index/index.wxml:2:4]
 1 │ <view class="cnt">
 2 │   <navigation-bar class="navigation-bar" bind:back="onBack"></navigation-bar>
   ·    ──────────────
 3 │   <view class="header">
   ╰────
  help: Skyline 只能用自定义导航

0 error, 1 warning, 0 info (14 pages, 13 skyline, 12 components)
```

渲染用 [miette](https://docs.rs/miette) 的图形报告，和 minidev 的错误出口同一套主题：
非 TTY（管道、CI 日志）自动关色。超长行不折行（WXML 常整段写一行，折了反而读不出来）。

- 规则 id 在上方独立一行，可直接喂给 `--ignore`；
- 下划线覆盖出问题的那个 token（标签名、属性名、声明值、文本节点）；
- 末行是计数与工程规模，退出码只由 error 决定。

## 规则

`skylint --rules` 直接打印下表（文档由代码生成，不手写）。

| 规则 | 级别 | 说明 |
|---|---|---|
| `skyline/no-inline-text` | warning | Skyline 下文本与元素混排会变成独立 flex item，需用 `<text>` 包裹 |
| `skyline/unsupported-component` | error | Skyline 下不可用的组件（如 web-view / movable-view / editor） |
| `skyline/scroll-view-type` | warning | `<scroll-view>` 必须显式指定合法 type，否则性能退化 |
| `skyline/component-attribute` | warning | Skyline 会静默忽略的组件属性取值与组合（枚举白名单 / 不支持的属性 / 数值范围 / 跨属性条件） |
| `skyline/scroll-view-family` | error | scroll-view 家族组件的结构约束（挂载位置 / 顺序 / 唯一性 / 必需属性 / 关联容器） |
| `skyline/component-children` | error | 组件对直接子节点的结构约束（只允许某类子节点 / 必须唯一子节点 / 条件必需的子节点） |
| `skyline/share-element-key` | error | <share-element> 的 key 必填且同一文件内唯一 |
| `skyline/horizontal-scroll-enable-flex` | warning | 横向滚动的 `<scroll-view>` 需开启 `enable-flex` 以兼容 WebView |
| `skyline/no-pseudo` | warning | Skyline 不支持该伪类/伪元素，选择器不会命中 |
| `skyline/box-sizing` | error | box-sizing 的值在 Skyline 下不受支持，声明会被忽略 |
| `skyline/flex-direction` | warning | display: flex 未声明 flex-direction，迁移后默认方向会从 row 变为 column |
| `skyline/text-overflow-ellipsis` | warning | text-overflow: ellipsis 在 Skyline 下只对 <text> 生效 |
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
| `skyline/text-decoration-target` | warning | text-decoration 在 Skyline 下只对 <text>/<span>/<input> 生效 |
| `skyline/calc-angle` | warning | calc() 不支持角度计算，表达式中的角度单位不生效 |
| `skyline/mask-image-gradient` | warning | mask-image 不支持渐变（linear / radial / conic / repeating-*），仅 url() 可用 |
| `skyline/unsupported-api` | warning | Skyline 不支持的 JS API / 页面配置（如 this.animate、Page.onPageScroll） |
| `skyline/worklet-api-scope` | error | scrollViewContext.scrollTo 必须在 worklet 内调用，worklet 内不能调用 wx.* |
| `skyline/worklet-directive` | error | worklet: 绑定、applyAnimatedStyle 与路由动画处理函数缺少 'worklet' 指令 |
| `skyline/worklet-data-freeze` | error | worklet 内解构 this.data 会冻结 this.data，导致 setData 失效 |
| `skyline/worklet-es5` | warning | 工程使用了 worklet，但 project.config.json 没有开启 setting.es6 |
| `skyline/scroll-context-ref` | warning | 用 scrollViewContext.scrollTo 前没有通过 NodesRef.ref 拿到 scroll-view 引用 |
| `skyline/draggable-sheet-scroll-to` | warning | DraggableSheetContext.scrollTo 同时传 size 与 pixels 时 pixels 被忽略 |
| `skyline/route-type` | warning | routeType 必须是 wx.router.addRouteBuilder 注册过的类型或 wx:// 预设 |
| `skyline/config-ignored` | warning | 配置写在不被读取的位置或类型上，静默无效 |
| `skyline/default-layout-drift` | info | app.json 未开启 defaultDisplayBlock / defaultContentBox，Skyline 的默认布局与盒模型仍与 WebView 不同 |
| `skyline/lib-version` | error | project.config.json 的 libVersion 低于所用 Skyline 特性的最低要求 |
| `skyline/renderer-options` | warning | rendererOptions 的写法或类型不会被运行时读取，配置静默失效 |

规则都面向**迁移**：不按 `is_skyline` 过滤页面，还在 WebView 上的工程同样会被检查——
那才是目标用户。

例外是**显式 opt-out**：页面 json 自己写了 `"renderer": "webview"` 的，整页跳过 WXML 规则
（典型是含 `<web-view>` 的页面，那是「这页永远不迁」的明确决定，对它报 Skyline 限制是误报）。
app 级 `"renderer": "webview"`、页面没表态的不跳——那只是「还没迁」，正是要检查的对象。
组件的 wxml 没有 renderer，一律检查；wxss 无法按页归属（`@import` / `app.wxss` 共享），同样不跳。

## 预处理器（`.less` / `.scss`）

词法/语法由 [`oxc-css-parser`](https://crates.io/crates/oxc-css-parser) 负责，本工具只做
「把嵌套摊平成规则」，`.wxss` 之外的文件不再被跳过。预处理器差异只体现在摊平：

| 写法 | 处理 |
|---|---|
| `.a { .b { … } }` | 摊平成 `.a .b`（SASS / LESS 的后代语义） |
| `.a { &:hover { … } }`、`.block { &-title { … } }` | `&` 原地替换 → `.a:hover`、`.block-title` |
| `.a { @media … { … } }` | 选择器上下文保留，声明归属 `.a` |
| `@mixin` / `@if` / `@include` / `@each` | 透明块，声明记到当前选择器；顶层 mixin 函数体没有选择器 |
| `//` 行注释 | 仅 `.less` / `.scss` 视为注释（`.wxss` 里不是，一份含 `//` 的 `.wxss` 会整份解析失败并报 `skylint/unparsed-style` 告警） |
| `.mixin(@x) { … }` | 识别为混入定义，不当选择器 |
| `$var` / `@var` / `#{…}` | 原样保留，值类规则看到变量就跳过 |

**不做的**：变量求值、mixin 展开、`@extend`、条件求值、`@function` 返回值。这些不解的后果是
保守的：值里含 `$`/`@`/`#{` 就不判（宁可漏报）；无选择器的函数体只做属性级检查
（属性本身不支持，与调用处无关），pairwise 规则（如 `display: flex` 配 `flex-direction`）跳过。

## 构建

依赖 [minidev](https://github.com/wechat-miniprogram/minidev) 的 `compiler-graph` /
`compiler-diag` / `compiler-common` 三个 crate（工程图与诊断 DTO）。minidev 目前只有内部
remote，因此：

```bash
# 1. 把 minidev clone 到与 skylint 同级，或在下面的 patch 里改成任意路径
#    git clone <minidev> ../minidev

# 2. 生成本地 cargo 配置
cp .cargo/config.toml.example .cargo/config.toml

# 3. 构建
cargo build --release
```

外部用户拿不到 minidev 源码，请安装预编译二进制（npm 分发包，见下）。

## 使用

```bash
# 默认检查当前目录
skylint

# 指定项目根（含 project.config.json）
skylint ../my-miniapp

# 机器可读输出（CI 用）
skylint --format json

# 跳过某些规则（可重复）
skylint --ignore skyline/no-pseudo --ignore skyline/flex-direction

# 列出全部规则
skylint --rules
```

### 抑制单行

注释紧贴在被抑制行的上一行：

```html
<!-- skylint-disable-next-line -->
<view>{{a}}<image/></view>

<!-- 只抑制指定规则 -->
<!-- skylint-disable-next-line skyline/no-pseudo -->
<view class="a b" />
```

WXSS 里用 `/* skylint-disable-next-line */` 同样有效。

退出码：`0` 没有问题，`1` 有 error 级问题，`2` 项目加载失败。

## License

MIT
