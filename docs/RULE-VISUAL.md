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
fn visual_files(&self) -> &'static [VisualFile] { ... }             // 写到页面目录，如 "probe.wxs"
fn visual_actions(&self) -> &'static [VisualAction] { ... }         // Tap / Swipe / Wait，截图前执行
```

四者都是默认空实现的 trait 方法，所以给 `VisualCase` 加字段不会逼着所有既有字面量一起改；
代价是同一规则的多条用例只能共享同一组组件、同一份 app.json 覆盖与同一串动作。

需要真实输入的规则（WXS 里用事件处理器第二参数做同步查询、`scroll-view` 与祖先 `touchmove` 抢手势）
用 `visual_files` 写真实 `.wxs`（内联 `<wxs>` 不进编译产物）、用 `visual_actions` 注入 `Tap` / `Swipe`。

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

纯位移的差异在交集里接近 0%，只能靠 bbox 边差兜底——当前有 6 条用例完全依赖它：

```
box-sizing 0.00%(bbox 80)   box-sizing-2 0.00%(bbox 80)   value-degraded 0.20%(bbox 120)
calc-min-max 0.00%(bbox 160)   default-layout-drift 0.00%(bbox 40)   renderer-options 0.00%(bbox 40)
```

另有 4 条处于 1%–2% 的边缘（`text-hard-break-ellipsis` 1.88%、`text-decoration-target` 1.83%、
`worklet-data-freeze` 1.74%、`text-overflow-ellipsis` 1.28%），报告会单列为 `nearThreshold`，
复核时优先看这四条。

`error` 不等于 `fail`：截图尺度不对、bbox 取不到、`reLaunch` 超时、serve 掉线都记 `error`，
必须解决后重跑，不能当「规则不成立」。

## 采集链路

1. `cargo run --example rule_visual_cases -- --renderer {skyline|webview} --out <dir>`：
   按 registry 遍历，一个 case 一页写成小程序工程，页面/组件 json 各自写 `renderer`，
   并输出 `cases.json`。
2. `minidev sim build` 两份工程（构建期串行，避免争编译缓存）。
3. 起两个 `sim serve`（默认 9783/9784，只绑 127.0.0.1）：Skyline 侧由 serve 托管 Electron，
   Skyline 引擎只活在开发者工具整包里；WebView 侧显式传 `--browser`。
4. `probe.js` 逐页 `reLaunch` → 等 `WAIT_MS`（默认 8000）→ 执行 `actions`（每个动作后 300ms，
   全部结束再等 500ms）→ 取 bbox → 两侧各自截图 → 判定。
   每页开始前做一次 CDP 健康检查；任何一步失败只写该页 `error`，不中断整批。
5. `run.sh` 按 `CHUNK_SIZE`（默认 8）分块，块前检查 serve 健康、挂了就按记录的 pid 重启，
   块间把结果并回 `results.jsonl`；全部跑完再统一 `--report-only` 出报告。

`MINIDEV_BIN` 必须指向**已构建的二进制**（`target/debug/minidev`），不要指向「先编译再 exec」的包装脚本：
采集时每页都要 spawn 一次 `sim shot`，包装脚本会让每次调用都去抢编译锁，30 秒后子进程超时，
整批表现为随机的 `error` 与反复重试。先 `cargo build -p daemon --bin minidev`，再指向二进制。

serve 的端点分层（旧版本只有 `/cdp`）：`/automator` 是协议层、`/browser-cdp` 是浏览器层、
`/inspectee-cdp` 是框架层。健康检查与截图都走 `/browser-cdp`。

清理只 kill 自己记录的 pid，禁止 `pkill -f`（会连带杀掉别的实例与开发者工具）。
并发上限 3 个实例，每个实例不同 `--dist` + 不同 `--port`。

## 覆盖现状

两套用例合计：**70 个截图用例覆盖 55 条规则 + 27 个证据用例覆盖 11 条规则**。

截图部分：66 条 `Differ`（其中 60 条像素差 ≥1%、6 条完全靠 bbox 边差）+ 4 条阴性对照；
其中 1 条是对照里**故意失败**的（`skyline/text-box-property` 的第二条：实测两端宽度必然不同，
用它标注「这类阴性对照不成立」）。

证据部分（产物 / 构建日志 / 运行期取值）：`component-framework`、`navigation-style`、
`lazy-code-loading`、`worklet-compile-setting`、`worklet-directive`、`autoprefix-build`、
`scroll-view-type`、`config-ignored`、`scroll-context-ref`、`worklet-api-scope`、`lib-version`。

覆盖率门禁 `skyline_rules_have_cases` 已启用（不再 `#[ignore]`）：skyline 规则必须至少有一条
截图用例或证据用例，否则测试失败。目前只豁免 4 条，每条都带实测理由：

| 规则 | 为什么无可自动用例 |
|---|---|
| `route-type` | 差异只在跨页转场的过渡帧上，单页 `reLaunch` 表达不了 |
| `share-element-key` | 共享元素转场同样跨页，且转场结束态两端一致，只有中间帧不同 |
| `scroll-context-enhanced` | 模拟器不实现「未开 enhanced → 告警并返回 false」：两种配置的返回值、告警、滚动行为都没差异 |
| `draggable-sheet-scroll-to` | sheet 几何不随 `scrollTo({size|pixels})` 变化，且传 `pixels` 会把 appservice 事件循环卡死 |

另有一条**用例存在但默认跳过**：`universal-selector` 的 WXSS 写 `*` 时编译器报
`unexpected token '*'` 并让整个工程构建失败（不是规则不生效，是一份 WXSS 编不出来）。
全批默认 `--skip universal-selector`；「生产构建是否同样拒绝」尚未确认，结论未定前不进报告。

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
- **注入的手势不等于真机手势。** `Swipe` 由宿主按 touchstart/move/end 注入，Skyline 的原生滚动
  控件不消费这串序列（列表 `top` 不动、事件照常冒泡给祖先），而 WebView 的 `scroll-view` 会滚动并
  阻止冒泡。`scroll-gesture-conflict` 的 Differ 正是这个终态差，它证明的是「两端对同一串触摸的反应不同」，
  不等于真机上「原生滚动与 JS `touchmove` 并行」的手感；真机结论仍需单独验证。
- **证据用例证明的是「事实成立」，不等于「后果成立」。** 例如 `worklet-compile-setting` 只能证明
  `compileWorklet` 开关决定产物里有没有 `__workletHash`，不能证明动画在运行期真的生效；
  `autoprefix-build` 只能证明「该配置下构建会注入前缀」，而规则主张里的「dev 构建跳过」在本工具链
  上无法验证（`sim build` 没有 dev/prod 开关）。
- **模拟器自身的实现缺口会让「无差异」变成假结论。** 已确认三处：`disableScroll` 在本机 WebView 上
  根本不实现（app 级与页面级都一样能滚，因此 `config-ignored` 的证据只能证明「写不写都没差异」）；
  工程 `libVersion` 不参与选库（运行时恒为 vendor 版本）；给 `scroll-view` 一份有效 ref 后调
  `scrollTo` 会把 appservice 事件循环卡死。凡指望这些行为出差异的断言，都要先确认模拟器实现了它。
- 视觉用例证明的是「两端在这份最小源码上不同」，不证明「真实工程里一定出现」。规则在真实工程
  是否命中、命中后修法是否两端一致，仍按原有流程验证。

## 覆盖率门禁

`rules::tests::skyline_rules_have_cases`（已启用）要求每条 skyline 规则至少有一条截图用例
或证据用例，否则测试失败。无法自动证的规则必须写进白名单并附实测理由——理由为空同样失败，
不允许无理由豁免。

## 证据用例（构建 / 日志 / 运行期）

两端本来就没有视觉差异的规则（构建期校验、产物改写、运行期告警）不能靠截图证明。
它们用 `EvidenceCase`：一份完整最小工程 + 一次构建或一次页面加载 + 一组字面断言。

```text
cargo run --example rule_evidence_cases -- --out .tmp/evidence-run   # 只生成工程与清单
MINIDEV_BIN=<已构建二进制> MINIDEV_SDK=<sdk/minidev-automator> tools/evidence/run.sh
open .tmp/evidence-run/report.html
```

`MINIDEV_BIN` 同样必须指向已构建二进制，理由见上一节。serve 类用例每条一个独立工程、
一个独立 serve（默认端口 9791），串行执行；端口先确认空闲，清理只 kill 自己记录的 pid。

用例与规则源码同文件（`fn evidence_cases`），接口见 `src/rules/mod.rs`：

| `run` | 做什么 | 可配的 `expect` |
|---|---|---|
| `Build` | `sim build --project <case> --out <case>/dist` | `BuildOk` / `BuildFails` / `BuildLog` / `Artifact` |
| `Serve { page }` | 构建 → 起 serve → reLaunch 页面 | `Console` / `Runtime` |

判定口径：

- 所有 `must` / `must_not` 是字面子串，不是正则；文案改了断言必须跟着改，避免静默通过。
- `pass` = 全部断言成立；`fail` = 证据取到了但断言不成立（exit 码不对、文案不匹配、
  不该出现的日志出现了）；`error` = 证据取不到（构建没跑、dist 文件缺失、serve 连不上、
  console 通道不通、evaluate 抛错）。`error` 不是 `fail`，不能当结论。
- `Console` 在 reLaunch 后先注入一条 `__EVIDENCE_READY_*__` 握手日志：没收到它说明
  `App.logAdded` 通道不可用，直接 `error`——否则「日志为空」会被误判成「没有告警」。
- 每条规则至少一条阴性对照：构建类用「只补缺键即构建通过」，运行期类用「不写触发值
  即不出现该日志」。

当前覆盖（M0）：

- `skyline/lazy-code-loading`：缺 `lazyCodeLoading` 时构建 exit 1、stderr 含
  `lazyCodeLoading` 与 `requiredComponents`；补齐后 `BuildOk`。
- `skyline/scroll-view-type`：`type="default"` 的页面日志出现
  `Do not support the attribute default in scroll-view`；不写 `type` 的对照页不出现。
- `skyline/config-ignored`：app.json `window.disableScroll: true` 时
  `wx.pageScrollTo({scrollTop:200})` 仍把页面滚到 200；不写该键的对照页同样 200。
- `skyline/scroll-context-ref`：不 `.ref()` 时同一调用打
  `scrollTo: fail invalid parameter ref` 且 `scrollTop` 保持 0；拿到 ref 的对照页不再打这行。
- `skyline/worklet-api-scope`：worklet 内 `wx.getStorageSync` 静默返回 `undefined`，
  JS 线程读同一 key 得到 `probe-value`（证据强度低于报错类：没有崩溃，只有返回值失效）。
- `skyline/lib-version`：`libVersion: "2.20.0"` 构建 exit 0；运行时 `SDKVersion` 与
  `3.3.0` 的对照工程同为 vendor `3.17.3`。

尚未迁移：`scroll-context-enhanced`、`draggable-sheet-scroll-to` 与 `route-type`。
前两条在本机模拟器上取不到能归因的读数：前者不产生规则主张的 warning（开/未开 `enhanced`
无差异），后者 `scrollTo` 不影响页面侧几何、`pixels` 还会卡死 appservice；两条都在规则源码里
写明了不可自动证的结论。`route-type` 仍需要跨页转场。
