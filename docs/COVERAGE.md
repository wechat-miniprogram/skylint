# 知识库约束覆盖台账

分母：175 条（口径：给定输入文件即可静态判定真假；其中 #172-#175 是实现期补入的审计漏项）。

统计：已实现 141 条 / 待实现 0 条 / 不实现 34 条。

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
| 47 | 1A WXSS 值 | `radial-gradient` 颜色停止位置仅支持 % | skyline-skills/skills/skyline-wxss/SKILL.md:79 | 已实现 | `skyline/gradient-limit`（按引擎放宽：`%` 与 `px` 都可用，见 AUDIT-2026-10 §2.6） |
| 48 | 1A WXSS 值 | `linear-gradient` 停止位置仅支持 % 与固定长度单位 | skyline-skills/skills/skyline-wxss/SKILL.md:80 | 已实现 | `skyline/gradient-limit` |
| 49 | 1A WXSS 值 | `mask-image` 不支持渐变，仅 url() | skyline-skills/skills/skyline-wxss/SKILL.md:107 | 不实现 | 引擎支持 linear/radial 渐变（`widgets/decorated_box.cc:596-612`），该条知识库断言不成立；只剩 repeating 与 mask 的 conic 不支持，由 `skyline/gradient-unsupported` 覆盖 |
| 50 | 1A WXSS 值 | `filter/backdrop-filter` 不支持 url() 与 drop-shadow() | skyline-skills/skills/skyline-wxss/SKILL.md:97 | 已实现 | `skyline/filter-limit` |
| 51 | 1A WXSS 值 | `filter/backdrop-filter` 不支持多个函数组合 | skyline-skills/skills/skyline-wxss/SKILL.md:97 | 已实现 | `skyline/filter-limit` |
| 52 | 1A WXSS 值 | `background-image` 最多 2 层 | skyline-skills/skills/skyline-wxss/SKILL.md:85 | 已实现 | `skyline/multi-value-limit` |
| 53 | 1A WXSS 值 | `mask-image` 最多 2 层 | skyline-skills/skills/skyline-wxss/SKILL.md:85 | 已实现 | `skyline/multi-value-limit` |
| 54 | 1A WXSS 值 | `background-position` 最多 2 组 | skyline-skills/skills/skyline-wxss/SKILL.md:86 | 已实现 | `skyline/multi-value-limit` |
| 55 | 1A WXSS 值 | `background-repeat` 不支持多组值 | skyline-skills/skills/skyline-wxss/SKILL.md:87 | 不实现 | 引擎接受 1~2 组（`css/image_property_utils.h:517-524`），上限已改 2，>2 才报 |
| 56 | 1A WXSS 值 | `background-size` 不支持多组值 | skyline-skills/skills/skyline-wxss/SKILL.md:87 | 不实现 | 引擎接受 1~2 组（`css/image_property_utils.h:650-660`），上限已改 2，>2 才报 |
| 57 | 1A WXSS 值 | `calc()` 不支持角度计算 | skyline-skills/skills/skyline-wxss/SKILL.md:91 | 已实现 | `skyline/calc-angle` |
| 58 | 1A WXSS 值 | `background` 简写中 attachment/origin/clip 不生效 | skyline-skills/skills/skyline-wxss/SKILL.md:115 | 已实现 | `skyline/shorthand-partial` |
| 59 | 1A WXSS 值 | `text-decoration` 简写中 thickness 不生效 | skyline-skills/skills/skyline-wxss/SKILL.md:116 | 已实现 | `skyline/shorthand-partial` |
| 60 | 1A WXSS 值 | `mask` 简写中 origin/clip 不生效 | skyline-skills/skills/skyline-wxss/SKILL.md:117 | 已实现 | `skyline/shorthand-partial` |
| 61 | 1A WXSS 值 | `env()` 仅支持四个 safe-area 变量 | skyline-skills/skills/skyline-wxss/references/basics.md:24 | 已实现 | `skyline/value-degraded` |
| 62 | 1A WXSS 值 | transition/animation 不支持布局属性 | skyline-skills/skills/skyline-wxss/references/animation.md:35 | 已实现 | `skyline/transition-property` |
| 63 | 1A WXSS 值 | `transition-property` 仅支持文档列出的可动画属性 | skyline-skills/skills/skyline-wxss/references/animation.md:58-61 | 已实现 | `skyline/transition-property` |
| 64 | 1B 默认值/选择器 | Skyline 下元素默认 `display: flex` | skyline-skills/skills/skyline-wxss/SKILL.md:163 | 已实现 | `skyline/default-layout-drift`（工程级：app 级未开 `defaultDisplayBlock` 时一条 info） |
| 65 | 1B 默认值/选择器 | Skyline 下 `position` 默认 relative | skyline-skills/skills/skyline-wxss/SKILL.md:164 | 已实现 | `skyline/default-layout-drift`（无开关可对齐，并入 #64 那条 info 的消息，不单独成条） |
| 66 | 1B 默认值/选择器 | Skyline 下 `box-sizing` 默认 border-box | skyline-skills/skills/skyline-wxss/SKILL.md:165 | 已实现 | `skyline/default-layout-drift`（工程级：app 级未开 `defaultContentBox` 时一条 info） |
| 67 | 1B 默认值/选择器 | Skyline 下 `flex-direction` 默认 column | skyline-skills/skills/skyline-wxss/SKILL.md:166 | 已实现 | `skyline/flex-direction` |
| 68 | 1B 默认值/选择器 | Skyline 下 `align-items` 默认 stretch | skyline-skills/skills/skyline-wxss/SKILL.md:167 | 已实现 | `skyline/default-layout-drift`（随 #64 的 `defaultDisplayBlock` 一起对齐 WebView） |
| 69 | 1B 默认值/选择器 | Skyline 下 `text-align` 默认 start | skyline-skills/skills/skyline-wxss/SKILL.md:168 | 已实现 | `skyline/default-layout-drift`（无开关可对齐，并入 #64 那条 info 的消息，不单独成条） |
| 70 | 1B 默认值/选择器 | Skyline 下 `mask-repeat` 默认 no-repeat | skyline-skills/skills/skyline-wxss/SKILL.md:169 | 不实现 | 差异只在 mask 图小于元素盒子时可见，静态无法判断；businessview 有 317 处 mask 声明、多数不写 mask-repeat，逐条报是纯噪音 |
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
| 82 | 1B 默认值/选择器 | `navigator` 只能嵌套 text 组件或文本节点 | skyline-skills/skills/skyline-overview/references/introduction/component-support.md:23 | 已实现 | `skyline/component-children`（`only:text`） |
| 83 | 1B 默认值/选择器 | `picker-view` 不支持 indicator-class / mask-style | skyline-skills/skills/skyline-overview/references/introduction/component-support.md:33 | 已实现 | `skyline/component-attribute` |
| 84 | 1C 组件契约 | `scroll-view` 必须显式声明 `type` | skyline-skills/skills/skyline-components/references/scroll/scroll-view.md:16 | 已实现 | `skyline/scroll-view-type` |
| 85 | 1C 组件契约 | `scroll-view` 的 `type` 仅接受 list/custom/nested | skyline-skills/skills/skyline-components/references/scroll/scroll-view.md:20-24 | 已实现 | `skyline/scroll-view-type` |
| 86 | 1C 组件契约 | `list-view`/`grid-view` 必须是 custom 模式或 sticky-section 的直接子节点 | skyline-skills/skills/skyline-components/references/scroll/list-grid-view.md:5 | 已实现 | `skyline/scroll-view-family` |
| 87 | 1C 组件契约 | `list-builder` 定高模式必须声明 child-height | skyline-skills/skills/skyline-components/references/scroll/list-grid-view.md:271 | 已实现 | `skyline/scroll-view-family` |
| 88 | 1C 组件契约 | `grid-view` 必须声明 cross-axis-count | skyline-skills/skills/skyline-components/references/scroll/list-grid-view.md:272 | 不实现 | 引擎默认 `cross_axis_count_{2}`（`shadow/components/grid_view_shadow_node.h:38`），省略合法 |
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
| 103 | 1C 组件契约 | 自定义下拉刷新必须提供 slot=refresher 节点 | skyline-skills/skills/skyline-components/references/scroll/scroll-view.md:377 | 已实现 | `skyline/component-children`（`requires-child:slot=refresher`） |
| 104 | 1C 组件契约 | refresher-default-style 仅接受 black/white/none | skyline-skills/skills/skyline-components/references/scroll/scroll-view.md:195 | 已实现 | `skyline/component-attribute` |
| 105 | 1C 组件契约 | scroll-into-view-alignment 仅接受 start/center/end/nearest | skyline-skills/skills/skyline-components/references/scroll/scroll-view.md:113 | 已实现 | `skyline/component-attribute` |
| 106 | 1C 组件契约 | 横向滚动的 scroll-view 必须开启 enable-flex | skyline-skills/skills/skyline-components/SKILL.md:76 | 已实现 | `skyline/horizontal-scroll-enable-flex` |
| 107 | 1C 组件契约 | swiper layout-type 仅接受 normal/stackLeft/stackRight/tinder/transformer | skyline-skills/skills/skyline-components/references/layout/swiper.md:44-50 | 已实现 | `skyline/component-attribute` |
| 108 | 1C 组件契约 | swiper transformer-type 仅接受六个合法值 | skyline-skills/skills/skyline-components/references/layout/swiper.md:56-63 | 已实现 | `skyline/component-attribute` |
| 109 | 1C 组件契约 | transformer-type 仅在 layout-type=transformer 时生效 | skyline-skills/skills/skyline-components/references/layout/swiper.md:54 | 已实现 | `skyline/component-attribute`（`[requires]`） |
| 110 | 1C 组件契约 | swiper indicator-type 仅接受文档列出的值 | skyline-skills/skills/skyline-components/references/layout/swiper.md:67-84 | 已实现 | `skyline/component-attribute` |
| 111 | 1C 组件契约 | stack/tinder 布局时 indicator-type 仅支持 normal | skyline-skills/skills/skyline-components/references/layout/swiper.md:87 | 已实现 | `skyline/component-attribute`（`[requires]`） |
| 112 | 1C 组件契约 | indicator-type 为 scrollFixedCenter/swap/swapYRotation 时不支持 circular | skyline-skills/skills/skyline-components/references/layout/swiper.md:88 | 已实现 | `skyline/component-attribute`（`[requires]`） |
| 113 | 1C 组件契约 | swiper direction 仅接受 all/positive/negative | skyline-skills/skills/skyline-components/references/layout/swiper.md:109-114 | 已实现 | `skyline/component-attribute` |
| 114 | 1C 组件契约 | share-element 必须声明 key | skyline-skills/skills/skyline-components/references/special/share-element.md:63 | 已实现 | `skyline/share-element-key` |
| 115 | 1C 组件契约 | 同页 share-element 的 key 必须唯一 | skyline-skills/skills/skyline-components/references/special/share-element.md:63 | 已实现 | `skyline/share-element-key`（仅文件内唯一，跨文件是已知边界） |
| 116 | 1C 组件契约 | share-element 的 transform 必须在两个页面都设置 | skyline-skills/skills/skyline-components/references/special/share-element.md:359 | 不实现 | 配对的两个 share-element 是运行期导航关系（同一 key 的两个元素可分散在任意两页），静态工程图里无法确定哪两页会配对，判必然误报 |
| 117 | 1C 组件契约 | share-element rect-tween-type 仅接受文档列出的轨迹值 | skyline-skills/skills/skyline-components/references/special/share-element.md:85-98 | 已实现 | `skyline/component-attribute` |
| 118 | 1C 组件契约 | shuttle-on-push/pop 仅接受 to/from | skyline-skills/skills/skyline-components/references/special/share-element.md:78-83 | 已实现 | `skyline/component-attribute` |
| 119 | 1C 组件契约 | snapshot mode 仅接受 view/picture | skyline-skills/skills/skyline-components/references/special/snapshot.md:11 | 已实现 | `skyline/component-attribute` |
| 120 | 1C 组件契约 | snapshot 只能在 Skyline 页面使用 | skyline-skills/skills/skyline-components/references/special/snapshot.md:325 | 不实现 | `snapshot` 是 Skyline 专属能力，用到它本身就表明页面已在 Skyline；纯 WebView 页面写它在 WebView 下同样无效，不属于 Skyline 语义差异 |
| 121 | 1C 组件契约 | 手势组件必须且只能有一个直接子节点 | skyline-skills/skills/skyline-components/references/gesture/gesture-system.md:187 | 已实现 | `skyline/component-children`（`exactly:1`，8 个手势组件；互斥分支合并计数） |
| 122 | 1C 组件契约 | simultaneous-handlers 必须双方互相声明 | skyline-skills/skills/skyline-components/references/gesture/gesture-negotiation.md:51 | 不实现 | 示例里 `simultaneous-handlers` 的取值始终是动态数组（`{{['outer']}}`），静态读不到 tag 列表；按缺失报会把所有动态值判成违规 |
| 123 | 1C 组件契约 | native-view 仅接受 scroll-view/swiper | skyline-skills/skills/skyline-components/references/gesture/gesture-negotiation.md:127-129 | 已实现 | `skyline/component-attribute`（`[enum]` native-view） |
| 124 | 1C 组件契约 | 手势组件仅在 Skyline 下可用 | skyline-skills/skills/skyline-components/references/gesture/gesture-system.md:186 | 不实现 | 手势组件仅在 Skyline 下可用——用到手势组件即已决定用 Skyline，与 #120 同理，不是迁移差异 |
| 125 | 1C 组件契约 | input type 仅接受 text/number/idcard/digit/nickname | skyline-skills/skills/skyline-components/references/form/input.md:44-50 | 已实现 | `skyline/component-attribute` |
| 126 | 1C 组件契约 | input confirm-type 仅接受 send/search/next/go/done | skyline-skills/skills/skyline-components/references/form/input.md:54-60 | 已实现 | `skyline/component-attribute` |
| 127 | 1C 组件契约 | input 不支持 placeholder-class / safe-password-* | skyline-skills/skills/skyline-components/references/form/input.md:306-313 | 已实现 | `skyline/component-attribute`（`[unsupported]`：placeholder-class、safe-password-* 前缀） |
| 128 | 1D 配置 | 全局 Skyline 时 app.json renderer 必须为 skyline | skyline-skills/skills/skyline-config/SKILL.md:140 | 不实现 | 不实现：全局 Skyline 是迁移选择而非缺陷；页面级 renderer:webview 是受支持的 opt-out（skylint 已按页面 opt-out 跳过） |
| 129 | 1D 配置 | 全局 Skyline 时 componentFramework 必须为 glass-easel | skyline-skills/skills/skyline-config/SKILL.md:141 | 不实现 | 不实现：minidev build 实测 已硬拦截（skyline 页缺 componentFramework=glass-easel 直接报错，minidev M4） |
| 130 | 1D 配置 | 全局 Skyline 时 lazyCodeLoading 必须为 requiredComponents | skyline-skills/skills/skyline-config/SKILL.md:142 | 不实现 | 不实现：minidev build 实测 已硬拦截（skyline + lazyCodeLoading≠requiredComponents 直接报错） |
| 131 | 1D 配置 | Skyline 页面必须 navigationStyle=custom | skyline-skills/skills/skyline-config/SKILL.md:143 | 不实现 | 不实现：minidev build 实测 已处理——check_renderer 把 skyline 页的 navigationStyle 改写成 custom |
| 132 | 1D 配置 | Skyline 项目 app.json window.navigationStyle 必须为 custom | skyline-skills/skills/skyline-config/references/app-config.md:133 | 不实现 | 不实现：minidev build 实测 已处理（同上，逐页改写，app 级 window.navigationStyle 不参与判定） |
| 133 | 1D 配置 | Skyline 页面应 disableScroll=true | skyline-skills/skills/skyline-config/references/page-config.md:51 | 不实现 | 不实现：minidev build 实测 已处理——check_renderer 把 skyline 页的 disableScroll 缺省改写为 true |
| 134 | 1D 配置 | page.json renderer 仅接受 webview/skyline | skyline-skills/skills/skyline-config/references/page-config.md:83-98 | 不实现 | 不实现：minidev build 实测 已硬拦截（页面 json 的 renderer 走 schema 枚举校验） |
| 135 | 1D 配置 | componentFramework 仅接受 exparser/glass-easel | skyline-skills/skills/skyline-config/references/app-config.md:30-33 | 不实现 | 不实现：minidev build 实测 已硬拦截（页面 json 的 componentFramework 走 schema 枚举校验） |
| 136 | 1D 配置 | lazyCodeLoading 仅接受 requiredComponents | skyline-skills/skills/skyline-config/references/app-config.md:47 | 不实现 | 不实现：minidev build 实测 已硬拦截（app.json lazyCodeLoading 非法值直接报错） |
| 137 | 1D 配置 | 页面 renderer=skyline 时 componentFramework 必须为 glass-easel | skyline-skills/skills/skyline-config/references/page-config.md:130-137 | 不实现 | 不实现：minidev build 实测 已硬拦截（同 #129） |
| 138 | 1D 配置 | tagNameStyleIsolation 仅接受 isolated/legacy | skyline-skills/skills/skyline-config/references/app-config.md:69 | 已实现 | `skyline/renderer-options` |
| 139 | 1D 配置 | rendererOptions.skyline 下各开关必须为 boolean | skyline-skills/skills/skyline-config/references/app-config.md:65-71 | 已实现 | `skyline/renderer-options` |
| 140 | 1D 配置 | backgroundColorContent 必须为 8 位十六进制颜色 | skyline-skills/skills/skyline-config/references/page-config.md:66 | 不实现 | 不实现：知识库只给类型 HexColor(#RRGGBBAA)，未说明 6 位十六进制与 rgba() 是否被接受；无源码证据，按格式报会误伤 |
| 141 | 1D 配置 | project.config.json libVersion 必须 ≥ 2.30.4 | skyline-skills/skills/skyline-config/references/project-config.md:67-71 | 已实现 | `skyline/lib-version` |
| 142 | 1D 配置 | 真机基础库必须 ≥ 3.0.2 | skyline-skills/skills/skyline-overview/references/migration/getting-started.md:9 | 不实现 | 不实现：真机基础库由微信客户端决定，project.config.json 的 libVersion 只影响开发者工具；可配置侧已由 `skyline/lib-version` 覆盖 |
| 143 | 1D 配置 | 页面级 rendererOptions 需基础库 ≥ 3.1.0 | skyline-skills/skills/skyline-config/references/page-config.md:111 | 已实现 | `skyline/lib-version` |
| 144 | 1D 配置 | rendererOptions.skyline.convertRpxToVw 必须为 boolean | skyline-skills/skills/skyline-config/references/app-config.md:145-153 | 已实现 | `skyline/renderer-options`（位置更正：只读 app.json 顶层，写进 rendererOptions 会报错） |
| 145 | 1D 配置 | project.config.json setting.skylineRenderEnable 必须为 boolean | skyline-skills/skills/skyline-config/references/project-config.md:18-29 | 已实现 | `skyline/config-ignored` |
| 146 | 1D 配置 | 分包可单独声明 renderer/componentFramework | skyline-skills/skills/skyline-overview/references/migration/best-practice.md:42-46 | 不实现 | 不实现：这是能力说明（分包可单独声明 renderer/componentFramework），不是缺陷；minidev 已按分包解析 |
| 147 | 1D 配置 | disableScroll 只能出现在 page.json | skyline-skills/skills/skyline-config/references/page-config.md:49 | 已实现 | `skyline/config-ignored` |
| 148 | 1D 配置 | defaultDisplayBlock=true 时 flex-direction 默认值变为 row | skyline-skills/skills/skyline-config/references/app-config.md:67 | 已实现 | `skyline/flex-direction` |
| 149 | 1D 配置 | defaultContentBox 会改变 box-sizing 默认值前提 | skyline-skills/skills/skyline-config/references/app-config.md:68 | 不实现 | 不实现：这是配置事实而非缺陷——`flex-direction` 规则已经把 defaultDisplayBlock 读进判定，defaultContentBox 同理属于规则前提 |
| 150 | 1D 配置 | keyframeStyleIsolation 仅接受 isolated/legacy | skyline-skills/skills/skyline-overview/references/migration/getting-started.md:35 | 已实现 | `skyline/renderer-options` |
| 151 | 1E JS/worklet | worklet 函数体首部必须含 'worklet' 指令 | skyline-skills/skills/skyline-worklet/SKILL.md:64 | 已实现 | `skyline/worklet-directive`（worklet 函数由 152/154/153 三个来源识别） |
| 152 | 1E JS/worklet | 手势事件回调必须含 'worklet' 指令 | skyline-skills/skills/skyline-components/references/gesture/gesture-system.md:185 | 已实现 | `skyline/worklet-directive` |
| 153 | 1E JS/worklet | 路由动画处理函数必须含 'worklet' 指令 | skyline-skills/skills/skyline-route/SKILL.md:61 | 已实现 | `skyline/worklet-directive`（builder 返回的 handle*Animation） |
| 154 | 1E JS/worklet | applyAnimatedStyle 的 updater 必须含 'worklet' 指令 | skyline-skills/skills/skyline-worklet/references/core/worklet-overview.md:134 | 已实现 | `skyline/worklet-directive` |
| 155 | 1E JS/worklet | worklet 内不能直接调用 wx.* API | skyline-skills/skills/skyline-worklet/SKILL.md:132 | 已实现 | `skyline/worklet-api-scope` |
| 156 | 1E JS/worklet | worklet 内不能解构 this.data | skyline-skills/skills/skyline-worklet/SKILL.md:136 | 已实现 | `skyline/worklet-data-freeze` |
| 157 | 1E JS/worklet | worklet 内调用页面方法必须 bind(this) | skyline-skills/skills/skyline-worklet/SKILL.md:111 | 不实现 | 「方法调用」与「把方法当回调传」在 AST 上都是成员访问，静态分不清哪些属于必须 bind 的跨线程传递 |
| 158 | 1E JS/worklet | SharedValue 读写必须通过 .value | skyline-skills/skills/skyline-worklet/SKILL.md:79 | 不实现 | 把 SharedValue 当对象整体传递是合法用法（运行时会被 Object.freeze），判断「是否通过 .value 读写」需要数据流分析，静态必误报 |
| 159 | 1E JS/worklet | worklet.scrollViewContext.scrollTo 只能在 worklet 内调用 | skyline-skills/skills/skyline-scroll-api/SKILL.md:109 | 已实现 | `skyline/worklet-api-scope` |
| 160 | 1E JS/worklet | 获取 ScrollViewContext 时 scroll-view 必须开启 enhanced | skyline-skills/skills/skyline-scroll-api/SKILL.md:58 | 不实现 | 「哪个 scroll-view」由运行时选择器决定（`.select('.x')` 与 wxml 类名跨文件对应），且 enhanced 是运行期能力开关，判错率高 |
| 161 | 1E JS/worklet | DraggableSheetContext.scrollTo 的 size 与 pixels 不能同时传 | skyline-skills/skills/skyline-scroll-api/SKILL.md:68 | 已实现 | `skyline/draggable-sheet-scroll-to` |
| 162 | 1E JS/worklet | worklet 版滚动控制必须用 NodesRef.ref() | skyline-skills/skills/skyline-scroll-api/SKILL.md:82 | 已实现 | `skyline/scroll-context-ref` |
| 163 | 1E JS/worklet | wx.router.* 不支持小程序插件 | skyline-skills/skills/skyline-route/references/api/router-api.md:9 | 已实现 | `skyline/unsupported-api`（`plugin-call:wx.router`） |
| 164 | 1E JS/worklet | 自定义路由仅用于连续两个 Skyline 页面 | skyline-skills/skills/skyline-route/SKILL.md:55 | 不实现 | 依赖运行期导航栈（连续两个 Skyline 页面），静态与编译期都无法确定跳转链 |
| 165 | 1E JS/worklet | routeType 必须是已注册类型或 wx:// 预设类型 | skyline-skills/skills/skyline-route/SKILL.md:109-123 | 已实现 | `skyline/route-type`（工程级收集 `addRouteBuilder` 注册名） |
| 166 | 1E JS/worklet | 纵向下拉/多方向返回时需配置 associative-container=pop-gesture | skyline-skills/skills/skyline-route/references/pop-gesture/pop-gesture.md:61-71 | 不实现 | 是否「纵向下拉/多方向返回」由页面路由配置与手势方向共同决定，静态不可判定 |
| 167 | 1E JS/worklet | 不支持 this.animate API | skyline-skills/skills/skyline-overview/references/migration/compatibility.md:301 | 已实现 | `skyline/unsupported-api`（`call:this.animate`） |
| 168 | 1E JS/worklet | 不支持 Page.onPageScroll | skyline-skills/skills/skyline-overview/references/migration/compatibility.md:300 | 已实现 | `skyline/unsupported-api`（`page-option:onPageScroll`） |
| 169 | 1E JS/worklet | worklet 仅在 Skyline 渲染模式下可用 | skyline-skills/skills/skyline-worklet/SKILL.md:128 | 不实现 | worklet 仅在 Skyline 模式下可用——用到 worklet 即已决定用 Skyline；纯 WebView 页面写 worklet 本身无效，不是迁移差异 |
| 170 | 1E JS/worklet | 使用 worklet 时开发者工具需开启 ES5 编译 | skyline-skills/skills/skyline-worklet/references/core/worklet-overview.md:11 | 已实现 | `skyline/worklet-compile-setting`（判据更正：是 `setting.compileWorklet`，不是 `es6`） |
| 171 | 1E JS/worklet | 小程序插件内不能使用 worklet.scrollViewContext.scrollTo | skyline-skills/skills/skyline-scroll-api/SKILL.md:110 | 已实现 | `skyline/worklet-api-scope`（compileType=plugin） |
| 172 | 1C 组件契约 | 暂不支持的组件（web-view / movable-area / movable-view / editor / progress 等） | skyline-skills/skills/skyline-overview/references/introduction/component-support.md:54-61 | 已实现 | `skyline/unsupported-component` |
| 173 | 1C 组件契约 | 内联文本只能放在 `<text>` 里，不能与元素混排 | skyline-skills/skills/skyline-components/references/media/text.md:5、component-support.md:111 | 已实现 | `skyline/no-inline-text`（只报「文本 + 元素混排」，纯文本节点不报） |
| 174 | 1D 配置 | app.json 的 rendererOptions 需基础库 ≥ 2.31.1 | skyline-skills/skills/skyline-config/references/project-config.md:66 | 已实现 | `skyline/lib-version`（`rendererOptions` 存在时阈值取 2.31.1；页面级更高阈值覆盖它） |
| 175 | 1D 配置 | convertRpxToVw 需基础库 ≥ 3.3.0 | skyline-skills/skills/skyline-config/references/project-config.md:68 | 已实现 | `skyline/lib-version`（rendererOptions 子树出现 convertRpxToVw 时阈值取 3.3.0） |

## 收口说明

171 条审计项全部落账，加上实现期补入的四条审计漏项，台账共 **175 行：141 条已实现、
34 条有意不实现、0 条待实现**（2026-10 按引擎源码复核后，7 条知识库断言被更正，4 条从
「已实现」改为「不实现」，详见 `docs/AUDIT-2026-10.md`）。

补入的四行都是分母自身的不完整，不是新增判断：#172（暂不支持的组件）、#173（内联文本必须用
`<text>`）的规则早在 P0/P1 就写了；#174/#175 是 `project-config.md:64-75` 里 rendererOptions
（2.31.1）与 convertRpxToVw（3.3.0）两档阈值——`skyline/lib-version` 一直按它们判，但枚举时
只收了 2.30.4 与 3.1.0 两档。

**规则行为在后续全量审计（PLAN 的 P5）里修过一轮**：9 条 P0 缺陷（`scroll-view type=default`
误报 error、无值属性漏报、`wx:if` 分支计数、页面 opt-out、内联 `<template>` 定义体不可见、
组件间标签隔离等）与一批口径统一。那些修复改变的是「怎么判」，不改变本台账「某条约束是否
有规则负责」的状态，所以行数与结论不变。

另有两条**已实现但不在知识库条目里**的规则，来源是引擎源码而非文档，故未计入分母：

- `skyline/no-pseudo`：Skyline 的选择器匹配只支持部分伪类，具体支持面随基础库版本变化
  （`changelog.md:205-207` 记录 `:not()`/`:only-child()`/`:empty()` 是后来加的），
  知识库没有一条「哪些伪类不支持」的清单；规则按引擎源码的匹配器实现，判定保守。
- `skyline/component-attribute` 里 `[requires]` 的跨属性条件：文档分散在属性表各处，
  台账按「一条约束一行」收了其中会单独成条的部分。

不实现的 30 条都写了具体理由（运行期才能判、静态必然误报、`minidev build` 已硬拦截、
或根本不是 Skyline 语义差异），见各行。

## 已知边界（2026-10 全量审计后补记）

以下都不是「漏了一条知识库条目」，而是**当前判据明确不覆盖**的形态，写在这里避免重复排查：

- **宿主 `plugins.*.genericsImplementation` 注入的组件不参与检查**：这些组件由插件页在
  运行时渲染，宿主仓库里没有静态引用点（`app.json` 的映射是给插件页用的）。无条件种子化
  会把整棵树拉进来、产出无法证伪的诊断，因此按「未知」处理。已知受影响位置
  （`miniprogram-skit_1/components-for-playlet/*`）已记录在迁移报告里。
- **`-webkit-*` 属性不整体入表**：2572 条声明里约 2200 条同块存在标准同名声明，两端一致；
  逐个报会把 vendored weui 的 autoprefixer 产物变成主噪声。只对「让整条 `transition`
  回滚」的形态单独报（`skyline/transition-property`）。
- **`calc()` 里角度与长度混算**：这是 CSS 类型错误，WebView 同样整条拒绝，不构成渲染器差异；
  原 `skyline/calc-angle` 已删除（2026-10）。
- **自定义组件宿主上的尺寸声明**：宿主在 `defaultBlockLayout` 下被强制 `display:inline`，
  但「宿主上 `height` / 纵向 `padding` / `width` 各自生不生效」还缺一次对照实验，判据未定。
- **`aspect-ratio` 之外的定高来源**（grid 轨道、`min-content` 等）在 `scroll-view-height`
  里按「不确定」处理，宁可漏报。
- **`share-element` 的 `wx:for` 静态 key**、**`conic-gradient` 的 px 停止位**、
  **`dead-selector` 对含内联 `<template name>` 文件的组合选择器**：判据已明确，但由于需要
  解析层新增字段（`in_for`）或新的 conic 参数解析，暂未实现；当前六个参考工程命中为 0。
