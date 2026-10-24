# 发版

`skylint-core` 已无内网依赖，公开 runner 与内网机器都能构建。发版仍走脚本
（`crates/xtask`），理由是可重跑、可干跑、流程固定，不依赖 runner 的隐式状态：
构建、打包、上传三件事，没有隐藏状态。

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

## 与 CI 的分工

`.github/workflows/ci.yml` 只做 `fmt` / `clippy` / `test` / MSRV 检查，不产出二进制。
发版保持「本地打 tag + 跑 xtask」的手动节奏：先干跑确认，再 `--upload`。

如果要交给 GitHub Actions 打包，直接把 matrix 的每一格写成
`cargo run -p xtask -- release --target <triple> --upload` 即可——脚本本身不假设宿主环境。
