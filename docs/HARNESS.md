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
npm i          # workflow express nitro rollup（+ @types/express typescript）
```

`nitro.config.ts` 里 `modules: ["workflow/nitro"]` 是编译 `"use workflow"` /
`"use step"` 的地方。少了它，`start()` 会报
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
| `unit:shared`（共享样式） | 不排队，人工处理 |
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
