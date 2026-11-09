# 接编排器（M5）

`skylint` 只提供确定性命令；编排器负责循环、重试、等待人。本文说明两者怎么接、
边界在哪、以及为什么不是「再写一个 runner」。

代码在 [`harness/`](../harness/)。

## 1. 一图说清

```
┌──────────────────── 编排器（Workflow SDK）────────────────────┐
│  workflow：循环 / 重试 / 等 hook（挂起不占算力）               │
│                                                              │
│  step ─▶ skylint migration next      取下一个单元             │
│  step ─▶ skylint migration job       生成作业单               │
│  step ─▶ git stash create            快照                     │
│  step ─▶ $AGENT_CMD                  改代码（任意 agent）      │
│  step ─▶ git diff --name-only        越界检查                 │
│  step ─▶ skylint migration verify    L0 门禁（确定性）         │
│  step ─▶ $BUILD_CMD                  L1 门禁（prod 构建）      │
│  step ─▶ $JUDGE_CMD                  L4 判定（唯一不确定）      │
│  step ─▶ skylint migration record    写台账                   │
│  hook ◀── POST /api/accept           人工确认（可等几天）      │
└──────────────────────────────────────────────────────────────┘
```

**编排器不碰规则、不碰样式语义、不解析 agent 的自然语言。** 它只认退出码和 JSON。

## 2. 为什么不是自己写 runner

重试、断点续跑、并发、等待外部信号这四件事的长尾 bug 极多，而且和「迁移」无关。
自研 runner 等于把 600–900 行调度逻辑塞进一个迁移工具里，用户还得为此买单。

选 Workflow SDK 的三个具体理由：

| 理由 | 证据 |
|---|---|
| 本地零服务 | Local World 把状态写在 `.workflow-data/`，`npm run dev` 就能跑，不需要起数据库 |
| 挂起不占算力 | 等人工确认用 `createHook()` + `await hook`，可以等几天，进程关掉也不丢 |
| 重试语义分三档 | 默认重试 / `FatalError` 不重试 / 可配 `maxRetries`，正好对上「门禁失败不该重试」 |

> **修正**：我早先说「`workflow build` + 30 行 server」即可脱离框架。v5 改掉了打包
> 产物（`flow.js`→`flow.mjs`、`webhook.mjs`、`step.js` 变成内部模块，且 step 投递
> 也由 flow handler 承接），手接这条路的成本与脆弱性都上升。**用官方集成**
> （`workflow/nitro`）而不是自己拼产物。

## 3. 装

```bash
cd harness
npm i
```

两个版本约束是踩出来的，不是猜的：

| 依赖 | 为什么 |
|---|---|
| `nitro@3.0.261103-rc` | `routes` 这个配置键只在这版及以后存在（3.0.0 只有文件路由）。低版本会 TS 报错、路由也不挂载。 |
| `rolldown` | 官方文档写的是 `rollup`，但 nitro v3 用 rolldown。缺了它会退化成「无 builder」模式，module 加的编译插件**不生效**，`start()` 报 `invalid workflow function`。 |

`nitro.config.ts` 里 `modules: ["workflow/nitro"]` 是编译 `"use workflow"` /
`"use step"` 的地方。少了它，`start()` 同样会报
`'start' received an invalid workflow function`。

## 4. 目录

```
harness/
  nitro.config.ts            workflow/nitro 编译指令
  src/server.ts              POST /api/run、POST /api/accept
  src/flows/migrate.ts       "use workflow" —— 唯一的编排逻辑
  src/steps/skylint.ts       next / job / verify / record
  src/steps/agent.ts         agent / 构建 / git 快照 / 越界检查
  src/steps/judge.ts         L4 判定（输出按 schema 校验）
  src/schemas/judge.json     判定的契约
```

## 5. 与 skylint 的契约

编排器只依赖这五条命令，全部是 `--json` + 退出码：

| 命令 | 用途 | 退出码含义 |
|---|---|---|
| `skylint migration next <root> --json` | 取下一个单元 | 0；`unit: null` 表示做完 |
| `skylint migration job <root> --unit <id> --out <f>` | 生成作业单 | 0 / 1 找不到单元 / 2 环境错 |
| `skylint migration verify <root> --baseline <f> --unit <id> --json` | L0 门禁 | 0 通过 / 1 有新增 / 2 环境错 |
| `skylint migration record <root> --unit <id> --status <s>` | 写台账 | 0 / 1 单元不在计划里 |
| `skylint migration ledger <root> --json` | 读进度 | 0 |

**退出码 2 与 1 必须分开**：1 是「代码改坏了」（该回滚重试），2 是「工具跑不起来」
（该停下来看环境）。混在一起会让 agent 无限重试一个环境问题。

## 6. 失败怎么分

| 情况 | 处理 | 为什么 |
|---|---|---|
| agent 退出码非 0 / 超时 | `revert` + `record migrated` + 下一轮 | 是「这次没成」，不是异常 |
| 改了单元目录之外的文件 | `revert` + `record migrated`（gate=scope） | 越界改动无法归因 |
| L0 有新增诊断 | `revert` + `record migrated`（gate=l0-static） | **不能 throw**：throw 会让引擎重试，重试会换一次结果 |
| L1 构建失败 | 同上（gate=l1-build） | 同上 |
| L4 拒收 | 同上（gate=l4-judge） | 拒收是**数据**，不是异常 |
| L4 返回的不是合法 JSON | throw（引擎重试） | 这是「这一轮白跑了」，重试通常就好 |
| 尝试次数用尽 | `record blocked` | 终态，等人介入 |

一句话：**确定性的失败一律 `return` 数据，只有基础设施故障才 `throw`。**

## 7. 人工确认

```ts
const hook = createHook<{ approved: boolean; comment?: string }>({
  token: `accept:${unit}`,
});
await hook.getConflict();      // 先确保注册已提交
const decision = await hook;   // 挂起；人不点就一直等，不占算力
```

`POST /api/accept {unit, approved}` → `resumeHook('accept:' + unit, {...})`。

打回不杀 run：`record --status migrated` 之后，`next` 会把这个单元重新派出来，
带着上次的失败原因进作业单的 `attempts`。

**为什么 `next` 跳过 `verified`**：已经过门禁、只在等人的单元，如果再派给 agent，
人的等待就变成了重复劳动。要打回必须显式 `record --status migrated`。

## 8. 两层状态

| 层 | 位置 | 回答 | 提交 |
|---|---|---|---|
| journal | `harness/.workflow-data/` | 哪个 step 跑过了（重放用） | 否 |
| 台账 | `<repo>/.skylint/ledger.json` | 哪个单元做到哪一步 | **是** |

台账是进度的权威；journal 只保证「同一步不重跑、结果不重算」。两者不一致时
以台账为准（`record` 幂等）。

这也解决了「AI 判定」的可复现性：判定的结论是 step 结果，重放时复用，
不会重新问一次模型、也不会换一个答案。

## 9. 为什么不并行

- 台账是单个 JSON 文件，并行写会互相覆盖；
- `verify` 比的是整仓诊断，两个 agent 同时改会互相污染；
- 验收是瓶颈，并行省不下墙上时间。

v2 的做法：每个单元一个 `git worktree` + 台账分片，最后合并。现在不做。

## 10. 还没接的

| 项 | 状态 |
|---|---|
| L2 automator 冒烟 | 位置留了（`steps/agent.ts`），没实现 |
| L3 截图对比 | 同上 |
| `unit:shared`（共享样式） | 不排队，人工处理；第三方库先用 `.skylint/config.json` 的 `exclude` 排掉 |
| 并行 | 见 §9 |
| CI 集成 | 直接调 `skylint migration verify` 即可，不需要编排器 |

## 11. 验证清单（接之前先跑这五条）

| # | 验证 | 判据 |
|---|---|---|
| 1 | `npm run dev` 起得来，`POST /api/run` 返回 runId | `.workflow-data/` 出现 run |
| 2 | 一个单元跑完并 `record verified` | `skylint migration ledger` 显示 `verified` |
| 3 | kill 进程后重启，run 能续跑 | 已完成的 step 不重跑（`workflow inspect` 看 attempts） |
| 4 | L4 判定只执行一次 | 重放后结论不变（step 结果被复用） |
| 5 | `POST /api/accept` 能唤醒挂起的 run | run 从 `verified` 走到 `accepted` |

第 3、4 条是这个架构真正的价值：**不确定的判定被固化成确定的数据。**

## 12. 已经跑过的 spike（2026-11-07 / 11-09）

### spike 1 · 假 agent，验管道（11-07）

目标工程：`weapp-skit-player-plugin` 的拷贝（9 个待迁移单元，13 条诊断），
agent 与 judge 都是 shell 脚本（不花钱）。

| # | 验的什么 | 结果 |
|---|---|---|
| 1 | `POST /api/run` 返回 runId | `wrun_…AS4T` |
| 2 | agent 故意引入新错误 → L0 拦住 | `failures: 1, gate: l0-static, 新增 1 条` |
| 3 | 回滚干净 | 回滚后 `git status` 只剩 `.skylint/` |
| 4 | 第二次改对 → 判定 → `verified` | agent 跑 2 次、judge 跑 1 次 |
| 5 | 挂起等人，不占算力 | run 状态 `running` 但无 step 在执行 |
| 6 | **kill 进程重启后 accept 能唤醒** | ledger `verified → accepted` |
| 7 | **重放不重跑 agent/judge** | 计数仍为 2 / 1 |
| 8 | 同工程重复 start | 409，返回已在跑的 runId |

### 两个真 bug（spike 才暴露的）

**① `git clean -fd` 会把 `.skylint/` 删掉。** 台账、基线、作业单都在里面，
而它们恰好是未跟踪文件。回滚一次就丢全部状态。现在改为自己枚举
`git ls-files --others --exclude-standard` 并跳过 `.skylint/`。

**② 单元文件被 `.gitignore` 排除时，整套隔离静默失效。** 短剧插件的单元在
`dist/`（构建产物，被忽略），于是 `git diff` 看不见改动、`git checkout` 不
回滚、越界检查永远通过——而门禁照常给「通过」。现在开工前先
`git ls-files --error-unmatch` 检查，不在索引里就 `FatalError` 直接停：

```
FatalError: dist/plugin/components/actor-dialog-inner/actor-dialog-inner.wxml
不在 git 索引里（被 .gitignore 排除？）
```

### 一个环境坑

目标仓库里有 `mise.toml` 而没 `mise trust` 过时，step 里所有命令都会以
退出码 1 失败（错误信息在 stderr）。表现为「agent 永远返回 1」。
先在该仓库里 `mise trust`，或让 harness 的 `PATH` 不经过 shim。

### spike 2 · 真 agent，验作业单够不够（11-09）

目标工程：`businessview` 的拷贝（269 条诊断），agent = `opencode run --auto
--model codebuddy/claude-sonnet-5.5`，judge 仍是假脚本（把「作业单够不够」和
「判定准不准」分开验）。

**单元 A · `custom-switch`（1 条 transition-property）**

agent 把 `transition: background-color 0.1s, border 0.1s` 改成
`transition: background-color 0.1s`——正确且最小（`border` 本来就不是可动画属性，
WebView 下也是空转）。L0 一次过，`verified / failures: 0`。

**单元 B · `auth-mgr`（scroll-view-height + scroll-view-type）**

这一条才是真考验：两个 finding 落在同一个 `<scroll-view>` 上，且带一个陷阱——
它只有内联 `max-height: 500px`，不产生高度。agent 的处理：

```diff
- <scroll-view ... style="max-height:500px;" scroll-y="{{true}}" class="related-list">
+ <scroll-view type="list" ... scroll-y="{{true}}" class="related-list">
  .related-list {
+   height: 500px;
  }
```

L0：新增 0、修复 2、exit 0。它自己的总结：

> `max-height` 换成固定 `height` 后，列表项很少时弹窗里会留出空白，不再按内容收缩。
> 要保留「内容少就收缩」，得在 `app.json` 里打开 `enableScrollViewAutoSize`。
> 那是全局配置，不在这个单元的范围内，所以我没改。要不要改，由你定。

**这就是 B 档该有的行为**：改对了、说清了行为变化、把超出单元范围的开关
交给人和它自己没做验证——不需要人中途介入追问。作业单里的 `cascade`
（内联 `max-height: 500px`）+ `layout.definiteHeight: false` + help 文字三者合起来
就够它做这个判断。

### spike 2 暴露的三个 bug

**① `plan` 与 `job` 对单元归属的判定不一致（严重）。** `plan` 按单元的真实文件
列表统计诊断，`job` 却按后缀白名单（`wxml|wxss|js|ts|wxs|json`）判——`.scss`
不在名单里，于是 businessview 的 `custom-switch` 在 `plan` 里有 1 条诊断、
`job` 里却是 `findings: []`。**agent 拿到的是空作业单**，只能瞎猜。
现在 `PlanUnit.files` 直接列出单元的真实文件，`owns()` 做精确匹配，
不再维护后缀白名单（这类 bug 与之前 `.less/.scss` 样式规则覆盖率归零是同一类）。

**② 报告路径不是文件路径。** `plan`/`job` 的 `path` 是报告路径（宿主视图相对
`miniprogramRoot`，插件视图带 `pluginRoot/`），runner 却拿它去 `git diff`。
新增 `fsPath`（工程根相对）供 runner 用。

**③ agent CLI 会等 stdin 的 EOF。** `execFile` 默认给子进程一个不关闭的 stdin，
`opencode run` 和 `codex exec` 都会停下来等输入——表现为「agent 卡住不动、CPU 0%」。
现在所有 `sh -c` 都带 `< /dev/null`。

### 接 agent 的两条硬要求

| 要求 | 原因 | 各家写法 |
|---|---|---|
| 必须关掉 stdin | 否则子进程等 EOF 挂住 | 统一用 `sh -c '… < /dev/null'` |
| 必须自动批准工具调用 | 非交互模式下等人批准＝挂住 | opencode `--auto`；codex `--sandbox workspace-write`（approval never）；claude `--permission-mode acceptEdits` |

两个都命不中时的症状一样：进程在跑、CPU 0%、不产生 diff。
