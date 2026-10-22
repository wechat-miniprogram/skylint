# 知识库约束覆盖台账

分母：171 条（口径：给定输入文件即可静态判定真假）。

统计：已实现 104 条 / 待实现 62 条 / 不实现 5 条。

| # | 分组 | 约束（一句话） | 证据 | 状态 | 规则 / 理由 |
|---|------|----------------|------|------|-------------|
| 1 | 1A WXSS 值 | `float` 声明不生效 | skyline-skills/skills/skyline-wxss/SKILL.md:59 | 已实现 | `skyline/unsupported-property` |
| 2 | 1A WXSS 值 | `contain` 声明不生效 | skyline-skills/skills/skyline-wxss/SKILL.md:60 | 已实现 | `skyline/unsupported-property` |
| 3 | 1A WXSS 值 | `resize` 声明不生效 | skyline-skills/skills/skyline-wxss/SKILL.md:61 | 已实现 | `skyline/unsupported-property` |
| 4 | 1A WXSS 值 | `writing-mode` 声明不生效 | skyline-skills/skills/skyline-wxss/SKILL.md:62 | 已实现 | `skyline/unsupported-property` |
| 5 | 1A WXSS 值 | `text-indent` 声明不生效 | skyline-skills/skills/skyline-wxss/SKILL.md:63 | 已实现 | `skyline/unsupported-property` |
| 6 | 1A WXSS 值 | `overflow-wrap` 声明不生效 | skyline-skills/skills/skyline-wxss/SKILL.md:64 | 已实现 | `skyline/unsupported-property` |
| 7 | 1A WXSS 值 | `text-decoration-thickness` 声明不生效 | skyline-skills/skills/skyline-wxss/SKILL.md:65 | 已实现 | `skyline/unsupported-property` |
| 8 | 1A WXSS 值 | `background-attachment` 声明不生效 | skyline-skills/skills/skyline-wxss/SKILL.md:66 | 已实现 | `skyline/unsupported-property` |
| 9 | 1A WXSS 值 | `background-clip` 声明不生效 | skyline-skills/skills/skyline-wxss/SKILL.md:67 | 已实现 | `skyline/unsupported-property` |
| 10 | 1A WXSS 值 | `background-origin` 声明不生效 | skyline-skills/skills/skyline-wxss/SKILL.md:68 | 已实现 | `skyline/unsupported-property` |
| 11 | 1A WXSS 值 | `mask-origin` 声明不生效 | skyline-skills/skills/skyline-wxss/SKILL.md:69 | 已实现 | `skyline/unsupported-property` |
| 12 | 1A WXSS 值 | `mask-clip` 声明不生效 | skyline-skills/skills/skyline-wxss/SKILL.md:69 | 已实现 | `skyline/unsupported-property` |
| 13 | 1A WXSS 值 | `mask-mode` 声明不生效 | skyline-skills/skills/skyline-wxss/SKILL.md:69 | 已实现 | `skyline/unsupported-property` |
| 14 | 1A WXSS 值 | `justify-items` 声明不生效 | skyline-skills/skills/skyline-wxss/SKILL.md:70 | 已实现 | `skyline/unsupported-property` |
| 15 | 1A WXSS 值 | `list-style-type` 声明不生效 | skyline-skills/skills/skyline-wxss/SKILL.md:71 | 已实现 | `skyline/unsupported-property` |
| 16 | 1A WXSS 值 | `list-style-image` 声明不生效 | skyline-skills/skills/skyline-wxss/SKILL.md:71 | 已实现 | `skyline/unsupported-property` |
| 17 | 1A WXSS 值 | `list-style-position` 声明不生效 | skyline-skills/skills/skyline-wxss/SKILL.md:71 | 已实现 | `skyline/unsupported-property` |
| 18 | 1A WXSS 值 | `direction` 声明不生效 | skyline-skills/skills/skyline-wxss/references/text.md:18 | 不实现 | 知识库与引擎冲突：引擎已映射 kDirection 并在 Text 生效（css_parser_impl.h:136），按源码不报 |
| 19 | 1A WXSS 值 | `display: grid` 不受支持 | skyline-skills/skills/skyline-wxss/SKILL.md:127 | 已实现 | `skyline/unsupported-value` |
| 20 | 1A WXSS 值 | `display: flow-root` 不受支持 | skyline-skills/skills/skyline-wxss/SKILL.md:127 | 已实现 | `skyline/unsupported-value` |
| 21 | 1A WXSS 值 | `display: inline` 不受支持 | skyline-skills/skills/skyline-overview/references/introduction/features.md:234 | 不实现 | 引擎已实现 display:inline（display.cc:74-78），知识库旧结论；不报 |
| 22 | 1A WXSS 值 | `position: sticky` 不受支持 | skyline-skills/skills/skyline-wxss/SKILL.md:128 | 已实现 | `skyline/unsupported-value` |
| 23 | 1A WXSS 值 | `overflow: auto` 不受支持 | skyline-skills/skills/skyline-wxss/SKILL.md:129 | 已实现 | `skyline/unsupported-value` |
| 24 | 1A WXSS 值 | `overflow: scroll` 不受支持 | skyline-skills/skills/skyline-wxss/SKILL.md:129 | 已实现 | `skyline/unsupported-value` |
| 25 | 1A WXSS 值 | `overflow-x/overflow-y` 分轴独立设置不受支持 | skyline-skills/skills/skyline-wxss/references/layout.md:31 | 不实现 | 引擎支持属性，仅 auto/scroll 值被拒（已由 unsupported-value 覆盖 #23/#24）；不按知识库整体不支持处理 |
| 26 | 1A WXSS 值 | `visibility: collapse` 不受支持 | skyline-skills/skills/skyline-wxss/SKILL.md:130 | 已实现 | `skyline/unsupported-value` |
| 27 | 1A WXSS 值 | `box-sizing: padding-box` 不受支持 | skyline-skills/skills/skyline-wxss/SKILL.md:131 | 已实现 | `skyline/box-sizing` |
| 28 | 1A WXSS 值 | `white-space: pre / pre-wrap / pre-line` 不受支持 | skyline-skills/skills/skyline-wxss/SKILL.md:140 | 已实现 | `skyline/unsupported-value` |
| 29 | 1A WXSS 值 | `font-weight: bolder / lighter` 不受支持 | skyline-skills/skills/skyline-wxss/SKILL.md:141 | 已实现 | `skyline/unsupported-value` |
| 30 | 1A WXSS 值 | `font-weight` 数值必须在 100–900 且为 100 的倍数 | skyline-skills/skills/skyline-wxss/references/text.md:27 | 已实现 | `skyline/value-degraded`（按引擎 `font_weight.cc` 的 round 到百位 + `computed_style.h` 的 switch 回落 400 判定） |
| 31 | 1A WXSS 值 | `font-style: oblique` 不受支持 | skyline-skills/skills/skyline-wxss/SKILL.md:142 | 已实现 | `skyline/unsupported-value` |
| 32 | 1A WXSS 值 | `word-break: break-word` 被映射为 normal | skyline-skills/skills/skyline-wxss/SKILL.md:143 | 已实现 | `skyline/value-degraded` |
| 33 | 1A WXSS 值 | `word-break: keep-all` 可解析但无实际效果 | skyline-skills/skills/skyline-wxss/references/text.md:58 | 已实现 | `skyline/value-degraded` |
| 34 | 1A WXSS 值 | `vertical-align: text-top / text-bottom` 不受支持 | skyline-skills/skills/skyline-wxss/SKILL.md:144 | 已实现 | `skyline/unsupported-value` |
| 35 | 1A WXSS 值 | `text-align: justify-all / match-parent` 不受支持 | skyline-skills/skills/skyline-wxss/SKILL.md:145 | 已实现 | `skyline/unsupported-value` |
| 36 | 1A WXSS 值 | `text-decoration-line` 只接受单值 | skyline-skills/skills/skyline-wxss/SKILL.md:146 | 已实现 | `skyline/text-decoration-line` |
| 37 | 1A WXSS 值 | `text-decoration-line: blink` 不受支持 | skyline-skills/skills/skyline-wxss/SKILL.md:146 | 已实现 | `skyline/unsupported-value` |
| 38 | 1A WXSS 值 | `text-overflow` 仅对 `text` 节点生效 | skyline-skills/skills/skyline-wxss/references/text.md:107 | 已实现 | `skyline/text-overflow-ellipsis` |
| 39 | 1A WXSS 值 | `border-*-style` 仅支持 none/solid/dashed/dotted | skyline-skills/skills/skyline-wxss/SKILL.md:153 | 已实现 | `skyline/unsupported-value` |
| 40 | 1A WXSS 值 | `background-repeat: space / round` 被降级为 repeat | skyline-skills/skills/skyline-wxss/SKILL.md:154 | 已实现 | `skyline/value-degraded`（由 error 降为 warning：被降级为 repeat，不是无效） |
| 41 | 1A WXSS 值 | `will-change` 仅支持 auto/contents，scroll-position 与自定义 ident 不支持 | skyline-skills/skills/skyline-wxss/references/animation.md:106 | 已实现 | `skyline/value-degraded` |
| 42 | 1A WXSS 值 | `content: url(...)` 不受支持 | skyline-skills/skills/skyline-wxss/references/layout.md:56 | 已实现 | `skyline/value-degraded` |
| 43 | 1A WXSS 值 | `animation-fill-mode: none / backwards` 实际按 forwards 处理 | skyline-skills/skills/skyline-wxss/references/animation.md:78 | 已实现 | `skyline/value-degraded` |
| 44 | 1A WXSS 值 | `animation-fill-mode` 仅 forwards/both 可用 | skyline-skills/skills/skyline-wxss/references/animation.md:75 | 已实现 | `skyline/value-degraded` |
| 45 | 1A WXSS 值 | `radial-gradient` 不支持 ellipse 形状 | skyline-skills/skills/skyline-wxss/SKILL.md:79 | 已实现 | `skyline/gradient-limit` |
| 46 | 1A WXSS 值 | `radial-gradient` 尺寸仅支持 px | skyline-skills/skills/skyline-wxss/SKILL.md:79 | 已实现 | `skyline/gradient-limit` |
| 47 | 1A WXSS 值 | `radial-gradient` 颜色停止位置仅支持 % | skyline-skills/skills/skyline-wxss/SKILL.md:79 | 已实现 | `skyline/gradient-limit` |
| 48 | 1A WXSS 值 | `linear-gradient` 停止位置仅支持 % 与固定长度单位 | skyline-skills/skills/skyline-wxss/SKILL.md:80 | 已实现 | `skyline/gradient-limit` |
| 49 | 1A WXSS 值 | `mask-image` 不支持渐变，仅 url() | skyline-skills/skills/skyline-wxss/SKILL.md:107 | 已实现 | `skyline/mask-image-gradient` |
| 50 | 1A WXSS 值 | `filter/backdrop-filter` 不支持 url() 与 drop-shadow() | skyline-skills/skills/skyline-wxss/SKILL.md:97 | 已实现 | `skyline/filter-limit` |
| 51 | 1A WXSS 值 | `filter/backdrop-filter` 不支持多个函数组合 | skyline-skills/skills/skyline-wxss/SKILL.md:97 | 已实现 | `skyline/filter-limit` |
| 52 | 1A WXSS 值 | `background-image` 最多 2 层 | skyline-skills/skills/skyline-wxss/SKILL.md:85 | 已实现 | `skyline/multi-value-limit` |
| 53 | 1A WXSS 值 | `mask-image` 最多 2 层 | skyline-skills/skills/skyline-wxss/SKILL.md:85 | 已实现 | `skyline/multi-value-limit` |
| 54 | 1A WXSS 值 | `background-position` 最多 2 组 | skyline-skills/skills/skyline-wxss/SKILL.md:86 | 已实现 | `skyline/multi-value-limit` |
| 55 | 1A WXSS 值 | `background-repeat` 不支持多组值 | skyline-skills/skills/skyline-wxss/SKILL.md:87 | 已实现 | `skyline/multi-value-limit` |
| 56 | 1A WXSS 值 | `background-size` 不支持多组值 | skyline-skills/skills/skyline-wxss/SKILL.md:87 | 已实现 | `skyline/multi-value-limit` |
| 57 | 1A WXSS 值 | `calc()` 不支持角度计算 | skyline-skills/skills/skyline-wxss/SKILL.md:91 | 已实现 | `skyline/calc-angle` |
| 58 | 1A WXSS 值 | `background` 简写中 attachment/origin/clip 不生效 | skyline-skills/skills/skyline-wxss/SKILL.md:115 | 待实现 | 缺简写子属性失效分析 |
| 59 | 1A WXSS 值 | `text-decoration` 简写中 thickness 不生效 | skyline-skills/skills/skyline-wxss/SKILL.md:116 | 待实现 | 缺简写子属性失效分析 |
| 60 | 1A WXSS 值 | `mask` 简写中 origin/clip 不生效 | skyline-skills/skills/skyline-wxss/SKILL.md:117 | 待实现 | 缺简写子属性失效分析 |
| 61 | 1A WXSS 值 | `env()` 仅支持四个 safe-area 变量 | skyline-skills/skills/skyline-wxss/references/basics.md:24 | 已实现 | `skyline/value-degraded` |
| 62 | 1A WXSS 值 | transition/animation 不支持布局属性 | skyline-skills/skills/skyline-wxss/references/animation.md:35 | 待实现 | 缺 transition 可动画属性白名单 |
| 63 | 1A WXSS 值 | `transition-property` 仅支持文档列出的可动画属性 | skyline-skills/skills/skyline-wxss/references/animation.md:58-61 | 待实现 | 缺 transition-property 值白名单 |
| 64 | 1B 默认值/选择器 | Skyline 下元素默认 `display: flex` | skyline-skills/skills/skyline-wxss/SKILL.md:163 | 待实现 | 缺默认 display 语义检测 |
| 65 | 1B 默认值/选择器 | Skyline 下 `position` 默认 relative | skyline-skills/skills/skyline-wxss/SKILL.md:164 | 待实现 | 缺 position 默认值漂移检测 |
| 66 | 1B 默认值/选择器 | Skyline 下 `box-sizing` 默认 border-box | skyline-skills/skills/skyline-wxss/SKILL.md:165 | 待实现 | 缺 box-sizing 默认值漂移检测 |
| 67 | 1B 默认值/选择器 | Skyline 下 `flex-direction` 默认 column | skyline-skills/skills/skyline-wxss/SKILL.md:166 | 已实现 | `skyline/flex-direction` |
| 68 | 1B 默认值/选择器 | Skyline 下 `align-items` 默认 stretch | skyline-skills/skills/skyline-wxss/SKILL.md:167 | 待实现 | 缺 align-items 默认值漂移检测 |
| 69 | 1B 默认值/选择器 | Skyline 下 `text-align` 默认 start | skyline-skills/skills/skyline-wxss/SKILL.md:168 | 待实现 | 缺 text-align 默认值漂移检测 |
| 70 | 1B 默认值/选择器 | Skyline 下 `mask-repeat` 默认 no-repeat | skyline-skills/skills/skyline-wxss/SKILL.md:169 | 待实现 | 缺 mask-repeat 默认值漂移检测 |
| 71 | 1B 默认值/选择器 | 标签/id 选择器默认不跨组件匹配 | skyline-skills/skills/skyline-config/references/app-config.md:69 | 已实现 | `skyline/tag-selector-isolation` |
| 72 | 1B 默认值/选择器 | `text-decoration` 仅对 text/input 生效 | skyline-skills/skills/skyline-wxss/references/text.md:87 | 已实现 | `skyline/text-decoration-target` |
| 73 | 1B 默认值/选择器 | `text-decoration-line` 仅对 text/input 生效 | skyline-skills/skills/skyline-wxss/references/text.md:94 | 已实现 | `skyline/text-decoration-target` |
| 74 | 1B 默认值/选择器 | `text-decoration-style/color` 仅对 text/input 生效 | skyline-skills/skills/skyline-wxss/references/text.md:101 | 已实现 | `skyline/text-decoration-target` |
| 75 | 1B 默认值/选择器 | `text` 的 `overflow` 仅接受 visible/clip/fade/ellipsis | skyline-skills/skills/skyline-components/references/media/text.md:50-55 | 已实现 | `skyline/component-attribute` |
| 76 | 1B 默认值/选择器 | `text` 的 `max-lines` 必须为正整数 | skyline-skills/skills/skyline-components/references/media/text.md:46 | 已实现 | `skyline/component-attribute` |
| 77 | 1B 默认值/选择器 | `text` 的 `space` 属性不支持 | skyline-skills/skills/skyline-components/references/media/text.md:327 | 已实现 | `skyline/component-attribute` |
| 78 | 1B 默认值/选择器 | `text` 的 `decode` 属性不支持 | skyline-skills/skills/skyline-components/references/media/text.md:328 | 已实现 | `skyline/component-attribute` |
| 79 | 1B 默认值/选择器 | `text` 内只能嵌套 text | skyline-skills/skills/skyline-components/references/media/text.md:20 | 不实现 | 文档矛盾：text.md:20 说只能嵌套 text，changelog.md:260 说 1.4.14 起支持布局节点；无法稳定判定 |
| 80 | 1B 默认值/选择器 | `image` 的 `mode` 仅接受五个缩放值 | skyline-skills/skills/skyline-components/references/media/image.md:32-38 | 已实现 | `skyline/component-attribute` |
| 81 | 1B 默认值/选择器 | 九种 WebView 裁剪 `mode` 在 Skyline 下不支持 | skyline-skills/skills/skyline-components/references/media/image.md:42-54 | 已实现 | `skyline/component-attribute` |
| 82 | 1B 默认值/选择器 | `navigator` 只能嵌套 text 组件或文本节点 | skyline-skills/skills/skyline-overview/references/introduction/component-support.md:23 | 待实现 | 缺 navigator 子节点结构检查 |
| 83 | 1B 默认值/选择器 | `picker-view` 不支持 indicator-class / mask-style | skyline-skills/skills/skyline-overview/references/introduction/component-support.md:33 | 已实现 | `skyline/component-attribute` |
| 84 | 1C 组件契约 | `scroll-view` 必须显式声明 `type` | skyline-skills/skills/skyline-components/references/scroll/scroll-view.md:16 | 已实现 | `skyline/scroll-view-type` |
| 85 | 1C 组件契约 | `scroll-view` 的 `type` 仅接受 list/custom/nested | skyline-skills/skills/skyline-components/references/scroll/scroll-view.md:20-24 | 已实现 | `skyline/scroll-view-type` |
| 86 | 1C 组件契约 | `list-view`/`grid-view` 必须是 custom 模式或 sticky-section 的直接子节点 | skyline-skills/skills/skyline-components/references/scroll/list-grid-view.md:5 | 已实现 | `skyline/scroll-view-family` |
| 87 | 1C 组件契约 | `list-builder` 定高模式必须声明 child-height | skyline-skills/skills/skyline-components/references/scroll/list-grid-view.md:271 | 已实现 | `skyline/scroll-view-family` |
| 88 | 1C 组件契约 | `grid-view` 必须声明 cross-axis-count | skyline-skills/skills/skyline-components/references/scroll/list-grid-view.md:272 | 已实现 | `skyline/scroll-view-family` |
| 89 | 1C 组件契约 | `list-builder` 的 type 仅接受 static/dynamic | skyline-skills/skills/skyline-components/references/scroll/list-grid-view.md:155 | 已实现 | `skyline/scroll-view-family` |
| 90 | 1C 组件契约 | `grid-view` 的 type 仅接受 aligned/masonry | skyline-skills/skills/skyline-components/references/scroll/list-grid-view.md:251 | 已实现 | `skyline/scroll-view-family` |
| 91 | 1C 组件契约 | `sticky-section` 必须在 custom 模式 scroll-view 内 | skyline-skills/skills/skyline-components/references/scroll/sticky.md:152 | 已实现 | `skyline/scroll-view-family` |
| 92 | 1C 组件契约 | `sticky-header` 必须是 sticky-section 第一个子元素 | skyline-skills/skills/skyline-components/references/scroll/sticky.md:153 | 已实现 | `skyline/scroll-view-family` |
| 93 | 1C 组件契约 | 每个 sticky-section 只能有一个 sticky-header | skyline-skills/skills/skyline-components/references/scroll/sticky.md:154 | 已实现 | `skyline/scroll-view-family` |
| 94 | 1C 组件契约 | `sticky-header` 不能放在 sticky-section 之外 | skyline-skills/skills/skyline-components/references/scroll/sticky.md:159 | 不实现 | 文档矛盾：sticky.md:159 禁止，scroll-view.md:54 将 sticky-header 列为 custom 支持子组件；不报 |
| 95 | 1C 组件契约 | 外层 scroll-view 必须 type=nested | skyline-skills/skills/skyline-components/references/scroll/nested-scroll.md:163 | 已实现 | `skyline/scroll-view-family` |
| 96 | 1C 组件契约 | 一个 nested scroll-view 只能有一个 nested-scroll-body | skyline-skills/skills/skyline-components/references/scroll/nested-scroll.md:166 | 已实现 | `skyline/scroll-view-family` |
| 97 | 1C 组件契约 | nested-scroll-header 必须排在 body 之前 | skyline-skills/skills/skyline-components/references/scroll/nested-scroll.md:170 | 已实现 | `skyline/scroll-view-family` |
| 98 | 1C 组件契约 | header/body 只渲染第一个直接子节点 | skyline-skills/skills/skyline-components/references/scroll/nested-scroll.md:29 | 已实现 | `skyline/scroll-view-family` |
| 99 | 1C 组件契约 | 内层 scroll-view 必须 associative-container=nested-scroll-view | skyline-skills/skills/skyline-components/references/scroll/nested-scroll.md:164 | 已实现 | `skyline/scroll-view-family` |
| 100 | 1C 组件契约 | associative-container 仅接受三个合法值 | skyline-skills/skills/skyline-components/references/scroll/scroll-view.md:182-185 | 已实现 | `skyline/component-attribute` |
| 101 | 1C 组件契约 | draggable-sheet 内层 scroll-view 必须 associative-container=draggable-sheet | skyline-skills/skills/skyline-components/references/scroll/draggable-sheet.md:50 | 已实现 | `skyline/scroll-view-family` |
| 102 | 1C 组件契约 | draggable-sheet 的 size 属性取值 0–1 | skyline-skills/skills/skyline-components/references/scroll/draggable-sheet.md:343 | 已实现 | `skyline/component-attribute` |
| 103 | 1C 组件契约 | 自定义下拉刷新必须提供 slot=refresher 节点 | skyline-skills/skills/skyline-components/references/scroll/scroll-view.md:377 | 待实现 | 缺 slot 结构检查 |
| 104 | 1C 组件契约 | refresher-default-style 仅接受 black/white/none | skyline-skills/skills/skyline-components/references/scroll/scroll-view.md:195 | 已实现 | `skyline/component-attribute` |
| 105 | 1C 组件契约 | scroll-into-view-alignment 仅接受 start/center/end/nearest | skyline-skills/skills/skyline-components/references/scroll/scroll-view.md:113 | 已实现 | `skyline/component-attribute` |
| 106 | 1C 组件契约 | 横向滚动的 scroll-view 必须开启 enable-flex | skyline-skills/skills/skyline-components/SKILL.md:76 | 已实现 | `skyline/horizontal-scroll-enable-flex` |
| 107 | 1C 组件契约 | swiper layout-type 仅接受 normal/stackLeft/stackRight/tinder/transformer | skyline-skills/skills/skyline-components/references/layout/swiper.md:44-50 | 已实现 | `skyline/component-attribute` |
| 108 | 1C 组件契约 | swiper transformer-type 仅接受六个合法值 | skyline-skills/skills/skyline-components/references/layout/swiper.md:56-63 | 已实现 | `skyline/component-attribute` |
| 109 | 1C 组件契约 | transformer-type 仅在 layout-type=transformer 时生效 | skyline-skills/skills/skyline-components/references/layout/swiper.md:54 | 待实现 | 缺 layout-type 与 transformer-type 组合校验 |
| 110 | 1C 组件契约 | swiper indicator-type 仅接受文档列出的值 | skyline-skills/skills/skyline-components/references/layout/swiper.md:67-84 | 已实现 | `skyline/component-attribute` |
| 111 | 1C 组件契约 | stack/tinder 布局时 indicator-type 仅支持 normal | skyline-skills/skills/skyline-components/references/layout/swiper.md:87 | 待实现 | 缺 layout-type 与 indicator-type 条件组合校验 |
| 112 | 1C 组件契约 | indicator-type 为 scrollFixedCenter/swap/swapYRotation 时不支持 circular | skyline-skills/skills/skyline-components/references/layout/swiper.md:88 | 待实现 | 缺 circular 与 indicator-type 互斥校验 |
| 113 | 1C 组件契约 | swiper direction 仅接受 all/positive/negative | skyline-skills/skills/skyline-components/references/layout/swiper.md:109-114 | 已实现 | `skyline/component-attribute` |
| 114 | 1C 组件契约 | share-element 必须声明 key | skyline-skills/skills/skyline-components/references/special/share-element.md:63 | 待实现 | 缺 share-element 必填属性检查 |
| 115 | 1C 组件契约 | 同页 share-element 的 key 必须唯一 | skyline-skills/skills/skyline-components/references/special/share-element.md:63 | 待实现 | 缺同页 key 唯一性检查 |
| 116 | 1C 组件契约 | share-element 的 transform 必须在两个页面都设置 | skyline-skills/skills/skyline-components/references/special/share-element.md:359 | 待实现 | 缺跨页面 key/transform 配对检查 |
| 117 | 1C 组件契约 | share-element rect-tween-type 仅接受文档列出的轨迹值 | skyline-skills/skills/skyline-components/references/special/share-element.md:85-98 | 已实现 | `skyline/component-attribute` |
| 118 | 1C 组件契约 | shuttle-on-push/pop 仅接受 to/from | skyline-skills/skills/skyline-components/references/special/share-element.md:78-83 | 已实现 | `skyline/component-attribute` |
| 119 | 1C 组件契约 | snapshot mode 仅接受 view/picture | skyline-skills/skills/skyline-components/references/special/snapshot.md:11 | 已实现 | `skyline/component-attribute` |
| 120 | 1C 组件契约 | snapshot 只能在 Skyline 页面使用 | skyline-skills/skills/skyline-components/references/special/snapshot.md:325 | 待实现 | 缺页面 renderer 配置与 snapshot 使用联合检查 |
| 121 | 1C 组件契约 | 手势组件必须且只能有一个直接子节点 | skyline-skills/skills/skyline-components/references/gesture/gesture-system.md:187 | 待实现 | 缺手势组件单子节点结构检查 |
| 122 | 1C 组件契约 | simultaneous-handlers 必须双方互相声明 | skyline-skills/skills/skyline-components/references/gesture/gesture-negotiation.md:51 | 待实现 | 缺 simultaneous-handlers 双向声明校验 |
| 123 | 1C 组件契约 | native-view 仅接受 scroll-view/swiper | skyline-skills/skills/skyline-components/references/gesture/gesture-negotiation.md:127-129 | 待实现 | 缺 native-view 值白名单 |
| 124 | 1C 组件契约 | 手势组件仅在 Skyline 下可用 | skyline-skills/skills/skyline-components/references/gesture/gesture-system.md:186 | 待实现 | 缺页面 renderer 配置与手势组件联合检查 |
| 125 | 1C 组件契约 | input type 仅接受 text/number/idcard/digit/nickname | skyline-skills/skills/skyline-components/references/form/input.md:44-50 | 已实现 | `skyline/component-attribute` |
| 126 | 1C 组件契约 | input confirm-type 仅接受 send/search/next/go/done | skyline-skills/skills/skyline-components/references/form/input.md:54-60 | 已实现 | `skyline/component-attribute` |
| 127 | 1C 组件契约 | input 不支持 placeholder-class / safe-password-* | skyline-skills/skills/skyline-components/references/form/input.md:306-313 | 待实现 | 缺 input WebView 专属属性黑名单 |
| 128 | 1D 配置 | 全局 Skyline 时 app.json renderer 必须为 skyline | skyline-skills/skills/skyline-config/SKILL.md:140 | 待实现 | 缺 JSON 配置规则：app.renderer 必填校验 |
| 129 | 1D 配置 | 全局 Skyline 时 componentFramework 必须为 glass-easel | skyline-skills/skills/skyline-config/SKILL.md:141 | 待实现 | 缺 JSON 配置规则：componentFramework 必填校验 |
| 130 | 1D 配置 | 全局 Skyline 时 lazyCodeLoading 必须为 requiredComponents | skyline-skills/skills/skyline-config/SKILL.md:142 | 待实现 | 缺 JSON 配置规则：lazyCodeLoading 必填校验 |
| 131 | 1D 配置 | Skyline 页面必须 navigationStyle=custom | skyline-skills/skills/skyline-config/SKILL.md:143 | 待实现 | 缺页面配置必填校验 |
| 132 | 1D 配置 | Skyline 项目 app.json window.navigationStyle 必须为 custom | skyline-skills/skills/skyline-config/references/app-config.md:133 | 待实现 | 缺全局 window 配置校验 |
| 133 | 1D 配置 | Skyline 页面应 disableScroll=true | skyline-skills/skills/skyline-config/references/page-config.md:51 | 待实现 | 缺页面滚动配置检查 |
| 134 | 1D 配置 | page.json renderer 仅接受 webview/skyline | skyline-skills/skills/skyline-config/references/page-config.md:83-98 | 待实现 | 缺 renderer 枚举校验 |
| 135 | 1D 配置 | componentFramework 仅接受 exparser/glass-easel | skyline-skills/skills/skyline-config/references/app-config.md:30-33 | 待实现 | 缺 componentFramework 枚举校验 |
| 136 | 1D 配置 | lazyCodeLoading 仅接受 requiredComponents | skyline-skills/skills/skyline-config/references/app-config.md:47 | 待实现 | 缺 lazyCodeLoading 枚举校验 |
| 137 | 1D 配置 | 页面 renderer=skyline 时 componentFramework 必须为 glass-easel | skyline-skills/skills/skyline-config/references/page-config.md:130-137 | 待实现 | 缺页面级双字段一致性校验 |
| 138 | 1D 配置 | tagNameStyleIsolation 仅接受 isolated/legacy | skyline-skills/skills/skyline-config/references/app-config.md:69 | 已实现 | `skyline/renderer-options` |
| 139 | 1D 配置 | rendererOptions.skyline 下各开关必须为 boolean | skyline-skills/skills/skyline-config/references/app-config.md:65-71 | 已实现 | `skyline/renderer-options` |
| 140 | 1D 配置 | backgroundColorContent 必须为 8 位十六进制颜色 | skyline-skills/skills/skyline-config/references/page-config.md:66 | 待实现 | 缺颜色格式校验 |
| 141 | 1D 配置 | project.config.json libVersion 必须 ≥ 2.30.4 | skyline-skills/skills/skyline-config/references/project-config.md:67-71 | 已实现 | `skyline/lib-version` |
| 142 | 1D 配置 | 真机基础库必须 ≥ 3.0.2 | skyline-skills/skills/skyline-overview/references/migration/getting-started.md:9 | 待实现 | 缺平台版本下限校验 |
| 143 | 1D 配置 | 页面级 rendererOptions 需基础库 ≥ 3.1.0 | skyline-skills/skills/skyline-config/references/page-config.md:111 | 已实现 | `skyline/lib-version` |
| 144 | 1D 配置 | rendererOptions.skyline.convertRpxToVw 必须为 boolean | skyline-skills/skills/skyline-config/references/app-config.md:145-153 | 已实现 | `skyline/renderer-options` |
| 145 | 1D 配置 | project.config.json setting.skylineRenderEnable 必须为 boolean | skyline-skills/skills/skyline-config/references/project-config.md:18-29 | 待实现 | 缺项目配置类型校验 |
| 146 | 1D 配置 | 分包可单独声明 renderer/componentFramework | skyline-skills/skills/skyline-overview/references/migration/best-practice.md:42-46 | 待实现 | 缺分包配置遍历 |
| 147 | 1D 配置 | disableScroll 只能出现在 page.json | skyline-skills/skills/skyline-config/references/page-config.md:49 | 待实现 | 缺配置位置约束 |
| 148 | 1D 配置 | defaultDisplayBlock=true 时 flex-direction 默认值变为 row | skyline-skills/skills/skyline-config/references/app-config.md:67 | 已实现 | `skyline/flex-direction` |
| 149 | 1D 配置 | defaultContentBox 会改变 box-sizing 默认值前提 | skyline-skills/skills/skyline-config/references/app-config.md:68 | 待实现 | 缺默认值开关识别 |
| 150 | 1D 配置 | keyframeStyleIsolation 仅接受 isolated/legacy | skyline-skills/skills/skyline-overview/references/migration/getting-started.md:35 | 已实现 | `skyline/renderer-options` |
| 151 | 1E JS/worklet | worklet 函数体首部必须含 'worklet' 指令 | skyline-skills/skills/skyline-worklet/SKILL.md:64 | 待实现 | 缺 JS AST 与 WXML 事件绑定映射 |
| 152 | 1E JS/worklet | 手势事件回调必须含 'worklet' 指令 | skyline-skills/skills/skyline-components/references/gesture/gesture-system.md:185 | 待实现 | 缺 JS AST 与 WXML 事件绑定映射 |
| 153 | 1E JS/worklet | 路由动画处理函数必须含 'worklet' 指令 | skyline-skills/skills/skyline-route/SKILL.md:61 | 待实现 | 缺 JS AST 与路由 builder 识别 |
| 154 | 1E JS/worklet | applyAnimatedStyle 的 updater 必须含 'worklet' 指令 | skyline-skills/skills/skyline-worklet/references/core/worklet-overview.md:134 | 待实现 | 缺 JS AST 调用点识别 |
| 155 | 1E JS/worklet | worklet 内不能直接调用 wx.* API | skyline-skills/skills/skyline-worklet/SKILL.md:132 | 待实现 | 缺 worklet 作用域数据流分析 |
| 156 | 1E JS/worklet | worklet 内不能解构 this.data | skyline-skills/skills/skyline-worklet/SKILL.md:136 | 待实现 | 缺 worklet 作用域数据流分析 |
| 157 | 1E JS/worklet | worklet 内调用页面方法必须 bind(this) | skyline-skills/skills/skyline-worklet/SKILL.md:111 | 待实现 | 缺 worklet 作用域数据流分析 |
| 158 | 1E JS/worklet | SharedValue 读写必须通过 .value | skyline-skills/skills/skyline-worklet/SKILL.md:79 | 待实现 | 缺 worklet 作用域数据流分析 |
| 159 | 1E JS/worklet | worklet.scrollViewContext.scrollTo 只能在 worklet 内调用 | skyline-skills/skills/skyline-scroll-api/SKILL.md:109 | 待实现 | 缺 JS 调用点与函数指令分析 |
| 160 | 1E JS/worklet | 获取 ScrollViewContext 时 scroll-view 必须开启 enhanced | skyline-skills/skills/skyline-scroll-api/SKILL.md:58 | 待实现 | 缺 JS selector 与 WXML 节点跨文件分析 |
| 161 | 1E JS/worklet | DraggableSheetContext.scrollTo 的 size 与 pixels 不能同时传 | skyline-skills/skills/skyline-scroll-api/SKILL.md:68 | 待实现 | 缺 JS 对象字面量参数检查 |
| 162 | 1E JS/worklet | worklet 版滚动控制必须用 NodesRef.ref() | skyline-skills/skills/skyline-scroll-api/SKILL.md:82 | 待实现 | 缺 JS 调用链识别 |
| 163 | 1E JS/worklet | wx.router.* 不支持小程序插件 | skyline-skills/skills/skyline-route/references/api/router-api.md:9 | 待实现 | 缺插件工程识别 |
| 164 | 1E JS/worklet | 自定义路由仅用于连续两个 Skyline 页面 | skyline-skills/skills/skyline-route/SKILL.md:55 | 待实现 | 缺路由调用与页面 renderer 配置联合分析 |
| 165 | 1E JS/worklet | routeType 必须是已注册类型或 wx:// 预设类型 | skyline-skills/skills/skyline-route/SKILL.md:109-123 | 待实现 | 缺路由注册表收集与调用点匹配 |
| 166 | 1E JS/worklet | 纵向下拉/多方向返回时需配置 associative-container=pop-gesture | skyline-skills/skills/skyline-route/references/pop-gesture/pop-gesture.md:61-71 | 待实现 | 缺 popGestureDirection 与 WXML scroll-view 联合检查 |
| 167 | 1E JS/worklet | 不支持 this.animate API | skyline-skills/skills/skyline-overview/references/migration/compatibility.md:301 | 待实现 | 缺 JS API 调用扫描 |
| 168 | 1E JS/worklet | 不支持 Page.onPageScroll | skyline-skills/skills/skyline-overview/references/migration/compatibility.md:300 | 待实现 | 缺 JS 生命周期扫描 |
| 169 | 1E JS/worklet | worklet 仅在 Skyline 渲染模式下可用 | skyline-skills/skills/skyline-worklet/SKILL.md:128 | 待实现 | 缺 worklet 使用与页面 renderer 配置联合检查 |
| 170 | 1E JS/worklet | 使用 worklet 时开发者工具需开启 ES5 编译 | skyline-skills/skills/skyline-worklet/references/core/worklet-overview.md:11 | 待实现 | 缺开发者工具配置读取 |
| 171 | 1E JS/worklet | 小程序插件内不能使用 worklet.scrollViewContext.scrollTo | skyline-skills/skills/skyline-scroll-api/SKILL.md:110 | 待实现 | 缺插件工程识别与 API 调用扫描 |

## 待实现汇总

共 69 条，按所需能力归并：

- **需要 WXSS 数据表/值语义扩充：12 条**（#30、#33、#41、#42、#43、#44、#58、#59、#60、#61、#62、#63）。现有 `unsupported-property` / `unsupported-value` / `multi-value-limit` 框架可直接复用，主要是补数据与简写分析。
- **需要 WXSS 默认值语义检测：6 条**（#64、#65、#66、#68、#69、#70）。需知道元素是否显式声明相关属性，并读取 `defaultDisplayBlock` / `defaultContentBox`。
- **需要 WXML 组件属性/结构扩充（单文件）：12 条**（#82、#103、#109、#111、#112、#114、#115、#121、#122、#123、#124、#127）。大部分可扩 `skyline/component-attribute` 数据表或加一个结构规则。
- **需要跨文件契约：4 条**（#116、#120、#164、#166）。需要页面配置、跨页面 key 配对或 JS 调用与 WXML 联合分析。
- **需要 JSON 配置规则：16 条**（#128–#137、#140、#142、#145–#147、#149）。`lib-version` / `renderer-options` 已覆盖 #138、#139、#141、#143、#144、#148、#150。
- **需要 JS AST / worklet / API 扫描：19 条**（#151–#163、#165、#167–#171）。需引入 JS 解析、函数作用域与调用链分析，是成本最高的一批。
