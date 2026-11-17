/**
 * agent / 构建 / git 快照。全部是普通进程，不认识任何具体 coding agent。
 */
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { FatalError } from "workflow";

const run = promisify(execFile);

type Project = { root: string };

/**
 * 单元文件必须在 git 索引里。
 *
 * 否则 `git diff` / `checkout` / `stash` 全部静默失效：改了什么看不见、回滚也不回滚，
 * 而门禁照常给“通过”。这是最危险的一类错（看起来在跑，实际上没有隔离）。
 */
export async function assertTracked(project: Project, unitPath: string): Promise<void> {
  "use step";
  try {
    await run("git", ["ls-files", "--error-unmatch", "--", unitPath], { cwd: project.root });
  } catch {
    throw new FatalError(
      `${unitPath} 不在 git 索引里（被 .gitignore 排除？）。\n` +
        "快照、回滚、越界检查都靠 git diff，单元文件不被跟踪时它们会静默失效。\n" +
        "把单元所在目录纳入版本控制（或换成源码目录作为单元）后再跑。",
    );
  }
}

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
  const command = template
    .replaceAll("{job}", jobPath)
    .replaceAll("{prompt}", prompt.replaceAll('"', '\\"'));

  try {
    const { stdout, stderr } = await run("sh", ["-c", `${command} < /dev/null`], {
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
    const { stdout, stderr } = await run("sh", ["-c", `${command} < /dev/null`], {
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

/**
 * 每个单元动手前的快照：重试时靠它回滚，避免「改一半再改一遍」。
 *
 * 存两份：`head`（防 agent 自己 commit）与 `tree`（工作区未提交的改动）。
 */
export type Snapshot = { head: string; tree: string };

export async function snapshot(project: Project): Promise<Snapshot> {
  "use step";
  const { stdout: head } = await run("git", ["rev-parse", "HEAD"], { cwd: project.root });
  const { stdout: tree } = await run("git", ["stash", "create"], { cwd: project.root });
  return { head: head.trim(), tree: tree.trim() || head.trim() };
}

/**
 * 回到快照。
 *
 * `git clean` 会连 `.skylint/` 一起删掉（台账、基线、作业单都在里面），所以这里
 * 自己枚举未跟踪文件并跳过它，不用 `clean -fd`。
 */
export async function revert(project: Project, snap: Snapshot): Promise<void> {
  "use step";
  await run("git", ["reset", "--hard", snap.head], { cwd: project.root });
  await run("git", ["checkout", snap.tree, "--", "."], { cwd: project.root });

  const { stdout } = await run("git", ["ls-files", "--others", "--exclude-standard"], {
    cwd: project.root,
  });
  const strays = stdout
    .split("\n")
    .filter(Boolean)
    .filter((file) => !file.startsWith(".skylint/"));
  if (strays.length > 0) {
    await run("rm", ["-f", "--", ...strays], { cwd: project.root });
  }
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


