# Skyline 渲染器疑似缺陷记录（迁移实测）

不是规则问题、也不是迁移写法问题，是在真实工程（`meituan` 离线还原工程，78 页切片）
从 WebView 翻到 Skyline 时撞到的**渲染器级故障**。三条都在同一页/同一轮里复现，
症状高度一致：`appservice` 不报错、不打日志、`navigateTo` 永不回调，
截图接口持续返回上一页的旧帧，开发者工具本身不崩。

**环境**

- 微信开发者工具 `36.6.0`（macOS, arm64，模拟器）
- 工程 `app.json`：`rendererOptions.skyline = { defaultDisplayBlock: true, tagNameStyleIsolation: "legacy" }`
- 页面：`movie/pages/cinema/*`（movie 分包的影院三页）
- 复现方式：`minidev build --no-pack` 通过（exit 0），构建期与静态 lint 都**看不到任何异常**；
  只有模拟器运行时卡死。

判定口径：删除触发代码 → 立刻正常；恢复 → 稳定复现。以下每条都做过这个往返。

---

## B1. `filter: blur()` 让 appservice 卡死

**位置**：`movie/pages/cinema/cinema.wxss`，轮播背景 `.mt-bg { filter: blur(10px) }`。

**症状**：进入该页后 `navigateTo` 的 success/fail 都不回调；`console` 停在
`onLoad` / `onReady` 之后；此后所有 `automator.navigateTo` 都超时，截图返回上一页画面。
杀进程重开、清缓存都无效，只有删掉 `filter: blur` 才恢复。

**往返验证**：加回 `filter: blur(10px)` → 稳定卡死；删掉 → 正常，反复两次都成立。

**当前规避**：删除模糊，轮播背景为清晰图。观感损失（基线有毛玻璃）记为已知差异。

**为什么可疑**：`filter` 在 Skyline 的 CSS 支持表里是「部分支持」，
静态规则 `skyline/filter-limit` 只描述「只应用第一个 operation」这类**降级**行为，
从未提示可能挂死。若这是引擎侧可复现的问题，值得报给 Skyline 团队；
若是特定 blur 半径/图层组合的触发条件，也需要一份最小复现。

**待补**：最小复现工程（单页 + 单张背景图 + 仅 `filter: blur`）尚未构造，只有业务工程内的往返证据。

---

## B2. 轮播卡内加「绝对定位子元素」或伪元素让同一页卡死

**位置**：`movie/pages/cinema/cinema.wxml`，轮播卡 `.mt-movie-list-conn` 下方那个
白色小尖角（基线里是 `border` 三角 / 伪元素做的 10px 装饰）。

**症状**：与 B1 完全相同的卡死形态（`navigateTo` 不回调、日志停在 `onReady` 之后）。

**往返验证**（逐个试，每项都会复现，删掉即恢复）：

| 加法 | 结果 |
|---|---|
| 卡内加一个 `position: absolute; top: -10rpx` 的子元素 | 卡死 |
| 同一个卡上加 `box-sizing: content-box` | 卡死 |
| 用伪元素（`::before` + border 三角） | 不渲染（未见卡死，但也不出图） |
| 旋转正方形（`transform: rotate(45deg)`） | 卡死 |
| 删除以上全部 | 正常 |

**当前规避**：这个 10px 装饰尖角直接去掉，页面其余部分与基线一致。

**为什么可疑**：卡死与「加了一个子元素」强相关，而同一个绝对定位写法在别的页面
（`cinema/index` 的 `.label-container`、`seat` 的座位图）都是正常的。
说明触发条件与这个卡本身的布局状态有关（轮播 + 缩放 + 相对定位 + 子元素重排），
不是「Skyline 不支持 absolute」这种通用结论。`box-sizing` 也能触发，进一步说明
问题更像布局重算进入了不可恢复状态，而不是某个 CSS 属性没实现。

**待补**：把这张卡单独抽成最小页面（swiper + 一张图 + 一个绝对定位子元素）再试，
确认是 swiper 场景特有还是这个 DOM 形状特有。

---

## B3. 横向 `scroll-view` 放进 `sticky-header` 也会卡死（关联项）

**位置**：`movie/pages/cinema/movie.wxml` 的日期 tab 条——原实现是
`<scroll-view scroll-x>`，为了吸顶把它放进了 `<sticky-section>` / `<sticky-header>`。

**症状**：同上。删除横向 scroll-view、改成 `display: flex; flex-wrap: wrap` 之后正常。

**当前规避**：日期 tab 改为换行（已与产品确认保留换行，不找回横滑）。
副作用是日期多时 sticky 头会长高。

---

## 相关但不同类：全绝对定位的 scroll-view 内容尺寸为 0

不是卡死，是布局语义差异，但同一次迁移里也花了很久才定位，记在一起：

`movie/pages/cinema/seat.wxml` 的座位图：`scroll-view` 里全是 `position: absolute`
的座位块。Skyline 下 `scroll-view` 用「有布局的子节点」决定内容尺寸，结果内容高度为 0，
整个座位图被裁成只剩左侧排号；WebView 里绝对定位同样不参与文档流，但滚动容器高度由
显式高度给出，所以看不出来。**规避**：内容外包一层定尺寸 `view`
（`width: {{seatWidth}}px; height: {{seatHeight + 225}}px; position: relative`）。

---

## 结论

- 三条卡死都是「静默挂死」：没有异常、没有日志、构建与静态检查全绿，
  只能靠模拟器跑起来才暴露。这解释了为什么迁移门禁里 L2（模拟器冒烟）不能省。
- 目前只有业务工程内的往返证据，没有最小复现工程。下一步应该先把 B1 与 B2 各缩成
  最小页面，再决定是否报给 Skyline 团队——现在的证据还不足以让对方定位。
- 静态工具侧暂时没有可加的判据：这三条都依赖运行时布局状态，
  按「静态可判定 + 修法唯一」的门槛，`filter: blur` 与 `absolute` 都不该进规则面；
  若后续确认了稳定的触发形状（例如「swiper + absolute 子元素」），再考虑窄口径规则。
