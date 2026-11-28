# Seraphine Rust

简体中文 · Windows 10/11 x64 · v2.0.3

Seraphine Rust 是英雄联盟战绩查询与辅助工具的 Rust/Tauri 重写版：保留原有查询、对局信息、OP.GG、云顶之弈、ARAM 与自动化能力，同时把启动、主进程占用、写操作可靠性和国内下载/更新体验整体重做。

[下载安装](#下载与安装) · [性能对比](#性能对比) · [功能](#功能) · [更新机制](#更新机制) · [从源码构建](#从源码构建) · [致谢](#致谢)

[![License](https://img.shields.io/github/license/Mcloud136/Seraphine-rust?label=License)](LICENSE)
[![Release](https://img.shields.io/github/v/release/Mcloud136/Seraphine-rust?label=Release)](https://github.com/Mcloud136/Seraphine-rust/releases/latest)
[![Gitee](https://img.shields.io/badge/Gitee-wxbns%2Fseraphine--rust-c71d23)](https://gitee.com/wxbns/seraphine-rust)

## 简介

- **技术栈**：Tauri 2 + Rust 业务核心 + React 18 + Fluent UI v9。
- **开箱即用**：无需环境变量或隐藏开关；自动接受默认开启，自动禁用/选用/技能在“游戏功能”中按需启用。所有写入仍经过允许清单、阶段校验、读回确认与写守卫。
- **双源分发**：GitHub 负责构建与签名，Gitee 提供国内下载与更新；更新检查 Gitee 优先、GitHub 回退。
- **安装包**：NSIS 自动更新包（约 4.36 MB）与全中文 Inno Setup 安装包（约 6.12 MB）。
- **许可**：MIT；安装包内含中文用户协议。

## 性能对比

测试机：Intel i9-14900HX，Windows 11 x64。对比对象为仓库内 Python/PyQt 原版与 Rust/Tauri 2.0.2 release 构建。

| 指标 | Python/PyQt 原版 | Rust/Tauri 2.0.2 | 变化 |
| --- | ---: | ---: | ---: |
| 启动到窗口（5 次中位数） | 750 ms | 62 ms | 12.1× 更快 |
| 冷启动到窗口 | 4,479 ms | 289–305 ms | 约 15× 更快 |
| 主进程内存（任务管理器截图） | 148.2 MB | 50.7 MB | 约 -66% |
| 主进程 Working Set（启动后 8 s） | 262.0 MB | 107.2 MB | 约 -59% |
| 空闲 CPU（20 s，单核百分比） | 2.73% | 1.25% | 约 -54% |



### 游戏内帧时间 A/B

同一局匹配对局内做两对相邻窗口的“应用运行 / 停止应用”切换，PresentMon 采样 60 s：

| 采样 | 应用 | 平均 FPS | p50 | p95 | p99 | >50 ms |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 第 1 对 | 运行 | 235.12 | 4.19 | 5.09 | 6.16 | 0 |
| 第 1 对 | 停止 | 232.04 | 4.21 | 5.26 | 6.36 | 0 |
| 第 2 对 | 运行 | 232.26 | 4.21 | 5.34 | 6.42 | 0 |
| 第 2 对 | 停止 | 234.18 | 4.19 | 5.19 | 6.34 | 0 |

结论：p50/p95/p99 差异在 0.1–0.2 ms 内，两对方向不一致，属于对局阶段噪声；两对均无 >50 ms 停顿。常驻应用没有引入可测量的帧时间退化。

更完整的说明、功能清单与边界见 [Seraphine Rust 升级总结](docs/Seraphine-Rust-升级总结.md)。

## 功能

### 战绩与查询

- 召唤师信息、段位、最近战绩与对局详情。
- 名称 / PUUID 搜索、分页、队列筛选；对局与参与者可直接跳到该玩家本局详情。
- 对局信息（己方/敌方/近况）、实时对局 KDA/装备/等级。
- 云顶之弈战绩、前四率、平均排名、单位与强化符文。
- OP.GG 榜单、出装、符文与竞技场强化；段位与模式文案汉化。
- ARAM 加成展示与远端数据更新。

### 自动化与游戏功能

- 自动接受对局；事件驱动 + 1 秒兜底轮询，漏事件也能补上。
- 自动禁用：在 `BAN_PICK` 第一秒执行两步 PATCH；客户端会静默丢弃 `PLANNING` 阶段的 ban。
- 自动选用、自动设置分路召唤师技能、接受换位/交易。
- 每局 nonce、请求哈希、读回确认、`OutcomeUnknown` 阻断与 30 天写守卫。
- 一键锁定游戏设置、观战、重连、重启客户端、修复客户端窗口。

### 国服化体验

- 英雄搜索支持称号、玩家常用昵称、全拼、首字母与英文别名。
- 英雄 → 皮肤两步选择，大图预览与返回；支持“联盟不朽阿狸”签名版。
- 国服术语、正式服图标、中文段位；368 px 卡片与可折叠侧边栏。
- 用户看到友好错误文案，技术细节只写入日志。

## 下载与安装

| 渠道 | 安装包 | 说明 |
| --- | --- | --- |
| Gitee（推荐国内） | [SeraphineRust-2.0.3-setup-zh.exe](https://gitee.com/wxbns/seraphine-rust/releases/download/v2.0.3/SeraphineRust-2.0.3-setup-zh.exe) | 全中文 Inno Setup 向导，适合首次安装 |
| Gitee | [v2.0.3 Release](https://gitee.com/wxbns/seraphine-rust/releases/tag/v2.0.3) | 中文包、NSIS、签名与 ARAM 数据 |
| GitHub | [Seraphine.Rust_2.0.3_x64-setup.exe](https://github.com/Mcloud136/Seraphine-rust/releases/download/v2.0.3/Seraphine.Rust_2.0.3_x64-setup.exe) | NSIS 自动更新包，与更新器下载的是同一个 |
| GitHub | [v2.0.3 Release](https://github.com/Mcloud136/Seraphine-rust/releases/tag/v2.0.3) | 全部构建产物与源码 |

安装前请确认已安装 **Microsoft Edge WebView2 Runtime**（Windows 10/11 通常已自带）。两种安装包统一安装到：

- `%LOCALAPPDATA%\Seraphine Rust`

2.0.2 的中文安装包曾使用 `%LOCALAPPDATA%\Programs\Seraphine Rust`；升级到 2.0.3 前请先卸载旧中文版一次，安装向导也会检测并提示。用户数据位于 `%LOCALAPPDATA%\Seraphine` 与 `%APPDATA%\Seraphine`；卸载程序不会删除这些数据。

## 更新机制

- 更新清单使用固定标签 `updater`，由 Tauri updater 校验 minisign 签名。
- **2.0.3 及以后**：清单使用固定 raw 地址（Gitee 优先、GitHub 回退），发布时随版本提交原子更新，不会再出现附件删除导致的空窗；清单里的安装包地址指向 Gitee 下载源。
- **2.0.2**：检查 Gitee 的 `updater` 标签附件，失败回退 GitHub；清单里的安装包同样指向 Gitee。
- **2.0.0 / 2.0.1**：端点只有 GitHub，首次升级会从 GitHub 获取 2.0.2；升级后改为 Gitee 优先。
- 回退发生在清单获取阶段；一旦拿到 Gitee 清单，下载地址就是清单中的 Gitee 安装包。
- Gitee 原始文件可能缓存 1–5 分钟；发布后短时间内检查更新可能仍读到上一版，稍后重试即可。

固定地址：

```text
Gitee  https://gitee.com/wxbns/seraphine-rust/releases/download/updater/latest.json
GitHub https://github.com/Mcloud136/Seraphine-rust/releases/download/updater/latest.json
```

## 从源码构建

前置条件：Windows 10 1809+ x64、Node.js 22、Rust stable、Visual Studio Build Tools（C++ 桌面开发）、WebView2 Runtime。

```bash
git clone https://github.com/Mcloud136/Seraphine-rust.git
cd Seraphine-rust/rust/app
npm ci
npx tauri build
```

运行 Rust 测试：

```bash
cd ..
cargo test --release --workspace
```

构建中文 Inno Setup 安装包（需安装 Inno Setup 6.5+ 或 7.x）：

```text
cd ..
set SERAPHINE_INSTALLER_VERSION=2.0.3
"C:\Program Files\Inno Setup 7\ISCC.exe" installer\seraphine-rust.iss
```

签名构建需要本机的 updater 私钥；私钥不进入仓库，也不要在日志或截图中暴露：

```bash
export TAURI_SIGNING_PRIVATE_KEY="$(cat "$HOME/.seraphine/updater.key")"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
npx tauri build
```

旧 Python/PyQt 版已归档在 GitHub 的 `legacy-python` 分支；`main` 只维护 Rust/Tauri 与 Rust 发布工具，不再包含 Python 应用或 PowerShell 脚本。

## 项目结构

```text
rust/
  app/                  Tauri 2 工程（src-tauri + React 前端）
  crates/seraphine-core 配置、LCU/SGP、战绩、搜索、OP.GG、ARAM、自动化
  tools/aram-data       ARAM 数据生成 CLI（Wiki / jsdelivr）
  tools/release         latest.json 生成与 Gitee Release 发布
  sidecar/              旧 Python 后端的 Rust sidecar（保留）
  resources/data        ARAM 内置快照与清单
  resources/images      图标回退资源
installer/              Inno Setup / NSIS 相关资源
docs/                   升级总结与截图
.github/workflows/      Rust 构建、发布与 Gitee 镜像
```

## 安全与合规

- Seraphine 只使用英雄联盟客户端 API（LCU）与公开数据接口；除用户主动使用“锁定游戏设置”会切换客户端配置文件的只读属性外，不修改游戏内容文件、不读取游戏内存。
- 客户端写操作默认开启，但受允许清单、阶段校验、请求哈希、读回确认与 30 天写守卫约束；失败或结果未知时不会盲目重试。
- LCU API 的使用仍存在账号风险；本项目不保证不会被处罚，请自行评估并在自己的账号上使用。
- Seraphine 不是 Riot Games 官方产品，也未获得 Riot Games 认可；Riot Games、League of Legends 及相关资产归 Riot Games, Inc. 所有。

## 致谢

本项目实际使用或参考了以下开源项目与数据源：

- **Riot Games / League of Legends**：LCU API、Data Dragon 与游戏资源。
- **[League of Legends Wiki](https://wiki.leagueoflegends.com/)**：ARAM 加成数据来自 `Module:ChampionData/data`（CC BY-SA 3.0），由 CI 生成版本化快照。
- **[nomi-san/balance-buff-viewer](https://github.com/nomi-san/balance-buff-viewer)**（MIT）：Wiki 不可达时的预解析数据回退来源。
- **[CommunityDragon](https://raw.communitydragon.org/latest/)**：客户端资源与图标参考。
- **腾讯游戏**：国服英雄列表与部分资源（`game.gtimg.cn`）。
- **[OP.GG](https://op.gg/)**：榜单、出装与符文数据（非官方接口）。
- **[Tauri](https://tauri.app/)**、**[React](https://react.dev/)**、**[Fluent UI](https://react.fluentui.dev/)**、**[Vite](https://vite.dev/)**、**[TypeScript](https://www.typescriptlang.org/)**：桌面外壳与界面。
- **Rust 生态**：tokio、serde、reqwest、tokio-tungstenite、sha2、thiserror、minisign-verify 等。
- **[Intel PresentMon](https://github.com/GameTechDev/PresentMon)**：游戏帧时间 A/B 测量。
- **[Inno Setup](https://jrsoftware.org/isinfo.php)** 与 NSIS/Tauri bundler：安装包。
- **[GitHub Actions](https://github.com/features/actions)** 与 **[Gitee](https://gitee.com/)**：构建、镜像与分发。
- 感谢所有通过 issue、日志和实机对局反馈问题的玩家。

## 许可

本项目使用 [MIT License](LICENSE)。安装包内另附中文用户协议；第三方数据与游戏资源遵循其各自许可与权利归属。
