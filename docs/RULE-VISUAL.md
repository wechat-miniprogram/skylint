# 规则视觉用例

给每条「两端渲染有差异」的规则配一份最小 WXML/WXSS，同一份源码分别用 Skyline 与 WebView 渲染并截图，
用像素/几何证据证明规则主张成立。它不替代单测：单测证明「规则会报」，视觉用例证明「报的是真的」。

```
cargo test --lib                      # 规则单测（含每条用例必须真的触发本规则）
tools/visual/run.sh                   # 全批采集 + 判定 + 出报告
RESUME=1 tools/visual/run.sh          # 断点续跑（只补缺失/失败页）
open .tmp/visual/report.html          # 逐条对照截图与度量
```

## 用例写在哪儿

用例挨着规则源码放，与 `Rule::check` 同文件（`#[cfg(test)]` 之外，理由见「覆盖率门禁」）。

```rust
static VISUAL_CASES: &[VisualCase] = &[VisualCase {
    wxml: r#"<view class="wrap"><text id="target">甲</text></view>"#,
    wxss: ".wrap { padding: 20px; }\n",
    js: None,
    expect: VisualExpect::Differ,
    note: "一句话说明两端差在哪、差多大",
    target: "#target",
}];

impl Rule for X {
    fn visual_cases(&self) -> &'static [VisualCase] { VISUAL_CASES }
}
```

- `target` 非空：元素模式。取该选择器在两端 `boundingClientRect`，按各自截图尺度换到像素坐标，
  在两侧 bbox 的**交集**里算像素差（`rectPixelDiffPctOwn`）。
- `target` 为空串：整页模式，按整页像素差判定。组件内部节点从页面层查不到（选择器不跨组件边界），
  需要观察组件内部时用这个模式，并在 `note` 里写明证据强度弱于元素模式。
- `expect`：`Differ`（两端必须不同）或 `Same`（阴性对照，两端必须一致）。`Same` 用来证明框架
  不会把「一致」误判成「不同」，至少为每条机制保留一份。

规则级夹具（不是用例级）：

```rust
fn visual_components(&self) -> &'static [VisualComponent] { ... }   // 挂到本规则每个用例页面
fn visual_app_json(&self) -> Option<&'static str> { ... }           // 合并进 app.json
```

两者都是默认空实现的 trait 方法，所以给 `VisualCase` 加字段不会逼着所有既有字面量一起改；
代价是同一规则的多条用例只能共享同一组组件与同一份 app.json 覆盖。

`visual_app_json` 的覆盖里**不许**出现 `renderer` / `pages` / `componentFramework` / `lazyCodeLoading`，
生成器会直接报错而不是静默忽略；同理页面 json 与 app json 都不得写 `rendererOptions`——
`defaultContentBox` / `defaultDisplayBlock` 保持缺省是 `box-sizing`、`flex-direction`、
`default-layout-drift` 与三条 `fixed-*` 用例成立的前提。

## 判定口径

| 字段 | 含义 | 用途 |
|---|---|---|
| `rectPixelDiffPctOwn` | 两侧 bbox 交集区域的像素差（每侧先按自己的截图尺度裁剪） | **判定依据** |
| `rectPixelDiffPctUnion` | 两侧 bbox 并集区域的像素差 | 仅参考；邻居移进并集会稀释/污染 |
| `bboxEdgeDiffDevice` | 两侧 bbox 任一边差的设备像素 | **判定依据**（纯位移差异的唯一证据） |
| `scaleSky` / `scaleWv` | 截图宽 / (视口宽 × dpr)，正常为 1 | 任一轴偏差 > 0.01 直接判 `error`，不按错尺度裁剪 |
| `inkPctSky` / `inkPctWv` | 非背景像素（与纯白任一通道差 > 8）占比 | 判「该侧是不是真的没画东西」；< 0.01% 才算空白 |
| `weakEvidence` | 判为 `differ` 但像素差 < 1%，或两端都空白 | 人工复核入口 |

判定：`Differ` 成立当且仅当 `rectPixelDiffPctOwn ≥ 1%` **或** `bboxEdgeDiffDevice ≥ 4`；
`Same` 成立要求两者都在容差内。阈值不设成「像素差足够小」的原因是系统性色差：容差 1 个单位
就会把 2 个单位的偏色放大成整页 70% 的差异。

纯位移的差异在交集里是 0%（两块完全不重叠），只能靠 bbox 边差兜底——当前有 9 条用例属于这一类：

```
text-hard-break-ellipsis 1.88%(bbox 44)   box-sizing 0.00%(bbox 80)      box-sizing-2 0.00%(bbox 80)
text-overflow-ellipsis  1.28%(bbox 32)   value-degraded 0.20%(bbox 120) text-decoration-target 1.83%(bbox 48)
calc-min-max            0.00%(bbox 160)  default-layout-drift 0.00%(bbox 40)   renderer-options 0.00%(bbox 40)
```

`error` 不等于 `fail`：截图尺度不对、bbox 取不到、`reLaunch` 超时、serve 掉线都记 `error`，
必须解决后重跑，不能当「规则不成立」。

## 采集链路

1. `cargo run --example rule_visual_cases -- --renderer {skyline|webview} --out <dir>`：
   按 registry 遍历，一个 case 一页写成小程序工程，页面/组件 json 各自写 `renderer`，
   并输出 `cases.json`。
2. `minidev sim build` 两份工程（构建期串行，避免争编译缓存）。
3. 起两个 `sim serve`（默认 9783/9784，只绑 127.0.0.1）：Skyline 侧由 serve 托管 Electron，
   Skyline 引擎只活在开发者工具整包里；WebView 侧显式传 `--browser`。
4. `probe.js` 逐页 `reLaunch` → 等 `WAIT_MS`（默认 8000）→ 取 bbox → 两侧各自截图 → 判定。
   每页开始前做一次 CDP 健康检查；任何一步失败只写该页 `error`，不中断整批。
5. `run.sh` 按 `CHUNK_SIZE`（默认 8）分块，块前检查 serve 健康、挂了就按记录的 pid 重启，
   块间把结果并回 `results.jsonl`；全部跑完再统一 `--report-only` 出报告。

清理只 kill 自己记录的 pid，禁止 `pkill -f`（会连带杀掉别的实例与开发者工具）。
并发上限 3 个实例，每个实例不同 `--dist` + 不同 `--port`。

## 覆盖现状

59 个用例覆盖 47 条规则：48 条靠像素差（≥2%）、9 条靠 bbox 边差、1 条是**故意失败的阴性对照**
（`skyline/text-box-property` 的第二条用例：实测两端宽度必然不同，用它标注「这类阴性对照不成立」）。

尚未覆盖的 24 条按证明方式分类：

- **需要动作或时序**（`page-scroll-api`、`wxs-sync-query`、`worklet-data-freeze`、
  `scroll-gesture-conflict`、`transition-animation-conflict`、`transition-property`、
  `vendor-prefix`、`unsupported-api`）：多数可以用「页面 JS + 延时」表达（`VisualCase.js` 本身就是
  页面 JS，在 `onShow` 里 `setTimeout` 后调 API 或切 class，采集端等 8 秒足够），尚未逐条试。
- **只能产物/日志取证**（`worklet-api-scope`、`worklet-directive`、`worklet-compile-setting`、
  `scroll-context-enhanced`、`scroll-context-ref`、`draggable-sheet-scroll-to`、`autoprefix-build`、
  `config-ignored`、`lib-version`、`lazy-code-loading`、`component-framework`、`navigation-style`、
  `route-type`、`scroll-view-type`）：两端本来就该没有视觉差异，「截图一致」不是证据。
  这类需要另一套证据形式（产物文件、运行日志、配置回执），不在本框架内。
- **跨页共享元素转场**（`share-element-key`）：需要两个页面之间的转场，单页截图表达不了。
- **会被编译器直接拒绝**：`universal-selector` 的用例 WXSS 写 `*` 时，WXSS 编译器报
  `unexpected token '*'` 并让整个工程构建失败（不是这条规则不生效，是一份 WXSS 编不出来）。
  全批默认 `--skip universal-selector`；这条本身待确认「生产构建是否同样拒绝」，结论未定前不进报告。

## 口径与局限

- **两端都强制 `componentFramework: "glass-easel"`。** WebView 侧当初这么做是为了绕开旧 exparser
  在模拟器上的断言崩溃，但这意味着**截图里的 WebView 不等于生产默认 WebView**。凡机制依赖组件框架
  的规则（例如未注册标签：glass-easel 下是未知行内元素，旧 exparser 下退化为无模板默认组件、子树
  不显示），这条用例只代表 glass-easel 分支，落地前需另找证据。
- **Skyline 截图是视口尺寸的窗口导出**（窗口逻辑尺寸 × dpr），当页高超过视口时被压的是**布局**
  （Skyline 页面根是定高 flex 列，子项被等比压缩），不是截图像素。两者要分清：布局压缩是真实差异，
  截图缩放才是度量错误（由 `scale*` 判 `error`）。scroll 类用例的数值因此只能同口径比较。
- **1% 阈值会漏掉真实但细小的差异**（下划线 1–2px、单个字形），这类用例要么放大元素、要么改元素模式，
  不要靠调低阈值通过——阈值一松，系统性色差就会变成假阳性。
- **报告里的 diff% 含系统性色差与整页位移，只作参考**。偏色方向已确认是 WebView 侧偏，
  不是 Skyline 侧；判读时以 bbox 与区域像素分布为主。
- 视觉用例证明的是「两端在这份最小源码上不同」，不证明「真实工程里一定出现」。规则在真实工程
  是否命中、命中后修法是否两端一致，仍按原有流程验证。

## 覆盖率门禁

`rules::tests::skyline_rules_have_visual_cases` 列出所有还没有用例的 skyline 规则，当前是
`#[ignore]`。启用前需要先给「只能产物/日志取证」的那批建白名单，每条写明豁免理由；
否则门禁会把「本来就不该用截图证明」的规则判成缺失。
