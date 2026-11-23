#!/usr/bin/env node
// 规则视觉用例采集与判定。
//
// 由 tools/visual/run.sh 调用；两端各起一个 sim serve（不同 dist、不同端口）：
//   sky = 小程序工程的 renderer: skyline（Electron 宿主）
//   wv  = 同一工程的 renderer: webview（headless Chrome）
//
// 采集：
//   - element 模式：页面上下文 wx.createSelectorQuery().select(target).boundingClientRect()
//   - page 模式（target 为空串）：不查选择器，bbox 视为整张截图——组件内部节点从
//     页面层查不到（选择器不跨组件边界），只能退到整页比对。
//   - 截图：sky 用 `minidev sim shot --frame __pageframe__`（skyline 画布导出；画布尺寸
//           = 窗口逻辑尺寸×dpr，是视口而不是整页）；
//           wv 用 serve 的 CDP 代理 Page.captureScreenshot（先把视口钉成 375x667@2x，
//           与 Electron 宿主窗口一致，否则两端窗口尺寸不同，bbox 没法比）。
//   - 每侧都记截图实际像素尺寸并算 scale（= 截图像素 / 视口 CSS×dpr，横纵各一）。
//     scale 偏离 1 超过 1% 说明导出图不是视口原始分辨率（被缩放或导出成整页 frame），
//     bbox（CSS px）与截图像素不再一一对应，该 case 直接判 error，绝不按错误尺度裁剪。
//
// 判定：
//   element: Differ = bbox 任一边差 >= 4 设备 px 或元素自身区域像素差 >= 1%
//     - rectPixelDiffPctOwn：两侧 bbox 交集（且各自裁进图界）区域，代表目标自身是否不同；
//     - rectPixelDiffPctUnion：两侧 bbox 并集区域（旧口径，兄弟块挪动会污染），仅作参考；
//     - 交集为空（元素完全错位）退回 Union，regionMode="union"。
//   page   : Differ = 整页像素差 >= 1%
// 证据强度：
//   inkPct 是页面区域内非背景像素（与纯白 255,255,255 的通道差 > 8）占比，只有 < 0.01%
//   才算该侧完全空白（blankSide）；判定为 differ 但像素差 < 1%，或两端都 blank 时标
//   weakEvidence，提示该结论要人工复核。contentA/contentB 是旧的内容占比指标，仅作参考
//   记录，不再参与空白判定（稀疏页面会被它误判成白屏）。
// 单 case 失败写入 results 并继续，最终退出码非 0。
//
// 参数：
//   --only a,b,c   只采集指定 slug（逗号分隔）
//   --resume       读取已有 --results，跳过已完成的 slug，结果按 slug 合并回写
//   --retry-failed 配合 --resume：连判定为 fail 的 case 也重跑（默认只重跑 error）
//   --report-only  不采集，只把 --results 渲染成 --report
//   --report-only 可搭配 --cases 按 cases.json 的顺序排序并统计缺失 slug。

'use strict'

const fs = require('fs')
const http = require('http')
const path = require('path')
const { spawnSync } = require('child_process')

const { decodePNG, flattenToWhite, contentRatio } = require('./compare.js')

const BBOX_EDGE_DIFF_DEVICE = 4
const RECT_PIXEL_DIFF_PCT = 1
const PIXEL_TOLERANCE = 1
// 截图分辨率相对视口 CSS×dpr 的允许偏差；超出即认为导出图被缩放，度量不可比。
const SCALE_TOLERANCE = 0.01
// 空白判定：页面区域里与纯白通道差 > 8 的像素视为“画了东西”。
const BACKGROUND_CHANNEL_DELTA = 8
// 低于这个非背景像素占比才算该侧完全没画东西（稀疏页面通常也远高于它）。
const BLANK_INK_PCT = 0.01
// 像素差低于这个值的 differ 结论离 1% 判定线太近，列入 nearThreshold 供人工复核。
const NEAR_THRESHOLD_PCT = 2
const CSS_WIDTH = 375
const CSS_HEIGHT = 667
const DPR = 2
const HEALTH_TIMEOUT_MS = 2000
const CONNECT_TIMEOUT_MS = 15000
const RELAUNCH_TIMEOUT_MS = 20000
const RECT_TIMEOUT_MS = 10000
const SHOT_TIMEOUT_MS = 20000

const BOOLEAN_FLAGS = new Set(['resume', 'retryFailed', 'reportOnly'])

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

function httpStatus(url, timeoutMs) {
  return new Promise(resolve => {
    const req = http.get(url, res => {
      res.resume()
      resolve(res.statusCode || 0)
    })
    req.setTimeout(timeoutMs, () => {
      req.destroy()
      resolve(0)
    })
    req.on('error', () => resolve(0))
  })
}

/// serve 轻量健康检查：优先 /cdp/json/list，兼容只暴露 /cdp/json 的版本。
async function serveHealthy(base) {
  const root = base.replace(/\/+$/, '')
  for (const pathname of ['/cdp/json/list', '/cdp/json']) {
    const status = await httpStatus(root + pathname, HEALTH_TIMEOUT_MS)
    if (status >= 200 && status < 300) return { ok: true }
  }
  return { ok: false }
}

async function checkServes(args) {
  const [sky, wv] = await Promise.all([serveHealthy(args.skyBase), serveHealthy(args.wvBase)])
  if (sky.ok && wv.ok) return { ok: true }
  const down = []
  if (!sky.ok) down.push(`sky ${args.skyBase}`)
  if (!wv.ok) down.push(`wv ${args.wvBase}`)
  return {
    ok: false,
    reason: `serve 不健康：${down.join('、')} 未通过健康检查（GET /cdp/json/list，2s 超时或非 2xx）`,
  }
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

function loadResults(file) {
  const map = new Map()
  if (!fs.existsSync(file)) return map
  for (const line of fs.readFileSync(file, 'utf8').split('\n')) {
    const text = line.trim()
    if (!text) continue
    try {
      const record = JSON.parse(text)
      if (record && record.slug) map.set(record.slug, record)
    } catch (error) {
      console.error(`[visual] 忽略 results 中的坏行：${text.slice(0, 120)}`)
    }
  }
  return map
}

function caseOrder(casesPath) {
  if (!casesPath || !fs.existsSync(casesPath)) return null
  try {
    return JSON.parse(fs.readFileSync(casesPath, 'utf8'))
      .map(testCase => testCase.slug)
      .filter(Boolean)
  } catch (error) {
    return null
  }
}

function orderedRecords(map, casesPath) {
  const order = caseOrder(casesPath)
  if (!order) return [...map.values()]
  return [
    ...order.filter(slug => map.has(slug)).map(slug => map.get(slug)),
    ...[...map.keys()].filter(slug => !order.includes(slug)).map(slug => map.get(slug)),
  ]
}

/// 原子回写，避免进程中断留下半截文件。
function writeResults(file, map, casesPath) {
  const text = orderedRecords(map, casesPath)
    .map(record => JSON.stringify(record))
    .join('\n')
  const tmp = `${file}.tmp`
  fs.writeFileSync(tmp, text ? `${text}\n` : '')
  fs.renameSync(tmp, file)
}

/// 默认只把 error 视为未完成：fail 是有结论的结果，断点续跑不必重跑。
function isComplete(record, retryFailed) {
  if (!record || record.error || record.verdict === 'error') return false
  return retryFailed ? record.ok === true : true
}

function newRecord(testCase, shotsDir) {
  return {
    id: testCase.id,
    slug: testCase.slug,
    page: testCase.page,
    mode: modeOf(testCase),
    target: testCase.target,
    expect: testCase.expect,
    note: testCase.note,
    shots: {
      sky: path.join(shotsDir, `${testCase.slug}-sky.png`),
      wv: path.join(shotsDir, `${testCase.slug}-wv.png`),
    },
  }
}

async function waitPage(mp, timeoutMs = 30000) {
  const deadline = Date.now() + timeoutMs
  for (;;) {
    try {
      const page = await mp.currentPage()
      if (page) return page
    } catch (e) {
      // serve 刚起时 getPageMetaByWebviewId 可能还没就绪，继续轮询。
    }
    if (Date.now() >= deadline) throw new Error('等待当前页面就绪超时')
    await sleep(500)
  }
}

async function rectOf(mp, selector) {
  const rect = await mp.evaluate(function (selector) {
    return new Promise(function (resolve) {
      wx.createSelectorQuery()
        .select(selector)
        .boundingClientRect(function (r) {
          resolve(r || null)
        })
        .exec()
    })
  }, selector)
  if (!rect) throw new Error(`元素 ${selector} 取不到 bbox`)
  return rect
}

async function systemInfo(mp) {
  try {
    return await mp.evaluate(function () {
      const i = wx.getSystemInfoSync()
      return { pixelRatio: i.pixelRatio, windowWidth: i.windowWidth, windowHeight: i.windowHeight }
    })
  } catch (e) {
    return { pixelRatio: DPR, windowWidth: CSS_WIDTH, windowHeight: CSS_HEIGHT }
  }
}

function edge(rect, name) {
  if (typeof rect[name] === 'number') return rect[name]
  if (name === 'right') return rect.left + rect.width
  if (name === 'bottom') return rect.top + rect.height
  throw new Error(`bbox 缺少 ${name}`)
}

function clamp(value, low, high) {
  return Math.min(Math.max(value, low), high)
}

/// CSS bbox → 截图像素坐标：每侧用自己的截图比例，避免把缩放后的图当原尺寸裁。
function pixelRect(rect, img, viewport) {
  const sx = img.width / viewport.width
  const sy = img.height / viewport.height
  return {
    x0: edge(rect, 'left') * sx,
    y0: edge(rect, 'top') * sy,
    x1: edge(rect, 'right') * sx,
    y1: edge(rect, 'bottom') * sy,
  }
}

/// 裁进图界；裁完没有面积时返回 null。
function clampRect(rect, img) {
  const x0 = clamp(Math.floor(rect.x0), 0, img.width)
  const x1 = clamp(Math.ceil(rect.x1), 0, img.width)
  const y0 = clamp(Math.floor(rect.y0), 0, img.height)
  const y1 = clamp(Math.ceil(rect.y1), 0, img.height)
  return x1 > x0 && y1 > y0 ? { x0, x1, y0, y1 } : null
}

function intersectRects(a, b) {
  if (!a || !b) return null
  const rect = {
    x0: Math.max(a.x0, b.x0),
    y0: Math.max(a.y0, b.y0),
    x1: Math.min(a.x1, b.x1),
    y1: Math.min(a.y1, b.y1),
  }
  return rect.x1 > rect.x0 && rect.y1 > rect.y0 ? rect : null
}

function unionRects(a, b) {
  return {
    x0: Math.min(a.x0, b.x0),
    y0: Math.min(a.y0, b.y0),
    x1: Math.max(a.x1, b.x1),
    y1: Math.max(a.y1, b.y1),
  }
}

/// 截图实际的横/纵比例：1 表示导出图就是视口 CSS×dpr 的原分辨率。
function shotScale(img, viewport) {
  return {
    x: img.width / (viewport.width * viewport.dpr),
    y: img.height / (viewport.height * viewport.dpr),
  }
}

function scaleOff(scale) {
  return Math.max(Math.abs(scale.x - 1), Math.abs(scale.y - 1))
}

function round4(value) {
  return Number(value.toFixed(4))
}

function formatScale(scale) {
  return `${scale.x.toFixed(3)}x/${scale.y.toFixed(3)}y`
}

function regionDiff(a, b, rect) {
  if (a.width !== b.width || a.height !== b.height) {
    throw new Error(`两张截图尺寸不一致：${a.width}x${a.height} vs ${b.width}x${b.height}`)
  }
  let x0 = clamp(Math.floor(rect.x0), 0, a.width)
  let x1 = clamp(Math.ceil(rect.x1), 0, a.width)
  let y0 = clamp(Math.floor(rect.y0), 0, a.height)
  let y1 = clamp(Math.ceil(rect.y1), 0, a.height)
  // 越界（元素被页面裁掉）时退化为整页。
  if (x1 <= x0 || y1 <= y0) {
    x0 = 0
    x1 = a.width
    y0 = 0
    y1 = a.height
  }
  let diffPixels = 0
  let maxDelta = 0
  let total = 0
  for (let y = y0; y < y1; y++) {
    for (let x = x0; x < x1; x++) {
      const p = (y * a.width + x) * 4
      let delta = 0
      for (let c = 0; c < 3; c++) {
        delta = Math.max(delta, Math.abs(a.rgba[p + c] - b.rgba[p + c]))
      }
      if (delta > PIXEL_TOLERANCE) {
        diffPixels++
        if (delta > maxDelta) maxDelta = delta
      }
      total++
    }
  }
  return { diffPixels, totalPixels: total, pct: total ? (diffPixels / total) * 100 : 0, maxDelta }
}

function shotSky(minidevBin, baseUrl, outPath) {
  const result = spawnSync(
    minidevBin,
    ['sim', 'shot', '--url', baseUrl, '--frame', '__pageframe__', '--out', outPath],
    { encoding: 'utf8', timeout: 30000, maxBuffer: 8 * 1024 * 1024 },
  )
  if (result.error) throw new Error(`sim shot 执行失败：${result.error.message}`)
  if (result.status !== 0) {
    throw new Error(
      `sim shot 失败（exit ${result.status}）：${String(result.stderr || result.stdout || '').slice(-400)}`,
    )
  }
  if (!fs.existsSync(outPath)) throw new Error(`sim shot 未产出 ${outPath}`)
}

function isDiffer(edgeDiffDevice, pixelPct) {
  return edgeDiffDevice >= BBOX_EDGE_DIFF_DEVICE || pixelPct >= RECT_PIXEL_DIFF_PCT
}

function verdict(expect, edgeDiffDevice, pixelPct) {
  return expect === 'differ' ? isDiffer(edgeDiffDevice, pixelPct) : !isDiffer(edgeDiffDevice, pixelPct)
}

/// 区域内的非背景像素占比（0~1）：空白判据，不受“稀疏页只占很小面积”影响。
function inkRatio(img, region) {
  const x0 = clamp(Math.floor(region.x0), 0, img.width)
  const x1 = clamp(Math.ceil(region.x1), 0, img.width)
  const y0 = clamp(Math.floor(region.y0), 0, img.height)
  const y1 = clamp(Math.ceil(region.y1), 0, img.height)
  let ink = 0
  let total = 0
  for (let y = y0; y < y1; y++) {
    for (let x = x0; x < x1; x++) {
      const p = (y * img.width + x) * 4
      const delta = Math.max(
        255 - img.rgba[p],
        255 - img.rgba[p + 1],
        255 - img.rgba[p + 2],
      )
      if (delta > BACKGROUND_CHANNEL_DELTA) ink++
      total++
    }
  }
  return total ? ink / total : 0
}

/// 旧 cases.json 没有 mode：空 target 就是整页，其余按元素。
function modeOf(testCase) {
  if (testCase.mode === 'page' || testCase.mode === 'element') return testCase.mode
  return testCase.target ? 'element' : 'page'
}

function escapeHtml(text) {
  return String(text)
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
}

function imageTag(file) {
  if (!file || !fs.existsSync(file)) return '<div class="missing">无截图</div>'
  const data = fs.readFileSync(file).toString('base64')
  return `<img src="data:image/png;base64,${data}" alt="">`
}

/// 该侧是否完全没画东西：ink 占比低于阈值才算空白，稀疏页不算。
function blankSideOf(inkPctSky, inkPctWv) {
  const blankA = inkPctSky < BLANK_INK_PCT
  const blankB = inkPctWv < BLANK_INK_PCT
  return blankA && blankB ? 'both' : blankA ? 'a' : blankB ? 'b' : null
}

/// differ 结论只靠 bbox 边差（区域像素差 < 1%）时，证据偏弱。
function weakByPixels(edgeDiffDevice, pixelPct) {
  return isDiffer(edgeDiffDevice, pixelPct) && pixelPct < RECT_PIXEL_DIFF_PCT
}

function stateOf(record) {
  if (record.error) return 'error'
  if (record.verdict) return record.verdict
  return record.ok ? 'pass' : 'fail'
}

function tally(records) {
  const counts = { pass: 0, fail: 0, error: 0 }
  for (const record of records) counts[stateOf(record)]++
  return counts
}

/// expect=differ 但像素差离 1% 判定线太近（< 2%）的用例，供人工复核。
function nearThreshold(records) {
  return records
    .filter(
      record =>
        record.expect === 'differ' &&
        !record.error &&
        typeof record.rectPixelDiffPct === 'number' &&
        record.rectPixelDiffPct < NEAR_THRESHOLD_PCT,
    )
    .sort((a, b) => a.rectPixelDiffPct - b.rectPixelDiffPct)
}

function formatPct(value, digits) {
  return typeof value === 'number' ? `${value.toFixed(digits)}%` : '—'
}

/// 早期记录没有 inkPct：按存档截图在内存里补算（page 整页）。读不到就留空，不影响报告。
function recomputedPageInk(record) {
  try {
    const shots = record.shots || {}
    const inkPct = file => {
      const img = flattenToWhite(decodePNG(fs.readFileSync(file)))
      return Number((inkRatio(img, { x0: 0, y0: 0, x1: img.width, y1: img.height }) * 100).toFixed(3))
    }
    const inkPctSky = inkPct(shots.sky)
    const inkPctWv = inkPct(shots.wv)
    const blankSide = blankSideOf(inkPctSky, inkPctWv)
    return {
      inkPctSky,
      inkPctWv,
      blankSide,
      weakEvidence:
        weakByPixels(record.bboxEdgeDiffDevice || 0, record.rectPixelDiffPct || 0) ||
        blankSide === 'both',
    }
  } catch (error) {
    console.error(`[visual] ${record.slug} 无法从截图补算 inkPct：${error.message}`)
    return {}
  }
}

/// 只读旧记录：内存里回填 verdict、补算 page inkPct，不改写 results.jsonl。
function reportRecords(records) {
  return records.map(record => {
    let next = record.verdict
      ? record
      : { ...record, verdict: record.error ? 'error' : record.ok ? 'pass' : 'fail' }
    if (next.mode === 'page' && !next.error && typeof next.inkPctSky !== 'number') {
      next = { ...next, ...recomputedPageInk(next) }
    }
    return next
  })
}

function printSummary(records, reportPath) {
  const counts = tally(records)
  console.log(
    `[visual] ${counts.pass} pass / ${counts.fail} fail / ${counts.error} error（共 ${records.length} 条）`,
  )
  const near = nearThreshold(records)
  console.log(`[visual] nearThreshold（expect=differ 且像素差 < ${NEAR_THRESHOLD_PCT}%）：${near.length} 条`)
  for (const record of near) {
    console.log(`  - ${record.slug} ${record.rectPixelDiffPct.toFixed(3)}% (${stateOf(record)})`)
  }
  if (reportPath) console.log(`[visual] report: ${reportPath}`)
  return counts
}

function writeReport(reportPath, records) {
  const rows = records
    .map(record => {
      const state = stateOf(record)
      const conclusion = state === 'error' ? 'error' : state === 'pass' ? '成立' : '不成立'
      const tone = state
      const blankNote = record.blankSide
        ? `<li${record.blankSide === 'both' ? ' class="error"' : ''}>空白侧：<b>${record.blankSide}</b>（${record.blankSide === 'both' ? '两端' : record.blankSide === 'a' ? 'Skyline' : 'WebView'}完全没画东西）</li>`
        : ''
      const weakNote = record.weakEvidence
        ? '<li class="error">证据偏弱：differ 但差异像素 < 1%，或两端完全空白，结论需人工复核</li>'
        : ''
      const scaleNote =
        record.viewport && record.shotSky && record.shotWv
          ? `<li>截图：Skyline <b>${record.shotSky.width}x${record.shotSky.height}</b>（scale ${record.scaleSky}x/${record.scaleSkyY}y） / WebView <b>${record.shotWv.width}x${record.shotWv.height}</b>（scale ${record.scaleWv}x/${record.scaleWvY}y），视口 ${record.viewport.width}x${record.viewport.height}@${record.viewport.dpr}</li>`
          : ''
      const ownPct =
        typeof record.rectPixelDiffPctOwn === 'number'
          ? record.rectPixelDiffPctOwn
          : record.rectPixelDiffPct
      const unionPct =
        typeof record.rectPixelDiffPctUnion === 'number'
          ? record.rectPixelDiffPctUnion
          : record.rectPixelDiffPct
      const regionMode = record.regionMode || (record.mode === 'page' ? 'page' : 'union（旧口径）')
      const metrics = record.error
        ? `<p class="error">${escapeHtml(record.error)}</p>`
        : record.mode === 'page'
          ? `<ul>
              <li>整页像素差：<b>${record.rectPixelDiffPct.toFixed(3)}%</b>（${record.diffPixels}/${record.totalPixels} px，阈值 ${RECT_PIXEL_DIFF_PCT}%）</li>
              <li>非背景像素占比：Skyline <b>${formatPct(record.inkPctSky, 3)}</b> / WebView <b>${formatPct(record.inkPctWv, 3)}</b>（低于 ${BLANK_INK_PCT}% 才算该侧完全空白）</li>
              ${blankNote}
              ${weakNote}
              ${scaleNote}
              <li>最大通道差：${record.maxChannelDelta}</li>
            </ul>`
          : `<ul>
              <li>bbox 边差：<b>${record.bboxEdgeDiffDevice.toFixed(1)}</b> 设备 px（阈值 ${BBOX_EDGE_DIFF_DEVICE}）</li>
              <li>自身区域像素差 Own（判定）：<b>${ownPct.toFixed(3)}%</b>（${record.diffPixels}/${record.totalPixels} px，阈值 ${RECT_PIXEL_DIFF_PCT}%）</li>
              <li>并集区域像素差 Union（参考）：<b>${unionPct.toFixed(3)}%</b></li>
              <li>区域口径：<code>${escapeHtml(regionMode)}</code>${regionMode === 'union' ? '（两侧 bbox 无交集，退回并集）' : ''}</li>
              ${weakNote}
              ${scaleNote}
              <li>最大通道差：${record.maxChannelDelta}</li>
            </ul>`
      return `<section class="case ${tone}">
        <header>
          <h2>${escapeHtml(record.id)} <span class="slug">${escapeHtml(record.slug)}</span></h2>
          <span class="verdict ${tone}">${conclusion}</span>
        </header>
        <p class="note">${escapeHtml(record.note)}</p>
        <p class="meta">mode <code>${escapeHtml(record.mode)}</code> · target <code>${escapeHtml(record.target)}</code> · expect <code>${escapeHtml(record.expect)}</code> · page <code>${escapeHtml(record.page)}</code></p>
        ${metrics}
        <div class="shots">
          <figure><figcaption>Skyline</figcaption>${imageTag(record.shots && record.shots.sky)}</figure>
          <figure><figcaption>WebView</figcaption>${imageTag(record.shots && record.shots.wv)}</figure>
        </div>
        <details><summary>bbox</summary><pre>${escapeHtml(
          JSON.stringify({ sky: record.bboxSky, wv: record.bboxWv }, null, 2),
        )}</pre></details>
      </section>`
    })
    .join('\n')
  const counts = tally(records)
  const near = nearThreshold(records)
  const nearRows = near
    .map(
      record =>
        `<li><code>${escapeHtml(record.slug)}</code> ${record.rectPixelDiffPct.toFixed(3)}%（${stateOf(record)}）</li>`,
    )
    .join('')
  const html = `<!doctype html>
<meta charset="utf-8">
<title>规则视觉用例报告</title>
<style>
  :root { color-scheme: light dark; }
  body { margin: 0 auto; max-width: 980px; padding: 24px; font: 14px/1.5 -apple-system, "Helvetica Neue", Arial, sans-serif; }
  h1 { font-size: 20px; margin: 0 0 4px; }
  .summary { color: #666; margin-bottom: 20px; }
  .near { border: 1px solid #e0c080; border-radius: 8px; padding: 4px 16px; margin-bottom: 20px; }
  .near h2 { font-size: 14px; }
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
  .shots { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; margin-top: 10px; }
  figure { margin: 0; }
  figcaption { font-size: 12px; color: #666; margin-bottom: 4px; }
  img { width: 100%; border: 1px solid #ddd; border-radius: 4px; display: block; }
  .missing { border: 1px dashed #ccc; padding: 24px; text-align: center; color: #999; }
  .error { color: #a11; }
  pre { background: #f7f7f7; padding: 8px; overflow: auto; }
</style>
<h1>规则视觉用例报告</h1>
<p class="summary">${records.length} 个 case：<b>${counts.pass} pass</b> / <b>${counts.fail} fail</b> / <b>${counts.error} error</b></p>
${
  near.length
    ? `<section class="near"><h2>nearThreshold（expect=differ 且像素差 < ${NEAR_THRESHOLD_PCT}%）</h2><ul>${nearRows}</ul></section>`
    : ''
}
${rows}
`
  fs.writeFileSync(reportPath, html)
}

async function collectCase(testCase, record, ctx) {
  const { sky, wv, wvCdp, minidevBin, skyBase, waitMs, cssWidth, cssHeight, dpr } = ctx
  const mode = record.mode
  const pagePath = `/${testCase.page}`

  await withTimeout(sky.reLaunch(pagePath), RELAUNCH_TIMEOUT_MS, `sky reLaunch ${pagePath} `)
  await sleep(waitMs)
  const skyRect =
    mode === 'element'
      ? await withTimeout(rectOf(sky, testCase.target), RECT_TIMEOUT_MS, `sky 取 bbox(${testCase.target}) `)
      : null
  shotSky(minidevBin, skyBase, record.shots.sky)

  await withTimeout(wv.reLaunch(pagePath), RELAUNCH_TIMEOUT_MS, `wv reLaunch ${pagePath} `)
  await sleep(waitMs)
  const wvRect =
    mode === 'element'
      ? await withTimeout(rectOf(wv, testCase.target), RECT_TIMEOUT_MS, `wv 取 bbox(${testCase.target}) `)
      : null
  fs.writeFileSync(
    record.shots.wv,
    await withTimeout(wvCdp.captureScreenshot(), SHOT_TIMEOUT_MS, 'wv 截图 '),
  )

  const imgSky = flattenToWhite(decodePNG(fs.readFileSync(record.shots.sky)))
  const imgWv = flattenToWhite(decodePNG(fs.readFileSync(record.shots.wv)))
  const viewport = { width: cssWidth, height: cssHeight, dpr }
  record.viewport = viewport
  record.shotSky = { width: imgSky.width, height: imgSky.height }
  record.shotWv = { width: imgWv.width, height: imgWv.height }

  const scaleSky = shotScale(imgSky, viewport)
  const scaleWv = shotScale(imgWv, viewport)
  record.scaleSky = round4(scaleSky.x)
  record.scaleSkyY = round4(scaleSky.y)
  record.scaleWv = round4(scaleWv.x)
  record.scaleWvY = round4(scaleWv.y)
  if (scaleOff(scaleSky) > SCALE_TOLERANCE || scaleOff(scaleWv) > SCALE_TOLERANCE) {
    // 导出图不是视口原始分辨率：bbox 与像素不再一一对应，任何裁剪都是错的。
    record.ok = false
    record.verdict = 'error'
    record.error =
      '截图被缩放，度量不可比：' +
      `Skyline ${imgSky.width}x${imgSky.height}（scale ${formatScale(scaleSky)}） / ` +
      `WebView ${imgWv.width}x${imgWv.height}（scale ${formatScale(scaleWv)}），` +
      `视口 ${cssWidth}x${cssHeight}@${dpr}，截图边缘与视口 CSS×dpr 差超过 ${SCALE_TOLERANCE * 100}%`
    return
  }
  if (imgSky.width !== imgWv.width || imgSky.height !== imgWv.height) {
    throw new Error(
      `两张截图尺寸不一致：${imgSky.width}x${imgSky.height} vs ${imgWv.width}x${imgWv.height}`,
    )
  }

  let region
  if (mode === 'page') {
    // 整页比对：bbox 就是整张截图，没有元素几何可比。
    region = { x0: 0, y0: 0, x1: imgSky.width, y1: imgSky.height }
    record.bboxSky = null
    record.bboxWv = null
    record.bboxEdgeDiffDevice = 0
    record.contentA = Number((contentRatio(imgSky) * 100).toFixed(2))
    record.contentB = Number((contentRatio(imgWv) * 100).toFixed(2))
  } else {
    const edges = ['left', 'top', 'right', 'bottom']
    const edgeDiffCss = Math.max(
      ...edges.map(name => Math.abs(edge(skyRect, name) - edge(wvRect, name))),
    )
    record.bboxSky = skyRect
    record.bboxWv = wvRect
    record.bboxEdgeDiffDevice = edgeDiffCss * dpr

    // 旧口径：两侧 bbox 并集，兄弟块挪进并集也会被计入像素差。
    const union = unionRects(
      pixelRect(skyRect, imgSky, viewport),
      pixelRect(wvRect, imgWv, viewport),
    )
    // 新口径：各自 bbox 先裁进自己的图界，再取交集——只比目标自身重叠的部分。
    const own = intersectRects(
      clampRect(pixelRect(skyRect, imgSky, viewport), imgSky),
      clampRect(pixelRect(wvRect, imgWv, viewport), imgWv),
    )
    const unionDiff = regionDiff(imgSky, imgWv, union)
    const ownDiff = own ? regionDiff(imgSky, imgWv, own) : unionDiff
    record.rectPixelDiffPctUnion = unionDiff.pct
    record.rectPixelDiffPctOwn = ownDiff.pct
    record.unionDiffPixels = unionDiff.diffPixels
    record.unionTotalPixels = unionDiff.totalPixels
    // 判定用 Own；元素完全错位（交集为空）时退回 Union 并标注。
    region = own || union
    record.regionMode = own ? 'own' : 'union'
    record.diffPixels = ownDiff.diffPixels
    record.totalPixels = ownDiff.totalPixels
    record.maxChannelDelta = ownDiff.maxDelta
  }

  if (mode === 'page') {
    const diff = regionDiff(imgSky, imgWv, region)
    record.rectPixelDiffPct = diff.pct
    record.rectPixelDiffPctOwn = diff.pct
    record.rectPixelDiffPctUnion = diff.pct
    record.regionMode = 'page'
    record.diffPixels = diff.diffPixels
    record.totalPixels = diff.totalPixels
    record.maxChannelDelta = diff.maxDelta
  } else {
    record.rectPixelDiffPct = record.rectPixelDiffPctOwn
  }
  record.region = region
  if (mode === 'page') {
    record.inkPctSky = Number((inkRatio(imgSky, region) * 100).toFixed(3))
    record.inkPctWv = Number((inkRatio(imgWv, region) * 100).toFixed(3))
    record.blankSide = blankSideOf(record.inkPctSky, record.inkPctWv)
  } else {
    record.blankSide = null
  }
  record.ok = verdict(testCase.expect, record.bboxEdgeDiffDevice, record.rectPixelDiffPct)
  record.weakEvidence =
    weakByPixels(record.bboxEdgeDiffDevice, record.rectPixelDiffPct) || record.blankSide === 'both'
}

function reportOnly(args) {
  const results = loadResults(args.results)
  const order = caseOrder(args.cases)
  const records = reportRecords(orderedRecords(results, args.cases))
  writeReport(args.report, records)
  if (order) {
    const missing = order.filter(slug => !results.has(slug))
    console.log(`[visual] results 覆盖 ${order.length - missing.length}/${order.length} 个 slug`)
    if (missing.length) console.log(`[visual] 缺失：${missing.join(',')}`)
  } else {
    console.log(`[visual] results 共 ${records.length} 条`)
  }
  const counts = printSummary(records, args.report)
  process.exitCode = counts.fail === 0 && counts.error === 0 ? 0 : 1
}

async function main() {
  const args = parseArgs(process.argv.slice(2))
  if (!args.results) throw new Error('缺少参数 --results')
  if (args.reportOnly) {
    if (!args.report) throw new Error('缺少参数 --report')
    reportOnly(args)
    return
  }
  for (const [name, flag] of Object.entries({
    skyBase: '--sky-base',
    wvBase: '--wv-base',
    cases: '--cases',
    shots: '--shots',
  })) {
    if (!args[name]) throw new Error(`缺少参数 ${flag}`)
  }

  const waitMs = Number(args.waitMs || 8000)
  const retryFailed = Boolean(args.retryFailed)
  const minidevBin = args.minidev || process.env.MINIDEV_BIN || 'minidev'
  const sdk = args.sdk || process.env.MINIDEV_SDK
  if (!sdk) throw new Error('缺少 MINIDEV_SDK（minidev 的 sdk/minidev-automator 目录）')

  const cases = JSON.parse(fs.readFileSync(args.cases, 'utf8'))
  let selected = cases
  if (args.only) {
    const wanted = new Set(
      String(args.only)
        .split(',')
        .map(slug => slug.trim())
        .filter(Boolean),
    )
    const known = new Set(cases.map(testCase => testCase.slug))
    const unknown = [...wanted].filter(slug => !known.has(slug))
    if (unknown.length) console.error(`[visual] --only 里有未知 slug：${unknown.join(',')}`)
    selected = cases.filter(testCase => wanted.has(testCase.slug))
  }

  fs.mkdirSync(args.shots, { recursive: true })
  fs.mkdirSync(path.dirname(args.results), { recursive: true })
  const results = args.resume ? loadResults(args.results) : new Map()
  const todo = args.resume
    ? selected.filter(testCase => !isComplete(results.get(testCase.slug), retryFailed))
    : selected
  if (args.resume && todo.length < selected.length) {
    console.log(`[visual] 断点续跑：跳过 ${selected.length - todo.length} 个已完成 case`)
  }
  if (todo.length === 0) {
    console.log('[visual] 没有需要采集的 case')
    if (args.report) writeReport(args.report, reportRecords(orderedRecords(results, args.cases)))
    process.exitCode = 0
    return
  }

  const health = await checkServes(args)
  if (!health.ok) {
    // serve 挂了不阻塞整批：逐页写 error，由上层重启 serve 后按 --resume 重跑。
    for (const testCase of todo) {
      const record = newRecord(testCase, args.shots)
      record.ok = false
      record.verdict = 'error'
      record.error = health.reason
      results.set(record.slug, record)
      console.log(`[ERROR] ${record.id} ${record.slug} ${health.reason}`)
    }
    writeResults(args.results, results, args.cases)
    if (args.report) writeReport(args.report, reportRecords(orderedRecords(results, args.cases)))
    process.exitCode = 1
    return
  }

  const automator = require(sdk)
  const { CdpSession } = require(path.join(sdk, 'framework', 'cdp.js'))

  const sky = await withTimeout(
    automator.connect(`ws://${new URL(args.skyBase).host}/automator`, {
      cdpBaseUrl: `${args.skyBase}/cdp`,
    }),
    CONNECT_TIMEOUT_MS,
    '连接 skyline serve ',
  )
  const wv = await withTimeout(
    automator.connect(`ws://${new URL(args.wvBase).host}/automator`, {
      cdpBaseUrl: `${args.wvBase}/cdp`,
    }),
    CONNECT_TIMEOUT_MS,
    '连接 webview serve ',
  )
  const wvCdp = await withTimeout(
    CdpSession.connect(`${args.wvBase}/cdp`),
    CONNECT_TIMEOUT_MS,
    '连接 webview CDP ',
  )
  // 两端窗口必须同尺寸：Electron 宿主是 375x667@2x；Chrome 缺省抓到的视口不是这个，
  // 用 CDP 覆盖成同一组设备指标，否则 bbox 的宽度/右边差本身就会造成误判。
  await wvCdp.send('Emulation.setDeviceMetricsOverride', {
    width: CSS_WIDTH,
    height: CSS_HEIGHT,
    deviceScaleFactor: DPR,
    mobile: false,
  })

  for (const [label, wait] of [
    ['skyline', () => waitPage(sky)],
    ['webview', () => waitPage(wv)],
  ]) {
    try {
      await withTimeout(wait(), 30000, `等待 ${label} 页面就绪 `)
    } catch (error) {
      console.error(`[visual] ${error.message}，继续按页采集`)
    }
  }
  const info = await systemInfo(sky)
  const ctx = {
    sky,
    wv,
    wvCdp,
    minidevBin,
    skyBase: args.skyBase,
    waitMs,
    cssWidth: info.windowWidth || CSS_WIDTH,
    cssHeight: info.windowHeight || CSS_HEIGHT,
    dpr: info.pixelRatio || DPR,
  }

  for (const testCase of todo) {
    const record = newRecord(testCase, args.shots)
    const pageHealth = await checkServes(args)
    if (!pageHealth.ok) {
      record.ok = false
      record.verdict = 'error'
      record.error = pageHealth.reason
    } else {
      try {
        await collectCase(testCase, record, ctx)
        record.verdict = record.verdict === 'error' ? 'error' : record.ok ? 'pass' : 'fail'
      } catch (error) {
        record.ok = false
        record.verdict = 'error'
        record.error = error && error.message ? error.message : String(error)
      }
    }
    results.set(record.slug, record)
    writeResults(args.results, results, args.cases)
    const status = record.verdict === 'error' ? 'ERROR' : record.ok ? 'ok' : 'FAIL'
    const detail =
      record.verdict === 'error'
        ? record.error
        : record.mode === 'page'
          ? `pagePixel=${record.rectPixelDiffPct.toFixed(3)}% ink=${formatPct(record.inkPctSky, 3)}/${formatPct(record.inkPctWv, 3)}${record.blankSide ? ` blank=${record.blankSide}` : ''}`
          : `bboxEdge=${record.bboxEdgeDiffDevice.toFixed(1)}px own=${record.rectPixelDiffPctOwn.toFixed(3)}% union=${record.rectPixelDiffPctUnion.toFixed(3)}% region=${record.regionMode}`
    console.log(`[${status}] ${record.id} ${record.slug} (${record.expect}, ${record.mode}) ${detail}`)
  }

  for (const close of [() => sky.disconnect(), () => wv.disconnect(), () => wvCdp.close()]) {
    try {
      await close()
    } catch (error) {}
  }

  const all = orderedRecords(results, args.cases)
  if (args.report) writeReport(args.report, reportRecords(all))
  const counts = printSummary(all, args.report)
  process.exitCode = counts.fail === 0 && counts.error === 0 ? 0 : 1
}

main().catch(error => {
  console.error(error && error.stack ? error.stack : error)
  process.exit(2)
})
