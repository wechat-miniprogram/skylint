# 迁移接口

本文定义迁移工具链的契约：**单元划分**（`plan`）、**作业单**（`job`）、**元素事实**
（`facts`）、**门禁**（`baseline` / `verify`）、**台账**（`ledger` / `record` / `next`）。
AI 判定与编排器不在这里，见 [`HARNESS.md`](HARNESS.md)。

## 0. 命令空间

```
skylint [PATH]                     # lint（默认，不带子命令）
skylint migration <COMMAND>        # 迁移，Agent 调用
```

顶层只留 lint，是为了让 `skylint --help` 一眼看出「这是个 linter，附带一套迁移工具」。
迁移命令的使用者是 agent，名字长短不重要，所以统一收在 `migration` 下。

## 1. 职责与不变量

| 产物 | 生成者 | 消费者 | 回答 |
|---|---|---|---|
| `plan.json` | `skylint migration plan` | 编排器、人 | 有哪些单元、什么顺序、各有多少问题 |
| `job.json` | `skylint migration job` | **agent** | 这个单元改什么、怎么算过 |
| `facts` | `skylint migration facts` | job 生成器、人、规则作者 | 这个元素到底发生了什么 |

**不变量**（后续所有实现都守它）：

> agent 只读 `job.json`；门禁只读 `job.acceptance`；ledger 只读 `unit.id` 与
> `findings[].id`。三者互不知道对方的实现。

这条是「换 agent / 换编排器成本可控」的唯一保证。

## 2. 单元

一个单元 = 一份 WXML 视图（页面或组件）及其同名资源，外加它**实际引用**的
模板与样式：`<import>` / `<include>` 的模板、`@import` 的样式（均含传递闭包）。
同一文件被多个单元引用时同时属于多个单元（诊断在各自的 `job` 里各出现一次）。

| kind | id | path |
|---|---|---|
| `page` | `unit:<wxml 路径去后缀>` | 页面 WXML 的项目相对路径 |
| `component` | 同上 | 组件 WXML 的项目相对路径 |
| `shared` | `unit:shared` | 空串 |

路径一律是**报告路径**（插件视图带 `pluginRoot/` 前缀），与诊断的 `path` 同源，
这样 `findings` 与 `plan` 可以直接对账。

**报告路径不等于文件路径**：宿主视图的 `path` 相对 `miniprogramRoot`（`miniprogram/`），
插件视图又带 `pluginRoot/` 前缀。所以每个单元另给一个 `fsPath`（工程根相对，`/` 分隔），
runner 拿它去做 `git diff` / `checkout`。两者弄混，在「工程根不是 `miniprogramRoot`」的
所有真实工程上都会静默失效（改动看不见、回滚不回滚、越界检查永远通过）。

**`shared` 伪单元**承载所有不属于任何单元文件的诊断（`app.json` / `app.wxss`、
`project.private.config.json`、`utils/*.js` 等全工程性文件）。它的改动会影响多个
单元，因此必须单独审批，不允许塞进任何组件的 job。

### 2.1 第三方文件：`.skylint/config.json`

实测（businessview）：`unit:shared` 98 条诊断里 **90 条来自三份字节完全相同的
`weui.wxss`**——第三方库，本来就不该手改。不处理的话，共享资源看起来「做不完」，
而实际上真正要动的只有 3 份文件 8 条。

```json
{
  "schemaVersion": 1,
  "exclude": ["packages/*/styles/weui.wxss"]
}
```

- 匹配语义：`*` 匹配任意字符（**含 `/`**），`?` 匹配单字符；所以
  `packages/*/styles/weui.wxss` 与 `**/weui.wxss` 等价。
- 排除后 `plan` / `job` / `baseline` / `verify` 全部看不见这些诊断——**所以它必须在
  文件里，不能是命令行开关**：作业单里的验收命令如果和基线用不同的过滤集，门禁
  会立刻失真。
- 排除**不静默**：`Report.excluded` 会给出被跳过的文件与条数，终端输出的末尾也有
  一行 `excluded: 3 份文件（90 条诊断）来自 …`。「零命中」和「被排掉了」必须能分开。

剩下的共享诊断仍然人工做：改完跑
`skylint migration verify <root> --unit unit:shared --baseline .skylint/baseline.json`。

**批次（`batch`）**是依赖深度：`depth(u) = 0`（不引用其他组件）→
`1 + max(depth(被引用组件))`。数字小的先做，页面在最后一层。同层可并行。

## 3. `plan.json`

```json
{
  "schemaVersion": 1,
  "generator": { "name": "skylint", "version": "0.3.0", "commit": null },
  "baseline": { "file": null, "diagnostics": 269 },
  "units": [
    {
      "id": "unit:components/comment-dialog/comment-dialog",
      "path": "components/comment-dialog/comment-dialog.wxml",
      "fsPath": "miniprogram/components/comment-dialog/comment-dialog.wxml",
      "files": [
        "components/comment-dialog/comment-dialog.wxml",
        "components/comment-dialog/comment-dialog.scss",
        "components/comment-dialog/comment-dialog.ts",
        "components/comment-dialog/comment-dialog.json"
      ],
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
  "summary": { "units": 42, "withDiagnostics": 19, "diagnostics": 269 },
  "shared": {
    "diagnostics": 8,
    "files": [
      { "path": "app.json", "count": 5 },
      { "path": "project.private.config.json", "count": 3 }
    ]
  }
}
```

字段来源：

- `kind` / `renderer` / `reach` / `usages` ← `inspect::Facts`（`unit()` 与 `usages()`）
- `diagnostics` ← `Report` 按 `path` 归到单元
- `batch` ← `Project::deps` 的依赖深度
- `ruleFamilies` ← 规则 id 的静态分类（`structure` / `stacking` / `script` / `visual`）

`files` 是诊断归属的唯一依据：除四件套外，还包含该单元 `@import` 的样式与
`<import>` / `<include>` 的模板（含传递闭包）。因此诊断总数可能大于
`summary.diagnostics`——同一文件被多个单元引用时，每个单元各算一份。

`shared` 是 `unit:shared` 的**文件明细**：无归属诊断按文件聚合，条数降序、同数按
路径排序，`diagnostics` 与 `unit:shared` 的 `diagnostics.total` 相等。
**它只含不属于任何单元的文件**（`app.json` / `project.private.config.json` 等）；
被多个单元引用的文件（如 `styles/common.wxss`）不属于这里——它们在各单元
`files` 里，诊断以 `shared: true` 出现在各自作业单（见 §4.5）。

同一份工程跑两次 `plan`，除 `generator.commit` 外字节一致（可提交、可 diff）。

## 4. `job.json`

```json
{
  "schemaVersion": 1,
  "generator": { "name": "skylint", "version": "0.3.0", "commit": null },
  "baseline": { "file": ".skylint/baseline.json", "diagnostics": 269 },
  "unit": {
    "id": "unit:components/comment-dialog/comment-dialog",
    "path": "components/comment-dialog/comment-dialog.wxml",
    "fsPath": "miniprogram/components/comment-dialog/comment-dialog.wxml",
    "files": [
      "components/comment-dialog/comment-dialog.wxml",
      "components/comment-dialog/comment-dialog.scss",
      "components/comment-dialog/comment-dialog.ts",
      "components/comment-dialog/comment-dialog.json"
    ],
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
      "id": "skyline/fixed-z-index-inversion|components/comment-dialog/comment-dialog.wxml|view.write-comment|#0",
      "path": "components/comment-dialog/comment-dialog.wxml",
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
      "counterexamples": ["…"],
      "selector": null,
      "property": null,
      "declarations": [],
      "matches": [],
      "matchCount": 0,
      "shared": false,
      "affectedUnits": 0
    }
  ],
  "sharedFindings": [],
  "acceptance": [
    { "id": "l0-static", "kind": "command",
      "run": "skylint migration verify {root} --unit {unit} --baseline {baseline} --json",
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

**`layout` 里的 `display` / `position` 等是「声明值」**：来自级联里最近一条
生效声明，`null` 表示没有任何声明（不是 `display: none`）。解析后的默认布局
看 `blockRendered`（以及 `defaultDisplayBlock` / `defaultContentBox` 这些单元级
开关），不要把 `display: null` 读成「不显示」。

**不做**「内联全部级联」：短剧插件单个单元就能到几十 MB，agent 读不完，
还会把它当上下文塞进去。

非 WXML 的诊断（`.wxss` / `.js`）没有元素，`anchor` 与 `facts` 为 `null`。
样式诊断的定位信息见 §4.4。

### 4.2 `id` 算法

**WXML 诊断**（不变）：

```
id = "{rule}|{path}|{tag}.{classes 按字典序}|#{occurrence}"
```

`path` 是诊断所在文档的路径（单元 WXML 或它引用的模板）。
`occurrence` 是同文档内、同 `(tag, classes)` 元素中的序号。没有元素时退化为
`{rule}|{path}|{line}:{column}`。

**非 WXML 诊断**（结构稳定，按选择器 / 声明定位）：

```
声明：   "{rule}|{path}|{selector}|{property}|#{occurrence}"
选择器： "{rule}|{path}|{selector}|#{occurrence}"
at-rule："{rule}|{path}|@{at-rule 名} {prelude}|#{occurrence}"
```

- `selector` 是规则里逗号分隔的全部选择器按源码顺序用 `, ` 连接后的原文；
  `property` 小写化。
- `occurrence` 是同文件内、同 `selector` + 同 `property` 的第 n 条声明（从 0 起，
  按源码顺序）。同一选择器里两条 `background` 各有独立身份：删掉其中一条只会
  `fixed` 那一条，另一条照旧匹配。
- 因此改样式上方代码、插空行、整块挪动都不会改变 id；只有选择器 / 属性本身变了
  才算新诊断。

拿不到选择器 / 属性时（`.js` / `.json` 诊断、解析失败的样式、未建模的 at-rule，
如带前缀 `@keyframes`）退回：

```
id = "{rule}|{path}|{line}:{column}"
```

**这类 id 仍会随行号漂移**，改 `.js` / `.json` 或那类样式后建议重建基线。

**祖先链不参与**：`Facts::ancestors` 对组件是**多个使用点的并集**（不是单条链），
放进 id 会让「某个页面多引用了一次这个组件」把诊断变成新的。

### 4.3 `acceptance` 是命令模板

`{root}` / `{unit}` / `{unitPath}` / `{baseline}` / `{job}` 由 runner 替换。
`expect` 必须是机读判据（`exitCode` / `jsonPath` + `equals` / `threshold`），
不接受自然语言。

### 4.4 样式诊断的定位上下文

样式 finding（`.wxss` / `.scss` / `.less`）额外带五个字段，让 agent 不必自己 grep
就能回答「改哪个选择器、同块还有哪些声明、本单元命中谁」：

```json
{
  "id": "skyline/unsupported-property|styles/common.wxss|.mt-pull-left|float|#0",
  "path": "styles/common.wxss",
  "selector": ".mt-pull-left",
  "property": "float",
  "declarations": [
    { "property": "display", "value": "block", "line": 16, "column": 3 },
    { "property": "float", "value": "left", "line": 17, "column": 3 }
  ],
  "matches": [
    { "path": "pages/order/order.wxml", "line": 42, "column": 5, "tag": "view" }
  ],
  "matchCount": 1
}
```

- `selector` 是诊断所在规则的完整选择器原文（逗号分隔时按源码顺序用 `, ` 连接）。
  诊断在 `@media` 等 at-rule 内时，这里给的是**内层选择器**，不是 `@media` 的 prelude。
- `property` 是声明属性名（小写）；选择器级诊断（如 `dead-selector`）没有属性，为 `null`。
- `declarations` 是**整条规则块**的声明，顺序与源码一致。改 `float` 往往要顺手给
  父级加 `flex`，所以必须能看到同块的其他声明。
- `matches` 只扫 **`unit.files` 里的 WXML / 模板**，不跨单元：别的页面即使也有同名
  选择器，也不会出现在这里。最多 5 条，`matchCount` 是截断前的总数。匹配统一走
  `src/matcher`，与规则层同一套选择器判定。
- 拿不到时（`.js` / `.json` 诊断、样式解析失败、at-rule prelude）为 `null` / 空数组；
  **不要**把这些空值读成「没有命中」。

### 4.5 共享文件（`shared`）

一个文件被 ≥2 个单元引用时（典型是 `styles/common.wxss` 被多个页面 `@import`），
它上面的每条 finding 都带：

```json
{ "shared": true, "affectedUnits": 13 }
```

`job.sharedFindings` 再按文件汇总本作业单里的共享改动：

```json
"sharedFindings": [{ "path": "styles/common.wxss", "count": 8 }]
```

**`shared: true` 的 finding 不属于本单元的验收范围。** 本单元的 `l0-static`
只覆盖当前单元；改共享文件会同时影响 `affectedUnits` 个页面，必须单独提交、
单独过门禁：

- 不属于任何单元的文件（`app.json` 等）：`skylint migration verify <root> --unit unit:shared --baseline …`；
- 被多个单元引用的文件（如 `styles/common.wxss`）：不带 `--unit` 跑全工程 verify，
  或对每个受影响单元分别跑。`--unit unit:shared` 只包含前一类文件，不包含被单元
  引用的共享文件。

agent 可以只在报告里说明需要改什么，**不改**共享文件；是否改由人审批。

非共享文件为 `shared: false` / `affectedUnits: 0`（文件只被当前单元引用时也是
`false`，因为它的改动不会溢出到别的单元）。

## 5. `facts`

```
skylint migration facts <PROJECT> <UNIT_WXML>
skylint migration facts <PROJECT> <UNIT_WXML> --element 87:3
skylint migration facts <PROJECT> <UNIT_WXML> --element 87:3 --property z-index
```

- 不带 `--element`：单元事实 + 元素索引（`span` / `tag` / `classes` / `id`）
- 带 `--element`：展开该元素的 `layout` / `cascade` / `ancestors` / `handlers` / `usages`
- 带 `--property`：只输出该属性的级联

`--element` 的 `行:列` 只用于定位：按包含关系取**最深**元素。返回的 `span` 是
那个元素自身的完整范围（可能从更前面的行开始），不是传入位置。

元素索引/详情里的 `id` 是定位串 `element|<doc>|<tag>.<classes>|#<n>`，**不是**
诊断 id——不要拿去和 `baseline` / `finding.id` 对账。

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
| `skylint migration verify`（字段级 diff、基线对账、退出码） | **M2，已完成**（见 §9） |
| `.skylint/ledger.json`、`skylint migration next` | **M3，已完成**（见 §10） |
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

## 9. 门禁（M2）

```
skylint migration baseline <PATH> [--out <FILE>]
skylint migration verify <PATH> --baseline <FILE> [--unit <ID>] [--json]
```

`--out` 默认是 `<PATH>/.skylint/baseline.json`（落在工程根下，不落在当前目录）。

`baseline` 存的是**身份**（`id` / `unit` / `rule` / `severity` / `path` / `span`），
以及 id 格式标记 `idFormat`（新基线为 `"structural"`；旧基线没有这个字段）。
另外存一份 `message` / `help` 只用于 `verify` 的 `fixed` 条目展示——身份判定
仍只看 `id`，措辞变化不影响结论。

### 9.1 判据

| 情况 | 计入 | `findings[].kind` | 拦门禁 |
|---|---|---|---|
| 当前有、基线没有，且 severity 是 error / warning | `new` | `new` | **是** |
| 当前有、基线没有，且 severity 是 info | `newInfo` | `new-info` | 否 |
| 基线有、当前没有 | `fixed` | `fixed` | 否 |
| 同一 `id`，变严（info→warning/error、warning→error） | `changed` / `changedUp` | `severity-raised` | 否 |
| 同一 `id`，变松（error→warning、warning/error→info） | `changed` / `changedDown` | `severity-lowered` | 否 |

`changed == changedUp + changedDown`。`kind` 是逐条的权威判据：**`new` 只有一种
含义——新增且拦门禁**；info 级新增一律是 `new-info`，调用方不能拿 `kind == "new"`
当「有新诊断」用。判门禁请看 `new` 计数（或退出码），不要数 `findings`。

退出码：`new > 0` → 1，否则 0。`job.acceptance` 的 `l0-static` 就是判 `$.new == 0`。
调严的 `severity-raised` 不拦门禁（它不是 agent 这次改出来的），但会在人读输出里用
`↑` 标出，`changedUp` 计数供人工/上层策略判断。

### 9.2 为什么按 id 不按行列

行列一变，按位置 diff 会把「元素整体下移一行」算成「修了一条、新增一条」。
`tests/migrate.rs::verify_ignores_line_shift` 守着这条：插两行空行后
`new == 0 && fixed == 0`。

非 WXML 诊断（`.wxss`）用 §4.2 的结构身份（选择器 + 属性），在文件上方插空行、
整块挪动同样不会误报；`tests/migrate.rs::wxss_line_shift_keeps_identity` 守着这条。
只有拿不到结构的 `.js` / `.json` / 未建模 at-rule 才退回 `rule|path|line:column`，
那类仍会随行号漂移。

### 9.3 版本不一致

基线里带 `generator.version`。与当前 skylint 版本不同时，`versionMismatch: true`
并往 stderr 打一行警告：规则集变了，新增/修复的数量要人看一眼，而不是当成 agent 改坏了。

### 9.4 已知边界

- 规则措辞改变不算变化（baseline 里的 `message` / `help` 只用于展示，不参与身份），
  但**规则 id 改名**会让旧 id 全部变成 `fixed`、新 id 全部变成 `new`。改名需要
  同时重生基线。
- 元素被删掉时，它的旧诊断算 `fixed`（而不是「未知」）——这与「修好了」在数据上
  不可区分，需要人看一眼 diff。

### 9.5 旧基线（`idFormat: "legacy"`）

旧版 `baseline.json` 没有 `idFormat` 字段，非 WXML id 是
`rule|path|line:column`。`verify` 不会静默：`--json` 输出 `baselineFormat: "legacy"`，
人读输出也会打一行警告。匹配时按 `(rule, path)` 分组、再按源码顺序对齐，所以
插空行 / 整块挪动不会误报；代价是「删一条 + 加一条」这种同数量变化区分不出来，
`fixed` 的逐条定位也只是近似。

**升级到本版本后必须重建基线**（`skylint migration baseline <PATH>`）：新基线会
写入 `idFormat: "structural"`，才能获得逐条的稳定身份。

## 10. 台账与排队（M3）

```
skylint migration ledger <PATH> [--json] [--prune]
skylint migration record <PATH> --unit <ID> --status <S> [--gate G --reason R] [--note N]
skylint migration next   <PATH> [--json] [--job-out FILE] [--baseline FILE]
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

`ledger` 的文本输出把条数落到文件：

```
共享资源上还有 3 条诊断（`unit:shared`），不自动排队，需单独审批
  文件：app.wxss（2 条）、project.private.config.json（1 条）
  只含不属于任何单元的文件；被多个单元引用的文件（如 styles/common.wxss）按 `shared: true` 记在各单元作业单里，不在其中
```

文件多于 5 个时只列前 5 个，末尾追加 `等 N 个文件`。`--json` 在同一层新增
`shared: { "diagnostics": 3, "files": [{ "path": "…", "count": 2 }] }`，台账原有
字段（`schemaVersion` / `generator` / `units`）不变；`shared` 是派生数据，不写进
`ledger.json`。

**`unit:shared` ≠ 「被多个单元引用的文件」。** `plan` 只为「不属于任何单元」的诊断
建 `unit:shared`；`styles/common.wxss` 这类文件属于引用它的每个单元（诊断在各自
作业单里带 `shared: true`），不在 `unit:shared` 里，也不在 `plan.shared` / ledger 的
文件清单里。它们要按 §4.5 的方式验收（全工程 verify，或逐个受影响单元 verify）。

### 10.4 runner 的一轮

```
skylint migration next --json --job-out .skylint/jobs/<unit>.json   → 取作业单
<agent 读 job.json 改代码>
skylint migration verify --unit <unit> --baseline .skylint/baseline.json
skylint migration record --unit <unit> --status verified            → 或 --status migrated --gate l0-static --reason 'new=2'
<人工接受>
skylint migration record --unit <unit> --status accepted
```

四条命令都是确定性的；`record` 只记数（`failures`）与最后一条失败原因，
重试上限由 runner 自己定。
