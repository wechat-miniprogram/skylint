# 发版

`skylint-core` 依赖三个内网 crate（`common` / `diag` / `compiler-graph`），公开的
GitHub Actions runner 拉不到，所以发版打包在能访问内网的机器上跑；最后一个动作才是
调 GitHub API 把产物挂到 Release 上。

脚本在 `crates/xtask`，只有构建、打包、上传三件事，没有隐藏状态：

```bash
# 1. 提交 + 打 tag（tag 必须与 Cargo.toml 的版本一致）
git tag v2.0.0 && git push origin v2.0.0

# 2. 干跑：构建、打包、算校验和，打印将要调用的接口，不联网
cargo run -p xtask -- release

# 3. 真上传
GITHUB_TOKEN=*** cargo run -p xtask -- release --upload
```

## 多平台

四个目标平台：

| target | 说明 |
|---|---|
| `aarch64-apple-darwin` | macOS Apple Silicon |
| `x86_64-apple-darwin` | macOS Intel |
| `x86_64-unknown-linux-gnu` | Linux x64 |
| `x86_64-pc-windows-msvc` | Windows x64 |

`--target` 可重复，默认当前主机。**一台机器通常只能构建其中一部分**（Windows 目标必须在
Windows 上、或装好 MSVC 工具链），所以常规做法是每到一台机器就跑一次：

```bash
cargo run -p xtask -- release --target aarch64-apple-darwin --upload
cargo run -p xtask -- release --target x86_64-unknown-linux-gnu --upload
```

Release 按 tag 复用：第一次调用会创建 Release，之后几次往里追加产物；同名产物会先删后传，
所以重跑不会留下旧二进制。

## 产物

```
dist/
  skylint-2.0.0-aarch64-apple-darwin.tar.gz   # 内含 skylint + README.md + LICENSE
  SHA256SUMS
```

## 前置守卫

脚本会在动手前检查，任何一条不满足都直接退出：

- 工作区必须干净（发版只能从已提交的状态构建）；
- tag 必须存在且与 `Cargo.toml` 的 `[workspace.package] version` 一致；
- 目标必须在上面四平台之内。

## 为什么不放在 GitHub Actions

规则集与输入解析不依赖内网 crate，但这不足以让 CI 完成打包：`project.rs` 仍然需要
`compiler-graph`。把发版交给脚本还有一个好处——流程固定、可重跑、不依赖 runner 环境。
公开 CI 只负责 `fmt` 与（依赖解耦后）`clippy` / `test`。
