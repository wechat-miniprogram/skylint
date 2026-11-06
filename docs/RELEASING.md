# 发版

推一个 `vX.Y.Z` tag 就完成发版：四平台二进制进 GitHub Release，同时把两个 crate
（`skylint-core` → `skylint`，按此顺序）发到 crates.io。tag 必须是根的 `Cargo.toml`
版本号——不一致会在第一步就失败，不会产出半成品。

```bash
# 1. 改版本号（两个 crate 必须一致：根 Cargo.toml 与 crates/core/Cargo.toml）
$EDITOR Cargo.toml crates/core/Cargo.toml   # version = "0.2.0"
#    根的 skylint-core 依赖也要跟上：
#    skylint-core = { path = "crates/core", version = "0.2.0" }

# 2. 提交并打 tag（tag 一般打在主干上）
git commit -am "release: 0.2.0"
git tag v0.2.0
git push origin main v0.2.0
```

## 流水线（`.github/workflows/publish.yml`）

| job | 做什么 | 失败点 |
|---|---|---|
| `verify` | tag 格式、tag 与根 `Cargo.toml` 一致、`cargo test --workspace --locked` | 版本不一致 / 测试红 |
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

一次性配置（**`skylint` 已完成**：已在 crates.io 上，trusted publishing 指向
`publish.yml`；0.1.0 就是走 tag + OIDC 发的，没有用任何 token）：

> ⚠️ **`skylint-core` 还没在 crates.io 上**。工作区拆成两个 crate 后，下一次发版之前需要
> 给 `skylint-core` 再做一次同样的一次性配置（它是独立 crate，有自己的 trusted publishing），
> 否则 `publish` job 会在发 core 那一步失败：
>
> 1. 用组织账号的 API token 本地发一次：`cargo publish --locked -p skylint-core`
>    （此时根 crate 还没发新版，不受影响）；
> 2. crates.io 上 `skylint-core` → Settings → Trusted Publishing → GitHub，三项同下表，
>    Repository name 填 `skylint-core` 所在的仓库仍是 `skylint`；
> 3. 之后 tag 发版时 workflow 会先发 core、等索引可解析后再发 `skylint`
>    （见 `publish.yml` 的“cargo publish”步骤）。

1. **首次发布必须手工**（crates.io 要求 crate 已存在才能配 trusted publishing）。
   用组织账号在 crates.io 生成一个 API token，本地执行：

   ```bash
   CARGO_REGISTRY_TOKEN=*** cargo publish --locked -p <crate>
   ```

2. 发布后在 crates.io 的 crate 设置里打开 **Trusted Publishing → GitHub**，填：

   | 字段 | 值 |
   |---|---|
   | Repository owner | `wechat-miniprogram` |
   | Repository name | `skylint` |
   | Workflow filename | `publish.yml` |

3. 之后 tag 触发的 `publish` job 自动走 OIDC，不需要任何 secret。

工作流保留了 `secrets.CARGO_REGISTRY_TOKEN` 兜底（`publish` job 里 OIDC 失败时退回
secret），所以第 1 步之前也可以在仓库里配一个组织 token 应急；两种方式可并存。

### 踩过的坑：工作流文件名必须与 crates.io 上的配置一致

0.1.0 第一次跑发布时 `crates.io` job 失败，报：

```
Failed to retrieve token from Cargo registry. Status: 400.
The Trusted Publishing config for repository `wechat-miniprogram/skylint`
does not match the workflow filename `release.yml` in the JWT.
Expected workflow filenames: `publish.yml`
```

trusted publishing 校验的是**工作流文件名**，不是 workflow 的 `name`。所以本仓库的发布
工作流文件名固定为 `publish.yml`（内容里的 `name: Release` 与它无关）；改名会让发版在
最后一步失败，且只有 crates.io job 会红，其它产物已经产出。
