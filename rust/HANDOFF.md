# Seraphine Rust 迁移 HANDOFF

更新时间：2026-11-26

## 当前状态

- Rust 应用：`rust/app`（Tauri 2 + React 18 + Fluent UI），release/debug 均可构建运行。
- 业务库：`rust/crates/seraphine-core`（配置迁移、路径、LCU 会话/事件、职业页、搜索页数据）。
- 已实机验证：LCU 自动连接、事件流、配置迁移、职业页真实战绩、搜索与对局详情、TFT 战绩（20 场）、OP.GG 榜单与出装面板（韩服）、头像/英雄/装备/符文图标。
- 连接后会自动从客户端重建 catalog（写入 `%LOCALAPPDATA%/Seraphine/app/catalog.json`），确保装备/符文图标跟随当前游戏版本。
- OP.GG 模式文案已修正：`aram_mayhem_classic` = 海克斯大乱斗。
- 对局信息与 ARAM 加成后端/UI 已完成，等待选人与大乱斗真机验证；英雄选择只读状态卡片（选人/禁用/计时）已接入。
- 游戏内接入 Live Client Data（`127.0.0.1:2999`）：实时双方 KDA/补刀/装备/等级，本人高亮；无对局时静默返回空。
- 设置页已接入常规/网络/游戏功能/OP.GG 默认值/外观/自动禁用与选人名单/自动召唤师技能/客户端连接，保存写回 `config.json`；主题与 Mica 保存后即时应用，关闭到托盘由 Rust 处理。
- 窗口尺寸在关闭/退出时写回并在下次启动恢复（实测 1000×700 往返一致）；多客户端可在设置页切换连接。
- 日志：`%LOCALAPPDATA%\Seraphine\app\log\seraphine.log`，超过 5 MB 轮转为 `.1`；默认按配置级别（40=WARN），设为 20 可记录启动/连接信息。
- 安装版 smoke 已逐页验证：职业、搜索、OP.GG、TFT、设置；1000px 宽度下设置页改为单列，消除横向滚动。
- 安装包 smoke 通过：静默安装到 `%LOCALAPPDATA%\Seraphine Rust`、启动读取真实战绩、卸载后 `%APPDATA%\Seraphine\config.json` 与 `%LOCALAPPDATA%\Seraphine` 数据未被删除。
- ARAM 远端更新：manifest/sha256/大小/版本校验、主机白名单、10 分钟节流、原子替换；设置页可手动检查。CI 生成已补自动回退：Wiki 被 Cloudflare 拦截（403）时改用 jsdelivr 上 `balance-buff-viewer-plugin` 的预解析 `balance.json`（MIT，同 Wiki 数据；本机实测 132 英雄与内置快照逐字段一致）。工作流同时把两个文件发布到当前仓库固定标签 `aram-data-latest`（用内置 `GITHUB_TOKEN`，无需额外 Secret），稳定 feed：`https://github.com/Mcloud136/Seraphine-rust/releases/download/aram-data-latest/aram-manifest.json`。
- ARAM CI 已端到端跑通并由 Rust CLI 生成（2026-11-27）：`rust/tools/aram-data` 移植了 Lua 解析、条目构建与 manifest 写入，`Update ARAM data` 全绿，`aram-data-latest` 发布含 `aram-manifest.json`（318 B）与 `AramBuff.json`（21,488 B）；下载校验 sha256/大小/版本 16.23.1/132 英雄全部一致，GitHub runner 走 Wiki 主源。Rust 与 Python 生成器在真实 Wiki 模块与 bootstrap JSON 上逐字段一致；Python 脚本保留作参考与 Gitee 发布步骤。
- ARAM 应用侧真机热更新已通过（2026-11-27）：将缓存版本置为 16.22.1 后，应用经设置页代理（本机以 SSH→SOCKS→本地 CONNECT 桥接 GitHub）拉取 `aram-data-latest`，落盘 `AramBuff.json` 16.23.1（132 英雄）与 `aram-manifest.json`，sha256/大小与清单一致，日志无失败；随后恢复了原配置（feed 留空、代理关闭）。
- 仓库已同步 Rust 应用本体与工作流（`14cb684` 起），GitHub 上 `Build Seraphine`/`Release Rust app`/`Update ARAM data` 三个工作流均处于 active。Build 流水线中 Rust 全工作区校验、Python 契约测试（CI 注解定位并修复了短路径临时目录断言）、PyInstaller 打包均已通过；打包 smoke 已补无头环境变量。ARAM feed 默认值保持“按需配置”，避免首次启动依赖外网。
- 安装包：`rust/app/src-tauri/target/release/bundle/nsis/Seraphine Rust_2.0.0_x64-setup.exe`（4.12 MiB，currentUser，已用本地 updater 私钥签名，含 2026-11-27 自动化修复；`.sig` 同目录）。`latest.json` 可用 `make_latest_json.py` 生成（注意在 Git Bash 下输出路径用正斜杠）。已用**全新克隆**（`git clone` → `npm ci` → `npx tauri build`）复验：签名产物 4,322,871 B 构建成功，静默安装后 ProductVersion=2.0.0，卸载移除程序目录且 `%LOCALAPPDATA%\Seraphine`、`%APPDATA%\Seraphine` 数据保留。
- LCU 写操作灰度（已获批准）：需 `SERAPHINE_ENABLE_LCU_WRITES=1`；写入经 Allowlist、operation id、请求哈希、选人读回确认与 30 天守卫（`%LOCALAPPDATA%\Seraphine\app\write-guards.json`）。已实测 `PUT /lol-chat/v1/me` 成功并落盘守卫。
- 自动化已真机通过（2026-11-27）：自动接受（18:39:11 `auto accepted match`，未手动点击）；征召自定义技能 7/6→4/14、ban 安妮（BAN_PICK 首秒两步 PATCH）、pick 艾希并锁定；`write-guards.json` 落盘。客户端会静默丢弃 PLANNING 阶段的 ban 写入，故 ban 最早在 BAN_PICK 第一秒生效；操作 id 使用每局 nonce，避免 `epoch:seq` 跨局碰撞导致“重放未写”。
- 辅助工具页已上线在线状态/可用性/头像/背景/段位展示/观战/重连/重启/代币/徽章/锁定配置（已实测可逆）与窗口修复（2026-11-27 真机：1920×1080 → 1280×720 居中，UAC 自升生效）。
- 更新：Tauri updater + 本地签名密钥；本地更新源实测发现 2.0.1 并展示安装按钮；安装/重启待真实版本发布验证。
- 发布清单：`rust/app/scripts/make_latest_json.py --installer … --signature … --version … --url … --pub-date …`。
- 发布工作流：`.github/workflows/release_rust.yaml`（tag `v*` 触发；需要仓库 Secrets：`TAURI_SIGNING_PRIVATE_KEY`，空密码时同时设置/留空 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`）。
- 本机 NSIS 缓存：`%LOCALAPPDATA%\tauri\NSIS`，需包含 `Plugins\x86-unicode\additional\nsis_tauri_utils.dll`；否则 Tauri 会联网下载并可能因代理 407 失败。
- 性能（本机，2026-11-26）：启动 305 ms（Python 4,479 ms），空闲 CPU 1.25%（2.73%），主进程 WS 48.9 MB（221.9 MB）；WebView2 使进程树 WS 升至约 441 MB。详见 [PERF.md](PERF.md)。
- Python 版保持原样，可继续运行；两版配置分离。

## 下一步

1. Tauri updater：签名密钥、更新清单与回滚流程（W0-7/W5-3，需要确认私钥保管）。
2. 选人/大乱斗真机验证 W2-1/W2-3；配置真实 ARAM feed 验证远端更新。
3. Wave 2-2 BP 写操作（写守卫 + 政策审批后启用）。
4. 辅助工具页（在线状态/背景/段位等写操作，需审批）与游戏内信息条。
5. i18n（en/pt）与 W0-2 sidecar 薄壳化。

## 关键文件

- 决策：[DECISIONS.md](DECISIONS.md)
- 地图：[MAP.md](MAP.md)
- 功能对照：[PARITY.md](PARITY.md)
- 完成度审计：[AUDIT.md](AUDIT.md)
- 术语：[GLOSSARY.md](GLOSSARY.md)
- Tauri 后端：`rust/app/src-tauri/src/main.rs`
- 前端：`rust/app/src/App.tsx`、`rust/app/src/SearchPage.tsx`
- 共享库：`rust/crates/seraphine-core/src/`
- 截图与输入辅助：`rust/app/scripts/`

## 已知风险

- 最近回归：`cargo test --release --workspace` 全绿；app 2 通过；Python 93 通过（1 跳过）。sidecar 内存断言按 release 口径（release 9.22 MiB 通过；debug 11.3 MB 超 10 MiB 为已知构建差异）。PresentMon 真机 A/B 已完成（同局两对、各 60 s）：p99 6.16/6.42 ms（应用运行）vs 6.36/6.34 ms（停止），0 次 >50 ms；采集脚本 `bench_game_frames.py`/`analyze_game_frames.py` 已修复 ETW 残留会话与 UTF-16 CSV。

- Tauri 前端构建不通过 `rust-anything app lift` 验证，功能等价依赖真机核对表。
- 真实 LCU 写操作仍禁用，需真机灰度与政策审批。
- OP.GG、TFT、BP 与设置页尚未迁移。