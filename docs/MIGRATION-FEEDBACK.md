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
