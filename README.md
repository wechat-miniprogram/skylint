# skylint

Skyline 小程序迁移检查工具。适合 Webview 小程序使用 Agent 一轮迁移到 Skyline 渲染、或存量 Skyline 小程序反向适配 PC 端等不支持 Skyline 的小程序的 Webview 环境。

> [!NOTE]
> 本工具已完全重写。极大程度增加了规则条目数量；同时通过引入小程序源码工程图解析、glass-easel 模板编译器和 CSS Parser 提高了规则的准确性。

## 构建

此项目目前处于半开源阶段，`common` / `diag` / `compiler-graph` 等模块源码来自微信内部项目 minidev；规则集等部分为开源。

```bash
# 1. 把 minidev clone 到与 skylint 同级，或在下面的 patch 里改成任意路径
#    git clone <minidev> ../minidev

# 2. 生成本地 cargo 配置
cp .cargo/config.toml.example .cargo/config.toml

# 3. 构建
cargo build --release
```

外部用户使用可以直接安装预编译二进制。

## 使用

```bash
# 默认检查当前目录
skylint

# 指定项目根
skylint ../my-miniapp

# 机器可读输出（CI / Agent 使用）
skylint --format json

# 跳过某些规则（可重复）
skylint --ignore skyline/no-pseudo --ignore skyline/flex-direction
```

全部规则见 [docs/RULES.md](docs/RULES.md)（由代码生成，不手工维护）。

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

退出码：`0` 没有问题，`1` 有 error 级问题，`2` 项目加载失败。

## License

MIT
