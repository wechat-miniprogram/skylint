//! 规则到「作业单该带什么」的静态映射。
//!
//! 三样东西按规则 id 声明：
//! - `facts`：这条规则的结论依赖哪些 CSS 属性，`job` 只内联这些属性的级联；
//! - `preconditions`：成立前提。三条改错的历史（透明盒子加 box-shadow、`min-height`
//!   当 `height`、WXS 同步测量）都是「前提不成立但没人说」；
//! - `counterexamples`：反例，防止 agent 过度发挥。
//!
//! 没声明的规则只拿 `layout`（便宜且通用），不猜。
//!
//! 这些知识属于「怎么改」，不属于「什么坏了」，所以放在迁移层而不是 `RuleMeta`；
//! 将来若规则数量继续增长再考虑搬进 `RuleMeta`，但不要两边各写一份。

pub struct Guidance {
    pub facts: &'static [&'static str],
    pub preconditions: &'static [&'static str],
    pub counterexamples: &'static [&'static str],
    pub family: &'static str,
}

const NONE: Guidance = Guidance {
    facts: &[],
    preconditions: &[],
    counterexamples: &[],
    family: "other",
};

pub fn guidance_for(rule: &str) -> &'static Guidance {
    match rule {
        // ---- 层叠（fixed 提升、层叠上下文失效）
        "skyline/fixed-z-index-inversion" => &Guidance {
            facts: &["z-index", "position", "transform", "contain", "filter"],
            preconditions: &[
                "Skyline 把 position:fixed 节点提升到页面根，祖先的层叠上下文不再隔离它们",
                "比较的是「WebView 中最外层层叠上下文祖先的 z-index」与「Skyline 中自身 z-index」",
            ],
            counterexamples: &[
                "不要用 transform / filter / contain 造层叠上下文来「修」层级——它同时会改变 fixed 的包含块，元素会整体偏移",
            ],
            family: "stacking",
        },
        "skyline/fixed-layer" => &Guidance {
            facts: &[
                "position",
                "transform",
                "filter",
                "contain",
                "perspective",
                "will-change",
            ],
            preconditions: &[
                "只有 transform / filter / perspective / contain(size|layout|paint) 会让 fixed 相对祖先定位",
                "`contain: style` 不建立包含块",
            ],
            counterexamples: &["不要为了修定位给祖先加 `contain` —— 它会顺带裁掉溢出内容"],
            family: "stacking",
        },
        // ---- 结构（盒模型、原生组件、滚动容器）
        "skyline/native-padding" => &Guidance {
            facts: &[
                "padding",
                "padding-top",
                "padding-right",
                "padding-bottom",
                "padding-left",
            ],
            preconditions: &[
                "这类原生组件没有 CSS 盒，`padding` 走的是组件自己的 sliver padding",
                "滚动方向上的 padding 仍然占位，不等于 CSS padding",
            ],
            counterexamples: &[
                "不要把 padding 从组件挪到外层 view 再指望布局等价——外层的盒子会多出一圈可点击区域",
            ],
            family: "structure",
        },
        "skyline/native-layout-ignored" => &Guidance {
            facts: &[
                "width",
                "height",
                "min-width",
                "min-height",
                "max-width",
                "max-height",
                "margin",
                "display",
                "position",
                "flex",
                "flex-grow",
                "flex-shrink",
                "flex-basis",
                "justify-content",
                "align-items",
                "gap",
                "overflow",
            ],
            preconditions: &[
                "这些组件没有 CSS 盒，布局类声明一律不生效（背景色等绘制属性仍生效）",
                "宿主尺寸由组件自身决定，只能通过组件属性控制",
            ],
            counterexamples: &[
                "不要用 `!important` 或提高选择器优先级试图让它生效——声明根本进不了布局",
            ],
            family: "structure",
        },
        "skyline/scroll-view-height" => &Guidance {
            facts: &[
                "height",
                "min-height",
                "max-height",
                "flex",
                "flex-grow",
                "flex-basis",
                "display",
            ],
            preconditions: &[
                "scroll-view 需要一个确定高度才能滚动；高度为 auto 时塌成 0",
                "`flex: 1` 只在父级是 flex 容器且父级自身有确定高度时才有确定高度",
            ],
            counterexamples: &["不要用 `height: 100%` 绕过——父级没有确定高度时它同样是 0"],
            family: "structure",
        },
        "skyline/scroll-view-type" => &Guidance {
            facts: &["display", "flex-direction"],
            preconditions: &["缺省 type 只渲染第一个子元素"],
            counterexamples: &["不要把子元素包进一个外层 view 了事——滚动与布局语义都会变"],
            family: "structure",
        },
        "skyline/scroll-view-family" => &Guidance {
            facts: &["display", "flex-direction"],
            preconditions: &["引擎只认直接子节点作为 header / body / section"],
            counterexamples: &["不要在中间插一层 view 做样式包装"],
            family: "structure",
        },
        "skyline/component-children" => &Guidance {
            facts: &[],
            preconditions: &["子节点数量与标签按渲染树判定，`<slot>` 是占位不是元素"],
            counterexamples: &["不要为了满足数量要求加空 view —— 它会参与布局"],
            family: "structure",
        },
        "skyline/scroll-gesture-conflict" => &Guidance {
            facts: &[],
            preconditions: &["滚动容器内的 touchmove 会与滚动抢手势"],
            counterexamples: &["不要用 `catchtouchmove` 一刀切——它会连带吃掉滚动"],
            family: "structure",
        },
        "skyline/default-layout-drift" => &Guidance {
            facts: &["display", "flex-direction", "align-items", "box-sizing"],
            preconditions: &[
                "`rendererOptions.skyline.defaultDisplayBlock: true` 时 Skyline 的 display 默认 block",
                "`defaultContentBox: true` 时 box-sizing 默认 content-box",
            ],
            counterexamples: &[
                "不要在页面级再写一遍 `display: block` —— 默认值已经在工程配置里改了",
            ],
            family: "structure",
        },
        // ---- 脚本（WXS / API / worklet）
        "skyline/wxs-sync-query" => &Guidance {
            facts: &[],
            preconditions: &[
                "WebView 的 DOM 能同步测量；Skyline 的布局在 UI 线程，引擎只提供回调式接口",
                "不带回调时返回默认值（矩形全 0、样式 undefined），不会报错",
            ],
            counterexamples: &[
                "不要在调用后立刻读返回值——必须改成回调，并在回调里算好、缓存",
                "不要在触摸回调里发起测量再同步使用——用上一次回调缓存的值",
            ],
            family: "script",
        },
        "skyline/worklet-directive" => &Guidance {
            facts: &[],
            preconditions: &["只有 `'worklet'` 指令的函数才会在 UI 线程执行"],
            counterexamples: &["不要给普通事件处理函数加 worklet —— 它拿不到 `this` 与组件实例"],
            family: "script",
        },
        "skyline/unsupported-api" => &Guidance {
            facts: &[],
            preconditions: &["这些 API 在 Skyline 下没有实现，调用会失败或返回 undefined"],
            counterexamples: &["不要用 try/catch 包住了事——调用根本没发生，后续状态不会更新"],
            family: "script",
        },
        // ---- 视觉（过渡、滤镜、文本）
        "skyline/transition-animation-conflict" => &Guidance {
            facts: &[
                "transition",
                "transition-property",
                "animation",
                "animation-name",
                "transform",
            ],
            preconditions: &[
                "同一属性上 transition 与 animation 同时存在时，Skyline 会先把元素推到终态",
            ],
            counterexamples: &["不要保留 transition 只为了「兜底」——两者必须二选一"],
            family: "visual",
        },
        "skyline/transition-property" => &Guidance {
            facts: &["transition", "transition-property"],
            preconditions: &["带厂商前缀的属性名会让整条 transition 声明被丢弃"],
            counterexamples: &["不要只删前缀而不补标准属性名——那等于删掉了整个过渡"],
            family: "visual",
        },
        "skyline/filter-limit" => &Guidance {
            facts: &["filter", "backdrop-filter", "box-shadow"],
            preconditions: &["Skyline 的 filter 不支持 drop-shadow"],
            counterexamples: &[
                "元素透明、图形在背景图或字体里时，不要用 box-shadow 替代——它沿边框盒画出一圈方框",
            ],
            family: "visual",
        },
        "skyline/vendor-prefix" => &Guidance {
            facts: &[
                "animation",
                "animation-name",
                "transition",
                "transition-property",
            ],
            preconditions: &["Skyline 只认无前缀的属性和关键帧名"],
            counterexamples: &["不要删掉 `@-webkit-keyframes` 就完事——必须补一份同名无前缀关键帧"],
            family: "visual",
        },
        "skyline/no-inline-text" | "skyline/inline-run" | "skyline/text-run-block" => &Guidance {
            facts: &["display"],
            preconditions: &["文本与块级元素混排时行盒会被拆开"],
            counterexamples: &[
                "不要把文本包进 view —— 行内与块级的分界会变",
                "不要把同一段文本拆成并列的 `<text>` —— Skyline 下它们是各自独立的块级盒子",
            ],
            family: "visual",
        },
        "skyline/text-overflow-ellipsis" => &Guidance {
            facts: &[
                "white-space",
                "overflow",
                "overflow-x",
                "text-overflow",
                "display",
                "width",
            ],
            preconditions: &[
                "单行省略需要三件套同时成立：nowrap、overflow hidden、宽度确定",
                "`white-space` 会继承，祖先的 nowrap 同样有效",
            ],
            counterexamples: &["不要只加 `text-overflow: ellipsis` —— 没有 nowrap 与裁剪它不生效"],
            family: "visual",
        },
        "skyline/text-hard-break-ellipsis" => &Guidance {
            facts: &["white-space", "word-break", "overflow-wrap"],
            preconditions: &["Skyline 在长英文串上的断行策略与 WebView 不同"],
            counterexamples: &["不要用 `word-break: break-all` 一刀切——中文标点会被拆开"],
            family: "visual",
        },
        "skyline/text-decoration-target" => &Guidance {
            facts: &[
                "text-decoration",
                "text-decoration-line",
                "text-decoration-color",
                "display",
            ],
            preconditions: &["装饰线画在声明它的元素上，不跟随子元素"],
            counterexamples: &["不要给容器加下划线指望覆盖文本——空容器不画线"],
            family: "visual",
        },
        "skyline/implicit-color" => &Guidance {
            facts: &["color", "background-color"],
            preconditions: &["Skyline 的默认文字颜色与 WebView 不同"],
            counterexamples: &[
                "不要依赖 `inherit` / `currentColor` 传递——链上任一环缺失就会落到默认色",
            ],
            family: "visual",
        },
        _ => &NONE,
    }
}

/// 单元内改法的分组：同类问题一次改完，diff 才能归因。
pub fn family_of(rule: &str) -> &'static str {
    guidance_for(rule).family
}
