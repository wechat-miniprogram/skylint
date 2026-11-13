# 迁移命令输出评估（美团 78 页切片实测）

评估对象：`skylint migration *` 八个子命令，在一次真实试点（`weapp` 离线还原工程
`meituan`，78 个 WebView 页 + 485 个组件）里的实际输出。目的不是打分，而是把
「这一步的输出够不够用、哪里会误导」记下来，等迁移结束再统一优化。

工程规模：564 个单元 / 783 条诊断（e260 w375 i148）/ 1655 份 WXML / 1336 份样式。

判定分三档：`可用` / `可优化` / `需修（会误导或用不了）`。

## baseline — 可用

输出：`wrote …/baseline.json（783 条：e260 w375 i148）`。一句话把落点、条数、
严重度分布都给了，够用。

可优化：`baseline.json` 里没有 `sha256` 字段，但 `verify --json` 的
`baseline.sha256` 恒为 `null`。要么算出来，要么别输出这个键。

**已修**：`baseline` 引用里直接去掉了 `sha256` 键（不再输出恒 `null`）。

## plan — 可用（模板/样式归属缺口已修）

输出结构完整（units / batches / summary），14 层拓扑序在真实工程上是对的，
加渲染树缓存后 17s。

**缺口：`<include>` 的模板与被 `@import` 的样式不归属任何单元。**
`unit.files` 只列四件套，于是这些文件上的 150 条诊断（占 19%）落在伪单元
`unit:shared`，既不排队也不进 `job`。实测分布里就包含试点批次自己的文件：

```
12 movie/tpls/cinema/cinema-item.wxml
 8 movie/styles/common.wxss
 4 movie/tpls/deal.wxss
 4 movie/styles/buy-common.wxss
```

后果：agent 拿到 `job` 时看不到这些文件的诊断，改完页面这些坑还在；`verify`
也只在整工程跑时才暴露。这是「会漏迁移」级别的，不是噪声。

建议：把 `@import` 的样式按 `StyleSheet.scopes` 归到引入它的单元，`<include>`
的模板按真实 include 边归属；同一文件被多个单元引用时允许重复归属（诊断按
文件仍然只有一份，但作业单要能看到）。

**已修**：`unit.files` 现在包含单元实际引用（含传递闭包）的模板与样式。同一份
冻结工程复测：诊断总数 779 不变（逐字段 IDENTICAL），`unit:shared` 从 **150 条
降到 3 条**，只剩 `app.json` / `app.wxss` / `project.private.config.json` 三条
真正全局的命中。试点文件已归到具体页面，例如
`movie/tpls/cinema/cinema-item.wxml`（12 条）→ `cinema/cinema`、`cinema/movie`；
`movie/styles/common.wxss`（8 条）→ 13 个 `@import` 它的页面。

次要：长时间运行没有进度输出（当前 17s 尚可，大工程会更久）。

## job — 可用，有两处语义含糊

人读与 JSON 都清晰：finding 带 span / message / help / preconditions / facts，
agent 上一轮 spike 已证明够用。

含糊点：

1. `layout.display: null` 与 `blockRendered: true` 并列出现。`null` 的真实含义是
   「没有声明」，不是「display 为 none」；agent 很容易读反。
2. `anchor.classes` 为空时 `id` 末尾出现 `view.|#2` 这样的空点，人读时像截断。

**已修**：字段结构不动，`docs/MIGRATE.md` 已写明 `display` 等是声明值、null =
未声明，解析后的布局看 `blockRendered`。另补了 `finding.path`（不用再从 id 里
split 文件名）与 `job.unit.files`（与 `next.unit.files` 同源）。

## facts — 可用，两处小问题

```json
"id": "<element>|movie/pages/goods/buy.wxml|view.|#2",
"span": { "line": 1, "column": 1427, ... }
```

1. `id` 里用字面量 `<element>` 当规则名占位，看起来像模板没渲染完。
2. 传 `--element 1:1433` 返回的是**包含该位置的元素**的 span（1427 起），不是
   传入的位置；调用方若拿它做二次定位会偏移。文档没写这层语义。
3. 同 job 的 `display: null` 问题。

**已修**：`id` 改为 `element|<doc>|<tag>.<classes>|#<n>`，并在 `--help` 与文档
说明它不是诊断 id；`--element` 返回 span 的语义也已写进 `--help` 和
`docs/MIGRATE.md` §5。

## verify — 判定正确，文案已修

`new > 0 → exit 1` 实测成立（把试点页 `navigationStyle` 改回 `default`，
`new: 1 / exit: 1`）。finding 结构够编排器用。

已修：三条配置规则的「构建会直接失败」在 `minidev build` 上不成立。实测
`minidev build --no-pack` 对以下三种情况全部 **exit 0**：

- Skyline 页面缺 `navigationStyle: custom`
- Skyline 页面缺 `componentFramework: glass-easel`
- app.json 缺 `lazyCodeLoading: requiredComponents`

而开发者工具会拒绝启动模拟器（日志原文）：

```
页面 "renderer" 为 "skyline"，页面配置 "navigationStyle" 需设置为 "custom"
"renderer" 设置为 "skyline"，需要同时设置 componentFramework 字段为 "glass-easel"
"renderer" 设置为 "skyline"，需在 app.json 添加 "lazyCodeLoading": "requiredComponents"
```

所以文案改成「开发者工具会拒绝编译（`minidev build` 不校验这条）」。**这同时
说明 L1 构建门禁对页面配置类错误是瞎的**：这类错只能靠 L0（规则）或 L2（模拟器
启动）拦，编排顺序里 L2 不能省。

可优化：`findings[]` 不带 `help`（job 里带），人看 JSON 时要回查规则文档。

**已修**：`new` / `severity-changed` 直接从当前诊断带 `help`；`fixed` 从基线里
带上 `message` / `help`（基线新增这两个展示字段，身份判定仍只看 `id`）。

## ledger / record / next — 可用

`record` 回显状态与失败次数；`ledger` 按 batch 列出单元、pending 条数、进度
分解（accepted / 待接受 / pending / notNeeded / shared）；`next` 的 `progress`
与 `unit` 结构够 runner 用。

可优化：`共享资源上还有 150 条诊断（unit:shared）` 只有数字，没有文件清单，
排查得自己去 `baseline.json` 里捞。建议顺手列出前若干个文件与条数。（修复
模板/样式归属后这个数在美团工程上是 3 条。）

## 跨命令

`next` → `job` → `verify` 三步各自重新加载整个工程（本工程约 5s/次），一个
单元一轮固定 15s 开销。迁移结束后可考虑常驻进程或 `.skylint/cache`，现在不值得做。

## 结论

没有到「不可用」的命令。原先唯一建议在批量迁移前处理的 **plan 的模板/样式归属
缺口**（150 条诊断不排队）已修复：诊断集合不变，`unit:shared` 剩余 3 条真正
全局的文件；其余记下来等迁移结束统一优化。

---

# 第二轮实测（批量迁移 movie 页面时）

这一轮开始真正逐页迁移，暴露的问题分两类：skylint 的、和工具链的。

## 已修

1. **`job.findings[]` 没有 `path`**：只有 `id` 里带路径，agent 必须自己 split。
   已给 finding 增加顶层 `path`（与 id 同源）。
2. **`job.unit` 没有 `files`**：`next` 有、`job` 没有，两边结构不一致。已统一。
3. **非 WXML 诊断 id 是 `rule|path|line:col`**：改共享样式（删两条规则、行号一平移）
   `verify` 就报 `new 4 / fixed 9`，逐条核对全是同一批诊断位移。共享样式恰恰是
   迁移的主要改动面，这条会让 L0 门禁不可用。已改为
   `rule|path|selector|property|#occurrence`，并在实测中确认「顶部插空行 → new 0 /
   fixed 0」。升级后必须重建基线（旧基线不再兼容，`Baseline::load` 直接报错）。
4. **`facts` 的元素 `id` 里带字面量 `<element>`**、`verify` 的 `baseline.sha256`
   恒为 null。已分别改成 `element|...` 与删除该键。

## 已修（第二轮后半）

- **样式类 finding 没有定位事实**：`anchor` / `facts` 为 null，agent 只知道
  「某 wxss 第 N 行有 float」。现已补 `selector` / `property` / `declarations` /
  `matches`（+`matchCount`）：匹配只遍历 `unit.files` 里的 wxml，最多回 5 条。
- **共享文件的 finding 被静默塞进页面作业单**：`movie/styles/common.wxss` 的 6 条
  同时归属 13 个页面，agent 改它会波及 13 页，而作业单验收只覆盖当前单元。
  现在 finding 带 `shared: true` + `affectedUnits`，job 顶层有 `sharedFindings`。
  注意：被多单元引用的文件**不进** `unit:shared`（那里只收「无归属」诊断），
  所以要按全工程 `verify` 或逐受影响单元验证——文档 §4.5 已写明。
- **伪元素参与的行内排版**：新增规则 `skyline/pseudo-inline-layout`（warning）。
  判据经引擎核实：Skyline 只实现 `::before/::after`，`content` 只接受字符串字面量
  （`css/css_properties/content.cc:60-83`），非文本宿主的伪元素只支持 block/flex
  （`shadow/pseudo_node.cc:47-52`），所以宿主里只要有裸文本或默认参与行内流的
  文本级子元素，Skyline 下伪元素必然独占一行（WebView 是 inline）。修法：宿主加
  `display:flex; flex-direction:row`，或换成真实节点包进同一个 `<text>`。
  美团工程实测新增 3 条（`cinema/cinema.wxss:234`、`cinema/movie.wxss:50`、
  `goods/_common.wxss:79`），忽略这条规则后与 776 条基线逐字段一致。

## 更正一处早前的误判

我先前说「裸文本 + 单个 `<text>` 被 `no-inline-text` 豁免、只有 `inline-run` 管
≥2 个文本级元素，中间有缺口」——**这是错的**。`no-inline-text` 从未使用
`INLINE_TEXT_TAGS`（该常量只在 `structure.rs` 的 `component-children` 约束里用），
`movie/tpls/_detail.wxml` 的「客服电话」那行本来就会被它报出（`1:2241`），
历史基线里也一直有这条 finding。真实原因是我在 `verify` 里看到 `fixed 1` 却没去
核对它属于谁。现已补一条精确回归测试锁住该写法，`inline-run` 的判据不动
（否则会与 `no-inline-text` 对同一行双重上报，并改变六工程基线）。

## 工具链（不是 skylint 的问题，但会拖慢迁移）

1. **模拟器偶发崩溃**：`appservice webview exit with reason: crashed (exitCode 11)`。
   同一页第一次 reLaunch 失败、第二次直接成功（`order/order` 实测），cinema 系列
   也出现过同样现象。结论：截图环节必须**重试**；崩两次再判定为问题。崩溃后
   `cli close` 不够，要整机 `quit` + `cli open` + `cli auto` 才能恢复自动化。
2. **页面注册位置写错会让模拟器直接起不来**：把测试页加到 `app.json.pages` 而它
   属于 `movie` 分包时，DevTools 报
   `["pages"][16]: "..." 不应该在 ["subPackages"][2] 中`，且不给出正常页面。
   切片脚本没有这个问题，是手工加页时的坑。
3. **部分页面的基线截图本身是空白**（数据没进来），例如 `movie/pages/goods/buy`、
   `movie/pages/order/order`。这类页面 L3 只能证明「没有变得更糟」，证明不了正确性；
   要真正验收得给 route 参数或补 fixture。

---

# 第三轮实测（movie 批次逐页翻 Skyline）

## 两个只有真跑才暴露的引擎语义

### 1. `position: sticky` 被降级成 relative + 偏移，会盖住内容

`skyline/css/css_properties/position.cc:69-71` 的 `ParseValue` 只认
static/relative/absolute/fixed，`sticky` 落到 `default:` 只 `Diag` 然后返回
`CSSUnsetValue`；`ApplyInitial`（`:39-42`）把 position 初值设成 **Relative**。
于是 `.select-sticky{position:sticky;top:{{selectBarFixedTop}}px}` 变成「相对定位
+ 向下偏移一个 bar 的高度」，相对偏移不占兄弟布局 → 筛选条正好压住列表首行。

实测（`cinema/index`）：`.cinema-container` top=179.5，筛选条视觉占 189–208，
卡片第一行的影院名与 `45元起` 整行被遮住。截图里看起来像「标题丢了」，其实是
被盖住且不可见。

全工程 94 个样式文件用了 `position: sticky`，所以这不是单页问题。Skyline 的替代
是 `sticky-section` / `sticky-header` 组件，且**必须在 `<scroll-view type="custom">`
里**；页面级滚动要整体搬进 scroll-view，`onReachBottom` → `bind:scrolltolower`、
下拉刷新 → `refresher-enabled`。宁可整页改结构，也不要把 sticky 留着当静态块。

### 2. `defaultDisplayBlock` 下 `<text>` 在非 text 父级里是块级

工程开了 `rendererOptions.skyline.defaultDisplayBlock: true`，WebView 里行内的
`<text>` 在 Skyline 中变成块级，于是 `.price` 里并排的三个 `<text>`
（`45` / `元` / `起`）竖排成三行（实测 19.7×48）。修法是把这串 `<text>` 包进
同一个 `<text>` 外层，而不是给它们加 `display:inline`（实测无效）。

## 一处配置决策

翻转 `order/order` 后新增了 `skyline/tag-selector-isolation`：`order.wxss` 的
`text{display:block;line-height:1.5}` 命中了新加的 `nav-bar` 组件里的 `<text>`。
根因是 WebView 下标签选择器无法加前缀（`allowIllegalSelector`），本来就会穿进
组件；Skyline 默认按 scope 匹配不穿。已在 `app.json` 设
`rendererOptions.skyline.tagNameStyleIsolation: "legacy"`，把标签选择器恢复成
全局匹配（与 WebView 一致），全项目 23 条该类诊断随之消失。

## 仍然存在的工具缺陷

**JSON / JS 诊断的 id 还是 `rule|path|line:col`**。改一行 `app.json`（加一个
`rendererOptions` 键）就让已有的 `skyline/default-layout-drift` 从「一进一出」
变成 `new 1 / fixed 1`——和样式诊断修好之前一模一样的漂移。样式侧已经改成
`rule|path|selector|property|#occ`，JSON 侧缺同样的稳定身份（可以按 JSON 指针
路径 + 值指纹生成）。在修好之前，改 `app.json` 的批次要人工核对这 1 条。
# 第四轮实测（组件收尾 + 页面批次）

## 一个会让 DevTools 起不来的 WXML 结构错误

lane 把 `<view wx:if="{{item.money}}" class="red-packet-price">` 改成
`<text class="red-packet-price">` 时把 `wx:if` 丢了，紧邻的 `<text wx:else>` 就此
没有前驱。后果：

| 检查 | 结果 |
| --- | --- |
| `minidev build --no-pack` | **exit 0，68867 entries，不报** |
| DevTools 编译 | `Bad attr wx:else with message: wx:if not found` → appservice 起不来 |
| 运行时表现 | `navigateTo` 不回调、`currentPage` 一直变、截图返回上一页旧帧——看起来像「Skyline 卡死」 |

所以「卡死」这一个症状至少有两种成因：渲染器级（B1/B2/B3）和**我们自己的编译错误**。
后者可以静态查出来，已固化成 `tools/checks/wx-branches.js`（扫每层的 `wx:elif` /
`wx:else` 是否紧跟同级 `wx:if` / `wx:elif`，有问题时退出码非 0）。**改过 WXML 就跑一次**，
比排查「卡死」便宜得多。

## 截图链路：实测时序（推翻上一轮的经验值）

上一轮为了稳，`shot.sh` 里写了「打开工程后等 ≥180s」。实测不需要：

| 步骤 | 实际耗时 |
| --- | --- |
| `cli quit` + `pkill` 清残留 | 3–5s |
| `cli open` | **3s**（返回即表示 IDE server 起来了） |
| `sleep 40` + `cli auto` | 50s |
| `shot-one.js`（内部轮询 readiness + `navigateTo` + 校身份 + 截图） | **45s** |

一页冷启动约 1.5–2 分钟，而等 300s 的写法把单页成本推到 6 分钟以上。`shot-one.js`
本来就会自己轮询 `pageStack` 直到 ready，多等没有收益。

**同一 DevTools 会话里第二次 `navigateTo` 之后截出来的是旧帧**：`currentPage`、
`pageStack` 都正确地变成新页，画面还是上一页。所以**每页必须独立冷启动**，不能
「开一次连续截 5 页」。

## 像素对比必须先位移对齐、再分带看

直接逐像素比在这个工程上没有意义，两个稳定噪声源：

1. 自定义导航让整页上下差 4–8px（`nav-bar` 高度与原生导航不同）；
2. 远程图片（海报、头像）每次跑是不同帧。

做法：在 `dy∈[-14,14]、dx∈[-3,3]` 里找最小差异，再按 100px 分带统计差异落点。
实测两页空内容页（`goods/order-detail`、`goods/buy`）最佳位移 (0,0)、总差异
7.9%，但**分带全部落在 y0–199（导航、状态栏）与 y1200+（Home 条），正文
y200–1199 逐像素 0 差异**——这种结论比「差异 7.9% < 阈值」更有说服力。

## 逐条豁免：agent 判定「不需要处理」的机械表达

`migration waive <PATH> --finding <ID> --reason <TEXT>`（另有 `--undo` / `--list`）。
语义按「宁可漏报，不许假装修过」设计：

| 位置 | 行为 |
| --- | --- |
| `plan` / `job` / `next` | 该诊断直接消失；单元只剩豁免项时 `diagnostics.total` 归零、不再排队 |
| `verify` | 当前侧存在→不算 `new`；基线侧存在→不算 `fixed`；单独计 `waived` |
| 豁免后又真改掉 | 不进 `fixed`（确实没修） |
| 豁免对不上当前诊断 | `verify` 告警，避免豁免腐烂 |

`--reason` 必填：没有理由的豁免等于静默忽略。本项目实际用过的三类口径：
`scroll-view-type` 的 info（规则 help 自己写「兼容传统滚动则省略 type」）、
`app.json` 的 `defaultContentBox`（开启会整体改盒模型，已有页面是按 border-box 修的）、
`app.wxss` 的 `.ellipsis-txt`（应随各页迁移逐处包 `<text max-lines>`，不该改全局）。
