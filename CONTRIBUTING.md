# 贡献指南

Seraphine Rust 的主线是 Tauri 2 + Rust 业务核心 + React/Fluent UI。提交前请确保变更符合“能用 API 就不调用外部命令/脚本”的原则。

## 代码位置

- Rust 业务核心：`rust/crates/seraphine-core`
- Tauri 后端与命令：`rust/app/src-tauri`
- 前端页面：`rust/app/src`
- ARAM 数据生成：`rust/tools/aram-data`
- 发布清单与 Gitee 发布：`rust/tools/release`
- 安装器：`installer/`

旧 Python/PyQt 实现已归档在 `legacy-python` 分支，不在 `main` 接受新功能修改。

## 提交前检查

```bash
cd rust
cargo fmt --all -- --check
cargo test --release --workspace
cargo clippy --workspace --all-targets -- -D warnings

cd app
npm ci
npm run build
```

## 变更原则

- 优先使用 LCU/SGP/HTTP API 与 Windows API；不要在应用功能里调用 PowerShell、`cmd` 或外部脚本。
- 写操作必须经过现有 Allowlist、阶段校验、operation id、请求哈希、读回确认与写守卫。
- 用户可见错误使用友好文案；技术细节写入日志。
- UI 文案使用国服术语；新增英雄搜索别名时更新生成数据。
- 不要在仓库中提交 updater 私钥、Gitee Token 或其他凭据。

## Pull Request

1. 从 `main` 创建分支。
2. 说明变更范围、测试方式与真机证据。
3. 涉及发布/更新的改动请同步检查 `release_rust.yaml` 与 `mirror_gitee.yaml`。

## 许可

提交即表示同意以项目的 [MIT License](LICENSE) 授权。