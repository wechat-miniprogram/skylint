/**
 * agent / 构建 / git 快照。全部是普通进程，不认识任何具体 coding agent。
 */
import { execFile } from "node:child_process";
import { promisify } from "node:util";

const run = promisify(execFile);

type Project = { root: string };

/**
 * 跑一次 agent。
 *
 * `AGENT_CMD` 是完整命令行（shell 语义），作业单路径以 `{job}` 占位，
 * 例如：
 *   AGENT_CMD='codex exec --json --sandbox workspace-write -o .skylint/agent.json "{prompt}"'
 *
 * 只要 agent 支持非交互执行 + 退出码，就能接进来；不需要它认识 skylint。
 */
export async function runAgent(
  project: Project,
  jobPath: string,
  attempt: number,
): Promise<{ exitCode: number; stdout: string; stderr: string }> {
  "use step";

  const template =
    process.env.AGENT_CMD ??
    'codex exec --sandbox workspace-write --skip-git-repo-check "{prompt}"';
  const prompt = [
    `按 ${jobPath} 里的 findings 修改代码。`,
    "只改这个单元的文件；不要动 .skylint/ 下的任何东西。",
    `这是第 ${attempt} 次尝试：如果 job.attempts 里有上次失败的原因，先解决它。`,
  ].join("\n");
  const command = template.replaceAll("{prompt}", prompt.replaceAll('"', '\\"'));

  try {
    const { stdout, stderr } = await run("sh", ["-c", command], {
      cwd: project.root,
      maxBuffer: 128 * 1024 * 1024,
      timeout: Number(process.env.AGENT_TIMEOUT_MS ?? 30 * 60 * 1000),
    });
    return { exitCode: 0, stdout, stderr };
  } catch (error) {
    const failure = error as { code?: number; stdout?: string; stderr?: string; killed?: boolean };
    // 超时被杀、退出码非 0 都是「这次没成」，交给上层决定重试，不在这里抛。
    return {
      exitCode: failure.killed ? 124 : (failure.code ?? 1),
      stdout: failure.stdout ?? "",
      stderr: failure.stderr ?? "",
    };
  }
}

/** L1：prod 构建。dev 构建不跑 autoprefixer，看不出真实产物。 */
export async function buildProd(
  project: Project,
): Promise<{ passed: boolean; tail: string }> {
  "use step";
  const command = process.env.BUILD_CMD ?? "npx minidev build --no-pack";
  try {
    const { stdout, stderr } = await run("sh", ["-c", command], {
      cwd: project.root,
      maxBuffer: 64 * 1024 * 1024,
      timeout: Number(process.env.BUILD_TIMEOUT_MS ?? 10 * 60 * 1000),
    });
    return { passed: true, tail: (stdout + stderr).slice(-2000) };
  } catch (error) {
    const failure = error as { stdout?: string; stderr?: string };
    return { passed: false, tail: `${failure.stdout ?? ""}${failure.stderr ?? ""}`.slice(-2000) };
  }
}

/** 每个单元动手前的快照：重试时靠它回滚，避免「改一半再改一遍」。 */
export async function snapshot(project: Project, unitId: string): Promise<string> {
  "use step";
  const { stdout } = await run("git", ["stash", "create"], { cwd: project.root });
  const sha = stdout.trim();
  await run("sh", ["-c", `echo ${JSON.stringify(sha)} > .skylint/snapshots/${sanitize(unitId)}`], {
    cwd: project.root,
  });
  return sha;
}

/** 回到快照。工作区里只应该有这个单元的改动，所以 `checkout -- .` 足够。 */
export async function revert(project: Project): Promise<void> {
  "use step";
  await run("git", ["checkout", "--", "."], { cwd: project.root });
  await run("git", ["clean", "-fd", "--", "."], { cwd: project.root });
}

/** 这个单元到底改了哪些文件——越界就判失败。 */
export async function changedFiles(project: Project): Promise<string[]> {
  "use step";
  const { stdout } = await run("git", ["diff", "--name-only", "HEAD"], { cwd: project.root });
  return stdout.split("\n").filter(Boolean);
}

/** 判定要看 diff 内容，不是文件名。超长截断——证据太多反而盖住重点。 */
export async function diffText(project: Project): Promise<string> {
  "use step";
  const { stdout } = await run("git", ["diff", "HEAD"], {
    cwd: project.root,
    maxBuffer: 128 * 1024 * 1024,
  });
  return stdout.length > 200_000 ? `${stdout.slice(0, 200_000)}\n…（已截断）` : stdout;
}

function sanitize(unitId: string): string {
  return unitId.replace(/^unit:/, "").replaceAll("/", "__");
}
