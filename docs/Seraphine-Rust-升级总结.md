# Seraphine Rust 升级总结

> 从 Python/PyQt 到 Rust/Tauri：同一套英雄联盟战绩与辅助能力，启动更快、主进程更轻、自动化更稳、更新与下载更适合国服。
>
> 版本：v2.0.2 · 2026-11-27

## 结论先行

这次不是简单“换语言重写”，而是把运行时、业务核心、自动化链路和发布方式一起重做：

- **启动**：热启动约 **62 ms**，原版约 **750 ms**，约 **12 倍**；冷启动约 **289–305 ms**，原版约 **4,479 ms**，约 **15 倍**。
- **主进程内存**：任务管理器实测 **50.7 MB vs 148.2 MB**，约降低 **66%**。
- **空闲 CPU**：**1.25% vs 2.73%**，约降低 **54%**。
- **游戏内开销**：同局 PresentMon A/B 的 p99 帧时间差异在 **0.1–0.2 ms** 内，两对采样均 **0 次 >50 ms 停顿**。
- **能力补齐**：自动接受/禁用/选用/技能、写守卫与读回确认、国服化英雄搜索、皮肤预览、OP.GG、TFT、ARAM 数据流水线、签名更新与中文安装包全部落地。

一句话：**更快的启动与交互、更轻的主进程、更可靠的自动化、更适合国内网络的下载与更新。**

## 一、核心数字

| 指标 | Python/PyQt 原版 | Rust/Tauri 2.0.2 | 变化 |
| --- | ---: | ---: | ---: |
| 启动到窗口（5 次中位数） | 750 ms | 62 ms | 12.1× 更快 |
| 冷启动到窗口 | 4,479 ms | 289–305 ms | 约 15× 更快 |
| 主进程内存（任务管理器截图） | 148.2 MB | 50.7 MB | 约 -66% |
| 主进程 Working Set（启动后 8 s） | 262.0 MB | 107.2 MB | 约 -59% |
| 空闲 CPU（20 s，单核百分比） | 2.73% | 1.25% | 约 -54% |

![任务管理器内存对比](assets/seraphine-memory.png)

> 口径说明：截图是任务管理器里的**主进程**对比（Seraphine Rust 50.7 MB vs Seraphine.exe 148.2 MB）。Tauri 会把界面放到 WebView2 子进程，因此“主进程更低”不等于“进程树总内存更低”；按完整进程树统计，Rust 约 494 MB、Python 约 262 MB。启动速度、主进程占用与 CPU 是明确优势，总驻留内存需要按进程树看。详见 [PERF.md](../rust/PERF.md)。

### 游戏内帧时间 A/B（Rust 版，同局两对）

| 采样 | 应用 | 平均 FPS | p50 | p95 | p99 | >50 ms |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 第 1 对 | 运行 | 235.12 | 4.19 | 5.09 | 6.16 | 0 |
| 第 1 对 | 停止 | 232.04 | 4.21 | 5.26 | 6.36 | 0 |
| 第 2 对 | 运行 | 232.26 | 4.21 | 5.34 | 6.42 | 0 |
| 第 2 对 | 停止 | 234.18 | 4.19 | 5.19 | 6.34 | 0 |

结论：应用运行与停止的 p50/p95/p99 差异在 0.1–0.2 ms 内，两对方向不一致，属于对局阶段噪声；两对均无 >50 ms 停顿。**常驻应用没有引入可测量的帧时间退化。**

## 二、为什么能更快

### 1. Rust 业务核心，而不是 Python 解释执行

新版把 LCU/SGP 会话、配置迁移、目录、战绩、搜索、TFT、OP.GG、ARAM、实时对局与自动化抽成 `seraphine-core` Rust 库，由 Tauri 后端直接调用：

```text
React + Fluent UI 前端
        │ Tauri commands
        ▼
Rust 后端（seraphine-core）
  ├─ LCU 发现 / WebSocket 事件 / 请求
  ├─ SGP 战绩 / OP.GG / TFT / ARAM
  ├─ 配置迁移 / 缓存 / 资源调度
  └─ 自动化与写守卫
```

没有 Python 解释器与 GIL，启动时不需要再加载 PyQt/Python 运行时；事件循环、网络请求与 JSON 解析都在 Rust 侧完成。

### 2. 事件驱动 + 兜底轮询

- LCU WebSocket 事件到达即处理，不必等固定轮询。
- 选人阶段保留 **1 秒兜底轮询**，防止客户端漏发 `session` 更新导致错过 ban/pick。
- 自动接受使用独立重试：最多 3 次、间隔 400 ms；已解除的 ReadyCheck 会重新检查，不盲目重试。

### 3. 单实例、托盘、窗口状态

- 单实例守卫：重复启动只保留一个进程，避免重复连接 LCU。
- 托盘、Mica、窗口尺寸记忆保留。
- 窗口修复工具仍支持 UAC 自升，实测 `1920×1080 → 1280×720` 并居中。

## 三、自动化：从“能点”到“可验证”

新版自动化不是简单调用接口，而是把每一步做成可观察、可回滚的写操作：

| 能力 | 新版实现 |
| --- | --- |
| 自动接受 | 事件触发 + 3 次重试；日志记录 `auto accepted match` |
| 自动禁用 | 在 `BAN_PICK` 第一秒执行两步 PATCH：先 `set`，再 `done`；客户端会静默丢弃 `PLANNING` 阶段的 ban |
| 自动选用 | 仅选择自己的 action，失败重试，完成后读回确认 |
| 自动技能 | 按分路保存，选人阶段写入；可配置 D/F 技能 |
| 交换/交易 | 保留原版能力，按会话安全检查 |

可靠性细节：

- **每局 nonce**：操作 id 使用 `epoch + 时间戳`，避免跨局守卫碰撞导致“看似重放、实际没写”。
- **请求哈希**：method + path + body 的 SHA-256，防止同一 operation id 被不同请求复用。
- **读回确认**：写入后 3 次、每次 150 ms 轮询确认；结果未知时标记 `OutcomeUnknown` 并阻止盲目重试。
- **写守卫**：Allowlist、阶段校验、30 天完成记录（`%LOCALAPPDATA%\Seraphine\app\write-guards.json`）。
- **开箱即用**：不再需要环境变量或额外开关；动作触发时直接调用允许的接口，守卫与读回确认始终生效。

真机证据：自动接受（未手动点击）、征召技能 `7/6 → 4/14`、ban 安妮、pick 艾希并锁定，均已落盘守卫记录。

## 四、能力对照：原版 → Rust 版

| 维度 | 原版（Python/PyQt） | Rust/Tauri 2.0.2 |
| --- | --- | --- |
| 运行时 | Python + PyQt，启动需加载解释器与 Qt | Rust 业务核心 + Tauri 2/React，直接调用 LCU/SGP |
| 启动与占用 | 热启动 750 ms；主进程 148.2 MB；空闲 CPU 2.73% | 62 ms；50.7 MB；1.25% |
| 自动接受/BP | Python 工具脚本；基础写入 | 事件驱动 + 1 s 兜底；两步 ban；每局 nonce；读回确认 |
| 写安全 | 基础调用 | Allowlist、请求哈希、operation id、`OutcomeUnknown`、30 天守卫 |
| 搜索 | 名称/PUUID/筛选 | 新增昵称、全拼、首字母、英文别名；对局/参与者跨页跳转 |
| 皮肤 | 背景设置 | 英雄 → 皮肤两步、大图预览、签名版选择 |
| ARAM 数据 | Python 更新器 | Rust CLI 生成、Wiki/jsdelivr 回退、sha256/大小/版本校验、Gitee feed |
| 更新 | GitHub 更新 | 签名清单、Gitee 优先 + GitHub 回退、固定 `updater` 标签 |
| 安装 | Python 打包 | NSIS 4.36 MB + 全中文 Inno 6.12 MB；数据目录保留 |
| 工程 | Python 测试 | Rust workspace + Tauri app + 契约测试 + GitHub Actions + Gitee 镜像 |
| 许可 | — | MIT + 中文用户协议 |

## 五、体验与国服化

| 场景 | 新版体验 |
| --- | --- |
| 首页 | 客户端连接信息居中卡片，状态与操作一屏可见 |
| 侧边栏 | 128 px 展开 / 64 px 收起；修复窗口与重启固定在底部 |
| 页面结构 | 个人信息、搜索信息、对局信息、游戏功能、OP.GG、云顶之弈、辅助工具、设置 |
| 搜索 | 支持英雄称号、玩家常用昵称、全拼、首字母与英文别名；点击对局/参与者可带着 `gameId` 跳到该玩家本局详情 |
| 皮肤 | 先选英雄，再选皮肤；大图预览 + 返回；支持“联盟不朽阿狸”签名版 |
| OP.GG | 模式与段位文案汉化；移除已失效的旧模式数据 |
| 术语与图标 | 采用国服术语；怀旧服统一使用正式服图标 |
| 错误提示 | 用户看到友好文案，技术细节只进日志 |
| 布局 | 卡片宽度上限 368 px，多栏平铺，避免超宽屏单个模块拉满 |

## 六、数据与更新：更适合国内网络

### ARAM 数据流水线

- 生成器改为 Rust CLI（`rust/tools/aram-data`），解析 League Wiki 的 Lua 表（含数字键与算术表达式）。
- Wiki 被 Cloudflare 拦截时自动回退到 jsdelivr 上的预解析 `balance.json`。
- 每次生成发布 `AramBuff.json`（21,488 B、132 英雄、16.23.1）与清单，带 sha256、大小、版本校验。
- 应用侧 10 分钟节流、主机白名单、原子替换；真机热更新 16.22.1 → 16.23.1 通过。
- 数据源默认指向 Gitee，旧 GitHub 默认值自动迁移。

### 签名更新

- Tauri updater + minisign 签名；私钥保存在仓库外，不入库。
- 更新清单固定标签 `updater`，不依赖 GitHub “Latest” 指针。
- 端点顺序：**Gitee 优先 → GitHub 回退**；回退发生在清单获取失败时。2.0.2 的 GitHub 安装版同样使用这套顺序。
- 2.0.1 及更早版本仍从 GitHub 检查更新；升级到 2.0.2 后改为 Gitee 优先。

### Gitee 镜像与中文安装包

- 公开镜像：`https://gitee.com/wxbns/seraphine-rust`
- GitHub main/tag 推送后自动同步代码与标签到 Gitee。
- GitHub 负责构建与签名；Gitee 提供下载与更新。
- 中文 Inno Setup 安装包（全程中文、MIT + 中文 EULA）：
  - 本机 Inno Setup 7.2 构建：`SeraphineRust-2.0.2-setup-zh.exe`（约 6.12 MB）
  - 仓库内置 `installer/ChineseSimplified.isl`；本机 Inno 7.2 与后续 CI 构建共用，不依赖编译器是否自带中文
- NSIS 自动更新包约 4.36 MB；安装到 `%LOCALAPPDATA%\Seraphine Rust`。

## 七、工程质量

- **工作区拆分**：`seraphine-core` 业务库 + `rust/app` Tauri 工程，核心逻辑可单独测试。
- **回归**：`cargo test --release --workspace`、Tauri app 单测、Python 契约测试全绿；`Build Seraphine` CI 通过。
- **发布**：GitHub Actions 构建签名 NSIS + 中文 Inno，发布 GitHub Release 与 Gitee Release；`v2.0.2` 已上线。
- **可复现**：2.0.0 已用全新克隆 `npm ci` → `npx tauri build` 验证签名产物，静默安装/卸载 smoke 通过且用户数据保留；2.0.2 沿用同一构建与发布链。
- **许可**：MIT + 中文用户协议。

## 八、边界与下一步

- **总内存**：Tauri 多进程模型会把一部分内存放到 WebView2；主进程更轻，但进程树总内存高于 Python 版。需要按“主进程/进程树”两种口径分别展示。
- **i18n**：当前中文优先；英文/葡萄牙语尚未接入。
- **目标机复测**：PresentMon 目前在本机 i9-14900HX 完成同局 A/B；Intel 250KF 的复测仍待执行。
- **Python 回退**：Python 版与 sidecar 暂时保留；用户逐模式自测通过后再退役。
- **真机细节**：选人/大乱斗部分字段仍在按真机核对表迭代（见 [PARITY.md](../rust/PARITY.md)）。

## 九、证据索引

- 性能与帧时间：[rust/PERF.md](../rust/PERF.md)
- 完成度审计：[rust/AUDIT.md](../rust/AUDIT.md)
- 功能对照：[rust/PARITY.md](../rust/PARITY.md)
- 发布与运维：[rust/HANDOFF.md](../rust/HANDOFF.md)
- 架构决策：[rust/DECISIONS.md](../rust/DECISIONS.md)
- GitHub Release：`https://github.com/Mcloud136/Seraphine-rust/releases/tag/v2.0.2`
- Gitee Release：`https://gitee.com/wxbns/seraphine-rust/releases/tag/v2.0.2`

---

**总结**：Rust 版把“启动、交互、自动化、数据更新、国内分发”这几件真正影响日常使用的事做成了系统优势；内存要分主进程与 WebView2 进程树看，但启动速度、主进程占用、CPU 与自动化可靠性都是实打实的提升。