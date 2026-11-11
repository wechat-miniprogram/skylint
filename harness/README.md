# skylint harness

把 `skylint` 的确定性命令串成一条可恢复的迁移流水线。编排器是
[Workflow SDK](https://workflow-sdk.dev)（`workflow` 包），不是我们自己写的调度器。

**这不是 skylint 的一部分**：只想要 linter 的人装 `skylint` 就够了，不需要 Node。

## 它做什么

```
next ──▶ job ──▶ agent ──▶ L0 静态 ──▶ L1 构建 ──▶ L4 AI 判定 ──▶ 人工确认 ──▶ accepted
                   │           │            │             │
                   └───────────┴────────────┴─────────────┴──▶ 回滚 + record(migrated) + 重试
```

每一步都是一条 `skylint` 命令或一个进程，没有一步依赖 agent 的内部实现。

## 装

```bash
cd harness
npm install          # 或 pnpm i / bun i
```

注意：`nitro` 锁在 `3.0.261103-rc`（`routes` 配置键只在这版有），并且必须有
`rolldown`（官方文档写的是 `rollup`，但 nitro v3 用 rolldown；缺了它编译插件
不生效，`start()` 会报 `invalid workflow function`）。

## 跑

```bash
# 1) 先在被迁移的仓库里冻结基线（在 skylint 仓库外执行）
cd /path/to/your-miniprogram
skylint migration baseline . --out .skylint/baseline.json

# 2) 起编排器
cd harness
PROJECT_ROOT=/path/to/your-miniprogram npm run dev

# 3) 触发一次（maxUnits: 1 表示做到第一个被接受的单元就停）
curl -X POST localhost:3000/api/run \
  -H 'content-type: application/json' \
  -d '{"root":"/path/to/your-miniprogram","maxUnits":1}'

# 4) 进度 / 运行详情
skylint migration ledger /path/to/your-miniprogram
npx workflow web            # 本地 UI，读 .workflow-data/
npx workflow inspect runs

# 5) 人工确认（run 会挂起等人，进程可以关掉）
curl -X POST localhost:3000/api/accept \
  -H 'content-type: application/json' \
  -d '{"unit":"unit:components/x/x","approved":true}'
```

## 环境变量

| 变量 | 默认 | 说明 |
|---|---|---|
| `SKYLINT_BIN` | `skylint` | 二进制路径；开发时可指向 `../target/debug/skylint` |
| `AGENT_CMD` | `codex exec --sandbox workspace-write ... "{prompt}"` | 改代码的 agent，`{prompt}` 会被替换 |
| `JUDGE_CMD` | `codex exec --sandbox read-only --output-schema ... "{prompt}"` | L4 判定，可换成更便宜的模型 |
| `BUILD_CMD` | `npx minidev build --no-pack` | L1 门禁 |
| `AGENT_TIMEOUT_MS` | `1800000` | 单次 agent 超时 |
| `JUDGE_TIMEOUT_MS` | `600000` | 单次判定超时 |

## 换成别的 agent

只改 `AGENT_CMD`：

```bash
# Claude Code
AGENT_CMD='claude -p --permission-mode acceptEdits "{prompt}"'

# opencode（--auto 必须带，否则非交互模式下等人批准会挂住）
AGENT_CMD='opencode run --auto --model codebuddy/claude-sonnet-5.5 "{prompt}"'

# 任何 CLI
AGENT_CMD='my-agent --task "{prompt}"'
```

硬门槛两条：**必须能非交互执行并用退出码表达成败**；**必须自动批准工具调用**。
只能开 TTY 的 agent 进不来。

stdin 已经统一关掉（`sh -c '… < /dev/null'`）：`codex exec` 与 `opencode run`
都会等 stdin 的 EOF，不关就表现为「进程在跑、CPU 0%、不产生 diff」。

## 两层状态，别混

| 层 | 位置 | 回答 | 该不该提交 |
|---|---|---|---|
| 编排 journal | `harness/.workflow-data/` | 哪个 step 已经跑过（重放用） | 否，gitignore |
| 迁移台账 | `<repo>/.skylint/ledger.json` | 哪个单元做到哪一步 | **是** |

台账是进度的唯一权威；journal 只保证「同一步不重跑、结果不重算」。
`record` 幂等，所以两层不同步时以台账为准重新对齐即可。

## 为什么串行

- 台账是单个 JSON 文件，并行写会互相覆盖；
- `verify` 比的是整个仓库的诊断，两个 agent 同时改会互相污染基线；
- 验收是瓶颈，并行省不下墙上时间。

要并行时的做法是：每个单元一个 `git worktree` + 一份台账分片，最后合并。
这是 v2 的事，不是现在。

## 已知边界

- **`unit:shared` 不排队**：`app.json` / `app.wxss` / `project.private.config.json` 这类
  **无归属**文件的诊断改一处影响多个单元，必须人工单独处理；`skylint migration ledger`
  会把这些文件按条数列出来。第三方库先用 `.skylint/config.json` 的 `exclude` 排掉
  （businessview 上 98 → 8，见 `docs/MIGRATE.md` §2.1），剩下的手工改、用
  `verify --unit unit:shared` 过门禁。被多个单元引用的文件（如 `styles/common.wxss`）
  **不属于** `unit:shared`：它们在各单元作业单里带 `shared: true`，要跑全工程 verify。
- **越界检查是目录级的**：`unitDirOf(unit.path)` 之外的文件一律判失败，所以
  跨单元的重构要人工做。
- **单元文件必须在 git 索引里**：在 `dist/` 这类被忽略的目录上跑，快照/回滚/
  越界检查都会静默失效，所以开工前会 `git ls-files --error-unmatch` 检查，
  不在索引里就 `FatalError` 停住。
- **一个工程同时只能一个 run**：`start()` 带幂等 token，重复调用返回 409。
- **L2/L3 没接**：automator 冒烟与截图对比留了位置，但没实现。
