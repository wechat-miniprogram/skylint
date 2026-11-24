# skylint

Skyline 小程序迁移检查工具。适合 Webview 小程序使用 Agent 一轮迁移到 Skyline 渲染、或存量 Skyline 小程序反向适配 PC 端等不支持 Skyline 的环境到 Webview 渲染。

> [!NOTE]
> 本工具已完全重写。极大程度增加了规则条目数量；同时通过引入小程序源码工程图解析、glass-easel 模板编译器和 CSS Parser 提高了规则的准确性。
>
> 项目在快速迭代中。

## 构建

```bash
cargo build --release       
```

## 安装

```bash
# 使用 cargo
cargo install skylint

# 使用 mise
mise use -g github:wechat-miniprogram/skylint
mise use -g cargo:skylint
```

## 使用

```bash
# 默认检查当前目录
skylint

# 指定项目根
skylint ../my-miniapp

# 只检查一个文件
skylint page.wxml
skylint app.wxss

# 机器可读输出（CI / Agent 使用）
skylint --json

# 跳过某些规则（可重复）
skylint --ignore skyline/no-pseudo --ignore skyline/flex-direction

# 只跑某几条规则（写规则、回归时用）
skylint --only skyline/box-sizing
```

### 收窄范围

默认检查整个工程。以下开关都在**全量分析之后**过滤诊断，所以跨文件规则
（组件是否注册、样式继承、共享样式）仍然看到完整工程：

```bash
# 只看某个页面 / 组件（`unit:` 前缀与 `.wxml` 后缀都可省）
skylint --unit pages/detail/index
skylint --unit unit:pages/detail/index.wxml --unit components/card/card

# 只看路径匹配的诊断（glob：`*` 跨 `/`）
skylint --include '**/components/**' --exclude '**/vendor/**'

# 只看 git 里改过的文件所影响的单元（默认对比 HEAD，含未跟踪新文件）
skylint --changed
skylint --changed --since origin/main

# 只保留不低于某级别的诊断
skylint --min-severity error
```

迁移命令按单元 / 分包 / 类型分工：

```bash
skylint migration plan --package mt            # 只看某个分包（root 或 name）
skylint migration plan --kind component        # 只看页面 / 只看组件
skylint migration plan --renderer webview      # 只看某渲染器下的单元
skylint migration job --unit pages/detail/index
```

从代码中导出的全部规则见 [docs/RULES.md](docs/RULES.md)。

### 抑制单行

注释紧贴在被抑制行的上一行：

```html
<!-- skylint-disable-next-line -->
<view>{{a}}<image/></view>

<!-- 只抑制指定规则 -->
<!-- skylint-disable-next-line skyline/no-pseudo -->
<view class="a b" />
```

WXSS 里用 `/* skylint-disable-next-line */` 同样有效。

退出码：`0` 没有问题，`1` 达到失败阈值（默认有 error），`2` 项目加载失败。
阈值可用 `--fail-on warning` 收紧、或用 `--max-warnings 20` 给 warning 留额度。

只想要检查结果、不要 CLI 依赖时，直接依赖库 `skylint`；分层与用法见
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)。

## License

MIT
