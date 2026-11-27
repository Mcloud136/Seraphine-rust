# Seraphine Rust 重写 — 决策记录

本文件是整应用迁移的唯一决策来源。编号只增不改；决定变化时新增 ADR 并标注 supersedes。

## ADR-001: GUI 采用 Tauri 2 + React + Fluent UI

Status: accepted (2026-11-26)
Context: 现有界面为 PyQt5 + PyQt-Fluent-Widgets + qframelesswindow，约 1 万行 UI 代码。rust-anything 的 `analyze` 将应用误判为 CLI，其 `app record/front-door/lift` 不能录制或验证 GUI 行为。
Decision: Rust 侧使用 Tauri 2 作为桌面外壳，前端使用 React 18 + TypeScript + Vite + `@fluentui/react-components` v9；窗口无边框、Mica/亚克力、托盘由 Tauri/Rust 实现。
Consequences: 前端不是 Rust，但业务、网络、自动化与窗口控制全部为 Rust，满足“整个应用用 Rust 重写”的实质目标；UI 视觉是 Fluent 风格的重新实现，不追求逐像素等同。

## ADR-002: 验收标准为功能等价

Status: accepted (2026-11-26)
Context: GUI 行为无法像纯函数一样录制回放；现有 Python 版还包含真实 LCU 交互。
Decision: 每个页面/流程按功能清单验收：数据、交互、配置、缓存、错误提示、LCU 写入守卫与 Python 版一致；界面允许重新设计。使用真机 + 人工/截图清单，不采用像素级对照。
Consequences: 需要为每个波次维护功能核对表；视觉回归不阻断发布，功能缺失阻断。

## ADR-003: 新旧版本并行共存

Status: accepted (2026-11-26)
Context: 用户需要可回退路径，且 Rust 版分波交付。
Decision: Python 版在全部硬门槛通过前保持可用；Rust 版独立进程、独立状态目录，不修改 Python 版安装。灰度用户可先后运行两个版本。
Consequences: 两版不能同时连接并操控 LCU（由单实例互斥与文档约束）；缓存可只读复用，写入路径分离。

## ADR-004: 全功能等价范围

Status: accepted (2026-11-26)
Decision: 覆盖职业页、搜索、对局信息、英雄选择/BP、ARAM 加成、OP.GG 榜单/出装/符文、TFT 战绩、辅助工具、设置、自动更新与托盘。
Consequences: 迁移分为 6 个波次；任何页面缺失都视为未完成。

## ADR-005: 抽出 seraphine-core 库，Tauri 直接调用

Status: accepted (2026-11-26)
Context: 现有 `rust/sidecar/src/main.rs`（约 4 千行）已实现 LCU/SGP/外部数据、缓存、事件、写守卫与 OP.GG 解析；stdio 协议只为 Python 兼容存在。
Decision: 新建 `seraphine-core` crate，承载会话生命周期、配置、缓存、LCU 客户端、事件分发与领域服务；Tauri commands 直接调用该库。`seraphine-sidecar` 保留为薄壳，继续服务 Python 版，直到旧版退役。
Consequences: 需要在迁移期间避免两处逻辑漂移；以 core 为唯一实现，sidecar 只做协议适配。

## ADR-006: 新配置格式 + Tauri updater

Status: accepted (2026-11-26)
Context: 旧版使用 `config.json`（QConfig/INI 风格键值）与 `seraphine-updater.exe` + Inno Setup。
Decision: Rust 版使用带 `schemaVersion` 的新 JSON 配置，首次启动时从旧 `config.json` 一次性迁移；后续升级使用 Tauri updater（签名清单），旧升级器仅服务 Python 版。
Consequences: 两版配置不同步；迁移必须保留旧文件且可重复执行；发布流程需要生成 Tauri 签名与更新清单。

## ADR-007: 第一个交付切片

Status: accepted (2026-11-26)
Decision: 先交付“外壳 + LCU 连接 + 职业页”垂直切片：无边框窗口、托盘、单实例、配置迁移、LCU 发现/连接/事件、召唤师信息、段位与战绩。
Consequences: 该切片验证 Tauri IPC、core 复用、事件推送、图片加载与打包全链路，再按波扩展。

## ADR-008: rust-anything 的适用范围

Status: accepted (2026-11-26)
Context: rust-anything 的证明来自录制输入；GUI、WebSocket 时序与真实 LCU 写操作不在其证明范围。2026-11-26 复核候选单元：`sync.py` 在导入时读取 Gitee 凭据并请求网络，`scripts/` 整体不可安全桥接；`app/backend/` 为多文件单元，包含 aiohttp/PyQt 依赖，单独 bridge `aram_data.py` 需要裁剪单元。
Decision: 对可独立录制且导入无副作用的单元使用 rust-anything 的 analyze → record → port → diff → gate 流程；GUI 与长连接服务按 ADR-002 的真机功能验收。不使用 `app lift` 作为 GUI 的完成判据。
Consequences: `rust/app` 不通过 `app lift` 生成；完成判据是功能核对表 + Rust 测试 + 真机验证 + 性能对比。rust-anything 目前用于整应用分析与迁移方法（MAP/ADR/真机证据），单元级 gate 待出现安全可录制单元后启用。

## ADR-009: 平台与工具链

Status: accepted (2026-11-26)
Decision: 仅支持 Windows x64（Windows 10 1809+ / 11），依赖 WebView2 Runtime；Tauri 工程使用独立 workspace，不继承现有 `panic=abort` 发布配置；现有 crate 维持 Rust 1.80 起的兼容性。
Consequences: CI 需安装 Node 与 WebView2 构建依赖；安装包检测 WebView2。

## ADR-010: LCU 写入安全策略不变

Status: superseded by ADR-014 (2026-11-27)
Decision: Rust 版默认禁用真实 LCU 写入，沿用现有写守卫、稳定 operation id、读回确认与 `OutcomeUnknown` 语义；启用需真机灰度与政策审批。
Consequences: 新 UI 中所有写入口走 core 的守卫 API，不提供绕过开关。

## ADR-012: 安装目录与产品名隔离

Status: accepted (2026-11-27)
Context: Tauri NSIS 的 currentUser 模式把应用装到 `%LOCALAPPDATA%\{productName}`；若产品名仍为 `Seraphine`，安装目录会与旧版数据目录 `%LOCALAPPDATA%\Seraphine`（Cache/catalog/config）重叠，卸载与升级路径存在误删和混淆风险。
Decision: Rust 版产品名使用 `Seraphine Rust`，安装到 `%LOCALAPPDATA%\Seraphine Rust`；数据仍使用 `%LOCALAPPDATA%\Seraphine` 与 `%APPDATA%\Seraphine`，实现并行共存。窗口标题与界面品牌仍为 Seraphine。
Consequences: 安装包名称、开始菜单与卸载项显示“Seraphine Rust”；升级与卸载不会触碰旧版数据目录。

## ADR-014: LCU 写操作灰度授权

Status: accepted (2026-11-27，用户批准“完整写操作灰度”)
Context: 辅助工具、自动接受、BP 自动化依赖 LCU 写接口；此前按 ADR-010 默认禁用。用户已批准完整写操作灰度，但仍需可回滚与可审计。
Decision: 默认仍关闭；仅设置 `SERAPHINE_ENABLE_LCU_WRITES=1` 时启用。所有写入经 Allowlist、稳定 operation id、请求哈希、Champ Select 读回确认、`OutcomeUnknown` 阻断与 30 天完成记录（`%LOCALAPPDATA%\Seraphine\app\write-guards.json`）。先上线辅助工具手动操作与自动接受对局，再逐步开放 BP/禁用/技能自动化。
Consequences: 灰度运行需显式环境变量与真机确认；失败或结果未知时禁止盲目重试；Python 版仍保留作为回退。

## ADR-015: ARAM 数据由 CI 生成并发布到固定标签

Status: accepted (2026-11-27)
Context: Wiki 的 Module:ChampionData/data 对非浏览器客户端可能返回 Cloudflare 403；应用内热更新需要一个稳定、可校验的 feed。
Decision: 生成器实现为 Rust CLI（`rust/tools/aram-data`），以 Wiki 为主源，解析 Lua 表（含 `[1]=` 数字键与算术表达式）；403 等失败时回退到 jsdelivr 上 `balance-buff-viewer-plugin` 的预解析 `balance.json`（MIT，同 Wiki 数据）。Python 脚本保留作参考与 Gitee 发布步骤。CI 每次生成后把 `AramBuff.json` 与 `aram-manifest.json` 发布到当前仓库固定标签 `aram-data-latest`（内置 `GITHUB_TOKEN`），并提供 sha256/大小/版本校验。应用侧 feed 默认指向本仓库固定标签 `aram-data-latest`（设置中可覆盖或清空）。
Consequences: 数据更新不再依赖已停放的 jddld；GitHub runner 可直连 Wiki，本机被拦截时走回退源且与内置快照逐字段一致；固定标签下载 URL 长期稳定。

## ADR-016: 应用更新清单与数据发布使用独立固定标签

Status: accepted (2026-11-27)
Context: 更新检查原先指向 `releases/latest/download/latest.json`。GitHub 只有一个 “Latest” 指针，`aram-data-latest` 这类数据发布会夺取它，导致 `latest.json` 302 到不含该资源的标签而检查失败。
Decision: 更新清单改由固定标签 `updater` 提供：`releases/download/updater/latest.json`（预发布状态、`make_latest: false`），每次 tag 发布由工作流覆盖上传；ARAM 数据继续使用固定标签 `aram-data-latest`，同样标记预发布且不参与 Latest。两者与应用版本号解耦，不单独建库。
Consequences: 数据刷新与应用发版互不影响，也不依赖 GitHub “Latest” 语义；代价是仓库多一个 `updater` 预发布条目，客户端需能访问 github.com（或被代理）。

## ADR-013: 更新签名密钥由本地保管

Status: accepted (2026-11-27，用户批准)
Context: Tauri updater 需要 minisign 密钥对；私钥泄露可伪造更新，丢失则无法发布新版本。
Decision: 私钥保存在仓库外 `%USERPROFILE%\.seraphine\updater.key`，公钥写入 `tauri.conf.json`；构建时通过 `TAURI_SIGNING_PRIVATE_KEY` 指向私钥。私钥不入库、不写入日志；发布前应迁移到 CI Secrets 并离线备份。
Consequences: 本机构建 updater 产物需要该文件；仓库不含任何私钥；更换密钥会使旧客户端无法验证新更新。

## ADR-011: 目录布局

Status: accepted (2026-11-26)
Decision:

```
rust/
  Cargo.toml            # 旧 workspace：crates/* + sidecar
  crates/               # 既有可复用 crate
  crates/seraphine-core # 新增：共享业务库
  app/                  # Tauri 工程（独立 workspace）
    src-tauri/          # Rust 后端（Tauri commands）
    src/                # React + TS 前端
  MAP.md / DECISIONS.md / GLOSSARY.md / RULES.md
```

Consequences: `rust/app` 拥有自己的 `[workspace]`，与旧 workspace 隔离，避免发布 profile 影响测试与 gate。