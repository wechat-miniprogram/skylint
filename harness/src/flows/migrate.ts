/**
 * 迁移主流程。
 *
 * 这个函数是**编排**：循环、重试、等待人，都在这里；具体动作全在 step 里。
 * 它跑在沙箱 VM 里，不能碰 fs / 网络 / Date.now()，所以任何「读文件」都必须
 * 通过 step。
 *
 * 串行执行（一次一个单元）：`skylint` 的台账是单个 JSON 文件，并行写会互相覆盖；
 * 而且验收是瓶颈，并行省不下时间。
 */
import { createHook } from "workflow";

import {
  assertTracked,
  buildProd,
  changedFiles,
  diffText,
  revert,
  runAgent,
  snapshot,
} from "../steps/agent";
import { judge } from "../steps/judge";
import { nextUnit, record, verifyUnit, writeJob } from "../steps/skylint";

export type MigrateOptions = {
  /** 被迁移的仓库根（绝对路径） */
  root: string;
  /** 单个单元最多跑几次 agent */
  maxAttempts: number;
  /** 这次最多接受几个单元（0 = 一直做到没有活为止，配合 maxRounds 兜底） */
  maxUnits: number;
  /** 安全阀：外层最多转几圈，防止打回-重跑死循环 */
  maxRounds: number;
  /** 跳过 L1 构建（本地没装 minidev 时用） */
  skipBuild: boolean;
  /** 跳过人工确认（只在演示/CI 冒烟时用） */
  autoAccept: boolean;
};

export type MigrateResult = {
  accepted: string[];
  blocked: string[];
  rounds: number;
};

export async function migrate(options: MigrateOptions): Promise<MigrateResult> {
  "use workflow";

  const accepted: string[] = [];
  const blocked: string[] = [];
  let rounds = 0;

  while (rounds < options.maxRounds) {
    rounds += 1;

    const next = await nextUnit({ root: options.root });
    if (!next.unit) break;

    const unit = next.unit.id;
    // 先确认这个单元的文件被 git 跟踪，否则后面的快照/回滚/越界检查全是假的。
    // 用 fsPath：`path` 是报告路径，宿主视图下它相对 miniprogramRoot。
    await assertTracked({ root: options.root }, next.unit.fsPath);
    const jobPath = await writeJob({ root: options.root }, unit);

    let verified = false;
    let lastReason = "";

    for (let attempt = 1; attempt <= options.maxAttempts; attempt += 1) {
      const snap = await snapshot({ root: options.root });

      // 1) agent 改代码。退出码非 0 只说明「这次没成」，不抛错。
      const agent = await runAgent({ root: options.root }, jobPath, attempt);
      if (agent.exitCode !== 0) {
        lastReason = `agent 退出码 ${agent.exitCode}`;
        await revert({ root: options.root }, snap);
        await record({ root: options.root }, unit, "migrated", {
          gate: "agent",
          reason: lastReason,
        });
        continue;
      }

      // 2) 越界检查：只允许改这个单元目录下的文件。
      const files = await changedFiles({ root: options.root });
      const unitDir = unitDirOf(next.unit.fsPath);
      const outside = files.filter(
        (file) => !file.startsWith(unitDir) && !file.startsWith(".skylint/"),
      );
      if (outside.length > 0) {
        lastReason = `改了单元之外的文件：${outside.slice(0, 3).join(", ")}`;
        await revert({ root: options.root }, snap);
        await record({ root: options.root }, unit, "migrated", {
          gate: "scope",
          reason: lastReason,
        });
        continue;
      }

      // 3) L0 确定性门禁。只看 `new`（新增 error/warning）：新增 info 是 `new-info`，
      // 不拦门禁；`changedUp`（调严）只提示，不判失败。退出码已经是这个判据。
      const l0 = await verifyUnit({ root: options.root }, unit);
      if (!l0.passed) {
        lastReason = `新增 ${l0.report.new} 条（error+warning）`;
        await revert({ root: options.root }, snap);
        await record({ root: options.root }, unit, "migrated", {
          gate: "l0-static",
          reason: lastReason,
        });
        continue;
      }

      // 4) L1 prod 构建。dev 构建不跑 autoprefixer，看不出真实产物。
      let l1 = "skipped";
      if (!options.skipBuild) {
        const build = await buildProd({ root: options.root });
        l1 = build.tail;
        if (!build.passed) {
          lastReason = "prod 构建失败";
          await revert({ root: options.root }, snap);
          await record({ root: options.root }, unit, "migrated", {
            gate: "l1-build",
            reason: lastReason,
          });
          continue;
        }
      }

      // 5) L4 AI 判定。它只判一次，结论作为 step 结果固化。
      const diff = await diffText({ root: options.root });
      const verdict = await judge(
        { root: options.root },
        { unitId: unit, diff, l0: l0.report, l1 },
      );
      if (verdict.verdict !== "accept") {
        lastReason = verdict.reasons.map((reason) => reason.quote).join("; ").slice(0, 300) || "AI 拒收";
        await revert({ root: options.root }, snap);
        await record({ root: options.root }, unit, "migrated", {
          gate: "l4-judge",
          reason: lastReason,
        });
        continue;
      }

      verified = true;
      break;
    }

    if (!verified) {
      // 尝试用尽：终态，需要人介入。
      await record({ root: options.root }, unit, "blocked", {
        gate: "attempts",
        reason: lastReason || "重试次数用尽",
      });
      blocked.push(unit);
      continue;
    }

    await record({ root: options.root }, unit, "verified");

    if (options.autoAccept) {
      await record({ root: options.root }, unit, "accepted");
      accepted.push(unit);
      if (options.maxUnits > 0 && accepted.length >= options.maxUnits) break;
      continue;
    }

    // 6) 人工确认。挂起不占算力，人可以几天后再点；进程可以关掉。
    const hook = createHook<{ approved: boolean; comment?: string }>({
      token: `accept:${unit}`,
    });
    // 先等注册提交，否则外部立刻 resume 会拿到 HookNotFound。
    await hook.getConflict();
    const decision = await hook;

    if (decision.approved) {
      await record({ root: options.root }, unit, "accepted");
      accepted.push(unit);
      if (options.maxUnits > 0 && accepted.length >= options.maxUnits) break;
      continue;
    }

    // 打回：退回 migrated，下一轮它还是第一个需要 agent 的单元。
    await record({ root: options.root }, unit, "migrated", {
      gate: "human",
      reason: decision.comment ?? "人工打回",
    });
  }

  return { accepted, blocked, rounds };
}

function unitDirOf(path: string): string {
  const index = path.lastIndexOf("/");
  return index < 0 ? "" : path.slice(0, index + 1);
}
