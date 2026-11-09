/**
 * L4：AI 判定。
 *
 * 这是流程里唯一**不确定**的一步，所以它必须：
 * 1. 只判一次，结论作为 step 结果落盘（重放不会重问模型、不会换答案）；
 * 2. 吃 L0–L3 的证据，而不是自己重新看一遍代码；
 * 3. 输出结构化 JSON（schema 校验），不解析自然语言；
 * 4. 拒收时 **return 数据**，不要 throw——throw 会触发重试，重试会换结论。
 */
import { execFile } from "node:child_process";
import { promisify } from "node:util";

const run = promisify(execFile);

export type JudgeVerdict = {
  verdict: "accept" | "reject";
  confidence: number;
  reasons: Array<{ layer: "l0" | "l1" | "l2" | "l3" | "l4"; quote: string }>;
  evidenceHash: string;
};

export type Evidence = {
  unitId: string;
  diff: string;
  /** `skylint migration verify --json` 的完整输出 */
  l0: unknown;
  /** 构建日志尾部 */
  l1: string;
};

export async function judge(project: { root: string }, evidence: Evidence): Promise<JudgeVerdict> {
  "use step";

  // 判定用的 agent 可以和改代码的是同一个，也可以是另一个更便宜的模型。
  const command =
    process.env.JUDGE_CMD ??
    'codex exec --sandbox read-only --output-schema harness/src/schemas/judge.json "{prompt}"';

  const prompt = [
    "你是 Skyline 迁移的验收员。下面是一个单元在迁移前后的证据。",
    "判断这次改动是否可以接受：只依据给出的证据，不要重新推理整个仓库。",
    "拒收时给出具体到文件与行的理由；接受时 confidence 表示你对「没有引入回归」的信心。",
    "",
    `单元：${evidence.unitId}`,
    `L0 静态对比（new 必须为 0）：${JSON.stringify(evidence.l0)}`,
    `L1 构建日志尾部：${evidence.l1}`,
    "diff：",
    evidence.diff.slice(0, 200_000),
  ].join("\n");

  const { stdout } = await run(
    "sh",
    ["-c", `${command.replaceAll("{prompt}", prompt.replaceAll('"', '\\"'))} < /dev/null`],
    {
      cwd: project.root,
      maxBuffer: 64 * 1024 * 1024,
      timeout: Number(process.env.JUDGE_TIMEOUT_MS ?? 10 * 60 * 1000),
    },
  );

  const parsed = parseVerdict(stdout);
  if (!parsed) {
    // 模型没按 schema 返回：这是「这一轮白跑了」，重试一次通常就好，
    // 所以这里抛错让引擎按策略重试；连续不合法由 maxRetries 兜住。
    throw new Error(`judge 返回的不是合法 JSON：${stdout.slice(0, 500)}`);
  }
  return parsed;
}

function parseVerdict(stdout: string): JudgeVerdict | null {
  // 从输出里取最后一个 JSON 对象（agent 可能夹带进度日志）。
  const start = stdout.lastIndexOf("\n{");
  const text = start >= 0 ? stdout.slice(start + 1) : stdout;
  try {
    const value = JSON.parse(text) as JudgeVerdict;
    if (value.verdict !== "accept" && value.verdict !== "reject") return null;
    return {
      verdict: value.verdict,
      confidence: typeof value.confidence === "number" ? value.confidence : 0,
      reasons: Array.isArray(value.reasons) ? value.reasons : [],
      evidenceHash: value.evidenceHash ?? "",
    };
  } catch {
    return null;
  }
}
