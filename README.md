# skylint

Skyline 小程序迁移检查工具。适合 Webview 小程序使用 Agent 一轮迁移到 Skyline 渲染、或存量 Skyline 小程序反向适配 PC 端等不支持 Skyline 的小程序的 Webview 环境。

> [!NOTE]
> 本工具已完全重写。极大程度增加了规则条目数量；同时通过引入小程序源码工程图解析、glass-easel 模板编译器和 CSS Parser 提高了规则的准确性。

## 构建

```bash
cargo build --release        # 产物在 target/release/skylint
# 或
cargo install --path .
```

没有任何内网依赖：工程加载器、模板解析、样式解析都在本仓库里，`cargo build` 拿到的就是你
跑起来的那份二进制。需要 Rust 1.97+（oxc 前端的下限）。

也可以直接装预编译二进制，见 Release 页。

## 使用

```bash
# 默认检查当前目录
skylint

# 指定项目根
skylint ../my-miniapp

# 只检查一个文件（不要求工程结构；跨文件规则不出结果）
skylint page.wxml
skylint app.wxss

# 机器可读输出（CI / Agent 使用）
skylint --json

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
