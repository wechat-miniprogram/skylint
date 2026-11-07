# 迁移接口（M1）

本文只定义**单元划分**与三个产物（`plan.json` / `job.json` / `facts`）的契约。
`verify`、`ledger`、AI 判定与编排器不在本文范围内——它们都是这些契约的消费者。

## 1. 职责与不变量

| 产物 | 生成者 | 消费者 | 回答 |
|---|---|---|---|
| `plan.json` | `skylint plan` | 编排器、人 | 有哪些单元、什么顺序、各有多少问题 |
| `job.json` | `skylint job` | **agent** | 这个单元改什么、怎么算过 |
| `facts` | `skylint facts` | job 生成器、人、规则作者 | 这个元素到底发生了什么 |

**不变量**（后续所有实现都守它）：

> agent 只读 `job.json`；门禁只读 `job.acceptance`；ledger 只读 `unit.id` 与
> `findings[].id`。三者互不知道对方的实现。

这条是「换 agent / 换编排器成本可控」的唯一保证。

## 2. 单元

一个单元 = 一份 WXML 视图（页面或组件）及其同名资源。

| kind | id | path |
|---|---|---|
| `page` | `unit:<wxml 路径去后缀>` | 页面 WXML 的项目相对路径 |
| `component` | 同上 | 组件 WXML 的项目相对路径 |
| `shared` | `unit:shared` | 空串 |

路径一律是**报告路径**（插件视图带 `pluginRoot/` 前缀），与诊断的 `path` 同源，
这样 `findings` 与 `plan` 可以直接对账。

**`shared` 伪单元**承载所有不属于任何单元文件的诊断（`app.wxss`、共享的
`weui.wxss`、`utils/*.js` 等）。它的改动会影响多个单元，因此必须单独审批，
不允许塞进任何组件的 job。

**批次（`batch`）**是依赖深度：`depth(u) = 0`（不引用其他组件）→
`1 + max(depth(被引用组件))`。数字小的先做，页面在最后一层。同层可并行。

## 3. `plan.json`

```json
{
  "schemaVersion": 1,
  "generator": { "name": "skylint", "version": "0.3.0", "commit": null },
  "baseline": { "file": null, "sha256": null, "diagnostics": 269 },
  "units": [
    {
      "id": "unit:components/comment-dialog/comment-dialog",
      "path": "components/comment-dialog/comment-dialog.wxml",
      "kind": "component",
      "renderer": "skyline",
      "reach": "skyline",
      "batch": 0,
      "usages": 3,
      "diagnostics": {
        "total": 2, "error": 0, "warning": 2, "info": 0,
        "byRule": { "skyline/fixed-z-index-inversion": 2 }
      },
      "ruleFamilies": ["stacking"]
    }
  ],
  "batches": [["unit:…"], ["unit:…"]],
  "summary": { "units": 42, "withDiagnostics": 19, "diagnostics": 269 }
}
```

字段来源：

- `kind` / `renderer` / `reach` / `usages` ← `inspect::Facts`（`unit()` 与 `usages()`）
- `diagnostics` ← `Report` 按 `path` 归到单元
- `batch` ← `Project::deps` 的依赖深度
- `ruleFamilies` ← 规则 id 的静态分类（`structure` / `stacking` / `script` / `visual`）

同一份工程跑两次 `plan`，除 `generator.commit` 外字节一致（可提交、可 diff）。

## 4. `job.json`

```json
{
  "schemaVersion": 1,
  "generator": { "name": "skylint", "version": "0.3.0", "commit": null },
  "baseline": { "file": ".skylint/baseline.json", "sha256": "…", "diagnostics": 269 },
  "unit": {
    "id": "unit:components/comment-dialog/comment-dialog",
    "path": "components/comment-dialog/comment-dialog.wxml",
    "kind": "component",
    "renderer": "skyline",
    "reach": "skyline",
    "styleIsolation": "isolated",
    "defaultDisplayBlock": true,
    "defaultContentBox": true,
    "scrollViewAutoSize": false
  },
  "findings": [
    {
      "id": "sha256:1f3a…",
      "rule": "skyline/fixed-z-index-inversion",
      "severity": "warning",
      "message": "…",
      "help": "…",
      "span": { "line": 87, "column": 3, "endLine": 87, "endColumn": 41 },
      "anchor": {
        "tag": "view",
        "classes": ["write-comment"],
        "occurrence": 0
      },
      "facts": {
        "layout": { "display": "block", "position": "fixed", "outOfFlow": true,
                    "widthFollowsContent": false, "blockRendered": true,
                    "definiteHeight": false, "whiteSpace": null,
                    "clipsOverflow": false, "webviewTruncates": false,
                    "flexDirection": null },
        "cascade": {
          "z-index": [
            { "value": "4000", "important": false, "specificity": 1024,
              "inline": false, "selector": ".write-comment",
              "stylesheet": "components/comment-dialog/comment-dialog.less",
              "line": 12, "column": 1, "atRules": [] }
          ]
        },
        "ancestors": { "open": false, "nodes": [
          { "doc": "components/comment-dialog/comment-dialog.wxml",
            "tag": "half-screen-dialog", "line": 4, "column": 1, "slot": null }
        ] },
        "handlers": []
      },
      "preconditions": ["…"],
      "counterexamples": ["…"]
    }
  ],
  "acceptance": [
    { "id": "l0-static", "kind": "command",
      "run": "skylint verify --unit {unit} --baseline {baseline} --json",
      "expect": { "exitCode": 0, "jsonPath": "$.new", "equals": 0 } },
    { "id": "l1-build", "kind": "command",
      "run": "minidev build --no-pack",
      "expect": { "exitCode": 0 } }
  ],
  "workarounds": [],
  "attempts": []
}
```

### 4.1 `facts` 只内联必要的属性

`layout` 全量（10 个字段，便宜）。`cascade` 只含该规则声明关心的属性——
声明在 `migrate::guidance`，默认空。没有声明的规则只拿 `layout`。

**不做**「内联全部级联」：短剧插件单个单元就能到几十 MB，agent 读不完，
还会把它当上下文塞进去。

非 WXML 的诊断（`.wxss` / `.js`）没有元素，`anchor` 与 `facts` 为 `null`。

### 4.2 `id` 算法

```
id = "{rule}|{unit.path}|{tag}.{classes 按字典序}|#{occurrence}"
```

`occurrence` 是同文档内、同 `(tag, classes)` 元素中的序号。没有元素时退化为
`{rule}|{path}|{line}:{column}`。

**行列不参与（WXML 诊断）**：元素上移一行不该产生一个「新诊断」。
`span` 只用于展示与编辑器跳转。

**祖先链不参与**：`Facts::ancestors` 对组件是**多个使用点的并集**（不是单条链），
放进 id 会让「某个页面多引用了一次这个组件」把诊断变成新的。

### 4.3 `acceptance` 是命令模板

`{unit}` / `{unitPath}` / `{baseline}` / `{job}` 由 runner 替换。
`expect` 必须是机读判据（`exitCode` / `jsonPath` + `equals` / `threshold`），
不接受自然语言。

## 5. `facts`

```
skylint facts <PROJECT> <UNIT_WXML>
skylint facts <PROJECT> <UNIT_WXML> --element 87:3
skylint facts <PROJECT> <UNIT_WXML> --element 87:3 --property z-index
```

- 不带 `--element`：单元事实 + 元素索引（`span` / `tag` / `classes` / `id`）
- 带 `--element`：展开该元素的 `layout` / `cascade` / `ancestors` / `handlers` / `usages`
- 带 `--property`：只输出该属性的级联

字段名照抄 `inspect` 的公开类型（`LayoutFacts` / `CascadeView` / `AncestorView` /
`HandlerView`），不重命名。

**不提供 `--all`**：需要批量时用 `job`，它对每个 finding 只内联必要属性。

## 6. 验收判据

| # | 判据 |
|---|---|
| 1 | 六个参考工程都能生成 `plan.json`；`batch` 分层正确（叶子在前、页面在最后） |
| 2 | 短剧插件 `comment-dialog` 的 job 里，`z-index` 的 `cascade` 与 `facts --element` 一致 |
| 3 | 同一工程跑两次 `plan`，除 `generator.commit` 外字节一致 |
| 4 | 在元素上方插一行（不改该元素），重新生成 job，该 finding 的 `id` 不变 |
| 5 | job 里每条 `help` 都能直接执行，不含「可能 / 建议确认」这类需要 agent 再判断的措辞 |

第 3、4 条决定这份 schema 能不能用：**可 diff、可提交、可续跑**。

## 7. 不在 M1

| 项 | 归属 |
|---|---|
| `skylint verify`（字段级 diff、基线对账、退出码） | **M2，已完成**（见 §9） |
| `.skylint/ledger.json`、`skylint next` | **M3，已完成**（见 §10） |
| AI 判定（`judge.schema.json`） | M4 |
| 编排器（`vercel/workflow`） | M5 |
| 规则知识（`preconditions` / `counterexamples`）的全量补全 | 随规则演进 |

`guidance` 目前是 `migrate` 下的静态表，按规则 id 查。将来若规则数继续增长，
可以搬进 `RuleMeta`，但**不要**在两边各写一份。

## 8. 落地情况

M1 已实现并验证：

| 验收 | 结果 |
|---|---|
| 六个参考工程生成 `plan.json` | 64 / 27 / 34 / 54 / 56 / 45 个单元；诊断总数 269 / 23 / 10 / 120 / 122 / 13，与冻结基线一致 |
| 批次分层 | `businessview [28,21,15]`、`skit-plugin [31,10,3,1]`：叶子在前，页面在最后一层 |
| 两次 `plan` 字节一致 | 六个工程全部 `cmp` 相同 |
| 元素挪一行 id 不变 | `tests/migrate.rs::finding_id_survives_line_shift`（行号 1→2，id 不变） |
| 级联只含声明属性 | `tests/migrate.rs::cascade_only_carries_declared_properties` |

`baseline.sha256` 在 M1 恒为 `null`——摘要属于 `verify`（M2），这里不假装有。

## 9. 门禁（M2）

```
skylint baseline <PATH> [--out .skylint/baseline.json]
skylint verify <PATH> --baseline <FILE> [--unit <ID>] [--json]
```

`baseline` 存的是**身份**（`id` / `unit` / `rule` / `severity` / `path` / `span`），
不存 `message`：措辞随版本变，但问题没变。

### 9.1 判据

| 情况 | 计入 | 拦门禁 |
|---|---|---|
| 当前有、基线没有，且 severity 是 error / warning | `new` | **是** |
| 当前有、基线没有，且 severity 是 info | `newInfo` | 否 |
| 基线有、当前没有 | `fixed` | 否 |
| 同一 `id`，severity 变了 | `changed` | 否 |

退出码：`new > 0` → 1，否则 0。`job.acceptance` 的 `l0-static` 就是判 `$.new == 0`。

### 9.2 为什么按 id 不按行列

行列一变，按位置 diff 会把「元素整体下移一行」算成「修了一条、新增一条」。
`tests/migrate.rs::verify_ignores_line_shift` 守着这条：插两行空行后
`new == 0 && fixed == 0`。

非 WXML 诊断（`.wxss` / `.js` / `app.json`）没有元素，id 退化为
`rule|path|line:column`——这些位置由声明本身决定，改别处不会漂。

### 9.3 版本不一致

基线里带 `generator.version`。与当前 skylint 版本不同时，`versionMismatch: true`
并往 stderr 打一行警告：规则集变了，新增/修复的数量要人看一眼，而不是当成 agent 改坏了。

### 9.4 已知边界

- 规则措辞改变不算变化（baseline 不存 message），但**规则 id 改名**会让旧 id 全部变成
  `fixed`、新 id 全部变成 `new`。改名需要同时重生基线。
- 元素被删掉时，它的旧诊断算 `fixed`（而不是「未知」）——这与「修好了」在数据上
  不可区分，需要人看一眼 diff。

## 10. 台账与排队（M3）

```
skylint ledger <PATH> [--json] [--prune]
skylint record <PATH> --unit <ID> --status <S> [--gate G --reason R] [--note N]
skylint next   <PATH> [--json] [--job-out FILE] [--baseline FILE]
```

台账固定写在 `<PATH>/.skylint/ledger.json`，**应当提交进仓库**。

### 10.1 状态

| 状态 | 含义 | 终态 |
|---|---|---|
| `pending` | 还没开始 | |
| `planned` | 作业单已生成 | |
| `migrated` | agent 改完，还没过门禁 | |
| `verified` | 静态 + 构建都过了，等人接受 | |
| `accepted` | 接受 | ✓ |
| `blocked` | 尝试次数用尽，需人介入 | ✓ |
| `skipped` | 明确跳过 | ✓ |

不做状态机校验：允许 `pending → accepted` 这种跳步（人工直接判掉）。
**语义由 runner 决定，台账只负责持久化**——否则编排策略就长进 skylint 里了。

### 10.2 不写时间戳

台账是纯状态。何时改的由 git 提交记录回答；写进文件只会让 diff 变噪、
让两次运行的结果不可比。`--prune` 后文件字节不变（可验证）。

### 10.3 排队

`next` 按 `plan.batches` 顺序取第一个「有诊断且未到终态」的单元。

批次本身就是拓扑层（`batch(u) = 1 + max(batch(依赖))`），所以「叶子先做」由
批次顺序保证，不需要单独的依赖门控；同一批内可以并行。

**`unit:shared` 不排队**。它承载共享样式（`app.wxss` / `weui.wxss`）的诊断，
改一处会影响多个单元，必须单独审批。进度里单列 `shared` 条数。

### 10.4 runner 的一轮

```
skylint next --json --job-out .skylint/jobs/<unit>.json   → 取作业单
<agent 读 job.json 改代码>
skylint verify --unit <unit> --baseline .skylint/baseline.json
skylint record --unit <unit> --status verified            → 或 --status migrated --gate l0-static --reason 'new=2'
<人工接受>
skylint record --unit <unit> --status accepted
```

四条命令都是确定性的；`record` 只记数（`failures`）与最后一条失败原因，
重试上限由 runner 自己定。
