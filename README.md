# skylint

Skyline 小程序迁移检查工具。

检查 WebView 小程序的代码在迁到 Skyline 渲染后会不会出问题：哪些属性/值不再生效、
哪些默认值变了、哪些组件的父子结构约束被违反。目标是让迁移从「上线后逐个页面肉眼比对」
变成「迁移前一次性拿到问题清单」。

## 状态

正在重写（Rust）。`2.0.0` 是重写后的第一个版本，规则命名与 CLI 参数都与 1.x 不兼容——
1.x 的规则名本身有若干不一致，兼容只会把问题一起继承下来。

进度与设计见 [docs/PLAN.md](docs/PLAN.md)。

当前可用：工程视图（路由 → 实际源文件、组件闭包、Skyline 页面判定）、text/json 报告、
退出码。规则尚未接入。

## 构建

依赖 [minidev](https://github.com/wechat-miniprogram/minidev) 的 `compiler-graph` /
`compiler-diag` / `compiler-common` 三个 crate（工程图与诊断 DTO）。minidev 目前只有内部
remote，因此：

```bash
# 1. 把 minidev clone 到与 skylint 同级，或在下面的 patch 里改成任意路径
#    git clone <minidev> ../minidev

# 2. 生成本地 cargo 配置
cp .cargo/config.toml.example .cargo/config.toml

# 3. 构建
cargo build --release
```

外部用户拿不到 minidev 源码，请安装预编译二进制（npm 分发包，见下）。

## 使用

```bash
# 默认检查当前目录
skylint

# 指定项目根（含 project.config.json）
skylint ../my-miniapp

# 机器可读输出（CI 用）
skylint --format json
```

退出码：`0` 没有问题，`1` 有 error 级问题，`2` 项目加载失败。

## License

MIT
