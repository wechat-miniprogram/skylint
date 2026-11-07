/**
 * 对外接口只有三个：
 *
 *   POST /api/run     { root, maxAttempts?, maxUnits?, skipBuild?, autoAccept? } → { runId }
 *   POST /api/accept  { unit, approved, comment? }                              → { ok }
 *   GET  /api/health
 *
 * 没有 UI：进度看 `skylint migration ledger`，运行详情看 `npx workflow web`。
 */
import express from "express";
import { resumeHook, start } from "workflow/api";
import { HookConflictError } from "workflow/errors";

import { migrate } from "./flows/migrate";

const app = express();
app.use(express.json());

app.get("/api/health", (_req, res) => {
  res.json({ ok: true });
});

app.post("/api/run", async (req, res) => {
  const root = req.body?.root ?? process.env.PROJECT_ROOT;
  if (!root) {
    res.status(400).json({ error: "缺 root（或设 PROJECT_ROOT）" });
    return;
  }
  const options = {
    root,
    maxAttempts: req.body?.maxAttempts ?? 3,
    maxUnits: req.body?.maxUnits ?? 0,
    maxRounds: req.body?.maxRounds ?? 200,
    skipBuild: req.body?.skipBuild ?? false,
    autoAccept: req.body?.autoAccept ?? false,
  };

  try {
    // 一个工程同时只能有一个 run：两个 run 会互相覆盖工作区和台账。
    // 用 start 自带的幂等 hook 实现，不另做锁文件（进程死了锁也不会留）。
    const run = await start(migrate, [options], {
      experimental_hook: { token: `skylint-migrate:${root}` },
    });
    res.json({ runId: run.runId });
  } catch (error) {
    if (HookConflictError.is(error)) {
      res.status(409).json({
        error: "这个工程已经有一个迁移 run 在跑",
        runId: error.conflictingRunId,
      });
      return;
    }
    throw error;
  }
});

app.post("/api/accept", async (req, res) => {
  const { unit, approved, comment } = req.body ?? {};
  if (typeof unit !== "string" || typeof approved !== "boolean") {
    res.status(400).json({ error: "需要 unit 与 approved" });
    return;
  }
  try {
    await resumeHook(`accept:${unit}`, { approved, comment });
    res.json({ ok: true });
  } catch (error) {
    // 典型原因：这个单元还没走到等人那一步，或者 run 已经结束了。
    res.status(409).json({ error: String(error) });
  }
});

export default app;
