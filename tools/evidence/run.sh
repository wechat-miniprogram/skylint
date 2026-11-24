#!/usr/bin/env bash
# 规则证据用例：生成完整工程 → 逐条构建 / 起 serve 采集 → 判定 → 出报告。
#
# 用法：
#   MINIDEV_BIN=/path/to/minidev/target/debug/minidev \
#   MINIDEV_SDK=/path/to/minidev/sdk/minidev-automator \
#   tools/evidence/run.sh
#
# 环境变量：
#   MINIDEV_BIN  **已构建的 minidev 二进制**（默认 PATH 上的 minidev）。禁止指向 md-run.sh
#                这类「先编译再 exec」的包装脚本：每条证据用例都要 mutating 地调用一次，
#                包装脚本会让每次调用去抢编译锁，表现为随机超时与 error。
#   MINIDEV_SDK  minidev-automator SDK 目录；只有 serve 类用例需要。
#   CHROME_PATH  webview 页面的浏览器；只有 renderer=webview 的 serve 用例需要。
#   EVIDENCE_OUT 产物目录（默认 <repo>/.tmp/evidence-run）。
#   PORT         serve 端口（默认 9791，只绑 127.0.0.1）。
#   WAIT_MS      页面 reLaunch 后的沉降毫秒数（默认 2000）。
#   SKIP         逗号分隔的规则 / 用例关键字，传给生成器的 --skip，只生成匹配的用例
#                （例如 SKIP=scroll-view-family,autoprefix-build 只留下本次要跑的那几条）。
#
# 行为：
#   - 每条用例一个独立工程与（serve 类）一个独立 serve；构建与 serve 严格串行。
#   - 构建失败对 BuildFails 是预期结果，不是脚本错误；对 serve 用例则记 error（没有 dist 可跑）。
#   - 端口先确认空闲；只回收自己 pidfile 里记录的同 pid 服务，绝不 pkill。
#   - 采集/判定全部交给 run.js；三档 pass / fail / error，取不到证据一律 error。
#   - 最终退出码 0 = 全部 pass，1 = 存在 fail / error / 缺失结果，2 = 环境或参数错误。
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
OUT="${EVIDENCE_OUT:-$ROOT/.tmp/evidence-run}"
PORT="${PORT:-9791}"
WAIT_MS="${WAIT_MS:-2000}"

MINIDEV_BIN="${MINIDEV_BIN:-minidev}"
if [[ "$MINIDEV_BIN" == */md-run.sh ]]; then
  echo "MINIDEV_BIN 必须指向已构建的 minidev 二进制（target/debug/minidev），不能是 md-run.sh：$MINIDEV_BIN" >&2
  exit 2
fi
if [[ ! -x "$MINIDEV_BIN" ]]; then
  if command -v "$MINIDEV_BIN" >/dev/null 2>&1; then
    MINIDEV_BIN="$(command -v "$MINIDEV_BIN")"
  else
    echo "找不到可执行的 minidev：$MINIDEV_BIN（用 MINIDEV_BIN 指定已构建二进制）" >&2
    exit 2
  fi
fi

if ! [[ "$PORT" =~ ^[0-9]+$ ]] || (( PORT < 1 || PORT > 65535 )); then
  echo "PORT 必须是 1-65535 的整数：$PORT" >&2
  exit 2
fi
if ! [[ "$WAIT_MS" =~ ^[0-9]+$ ]]; then
  echo "WAIT_MS 必须是非负整数：$WAIT_MS" >&2
  exit 2
fi

log() {
  printf '[evidence %s] %s\n' "$(date '+%H:%M:%S')" "$*"
}

tail_log() {
  local file="$1"
  [[ -f "$file" ]] || return 0
  echo "--- $file (tail) ---" >&2
  tail -20 "$file" >&2
}

port_pid() {
  # lsof 查不到时退出码非 0；pipefail 下必须兜住。
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
  for _ in $(seq 1 100); do
    kill -0 "$pid" 2>/dev/null || return 0
    sleep 0.1
  done
  kill -9 "$pid" 2>/dev/null || true
  return 0
}

wait_port_free() {
  for _ in $(seq 1 100); do
    [[ -z "$(port_pid "$PORT")" ]] && return 0
    sleep 0.1
  done
  return 1
}

# 只回收「端口确实属于 pidfile 记录的那条服务」；其余占用一律报错，绝不 pkill。
reclaim_port() {
  local pid recorded
  pid="$(port_pid "$PORT")"
  [[ -n "$pid" ]] || return 0
  recorded="$(recorded_pid "$OUT/logs/serve.pid")"
  if [[ -n "$recorded" && "$recorded" == "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
    log "回收上次遗留的 serve（pid $pid，端口 $PORT）"
    stop_pid "$pid"
    wait_port_free || {
      echo "端口 $PORT 仍被占用（pid $pid）" >&2
      exit 2
    }
    return 0
  fi
  echo "端口 $PORT 已被占用（pid $pid，不是本脚本记录的）；换 PORT 或先结束它" >&2
  exit 2
}

serve_healthy() {
  curl -sf --max-time 2 "http://127.0.0.1:$PORT/browser-cdp/json/list" >/dev/null 2>&1 && return 0
  curl -sf --max-time 2 "http://127.0.0.1:$PORT/browser-cdp/json" >/dev/null 2>&1
}

wait_ready() {
  local log_file="$1" pid="$2"
  for _ in $(seq 1 240); do
    if serve_healthy; then
      return 0
    fi
    if ! kill -0 "$pid" 2>/dev/null; then
      echo "serve 提前退出（端口 $PORT，pid $pid）" >&2
      tail_log "$log_file"
      return 1
    fi
    sleep 0.5
  done
  echo "等待 serve 就绪超时（端口 $PORT，pid $pid）" >&2
  tail_log "$log_file"
  return 1
}

mkdir -p "$OUT/logs"
rm -f "$OUT/results.jsonl" "$OUT/report.html"
reclaim_port

log "生成工程"
gen_args=(--out "$OUT")
if [[ -n "${SKIP:-}" ]]; then
  IFS=',' read -r -a _skips <<<"$SKIP"
  for _skip in "${_skips[@]}"; do
    if [[ -n "$_skip" ]]; then
      gen_args+=(--skip "$_skip")
    fi
  done
fi
cargo run -q --manifest-path "$ROOT/Cargo.toml" --example rule_evidence_cases -- "${gen_args[@]}"

CASES=()
while IFS=$'\t' read -r dir kind renderer; do
  [[ -n "$dir" ]] && CASES+=("$dir"$'\t'"$kind"$'\t'"${renderer:-}")
done < <(node "$HERE/run.js" --cases "$OUT/evidence.json" --list)
TOTAL=${#CASES[@]}
if (( TOTAL == 0 )); then
  echo "evidence.json 里没有 case，检查 rule_evidence_cases 输出" >&2
  exit 2
fi
log "共 $TOTAL 个 case（端口 $PORT，WAIT_MS $WAIT_MS）"

NEEDS_SERVE=0
for item in "${CASES[@]}"; do
  IFS=$'\t' read -r _ kind _ <<<"$item"
  if [[ "$kind" == "serve" ]]; then
    NEEDS_SERVE=1
  fi
done
if (( NEEDS_SERVE == 1 )); then
  if [[ -z "${MINIDEV_SDK:-}" ]]; then
    for candidate in \
      "$(dirname "$(dirname "$(dirname "$MINIDEV_BIN")")")/sdk/minidev-automator" \
      "$(dirname "$(dirname "$MINIDEV_BIN")")/sdk/minidev-automator" \
      "$(dirname "$MINIDEV_BIN")/sdk/minidev-automator"; do
      if [[ -d "$candidate" ]]; then
        MINIDEV_SDK="$candidate"
        break
      fi
    done
  fi
  if [[ -z "${MINIDEV_SDK:-}" || ! -d "$MINIDEV_SDK" ]]; then
    echo "有 serve 类用例，但找不到 minidev-automator SDK：用 MINIDEV_SDK 指向 minidev 的 sdk/minidev-automator" >&2
    exit 2
  fi
fi

SERVE_PID=""
cleanup() {
  stop_pid "$SERVE_PID"
  rm -f "$OUT/logs/serve.pid"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

record_error() {
  local dir="$1" reason="$2" code=0
  node "$HERE/run.js" --error --cases "$OUT/evidence.json" --case-dir "$dir" \
    --results "$OUT/results.jsonl" --reason "$reason" || code=$?
  if (( code != 0 )); then
    log "写 error 记录失败（$dir，退出码 $code）"
  fi
}

wait_port_free || {
  echo "端口 $PORT 在采集开始前仍被占用" >&2
  exit 2
}

INDEX=0
for item in "${CASES[@]}"; do
  IFS=$'\t' read -r dir kind renderer <<<"$item"
  INDEX=$((INDEX + 1))
  case_dir="$OUT/$dir"
  log "[$INDEX/$TOTAL] $dir（$kind）"

  # 构建对所有 case 都要跑：serve 类也要 dist 才能起服务。
  set +e
  "$MINIDEV_BIN" sim build --project "$case_dir" --out "$case_dir/dist" --no-pack \
    >"$case_dir/build.stdout" 2>"$case_dir/build.stderr"
  build_exit=$?
  set -e
  echo "$build_exit" >"$case_dir/build.exit"

  if [[ "$kind" == "build" ]]; then
    code=0
    node "$HERE/run.js" --judge-build --cases "$OUT/evidence.json" --case-dir "$dir" \
      --results "$OUT/results.jsonl" || code=$?
    if (( code != 0 )); then
      log "判定失败（$dir，退出码 $code）"
    fi
    continue
  fi

  if (( build_exit != 0 )); then
    record_error "$dir" "serve 类用例构建失败（exit $build_exit），没有 dist 可跑；stderr 尾部：$(tail -5 "$case_dir/build.stderr" | tr '\n' ' ')"
    continue
  fi

  if [[ "$renderer" == "webview" ]]; then
    if [[ -z "${CHROME_PATH:-}" || ! -x "${CHROME_PATH:-}" ]]; then
      record_error "$dir" "页面 renderer=webview 但 CHROME_PATH 未指向可执行浏览器"
      continue
    fi
  fi

  wait_port_free || {
    record_error "$dir" "端口 $PORT 仍被占用，无法启动 serve"
    continue
  }
  serve_args=(sim serve --host 127.0.0.1 --dist "$case_dir/dist" --port "$PORT")
  if [[ "$renderer" == "webview" ]]; then
    serve_args+=(--browser "$CHROME_PATH")
  fi
  : >"$case_dir/serve.log"
  "$MINIDEV_BIN" "${serve_args[@]}" >>"$case_dir/serve.log" 2>&1 &
  SERVE_PID=$!
  echo "$SERVE_PID" >"$OUT/logs/serve.pid"

  if ! wait_ready "$case_dir/serve.log" "$SERVE_PID"; then
    record_error "$dir" "serve 未就绪；日志尾部：$(tail -5 "$case_dir/serve.log" | tr '\n' ' ')"
    stop_pid "$SERVE_PID"
    SERVE_PID=""
    rm -f "$OUT/logs/serve.pid"
    continue
  fi

  code=0
  node "$HERE/run.js" --drive --cases "$OUT/evidence.json" --case-dir "$dir" \
    --results "$OUT/results.jsonl" --base "http://127.0.0.1:$PORT" \
    --sdk "$MINIDEV_SDK" --wait-ms "$WAIT_MS" || code=$?
  if (( code != 0 )); then
    log "采集/判定失败（$dir，退出码 $code）"
  fi

  stop_pid "$SERVE_PID"
  SERVE_PID=""
  rm -f "$OUT/logs/serve.pid"
  if ! wait_port_free; then
    log "serve 已结束但端口 $PORT 尚未释放"
  fi
done

log "生成报告"
REPORT_CODE=0
node "$HERE/run.js" --report-only --cases "$OUT/evidence.json" \
  --results "$OUT/results.jsonl" --report "$OUT/report.html" || REPORT_CODE=$?
log "results: $OUT/results.jsonl"
log "report:  $OUT/report.html"
exit "$REPORT_CODE"
