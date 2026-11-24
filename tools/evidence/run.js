#!/usr/bin/env node
// 证据用例的采集与判定：构建断言、serve 驱动的 console / evaluate 断言，以及报告。
//
// 由 tools/evidence/run.sh 调用。职责分工：
//   - run.sh 管进程生命周期（构建、serve、端口、pid 清理）；
//   - 本脚本管「读证据 + 判定 + 落 results.jsonl / report.html」。
//
// 三档结论：
//   pass  = 所有断言成立；
//   fail  = 证据取到了，但断言不成立（文案不匹配、exit 码不对、日志里出现了不该出现的行）；
//   error = 证据取不到（构建没跑、dist 文件缺失、serve 连不上、console 通道不通、evaluate 抛错）。
//   error 绝不能记成 pass：它代表「这条用例没有结论」，必须修好环境重跑。
//
// 子命令：
//   --list            打印清单（TSV：dir / run kind / renderer），供 run.sh 遍历。
//   --judge-build     读取用例目录里的 build.exit / build.stdout / build.stderr 并判定。
//   --drive           连接 serve、加载页面、收集日志 / 执行 evaluate 并判定。
//   --error           追加一条 error 记录（--reason 说明取不到证据的原因）。
//   --report-only     读 results.jsonl 生成 --report 指定的 HTML，并给出退出码。

'use strict'

const fs = require('fs')
const path = require('path')

const CONNECT_TIMEOUT_MS = 15000
const RELAUNCH_TIMEOUT_MS = 25000
const PAGE_TIMEOUT_MS = 10000
const EVALUATE_TIMEOUT_MS = 10000
const CONSOLE_MARKER_TIMEOUT_MS = 5000
const TAIL_CHARS = 4000
const REPORT_LOG_CHARS = 6000

const BOOLEAN_FLAGS = new Set(['list', 'judgeBuild', 'drive', 'error', 'reportOnly'])

function parseArgs(argv) {
  const args = {}
  for (let i = 0; i < argv.length; i++) {
    const key = argv[i]
    if (!key.startsWith('--')) continue
    const name = key.slice(2).replace(/-([a-z])/g, (_, c) => c.toUpperCase())
    if (BOOLEAN_FLAGS.has(name)) {
      args[name] = true
      continue
    }
    const value = argv[i + 1]
    if (value === undefined || value.startsWith('--')) continue
    args[name] = value
    i++
  }
  return args
}

const sleep = ms => new Promise(resolve => setTimeout(resolve, ms))

function fatal(message) {
  console.error(`[evidence] ${message}`)
  process.exit(2)
}

function withTimeout(promise, ms, label) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`${label}超时（${ms}ms）`)), ms)
    promise.then(
      value => {
        clearTimeout(timer)
        resolve(value)
      },
      error => {
        clearTimeout(timer)
        reject(error)
      },
    )
  })
}

function tail(text, limit = TAIL_CHARS) {
  const value = String(text == null ? '' : text)
  return value.length > limit ? `…${value.slice(-limit)}` : value
}

function manifest(args) {
  if (!args.cases) fatal('缺少参数 --cases')
  const file = path.resolve(args.cases)
  return { outDir: path.dirname(file), cases: JSON.parse(fs.readFileSync(file, 'utf8')) }
}

function caseDirOf(outDir, entry) {
  return path.join(outDir, entry.dir)
}

function loadCase(args) {
  const { outDir, cases } = manifest(args)
  if (!args.caseDir) fatal('缺少参数 --case-dir')
  const wanted = String(args.caseDir).replaceAll('\\', '/').replace(/^\.?\//, '').replace(/\/+$/, '')
  const entry = cases.find(item => item.dir === wanted)
  if (!entry) fatal(`evidence.json 里没有 dir=${wanted} 的用例`)
  return { entry, dir: caseDirOf(outDir, entry) }
}

/// `console.log` 的 `App.logAdded` 参数：`{type, args}`，args 里既有字符串也有对象。
function logText(message) {
  const args = Array.isArray(message && message.args) ? message.args : []
  return args.map(arg => (typeof arg === 'string' ? arg : JSON.stringify(arg))).join(' ')
}

function newRecord(entry, state, checks, observed) {
  return {
    rule: entry.rule,
    name: entry.name,
    dir: entry.dir,
    run: entry.run,
    expect: entry.expect,
    note: entry.note,
    state,
    checks,
    observed,
    updatedAt: new Date().toISOString(),
  }
}

function stateOf(checks) {
  return checks.length > 0 && checks.every(check => check.ok === true) ? 'pass' : 'fail'
}

function addCheck(checks, label, ok, expected, actual) {
  checks.push({ label, ok, ...(expected === undefined ? {} : { expected }), ...(actual === undefined ? {} : { actual }) })
}

function readIfExists(file) {
  try {
    return fs.readFileSync(file, 'utf8')
  } catch (error) {
    return null
  }
}

/// 构建断言：exit 码、stderr 片段、构建日志、dist 产物。
function judgeBuild(entry, dir) {
  const exitText = readIfExists(path.join(dir, 'build.exit'))
  if (exitText == null) return newRecord(entry, 'error', [], { reason: '缺少 build.exit：构建没有执行' })
  const stdout = readIfExists(path.join(dir, 'build.stdout'))
  const stderr = readIfExists(path.join(dir, 'build.stderr'))
  if (stdout == null || stderr == null) {
    return newRecord(entry, 'error', [], { reason: '缺少 build.stdout / build.stderr：构建输出没有落盘' })
  }
  const exit = Number(exitText.trim())
  const log = `${stdout}\n${stderr}`
  const observed = { exit, stdoutTail: tail(stdout), stderrTail: tail(stderr) }
  const checks = []
  const expect = entry.expect

  if (expect.kind === 'build-ok') {
    addCheck(checks, '构建 exit 0', exit === 0, '0', String(exit))
    if (exit === 0 && !fs.existsSync(path.join(dir, 'dist'))) {
      return newRecord(entry, 'error', checks, { ...observed, reason: '构建 exit 0 但没有 dist/ 产物' })
    }
    return newRecord(entry, stateOf(checks), checks, observed)
  }

  if (expect.kind === 'build-fails') {
    addCheck(checks, '构建必须失败（exit ≠ 0）', exit !== 0, '≠ 0', String(exit))
    for (const needle of expect.stderr_contains || []) {
      addCheck(checks, `stderr 含 ${JSON.stringify(needle)}`, stderr.includes(needle), '包含', '缺失')
    }
    return newRecord(entry, stateOf(checks), checks, observed)
  }

  if (expect.kind === 'build-log') {
    addCheck(checks, '构建 exit 0', exit === 0, '0', String(exit))
    for (const needle of expect.must || []) {
      addCheck(checks, `日志含 ${JSON.stringify(needle)}`, log.includes(needle), '包含', '缺失')
    }
    for (const needle of expect.must_not || []) {
      addCheck(checks, `日志不含 ${JSON.stringify(needle)}`, !log.includes(needle), '不包含', '出现了')
    }
    return newRecord(entry, stateOf(checks), checks, observed)
  }

  if (expect.kind === 'artifact') {
    addCheck(checks, '构建 exit 0', exit === 0, '0', String(exit))
    if (exit !== 0) return newRecord(entry, stateOf(checks), checks, observed)
    const file = path.join(dir, 'dist', expect.path)
    const text = readIfExists(file)
    if (text == null) {
      return newRecord(entry, 'error', checks, { ...observed, reason: `dist 里没有 ${expect.path}，取不到产物证据` })
    }
    const maxNeedle = Math.max(0, ...(expect.must || []).map(n => n.length), ...(expect.must_not || []).map(n => n.length))
    observed.artifact = { path: expect.path, bytes: Buffer.byteLength(text), head: tail(text.slice(0, maxNeedle + 400), maxNeedle + 400) }
    for (const needle of expect.must || []) {
      addCheck(checks, `产物含 ${JSON.stringify(needle)}`, text.includes(needle), '包含', '缺失')
    }
    for (const needle of expect.must_not || []) {
      addCheck(checks, `产物不含 ${JSON.stringify(needle)}`, !text.includes(needle), '不包含', '出现了')
    }
    return newRecord(entry, stateOf(checks), checks, observed)
  }

  return newRecord(entry, 'error', checks, {
    ...observed,
    reason: `run=build 的用例不能用 ${expect.kind} 断言`,
  })
}

/// 轮询当前页：serve 刚就绪时 appservice 还没建页，`currentPage()` 会抛
/// `getPageMetaByWebviewId(...) is null`；这里把它当暂时状态。
async function waitForPage(mp, timeoutMs) {
  const deadline = Date.now() + timeoutMs
  for (;;) {
    try {
      const page = await mp.currentPage()
      if (page) return page
    } catch (error) {}
    if (Date.now() >= deadline) throw new Error('等待当前页面就绪超时：appservice 一直没有页面')
    await sleep(500)
  }
}

/// 连 serve、加载页面、等 console 通道握手，可选执行 evaluate。
///
/// 握手日志 `__EVIDENCE_READY_*__` 是「日志通道真的通了」的证据：没有它，日志为空
/// 既可能是「页面没告警」，也可能是「订阅没生效」，后者必须记 error 而不是 pass。
async function drive(entry, dir, args) {
  if (entry.run.kind !== 'serve') throw new Error(`run=${entry.run.kind} 的用例不能走 --drive`)
  const logs = []
  let mp = null
  const saveLogs = () => {
    fs.writeFileSync(path.join(dir, 'console.json'), `${JSON.stringify(logs, null, 2)}\n`)
    fs.writeFileSync(path.join(dir, 'console.txt'), `${logs.map(log => `[${log.type}] ${log.text}`).join('\n')}\n`)
  }
  try {
    if (!args.base) throw new Error('缺少 --base')
    if (!args.sdk) throw new Error('缺少 --sdk（minidev-automator 目录）')
    const automator = require(args.sdk)
    const host = new URL(args.base).host
    mp = await withTimeout(
      automator.connect(`ws://${host}/automator`, { cdpBaseUrl: `${args.base}/browser-cdp` }),
      CONNECT_TIMEOUT_MS,
      '连接 serve ',
    )
    mp.on('console', message => logs.push({ type: message && message.type, text: logText(message) }))

    await waitForPage(mp, RELAUNCH_TIMEOUT_MS)
    // on('console') 会异步补发 enableLog；页面就绪后显式再发一次，确保 reLaunch 之前通道已建立。
    await withTimeout(mp.send('App.enableLog').catch(() => {}), 5000, 'App.enableLog ')

    const route = `/${entry.run.page}`
    await withTimeout(mp.reLaunch(route), RELAUNCH_TIMEOUT_MS, `reLaunch ${route} `)
    const current = await waitForPage(mp, PAGE_TIMEOUT_MS)
    if (current.path !== entry.run.page) {
      throw new Error(`reLaunch 后当前页是 ${current.path}，不是 ${entry.run.page}`)
    }
    await sleep(Number(args.waitMs || 2000))

    const marker = `__EVIDENCE_READY_${process.pid}_${Date.now()}__`
    await withTimeout(
      mp.evaluate(function (value) {
        console.log(value)
      }, marker),
      EVALUATE_TIMEOUT_MS,
      'console 握手 evaluate ',
    )
    const deadline = Date.now() + CONSOLE_MARKER_TIMEOUT_MS
    while (!logs.some(log => log.text.includes(marker))) {
      if (Date.now() > deadline) throw new Error('console 通道没有回传握手日志：App.logAdded 不可用')
      await sleep(100)
    }

    let value
    if (entry.expect.kind === 'runtime') {
      // evaluate 在 appservice 上下文执行，页面可用 getCurrentPages() 拿到。
      value = await withTimeout(
        mp.evaluate(new Function(`return (${entry.expect.expr})`)),
        EVALUATE_TIMEOUT_MS,
        'evaluate ',
      )
    }
    return { logs: logs.slice(), value }
  } catch (error) {
    // 把已捕获的日志附着到错误上：console 通道坏了也能从 report 里看到页面到底打了什么。
    error.logs = logs.slice()
    throw error
  } finally {
    saveLogs()
    if (mp) {
      try {
        await mp.disconnect()
      } catch (error) {}
    }
  }
}

function judgeServe(entry, dir, args) {
  return drive(entry, dir, args).then(
    ({ logs, value }) => {
      const checks = []
      const text = logs.map(log => log.text).join('\n')
      const observed = { logLines: logs.length, logs, value }
      if (entry.expect.kind === 'console') {
        for (const needle of entry.expect.must || []) {
          addCheck(checks, `日志含 ${JSON.stringify(needle)}`, text.includes(needle), '包含', '缺失')
        }
        for (const needle of entry.expect.must_not || []) {
          addCheck(checks, `日志不含 ${JSON.stringify(needle)}`, !text.includes(needle), '不包含', '出现了')
        }
      } else if (entry.expect.kind === 'runtime') {
        const actual = typeof value === 'string' ? value : JSON.stringify(value)
        addCheck(checks, `evaluate 结果等于 ${JSON.stringify(entry.expect.equals)}`, actual === entry.expect.equals, entry.expect.equals, actual)
        observed.valueText = actual
      } else {
        return newRecord(entry, 'error', checks, {
          ...observed,
          reason: `run=serve 的用例不能用 ${entry.expect.kind} 断言`,
        })
      }
      if (checks.length === 0) {
        return newRecord(entry, 'error', checks, { ...observed, reason: '断言为空：用例没有可判定的 must / must_not' })
      }
      return newRecord(entry, stateOf(checks), checks, observed)
    },
    error => {
      const captured = Array.isArray(error && error.logs) ? error.logs : []
      return newRecord(entry, 'error', [], {
        reason: error && error.message ? error.message : String(error),
        logLines: captured.length,
        logs: captured,
      })
    },
  )
}

function resultKey(record) {
  return `${record.rule}\u0000${record.name}`
}

function saveRecord(file, record) {
  const map = new Map()
  const text = readIfExists(file)
  if (text) {
    for (const line of text.split('\n')) {
      const trimmed = line.trim()
      if (!trimmed) continue
      try {
        const item = JSON.parse(trimmed)
        if (item && item.dir) map.set(resultKey(item), item)
      } catch (error) {
        console.error(`[evidence] 忽略 results 里的坏行：${trimmed.slice(0, 120)}`)
      }
    }
  }
  map.set(resultKey(record), record)
  const body = [...map.values()].map(item => JSON.stringify(item)).join('\n')
  const tmp = `${file}.tmp`
  fs.mkdirSync(path.dirname(file), { recursive: true })
  fs.writeFileSync(tmp, body ? `${body}\n` : '')
  fs.renameSync(tmp, file)
  const state = record.state.toUpperCase()
  const detail = record.checks.map(check => `${check.ok ? '✓' : '✗'} ${check.label}`).join('；')
  console.log(`[${state}] ${record.rule} ${record.name}${detail ? ` — ${detail}` : ''}`)
}

function escapeHtml(text) {
  return String(text)
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
}

function describeExpect(entry) {
  const expect = entry.expect
  switch (expect.kind) {
    case 'build-ok':
      return '构建 exit 0 且产出 dist/'
    case 'build-fails':
      return `构建失败（exit ≠ 0），stderr 含 ${JSON.stringify(expect.stderr_contains)}`
    case 'build-log':
      return `构建日志含 ${JSON.stringify(expect.must)}，不含 ${JSON.stringify(expect.must_not)}`
    case 'artifact':
      return `dist/${expect.path} 含 ${JSON.stringify(expect.must)}，不含 ${JSON.stringify(expect.must_not)}`
    case 'console':
      return `页面日志含 ${JSON.stringify(expect.must)}，不含 ${JSON.stringify(expect.must_not)}`
    case 'runtime':
      return `evaluate(${expect.expr}) === ${JSON.stringify(expect.equals)}`
    default:
      return JSON.stringify(expect)
  }
}

function describeRun(entry) {
  return entry.run.kind === 'serve' ? `serve → ${entry.run.page}` : 'build'
}

function stateCounts(records) {
  const counts = { pass: 0, fail: 0, error: 0 }
  for (const record of records) counts[record.state] = (counts[record.state] || 0) + 1
  return counts
}

function sortedRecords(outDir, entries) {
  const map = new Map()
  const text = readIfExists(path.join(outDir, 'results.jsonl'))
  if (text) {
    for (const line of text.split('\n')) {
      const trimmed = line.trim()
      if (!trimmed) continue
      try {
        const item = JSON.parse(trimmed)
        if (item && item.dir) map.set(resultKey(item), item)
      } catch (error) {}
    }
  }
  const order = new Map(entries.map((entry, index) => [resultKey(entry), index]))
  return [...map.values()].sort((a, b) => (order.get(resultKey(a)) ?? 1e9) - (order.get(resultKey(b)) ?? 1e9))
}

function writeReport(file, records) {
  const counts = stateCounts(records)
  const rows = records
    .map(record => {
      const tone = record.state
      const conclusion = tone === 'pass' ? '成立' : tone === 'fail' ? '不成立' : 'error'
      const checks = record.checks.length
        ? `<ul>${record.checks
            .map(
              check =>
                `<li class="${check.ok ? 'ok' : 'bad'}">${check.ok ? '✓' : '✗'} ${escapeHtml(check.label)}${
                  check.ok ? '' : `（期望 ${escapeHtml(check.expected ?? '')}，实际 ${escapeHtml(check.actual ?? '')}）`
                }</li>`,
            )
            .join('')}</ul>`
        : ''
      const observed = record.observed || {}
      const facts = []
      if (observed.exit !== undefined) facts.push(`<li>构建 exit：<code>${observed.exit}</code></li>`)
      if (observed.valueText !== undefined) facts.push(`<li>evaluate 结果：<code>${escapeHtml(observed.valueText)}</code></li>`)
      if (observed.logLines !== undefined) facts.push(`<li>console 行数：${observed.logLines}</li>`)
      if (observed.artifact) {
        facts.push(`<li>产物：<code>${escapeHtml(observed.artifact.path)}</code>（${observed.artifact.bytes} 字节）</li>`)
      }
      const reason = observed.reason ? `<p class="bad">取不到证据：${escapeHtml(observed.reason)}</p>` : ''
      const stderr = observed.stderrTail
        ? `<details><summary>stderr（尾部）</summary><pre>${escapeHtml(tail(observed.stderrTail, REPORT_LOG_CHARS))}</pre></details>`
        : ''
      const logText = (observed.logs || []).map(log => `[${log.type}] ${log.text}`).join('\n')
      const logs = logText
        ? `<details${tone === 'fail' ? ' open' : ''}><summary>console（${(observed.logs || []).length} 行，超 ${REPORT_LOG_CHARS} 字符截断）</summary><pre>${escapeHtml(tail(logText, REPORT_LOG_CHARS))}</pre></details>`
        : ''
      return `<section class="case ${tone}">
        <header><h2>${escapeHtml(record.rule)} <span class="slug">${escapeHtml(record.name)}</span></h2><span class="verdict ${tone}">${conclusion}</span></header>
        <p class="note">${escapeHtml(record.note)}</p>
        <p class="meta">run <code>${escapeHtml(describeRun(record))}</code> · expect <code>${escapeHtml(describeExpect(record))}</code> · 工程 <code>${escapeHtml(record.dir)}</code></p>
        ${reason}${checks}${facts.length ? `<ul>${facts.join('')}</ul>` : ''}${stderr}${logs}
      </section>`
    })
    .join('\n')
  const html = `<!doctype html>
<meta charset="utf-8">
<title>规则证据用例报告</title>
<style>
  :root { color-scheme: light dark; }
  body { margin: 0 auto; max-width: 980px; padding: 24px; font: 14px/1.5 -apple-system, "Helvetica Neue", Arial, sans-serif; }
  h1 { font-size: 20px; margin: 0 0 4px; }
  .summary { color: #666; margin-bottom: 20px; }
  .case { border: 1px solid #ddd; border-radius: 8px; padding: 16px; margin-bottom: 16px; }
  .case.fail { border-color: #e0a0a0; }
  .case.error { border-color: #d08a00; }
  header { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  h2 { font-size: 15px; margin: 0; }
  .slug { color: #888; font-weight: normal; font-size: 12px; }
  .verdict { font-weight: 700; padding: 2px 8px; border-radius: 999px; font-size: 12px; }
  .verdict.pass { background: #e6f6e6; color: #176d17; }
  .verdict.fail { background: #fdeaea; color: #a11; }
  .verdict.error { background: #fff4d6; color: #8a5b00; }
  .note { margin: 8px 0 4px; }
  .meta { color: #666; margin: 0 0 8px; }
  code { background: #f2f2f2; padding: 1px 4px; border-radius: 3px; }
  ul { margin: 6px 0; padding-left: 20px; }
  li.ok { color: #176d17; }
  li.bad, .bad { color: #a11; }
  pre { background: #f7f7f7; padding: 8px; overflow: auto; max-height: 320px; }
  summary { cursor: pointer; margin-top: 8px; }
</style>
<h1>规则证据用例报告</h1>
<p class="summary">${records.length} 个 case：<b>${counts.pass} pass</b> / <b>${counts.fail} fail</b> / <b>${counts.error} error</b></p>
${rows}
`
  fs.mkdirSync(path.dirname(path.resolve(file)), { recursive: true })
  fs.writeFileSync(file, html)
}

function reportOnly(args) {
  const { outDir, cases } = manifest(args)
  const records = sortedRecords(outDir, cases)
  if (!args.report) fatal('缺少参数 --report')
  writeReport(args.report, records)
  const counts = stateCounts(records)
  const missing = cases.filter(entry => !records.some(record => resultKey(record) === resultKey(entry)))
  console.log(
    `[evidence] ${counts.pass} pass / ${counts.fail} fail / ${counts.error} error（共 ${records.length} 条）`,
  )
  if (missing.length) {
    console.log(`[evidence] 没有结果：${missing.map(entry => `${entry.rule}/${entry.name}`).join('、')}`)
  }
  console.log(`[evidence] report: ${args.report}`)
  process.exitCode = counts.fail === 0 && counts.error === 0 && missing.length === 0 ? 0 : 1
}

async function main() {
  const args = parseArgs(process.argv.slice(2))
  if (args.list) {
    const { cases } = manifest(args)
    for (const entry of cases) {
      console.log(`${entry.dir}\t${entry.run.kind}\t${entry.renderer || ''}`)
    }
    return
  }
  if (args.reportOnly) {
    reportOnly(args)
    return
  }
  if (!args.results) fatal('缺少参数 --results')
  const { entry, dir } = loadCase(args)

  let record
  if (args.error) {
    record = newRecord(entry, 'error', [], { reason: args.reason || '未提供原因' })
  } else if (args.judgeBuild) {
    record = judgeBuild(entry, dir)
  } else if (args.drive) {
    record = await judgeServe(entry, dir, args)
  } else {
    fatal('必须指定 --list / --judge-build / --drive / --error / --report 之一')
  }
  saveRecord(args.results, record)
  if (record.state === 'error' && record.observed && record.observed.reason) {
    console.error(`[evidence] ${entry.rule} ${entry.name}：${record.observed.reason}`)
  }
}

main().catch(error => {
  console.error(error && error.stack ? error.stack : error)
  process.exit(2)
})
