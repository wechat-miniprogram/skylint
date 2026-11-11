/**
 * 调 skylint 的唯一入口。
 *
 * 约定（与 `docs/MIGRATE.md` 一致）：
 * - 每个命令都吃 JSON、吐 JSON、用退出码表达成败；
 * - 这里**不解析**人类可读输出，也不解析 agent 的自然语言；
 * - skylint 的路径由 `SKYLINT_BIN` 给（默认 PATH 上的 `skylint`）。
 */
import { execFile } from "node:child_process";
import { promisify } from "node:util";

const run = promisify(execFile);

export type Project = {
  /** 被迁移的仓库根（绝对路径） */
  root: string;
};

export type NextUnit = {
  unit: {
    id: string;
    path: string;
    /** 工程根相对的文件路径（`path` 是报告路径，可能相对 miniprogramRoot）。 */
    fsPath: string;
    batch: number;
    diagnostics: { total: number };
  } | null;
  progress: Record<string, number>;
  failures: number;
  lastFailure?: { gate?: string; reason: string };
};

export type VerifyReport = {
  /** 新增 error/warning（拦门禁：`new > 0` 时退出码为 1） */
  new: number;
  /** 新增 info（不拦门禁） */
  newInfo: number;
  fixed: number;
  /** 严重级变化总数 = changedUp + changedDown（不拦门禁） */
  changed: number;
  changedUp: number;
  changedDown: number;
  versionMismatch: boolean;
  findings: Array<{
    kind:
      | "new"
      | "new-info"
      | "fixed"
      | "severity-raised"
      | "severity-lowered";
    id: string;
    rule: string;
    severity: string;
    path: string;
    span: { line: number; column: number };
    message?: string;
  }>;
};

async function skylint(project: Project, args: string[]) {
  const bin = process.env.SKYLINT_BIN ?? "skylint";
  const { stdout } = await run(bin, args, {
    cwd: project.root,
    maxBuffer: 64 * 1024 * 1024,
  });
  return stdout;
}

/** 取下一个需要 agent 的单元；全部做完时 `unit === null`。 */
export async function nextUnit(project: Project): Promise<NextUnit> {
  "use step";
  return JSON.parse(await skylint(project, ["migration", "next", project.root, "--json"]));
}

/** 生成作业单，返回它的绝对路径。 */
export async function writeJob(project: Project, unitId: string): Promise<string> {
  "use step";
  const out = `.skylint/jobs/${unitId.replace(/^unit:/, "").replaceAll("/", "__")}.json`;
  await skylint(project, [
    "migration",
    "job",
    project.root,
    "--unit",
    unitId,
    "--baseline",
    ".skylint/baseline.json",
    "--out",
    out,
  ]);
  return `${project.root}/${out}`;
}

/**
 * L0 门禁。**失败不抛错**——抛错会让引擎重试，重试会换一次结果。
 * 只有「skylint 自己跑不起来」才该抛。
 */
export async function verifyUnit(
  project: Project,
  unitId: string,
): Promise<{ passed: boolean; report: VerifyReport }> {
  "use step";
  const bin = process.env.SKYLINT_BIN ?? "skylint";
  try {
    const { stdout } = await run(
      bin,
      [
        "migration",
        "verify",
        project.root,
        "--baseline",
        ".skylint/baseline.json",
        "--unit",
        unitId,
        "--json",
      ],
      { cwd: project.root, maxBuffer: 64 * 1024 * 1024 },
    );
    return { passed: true, report: JSON.parse(stdout) };
  } catch (error) {
    const failure = error as { code?: number; stdout?: string; stderr?: string };
    if (typeof failure.stdout === "string" && failure.stdout.trim().startsWith("{")) {
      return { passed: false, report: JSON.parse(failure.stdout) };
    }
    // 退出码 2 = 基线读不到 / 工程加载失败，这不是「改坏了」，是环境问题。
    throw new Error(`skylint migration verify 跑不起来：${failure.stderr ?? ""}`);
  }
}

export async function record(
  project: Project,
  unitId: string,
  status: string,
  failure?: { gate?: string; reason: string },
): Promise<void> {
  "use step";
  const args = ["migration", "record", project.root, "--unit", unitId, "--status", status];
  if (failure) {
    args.push("--reason", failure.reason);
    if (failure.gate) args.push("--gate", failure.gate);
  }
  await skylint(project, args);
}
