# skylint

Skyline 小程序迁移检查工具。

检查 WebView 小程序的代码在迁到 Skyline 渲染后会不会出问题：哪些属性/值不再生效、
哪些默认值变了、哪些组件的父子结构约束被违反。目标是让迁移从「上线后逐个页面肉眼比对」
变成「迁移前一次性拿到问题清单」。

## 状态

正在重写（Rust）。`2.0.0` 是重写后的第一个版本，规则命名与 CLI 参数都与 1.x 不兼容——
1.x 的规则名本身有若干不一致，兼容只会把问题一起继承下来。

进度与设计见 [docs/PLAN.md](docs/PLAN.md)。

当前可用：工程视图（路由 → 实际源文件、组件闭包、Skyline 页面判定）、16 条规则、
终端报告（源码片段 + 行列定位）、json 报告、退出码。

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
| `skyline/scroll-view-family` | error | scroll-view 家族组件（sticky / list-view / grid-view / nested / draggable-sheet）的挂载位置 |
| `skyline/horizontal-scroll-enable-flex` | warning | 横向滚动的 `<scroll-view>` 需开启 `enable-flex` 以兼容 WebView |
| `skyline/no-pseudo` | warning | Skyline 不支持该伪类/伪元素，选择器不会命中 |
| `skyline/box-sizing` | error | box-sizing 的值在 Skyline 下不受支持，声明会被忽略 |
| `skyline/flex-direction` | warning | display: flex 未声明 flex-direction，迁移后默认方向会从 row 变为 column |
| `skyline/unsupported-property` | error | 声明了 Skyline 完全不支持的属性，该属性不会生效 |
| `skyline/unsupported-value` | error | 属性在 Skyline 下可用，但该值不支持，声明会被静默忽略 |
| `skyline/gradient-limit` | warning | 渐变的形状 / 尺寸 / 颜色停止位置超出 Skyline 支持范围 |
| `skyline/multi-value-limit` | warning | background / mask 的多值数量超出 Skyline 支持范围 |
| `skyline/filter-limit` | warning | filter / backdrop-filter 使用了不支持的函数或组合 |
| `skyline/text-decoration-line` | warning | text-decoration-line 只支持单值，多值组合不生效 |
| `skyline/calc-angle` | warning | calc() 不支持角度计算，表达式中的角度单位不生效 |
| `skyline/mask-image-gradient` | warning | mask-image 不支持 linear-gradient / radial-gradient，仅 url() 可用 |

规则都面向**迁移**：不按 `is_skyline` 过滤页面，还在 WebView 上的工程同样会被检查——
那才是目标用户。

例外是**显式 opt-out**：页面 json 自己写了 `"renderer": "webview"` 的，整页跳过 WXML 规则
（典型是含 `<web-view>` 的页面，那是「这页永远不迁」的明确决定，对它报 Skyline 限制是误报）。
app 级 `"renderer": "webview"`、页面没表态的不跳——那只是「还没迁」，正是要检查的对象。
组件的 wxml 没有 renderer，一律检查；wxss 无法按页归属（`@import` / `app.wxss` 共享），同样不跳。

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
