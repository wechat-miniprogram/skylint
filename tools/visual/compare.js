#!/usr/bin/env node
// 像素级截图对比，零依赖（Node 标准库 zlib）。
// 用法:
//   node tools/shots/compare.js <a.png> <b.png> [--threshold 0.5] [--tolerance 1] [--diff-out diff.png] [--json]
// 阈值: --threshold 为差异像素占全图百分比（默认 0.5%）；--tolerance 为忽略的每通道色差（默认 1，用于抗锯齿）。
// 退出码: 0 通过；2 超过阈值；1 参数/解码错误。
const fs = require('fs');
const zlib = require('zlib');

const PNG_SIG = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);

function decodePNG(buf) {
  if (!buf.subarray(0, 8).equals(PNG_SIG)) throw new Error('不是 PNG 文件');
  let off = 8;
  let ihdr = null;
  let plte = null;
  let trns = null;
  const idat = [];
  while (off < buf.length) {
    const len = buf.readUInt32BE(off);
    const type = buf.toString('ascii', off + 4, off + 8);
    const data = buf.subarray(off + 8, off + 8 + len);
    if (type === 'IHDR') {
      ihdr = {
        width: data.readUInt32BE(0),
        height: data.readUInt32BE(4),
        bitDepth: data[8],
        colorType: data[9],
        interlace: data[12],
      };
    } else if (type === 'PLTE') plte = data;
    else if (type === 'tRNS') trns = data;
    else if (type === 'IDAT') idat.push(data);
    else if (type === 'IEND') break;
    off += 12 + len;
  }
  if (!ihdr) throw new Error('缺少 IHDR');
  const { width, height, bitDepth, colorType, interlace } = ihdr;
  if (bitDepth !== 8) throw new Error(`只支持 8-bit PNG（当前 ${bitDepth}）`);
  if (interlace !== 0) throw new Error('不支持 interlaced PNG');
  const channels = { 0: 1, 2: 3, 3: 1, 4: 2, 6: 4 }[colorType];
  if (!channels) throw new Error(`不支持的 colorType ${colorType}`);
  const raw = zlib.inflateSync(Buffer.concat(idat));
  const stride = width * channels;
  const out = Buffer.alloc(stride * height);
  let pos = 0;
  for (let y = 0; y < height; y++) {
    const filter = raw[pos++];
    const line = raw.subarray(pos, pos + stride);
    pos += stride;
    const prev = y > 0 ? out.subarray((y - 1) * stride, y * stride) : null;
    const cur = out.subarray(y * stride, (y + 1) * stride);
    for (let x = 0; x < stride; x++) {
      const a = x >= channels ? cur[x - channels] : 0;
      const b = prev ? prev[x] : 0;
      const c = prev && x >= channels ? prev[x - channels] : 0;
      let v = line[x];
      switch (filter) {
        case 0: break;
        case 1: v += a; break;
        case 2: v += b; break;
        case 3: v += (a + b) >> 1; break;
        case 4: {
          const p = a + b - c;
          const pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c);
          v += pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
          break;
        }
        default: throw new Error(`未知行过滤器 ${filter}`);
      }
      cur[x] = v & 0xff;
    }
  }
  // 统一成 RGBA
  const rgba = Buffer.alloc(width * height * 4);
  for (let i = 0, p = 0; i < width * height; i++, p += channels) {
    let r, g, b, a = 255;
    if (colorType === 0) { r = g = b = out[p]; }
    else if (colorType === 2) { r = out[p]; g = out[p + 1]; b = out[p + 2]; }
    else if (colorType === 3) {
      const idx = out[p];
      if (!plte) throw new Error('调色板 PNG 缺少 PLTE');
      r = plte[idx * 3]; g = plte[idx * 3 + 1]; b = plte[idx * 3 + 2];
      if (trns && idx < trns.length) a = trns[idx];
    } else if (colorType === 4) { r = g = b = out[p]; a = out[p + 1]; }
    else { r = out[p]; g = out[p + 1]; b = out[p + 2]; a = out[p + 3]; }
    rgba[i * 4] = r; rgba[i * 4 + 1] = g;
    rgba[i * 4 + 2] = b; rgba[i * 4 + 3] = a;
  }
  return { width, height, rgba };
}

function encodePNG({ width, height, rgba }) {
  const stride = width * 4;
  const raw = Buffer.alloc((stride + 1) * height);
  for (let y = 0; y < height; y++) {
    raw[y * (stride + 1)] = 0;
    rgba.copy(raw, y * (stride + 1) + 1, y * stride, (y + 1) * stride);
  }
  const chunk = (type, data) => {
    const len = Buffer.alloc(4); len.writeUInt32BE(data.length);
    const body = Buffer.concat([Buffer.from(type, 'ascii'), data]);
    const crc = Buffer.alloc(4); crc.writeUInt32BE(crc32(body) >>> 0);
    return Buffer.concat([len, body, crc]);
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0); ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8; ihdr[9] = 6; ihdr[10] = 0; ihdr[11] = 0; ihdr[12] = 0;
  return Buffer.concat([PNG_SIG, chunk('IHDR', ihdr), chunk('IDAT', zlib.deflateSync(raw)), chunk('IEND', Buffer.alloc(0))]);
}

let crcTable = null;
function crc32(buf) {
  if (!crcTable) {
    crcTable = [];
    for (let n = 0; n < 256; n++) {
      let c = n;
      for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
      crcTable[n] = c >>> 0;
    }
  }
  let c = 0xffffffff;
  for (const b of buf) c = crcTable[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}


// 把带 alpha 的图像合成到白底：Skyline 画布导出的背景是 alpha=0，
// 直接逐通道比会把「透明 vs 白」算成 255 的差异。alpha 通道本身不参与比较。
function flattenToWhite(img) {
  const { width, height, rgba } = img;
  const out = Buffer.alloc(width * height * 4);
  for (let i = 0; i < width * height; i++) {
    const p = i * 4;
    const a = rgba[p + 3];
    if (a === 255) {
      out[p] = rgba[p]; out[p + 1] = rgba[p + 1]; out[p + 2] = rgba[p + 2];
    } else {
      const f = a / 255;
      out[p] = Math.round(rgba[p] * f + 255 * (1 - f));
      out[p + 1] = Math.round(rgba[p + 1] * f + 255 * (1 - f));
      out[p + 2] = Math.round(rgba[p + 2] * f + 255 * (1 - f));
    }
    out[p + 3] = 255;
  }
  return { width, height, rgba: out };
}

function compare(a, b, tolerance) {
  a = flattenToWhite(a);
  b = flattenToWhite(b);
  if (a.width !== b.width || a.height !== b.height) {
    return { dimensionMismatch: true, pass: false, diffPixels: Math.max(a.width * a.height, b.width * b.height), totalPixels: Math.max(a.width * a.height, b.width * b.height), ratioPct: 100, bbox: null, maxDelta: 255 };
  }
  const total = a.width * a.height;
  let diff = 0, maxDelta = 0;
  let x0 = Infinity, y0 = Infinity, x1 = -1, y1 = -1;
  const heat = Buffer.alloc(total * 4);
  for (let i = 0; i < total; i++) {
    const pa = a.rgba.subarray(i * 4, i * 4 + 4);
    const pb = b.rgba.subarray(i * 4, i * 4 + 4);
    let d = 0;
    for (let c = 0; c < 4; c++) d = Math.max(d, Math.abs(pa[c] - pb[c]));
    if (d > tolerance) {
      diff++;
      if (d > maxDelta) maxDelta = d;
      const x = i % a.width, y = (i / a.width) | 0;
      if (x < x0) x0 = x; if (y < y0) y0 = y;
      if (x > x1) x1 = x; if (y > y1) y1 = y;
      heat[i * 4] = 255; heat[i * 4 + 1] = 0; heat[i * 4 + 2] = 0; heat[i * 4 + 3] = 255;
    } else {
      heat[i * 4] = pa[0]; heat[i * 4 + 1] = pa[1]; heat[i * 4 + 2] = pa[2]; heat[i * 4 + 3] = 255;
    }
  }
  return {
    dimensionMismatch: false,
    width: a.width, height: a.height,
    diffPixels: diff, totalPixels: total,
    ratioPct: (diff / total) * 100,
    bbox: diff ? { x0, y0, x1, y1, width: x1 - x0 + 1, height: y1 - y0 + 1 } : null,
    maxDelta,
    heat,
  };
}

function contentRatio(img) {
  img = flattenToWhite(img);
  // 正文区内容占比：取导航以下、底部安全区以上的区域，统计偏离该区主背景色的像素比例。
  const { width, height, rgba } = img;
  const x0 = Math.floor(width * 0.04), x1 = Math.floor(width * 0.96);
  const y0 = Math.floor(height * 0.16), y1 = Math.floor(height * 0.94);
  if (x1 <= x0 || y1 <= y0) return 0;
  const hist = new Map();
  const at = (x, y) => (y * width + x) * 4;
  for (let y = y0; y < y1; y += 2) for (let x = x0; x < x1; x += 2) {
    const i = at(x, y);
    const k = ((rgba[i] >> 4) << 8) | ((rgba[i + 1] >> 4) << 4) | (rgba[i + 2] >> 4);
    const cur = hist.get(k);
    if (cur) cur.n++;
    else hist.set(k, { n: 1, rgb: [rgba[i], rgba[i + 1], rgba[i + 2]] });
  }
  let bg = [255, 255, 255], bgCount = -1;
  for (const v of hist.values()) if (v.n > bgCount) { bgCount = v.n; bg = v.rgb; }
  let content = 0, total = 0;
  for (let y = y0; y < y1; y += 2) for (let x = x0; x < x1; x += 2) {
    const i = at(x, y);
    total++;
    const d = Math.abs(rgba[i] - bg[0]) + Math.abs(rgba[i + 1] - bg[1]) + Math.abs(rgba[i + 2] - bg[2]);
    if (d > 24) content++;
  }
  return total ? content / total : 0;
}

if (require.main === module) {
const args = process.argv.slice(2);
const files = [];
let threshold = 0.5, tolerance = 1, diffOut = null, asJson = false;
for (let i = 0; i < args.length; i++) {
  const a = args[i];
  if (a === '--threshold') threshold = Number(args[++i]);
  else if (a === '--tolerance') tolerance = Number(args[++i]);
  else if (a === '--diff-out') diffOut = args[++i];
  else if (a === '--json') asJson = true;
  else files.push(a);
}
if (files.length !== 2) {
  console.error('用法: compare.js <a.png> <b.png> [--threshold 0.5] [--tolerance 1] [--diff-out x.png] [--json]');
  process.exit(1);
}

try {
  const a = decodePNG(fs.readFileSync(files[0]));
  const b = decodePNG(fs.readFileSync(files[1]));
  const r = compare(a, b, tolerance);
  const pass = !r.dimensionMismatch && r.ratioPct <= threshold;
  if (diffOut && r.heat) fs.writeFileSync(diffOut, encodePNG({ width: a.width, height: a.height, rgba: r.heat }));

  const summary = {
    a: files[0], b: files[1],
    width: r.width, height: r.height,
    diffPixels: r.diffPixels, totalPixels: r.totalPixels,
    diffPercent: Number(r.ratioPct.toFixed(4)),
    thresholdPercent: threshold, tolerance,
    bbox: r.bbox, maxChannelDelta: r.maxDelta, pass,
    contentA: Number((contentRatio(a) * 100).toFixed(2)),
    contentB: Number((contentRatio(b) * 100).toFixed(2)),
    blankSide: undefined,
    reason: r.dimensionMismatch ? 'dimension mismatch' : undefined,
  };
  const blankA = summary.contentA < 0.5, blankB = summary.contentB < 0.5;
  if (blankA || blankB) summary.blankSide = blankA && blankB ? 'both' : blankA ? 'a' : 'b';
  if (asJson) {
    console.log(JSON.stringify(summary));
  } else {
    if (r.dimensionMismatch) {
      console.log(`FAIL 尺寸不一致: ${a.width}x${a.height} vs ${b.width}x${b.height}`);
    } else {
      console.log(
        `${pass ? 'PASS' : 'FAIL'} diffPixels=${r.diffPixels} totalPixels=${r.totalPixels} ` +
        `diff=${r.ratioPct.toFixed(4)}% threshold=${threshold}% tolerance=${tolerance} ` +
        `bbox=${r.bbox ? `[${r.bbox.x0},${r.bbox.y0},${r.bbox.x1},${r.bbox.y1}]` : 'none'} maxDelta=${r.maxDelta} ` +
        `content=${summary.contentA}%/${summary.contentB}%`
      );
      if (summary.blankSide) console.log(`WARN ${summary.blankSide === 'both' ? '两侧都' : summary.blankSide === 'a' ? '基线' : '当前'}是业务白屏：这个 diff 不能当通过判据`);
    }
    if (diffOut) console.log(`diff image: ${diffOut}`);
  }
  process.exit(pass ? 0 : 2);
} catch (e) {
  console.error(`compare 失败: ${e.message}`);
  process.exit(1);
}
}

module.exports = { decodePNG, compare, encodePNG, contentRatio, flattenToWhite };
