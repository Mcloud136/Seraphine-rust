# 整应用 Rust 化 — 完成度审计

目标：把 Seraphine 整个应用升级为 Rust（Tauri 外壳 + Rust 业务核心 + React 前端），功能等价并提升运行效率。

结论：**未完成**。核心功能、设置、写操作、更新与打包已落地并有真机证据；但仍有需要真实对局/真实发布才能关闭的硬门禁。

## 已达成（证据）

| 交付项 | 证据 |
| --- | --- |
| Tauri 2 + React/Fluent UI 外壳、托盘、单实例、Mica、窗口尺寸记忆 | 真机启动；1000×700 保存/恢复 |
| Rust 业务核心：配置迁移、LCU 会话/事件、目录、职业页、搜索、对局信息、TFT、OP.GG、ARAM、实时对局 | `rust/crates/seraphine-core`；真机逐页截图验证 |
| 设置页：常规/网络/功能/OP.GG/外观/名单/技能/队列筛选/客户端/更新 | 保存写回 `config.json`；窄窗响应式修复 |
| LCU 写操作：Allowlist、operation id、请求哈希、读回确认、30 天守卫 | `PUT /lol-chat/v1/me` 真机成功；`write-guards.json` 落盘 |
| 自动化：接受对局、禁用/选用/锁定、分路技能、交易/换位 | 2026-11-27 真机征召连测：技能 7/6→4/14、ban 安妮、pick 艾希并锁定（18:28:17 / 18:28:24），`write-guards.json` 落盘；PLANNING 的 ban 被客户端静默丢弃，改为 BAN_PICK 首秒两步 PATCH；每局 nonce 防止跨局守卫碰撞。自动接受仍待“无人手动点击”的通过证据 |
| 辅助工具：状态/可用性/头像/背景/段位/观战/重连/重启/代币/徽章/锁定配置 | 锁定配置实测可逆；其余接口已接入 |
| Tauri updater：签名、检查、下载、安装、重启 | 本地源 2.0.0→2.0.1 全链路；安装版 ProductVersion=2.0.1 |
| NSIS 安装包（currentUser，独立安装目录） | 静默安装/启动/卸载 smoke；旧数据保留 |
| 性能 | 启动中位数 62 ms vs 750 ms；主进程 WS 107.2 MB vs 262.0 MB；空闲 CPU 1.25% vs 2.73%；同局两对 PresentMon A/B：p99 6.16/6.42（运行）vs 6.36/6.34 ms（停止），0 次 >50ms |
| 回归 | `cargo test --release --workspace` 全绿；app 2 通过；Python 93 通过（1 跳过）。sidecar 内存断言按 release 口径：release 9.22 MiB 通过，debug 11.3 MB 超 10 MiB 属已知构建差异 |

## 未关闭的硬门禁

1. 自动接受对局：2026-11-27 18:39:11 真机匹配自动接受成功（`auto accepted match`，未手动点击）。
2. 目标机（Intel 250KF）复测 PresentMon；本机 i9-14900HX 已完成两对同局 A/B（见 PERF）。
3. GitHub Releases 真实发布：工作流已添加（tag 触发），仍需配置签名 Secrets 并推送 tag。
4. 窗口修复工具：2026-11-27 真机验证，客户端窗口 1920×1080 @(16,69) → 1280×720 @(557,323)，UAC 自升生效。
5. i18n 英/葡（用户选择中文优先）。
6. Python 版退役与 sidecar 删除（需完成上述灰度与七天观察）。

## 判定

在上述硬门禁关闭前，不得声明“整个项目已完成 Rust 化”。当前状态为：可运行的 Rust 主应用 + 完整回退路径。