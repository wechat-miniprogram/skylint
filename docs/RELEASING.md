# 发版

推一个 `vX.Y.Z` tag 就完成发版：四平台二进制进 GitHub Release，同时把 crate 发到
crates.io。tag 必须是 `Cargo.toml` 的版本号——不一致会在第一步就失败，不会产出半成品。

```bash
# 1. 改版本号（唯一来源）
$EDITOR Cargo.toml          # version = "0.1.0"

# 2. 提交并打 tag（tag 一般打在主干上）
git commit -am "release: 0.1.0"
git tag v0.1.0
git push origin main v0.1.0
```

## 流水线（`.github/workflows/release.yml`）

| job | 做什么 | 失败点 |
|---|---|---|
| `verify` | tag 格式、tag 与 `Cargo.toml` 一致、`cargo test --locked` | 版本不一致 / 测试红 |
| `build` | 四平台 `cargo build --release --locked`，产物作为 artifact 上传 | 任一平台编不过 |
| `release` | 打包（`tar.gz` / windows 用 `zip`，内含二进制 + README + LICENSE）并附 `SHA256SUMS`，创建或更新 GitHub Release | 打包 / 上传失败 |
| `publish` | `cargo publish --locked` 到 crates.io | 见下节凭据配置 |

`release` 与 `publish` 并行，各自都需要 `build` 全绿。两个 job 都可重跑：
Release 已存在就只覆盖同名产物（`--clobber`），版本已经发布到 crates.io 就跳过 `publish`。

## 产物

| target | 说明 |
|---|---|
| `aarch64-apple-darwin` | macOS Apple Silicon |
| `x86_64-apple-darwin` | macOS Intel |
| `x86_64-unknown-linux-gnu` | Linux x64 |
| `x86_64-pc-windows-msvc` | Windows x64 |

```text
skylint-0.1.0-aarch64-apple-darwin.tar.gz   # 内含 skylint + README.md + LICENSE
skylint-0.1.0-x86_64-pc-windows-msvc.zip
SHA256SUMS
```

## crates.io 凭据

和 [glass-easel](https://github.com/wechat-miniprogram/glass-easel) 一样走
**trusted publishing**：工作流用 `id-token: write` 拿 GitHub OIDC token，向 crates.io
换一个 30 分钟的临时 token（`rust-lang/crates-io-auth-action@v1`），仓库里不放组织的
长期 API token。

一次性配置：

1. **首次发布必须手工**（crates.io 要求 crate 已存在才能配 trusted publishing）。
   用组织账号在 crates.io 生成一个 API token，本地执行：

   ```bash
   CARGO_REGISTRY_TOKEN=*** cargo publish --locked
   ```

2. 发布后在 crates.io 的 crate 设置里打开 **Trusted Publishing → GitHub**，填：

   | 字段 | 值 |
   |---|---|
   | Repository owner | `wechat-miniprogram` |
   | Repository name | `skylint` |
   | Workflow filename | `release.yml` |

3. 之后 tag 触发的 `publish` job 自动走 OIDC，不需要任何 secret。

工作流保留了 `secrets.CARGO_REGISTRY_TOKEN` 兜底（`publish` job 里 OIDC 失败时退回
secret），所以第 1 步之前也可以在仓库里配一个组织 token 应急；两种方式可并存。
