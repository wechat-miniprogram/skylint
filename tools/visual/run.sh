#!/usr/bin/env bash
# 规则视觉用例：生成两套工程 → 串行构建 → 各起一个 sim serve → 分块采集与判定。
#
# 用法：
#   MINIDEV_BIN=/path/to/minidev/md-run.sh tools/visual/run.sh
#   RESUME=1 tools/visual/run.sh                 # 断点续跑
#   tools/visual/run.sh --resume --chunk-size 4
#
# 行为：
#   - case 按 CHUNK_SIZE 分块；每块开始前检查两个 serve，不健康就重启（只 kill 自己记录的 pid）
#     并重新等待就绪，块内因 serve 崩溃失败的 case 会在重启后重试。
#   - 每页采集结果由 probe.js 按 slug 合并写回 results.jsonl，重复执行不会产生重复行。
#   - 全部块结束后再统一生成一次报告；最终退出码 0 = 所有页都符合预期，1 = 存在失败/异常页。
#
# 环境变量：
#   MINIDEV_BIN    minidev 可执行文件或仓库的 md-run.sh（默认 PATH 上的 minidev）
#   MINIDEV_SDK    minidev-automator SDK 目录；MINIDEV_BIN 以 md-run.sh 结尾时自动推导
#   CHROME_PATH    webview 侧托管的 Chrome（macOS/Linux 常见路径会自动探测）
#   WAIT_MS        reLaunch 后等待渲染的毫秒数（默认 8000）
#   PORT_SKY        skyline 侧端口（默认 9783）
#   PORT_WV         webview 侧端口（默认 9784）
#   VISUAL_OUT     产物目录（默认 <repo>/.tmp/visual）
#   CHUNK_SIZE     每块 case 数（默认 8）
#   RESUME=1       断点续跑（等价 --resume）
set -euo pipefail

RESUME="${RESUME:-0}"
CHUNK_SIZE="${CHUNK_SIZE:-8}"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --resume)
      RESUME=1
      shift
      ;;
    --chunk-size)
      CHUNK_SIZE="${2:-}"
      shift 2
      ;;
    --chunk-size=*)
      CHUNK_SIZE="${1#*=}"
      shift
      ;;
    -h | --help)
      echo "用法：$0 [--resume] [--chunk-size N]"
      exit 0
      ;;
    *)
      echo "未知参数：$1（支持 --resume / --chunk-size N）" >&2
      exit 2
      ;;
  esac
done
if ! [[ "$CHUNK_SIZE" =~ ^[0-9]+$ ]] || (( CHUNK_SIZE < 1 )); then
  echo "CHUNK_SIZE 必须是正整数：$CHUNK_SIZE" >&2
  exit 2
fi

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
OUT="${VISUAL_OUT:-$ROOT/.tmp/visual}"
WAIT_MS="${WAIT_MS:-8000}"
PORT_SKY="${PORT_SKY:-9783}"
PORT_WV="${PORT_WV:-9784}"

MINIDEV_BIN="${MINIDEV_BIN:-minidev}"
if ! command -v "$MINIDEV_BIN" >/dev/null 2>&1 && [[ ! -x "$MINIDEV_BIN" ]]; then
  echo "找不到 minidev：$MINIDEV_BIN（用 MINIDEV_BIN 指定）" >&2
  exit 2
fi

MINIDEV_SDK="${MINIDEV_SDK:-}"
if [[ -z "$MINIDEV_SDK" && "$MINIDEV_BIN" == */md-run.sh ]]; then
  MINIDEV_SDK="$(cd "$(dirname "$MINIDEV_BIN")" && pwd)/sdk/minidev-automator"
fi
if [[ -z "$MINIDEV_SDK" || ! -d "$MINIDEV_SDK" ]]; then
  echo "找不到 minidev-automator SDK：用 MINIDEV_SDK 指向 minidev 的 sdk/minidev-automator" >&2
  exit 2
fi

if [[ -z "${CHROME_PATH:-}" ]]; then
  for candidate in \
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
    "$(command -v google-chrome 2>/dev/null || true)" \
    "$(command -v google-chrome-stable 2>/dev/null || true)" \
    "$(command -v chromium 2>/dev/null || true)" \
    "$(command -v chromium-browser 2>/dev/null || true)"; do
    if [[ -n "$candidate" && -x "$candidate" ]]; then
      CHROME_PATH="$candidate"
      break
    fi
  done
fi
if [[ -z "${CHROME_PATH:-}" || ! -x "$CHROME_PATH" ]]; then
  echo "找不到 Chrome：用 CHROME_PATH 指定 webview 侧浏览器" >&2
  exit 2
fi

log() {
  printf '[visual %s] %s\n' "$(date '+%H:%M:%S')" "$*"
}

tail_log() {
  local file="$1"
  [[ -f "$file" ]] || return 0
  echo "--- $file (tail) ---" >&2
  tail -20 "$file" >&2
}

port_pid() {
  # lsof 查不到时退出码非 0；pipefail 下必须兜住，否则 set -e 会直接结束脚本。
  lsof -nP -t -iTCP:"$1" -sTCP:LISTEN 2>/dev/null | head -1 || true
}

recorded_pid() {
  [[ -f "$1" ]] && tr -d '[:space:]' <"$1" || true
}

stop_pid() {
  local pid="${1:-}"
  [[ -n "$pid" ]] || return 0
  kill -0 "$pid" 2>/dev/null || return 0
  kill "$pid" 2>/dev/null || true
  for _ in $(seq 1 50); do
    kill -0 "$pid" 2>/dev/null || return 0
    sleep 0.1
  done
  kill -9 "$pid" 2>/dev/null || true
  return 0
}

# 只回收「本脚本记录过、且端口确实属于该 pid」的服务；其余占用一律报错退出，绝不 pkill。
reclaim_port() {
  local name="$1" port="$2" pidfile="$3"
  local pid
  pid="$(port_pid "$port")"
  [[ -n "$pid" ]] || return 0
  local recorded
  recorded="$(recorded_pid "$pidfile")"
  if [[ -n "$recorded" && "$recorded" == "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
    log "回收上次遗留的 $name serve（pid $pid，端口 $port）"
    stop_pid "$pid"
    for _ in $(seq 1 50); do
      [[ -z "$(port_pid "$port")" ]] && return 0
      sleep 0.1
    done
    echo "端口 $port 仍被占用（pid $pid）" >&2
    exit 2
  fi
  echo "端口 $port 已被占用（pid $pid，不是本脚本记录的）；换端口（PORT_SKY/PORT_WV）或先结束它" >&2
  exit 2
}

mkdir -p "$OUT/logs" "$OUT/shots"
reclaim_port "skyline" "$PORT_SKY" "$OUT/logs/serve-sky.pid"
reclaim_port "webview" "$PORT_WV" "$OUT/logs/serve-wv.pid"

if [[ "$RESUME" == 1 ]]; then
  log "断点续跑模式：保留已有 results/shots"
  if [[ ! -f "$OUT/results.jsonl" ]]; then
    log "未找到 $OUT/results.jsonl，将从头采集"
  fi
else
  rm -f "$OUT/results.jsonl"
  rm -rf "$OUT/shots"
  mkdir -p "$OUT/shots"
  : >"$OUT/logs/serve-sky.log"
  : >"$OUT/logs/serve-wv.log"
fi

log "生成工程"
# 默认跳过 `universal-selector`：它的用例含字面 `*`，WXSS 编译器直接报
# `unexpected token '*'` 并让整个工程构建失败（这本身是结论，见 KNOWN-COMPILE-FAIL.md）。
SKIP_ARGS=()
for skip in ${VISUAL_SKIP-universal-selector}; do
  SKIP_ARGS+=(--skip "$skip")
done
cargo run -q --manifest-path "$ROOT/Cargo.toml" --example rule_visual_cases -- \
  --renderer skyline --out "$OUT/probe-sky" "${SKIP_ARGS[@]}"
cargo run -q --manifest-path "$ROOT/Cargo.toml" --example rule_visual_cases -- \
  --renderer webview --out "$OUT/probe-wv" "${SKIP_ARGS[@]}"

# 构建期串行：并行跑两个 sim build 会争用同一份编译缓存。
log "构建 skyline 工程"
if ! "$MINIDEV_BIN" sim build --project "$OUT/probe-sky/project" --out "$OUT/probe-sky/dist" \
  >"$OUT/logs/build-sky.log" 2>&1; then
  tail_log "$OUT/logs/build-sky.log"
  exit 1
fi
log "构建 webview 工程"
if ! "$MINIDEV_BIN" sim build --project "$OUT/probe-wv/project" --out "$OUT/probe-wv/dist" \
  >"$OUT/logs/build-wv.log" 2>&1; then
  tail_log "$OUT/logs/build-wv.log"
  exit 1
fi

SKY_PID=""
WV_PID=""

cleanup() {
  stop_pid "$WV_PID"
  stop_pid "$SKY_PID"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

start_sky() {
  printf '\n=== %s 启动 skyline serve（端口 %s） ===\n' "$(date '+%F %T')" "$PORT_SKY" \
    >>"$OUT/logs/serve-sky.log"
  # skyline 侧不传 --browser：serve 检测到 skyline 页会自动托管 Electron（skyline 引擎只活在
  # 开发者工具整包里）。只绑 127.0.0.1。
  "$MINIDEV_BIN" sim serve --host 127.0.0.1 --dist "$OUT/probe-sky/dist" --port "$PORT_SKY" \
    >>"$OUT/logs/serve-sky.log" 2>&1 &
  SKY_PID=$!
  echo "$SKY_PID" >"$OUT/logs/serve-sky.pid"
  log "启动 skyline serve（pid $SKY_PID，端口 $PORT_SKY）"
}

start_wv() {
  printf '\n=== %s 启动 webview serve（端口 %s） ===\n' "$(date '+%F %T')" "$PORT_WV" \
    >>"$OUT/logs/serve-wv.log"
  # webview 侧显式传 --browser，只绑 127.0.0.1。
  "$MINIDEV_BIN" sim serve --host 127.0.0.1 --dist "$OUT/probe-wv/dist" --port "$PORT_WV" \
    --browser "$CHROME_PATH" >>"$OUT/logs/serve-wv.log" 2>&1 &
  WV_PID=$!
  echo "$WV_PID" >"$OUT/logs/serve-wv.pid"
  log "启动 webview serve（pid $WV_PID，端口 $PORT_WV）"
}

serve_healthy() {
  curl -sf --max-time 2 "http://127.0.0.1:$1/browser-cdp/json/list" >/dev/null 2>&1 && return 0
  curl -sf --max-time 2 "http://127.0.0.1:$1/browser-cdp/json" >/dev/null 2>&1
}

wait_ready() {
  local port="$1" log_file="$2" pid="$3"
  for _ in $(seq 1 240); do
    if serve_healthy "$port"; then
      return 0
    fi
    if ! kill -0 "$pid" 2>/dev/null; then
      echo "serve 提前退出（端口 $port，pid $pid）" >&2
      tail_log "$log_file"
      return 1
    fi
    sleep 0.5
  done
  echo "等待 serve 就绪超时（端口 $port，pid $pid）" >&2
  tail_log "$log_file"
  return 1
}

ensure_sky() {
  if [[ -n "$SKY_PID" ]] && kill -0 "$SKY_PID" 2>/dev/null && serve_healthy "$PORT_SKY"; then
    return 0
  fi
  log "skyline serve 不健康（pid ${SKY_PID:-无}），重启"
  stop_pid "$SKY_PID"
  start_sky
  wait_ready "$PORT_SKY" "$OUT/logs/serve-sky.log" "$SKY_PID"
}

ensure_wv() {
  if [[ -n "$WV_PID" ]] && kill -0 "$WV_PID" 2>/dev/null && serve_healthy "$PORT_WV"; then
    return 0
  fi
  log "webview serve 不健康（pid ${WV_PID:-无}），重启"
  stop_pid "$WV_PID"
  start_wv
  wait_ready "$PORT_WV" "$OUT/logs/serve-wv.log" "$WV_PID"
}

read_slugs() {
  node -e '
    const fs = require("fs")
    const cases = JSON.parse(fs.readFileSync(process.argv[1], "utf8"))
    for (const item of cases) if (item.slug) console.log(item.slug)
  ' "$1"
}

# 列出传入 slug 里还没有有效结果的项：error 与缺行都算未完成（fail 是有结论的结果）。
remaining_slugs() {
  node -e '
    const fs = require("fs")
    const [file, ...slugs] = process.argv.slice(1)
    const done = new Set()
    if (fs.existsSync(file)) {
      for (const line of fs.readFileSync(file, "utf8").split("\n")) {
        const text = line.trim()
        if (!text) continue
        try {
          const record = JSON.parse(text)
          if (record && record.slug && !record.error && record.verdict !== "error") done.add(record.slug)
        } catch (error) {}
      }
    }
    for (const slug of slugs) if (!done.has(slug)) console.log(slug)
  ' "$OUT/results.jsonl" "$@"
}

run_probe() {
  local csv code=0
  csv="$(IFS=,; echo "$*")"
  MINIDEV_SDK="$MINIDEV_SDK" MINIDEV_BIN="$MINIDEV_BIN" node "$HERE/probe.js" \
    --sky-base "http://127.0.0.1:$PORT_SKY" \
    --wv-base "http://127.0.0.1:$PORT_WV" \
    --cases "$OUT/probe-sky/cases.json" \
    --shots "$OUT/shots" \
    --results "$OUT/results.jsonl" \
    --wait-ms "$WAIT_MS" \
    --only "$csv" \
    --resume || code=$?
  return "$code"
}

ALL_SLUGS=()
while IFS= read -r slug; do
  [[ -n "$slug" ]] && ALL_SLUGS+=("$slug")
done < <(read_slugs "$OUT/probe-sky/cases.json")
TOTAL=${#ALL_SLUGS[@]}
if (( TOTAL == 0 )); then
  echo "cases.json 里没有 case，检查 rule_visual_cases 输出" >&2
  exit 2
fi
CHUNKS=$(( (TOTAL + CHUNK_SIZE - 1) / CHUNK_SIZE ))
log "共 $TOTAL 个 case，分 $CHUNKS 块（CHUNK_SIZE=$CHUNK_SIZE，RESUME=$RESUME）"

for ((offset = 0; offset < TOTAL; offset += CHUNK_SIZE)); do
  index=$(( offset / CHUNK_SIZE + 1 ))
  CHUNK=("${ALL_SLUGS[@]:offset:CHUNK_SIZE}")
  log "块 $index/$CHUNKS：${#CHUNK[@]} 个 case"
  for attempt in 1 2 3; do
    if ! ensure_sky; then
      log "skyline serve 重启失败（第 $attempt 次尝试）"
    fi
    if ! ensure_wv; then
      log "webview serve 重启失败（第 $attempt 次尝试）"
    fi

    REMAINING=()
    while IFS= read -r slug; do
      [[ -n "$slug" ]] && REMAINING+=("$slug")
    done < <(remaining_slugs "${CHUNK[@]}")
    if (( ${#REMAINING[@]} == 0 )); then
      log "块 $index 已全部有结果"
      break
    fi

    log "块 $index 采集 ${#REMAINING[@]} 个 case（第 $attempt 次尝试）"
    code=0
    run_probe "${REMAINING[@]}" || code=$?
    if (( code != 0 )); then
      log "块 $index probe 退出码 $code（继续，不中断整批）"
    fi

    if ! serve_healthy "$PORT_SKY" || ! serve_healthy "$PORT_WV"; then
      log "块 $index 结束后检测到 serve 不健康，重启并重试本块未完成 case"
      continue
    fi
    # probe 退出码 2 是启动阶段失败（连接/参数），不是单页结论，再试一次。
    if (( code == 2 )); then
      log "块 $index probe 启动失败（退出码 2），重试本块未完成 case"
      continue
    fi
    break
  done
  if [[ -f "$OUT/results.jsonl" ]]; then
    log "块 $index 结束，results 已有 $(wc -l <"$OUT/results.jsonl" | tr -d ' ') 行"
  else
    log "块 $index 结束，results 尚无输出"
  fi
done

# 收尾：处理前面因 serve 反复失败而遗留的 case，尽量让每个 slug 都有结果。
for attempt in 1 2; do
  FINAL=()
  while IFS= read -r slug; do
    [[ -n "$slug" ]] && FINAL+=("$slug")
  done < <(remaining_slugs "${ALL_SLUGS[@]}")
  if (( ${#FINAL[@]} == 0 )); then
    log "所有 case 都有结果"
    break
  fi
  log "收尾第 $attempt 次：重试 ${#FINAL[@]} 个未完成 case"
  ensure_sky || log "skyline serve 重启失败（收尾）"
  ensure_wv || log "webview serve 重启失败（收尾）"
  code=0
  run_probe "${FINAL[@]}" || code=$?
  if (( code == 0 )); then
    break
  fi
  log "收尾 probe 退出码 $code"
done

log "生成报告"
REPORT_CODE=0
node "$HERE/probe.js" --report-only \
  --results "$OUT/results.jsonl" \
  --report "$OUT/report.html" \
  --cases "$OUT/probe-sky/cases.json" || REPORT_CODE=$?

log "results: $OUT/results.jsonl"
log "report:  $OUT/report.html"
if (( REPORT_CODE == 0 )); then
  exit 0
fi
exit 1
