# skylint-taro

Taro 适配器：把 Taro 工程还原成一份「像原生小程序那样」的结构，再交给
skylint 现有规则检查。

## 为什么需要它

Taro（React）在**默认模式**下不生成静态 WXML。页面的 `.wxml` 只有两行：

```xml
<import src="../../base.wxml"/>
<template is="taro_tmpl" data="{{root:root}}"/>
```

真正的结构在运行期由 React 渲染器组装后 `setData({root})` 展开，`base.wxml`
是 100 多个与页面无关的递归模板。直接拿产物跑 skylint 会得到「0 error
0 warning」——不是没问题，是**一条结构都没读到**。

所以这里换个入口：读 `src` 里的 TSX，用 oxc 解析出元素树，把内联组件展开，
在临时目录合成一份 WXML/WXSS/JSON 齐全的工程，交给
`Registry::builtin().run`，最后把诊断的行列映射回 `src` 原始文件。

产物里的 `wxss` 与配置文件是构建期就确定了的，直接复用。

## 用法

```bash
# 先构建，适配器读的是产物目录（project.config.json 的 miniprogramRoot）
taro build --type weapp

cargo run --release -- <项目根>
cargo run --release -- <项目根> --json
```

| 参数 | 说明 |
| --- | --- |
| `PATH` | Taro 工程根（含 `project.config.json`），默认当前目录 |
| `--src DIR` | 源码根，默认自动识别 `src/` |
| `--dist DIR` | 产物目录，默认按 `project.config.json` / `dist` 自动识别 |
| `--json` | 输出 JSON |
| `--target skyline\|webview` | 检查方向，默认 `skyline` |
| `--ignore RULE` | 跳过某条规则（可重复） |
| `--dead-selector` | 一并报告死选择器（默认跳过，见下） |
| `--keep` | 保留合成工程目录，便于排查还原结果 |

退出码：`0` 无 error，`1` 有 error，`2` 用法错误（没找到产物 / 一个单元都没还原出来）。

## 还原语义

1. **内置组件** → 对应标签，并补上 Taro 运行时的默认属性
   （`scroll-view` 的 `type="list"`、`enable-flex=false` 等）；
   属性名按 Taro 的映射改写（`bindlongtap` → `bindlongpress` 等）。
   事件属性按 Taro 的规则改写成 `bind<event>`（`onClick` → `bindtap`、
   `onTouchMove` → `bindtouchmove`），`onXxxWorklet` 落到 `worklet:` 命名空间，
   `catchMove` 落成 `catchtouchmove`；值统一写运行时的处理函数名 `eh`。
2. **`usingComponents` 里声明的组件** → 保留为组件标签。结构在它自己的
   WXML 里，边界如实存在。
3. **其余本地组件**（`import` 进来的、同文件定义的）→ 读它的返回 JSX
   **内联**进调用点，`{props.children}` / `<Slot>` 换成调用处的子节点。
   barrel（`export * from './x'`、`export { default as X } from './x'`）会顺着找。
4. **静态可证的范围**：字面量、三元 / `&&` / `||` 的**每个分支**、`.map`
   回调体。分支落成 `wx:if` / `wx:else`、循环落成 `wx:for`，与原生 WXML
   同构：两个候选都保留（规则按「可能出现的结构」判定），但标成互斥——
   skylint 靠 `wx:if` 的分支栈判定它们不会同时渲染；条件本身不求值，写 `{{_}}`。
5. **其余一律动态**：写 `{{_}}`，与原生 WXML 的插值形态一致，规则层按
   Unknown 处理（宁可漏报）。
6. **读不到的组件**（node_modules 里的 taro-ui / echarts、`require('./x.svg')`
   生成的组件等）：子节点上提一级，并在 stderr 报一行 notice。丢掉整棵子树会
   让页面结构消失，上提至少保住作者自己写的结构。

诊断落在**真实来源**上：页面里内联进来的组件，报的是组件文件的行列，不是
页面里某个碰巧同偏移的位置。

## 默认跳过的规则

`skyline/dead-selector` 需要完整组件树才能判「这个选择器没命中任何节点」。
第三方组件的 JSX 读不到，它们运行时才出现的类名与标签不在合成树里，于是真
命中会被报成死代码（jiezhang 的 `.setting-chart-page canvas` 是 echarts 画的）。
用 `--dead-selector` 可以打开，代价是自己过滤这类噪声。

## 已知边界

- 只支持 React 版 Taro。`.vue` 单元会给出 notice，该单元的结构类规则不执行。
- **不还原 JS**：合成工程只复制 `json/wxss/wxs`，没有任何脚本单元，因此依赖脚本的
  规则（`skyline/unsupported-api`、`skyline/route-type`、worklet 系列等）不会产生
  诊断。运行时会打一行提示，避免把「0 条」当成「没问题」。
- `compileMode` 产物同样按源码还原（不读 `*-templates.wxml`）。
- 运行期才算出来的结构（`props.children` 的深层传递、`React.cloneElement`、
  高阶组件包出来的组件）静态不可知，按 §5 处理。
- 组件互相引用的环、超过 32 层的嵌套会被截断（避免把栈递归穿）。

## 验证语料

四个真实 Taro 工程（构建产物均已跑通，数字取自当前主干规则集）：

| 工程 | Taro | 页面 | 结构还原 | 诊断（error/warning/info） |
| --- | --- | --- | --- | --- |
| jiezhang | 4.2.1 | 30 | 30 | 43 / 121 / 4 |
| taro-mall | 3.4.14 | 25 | 25 | 93 / 75 / 5 |
| c-shopping | 3.6.25 | 15 | 15 | 10 / 26 / 4 |
| taro-music | 3.6.20 | 14 | 14 | 19 / 66 / 3 |

periodic-table-pro（3.6.23）在早期规则集上跑通过（15 页，89 / 99 / 21），本机
没有它的构建产物，本轮未复跑。规则集会随主干变化，数字过一阵就会漂。

对同一批工程直接跑 skylint（读 `dist`）拿不到任何页面结构：c-shopping 直读 dist
是 28 条、全部落在 json/wxss，wxml 上 0 条；走适配器是 40 条，其中 12 条在
`src/` 的 TSX 上。上面那些结构类诊断就是「换入口之后才看得见的部分」。
