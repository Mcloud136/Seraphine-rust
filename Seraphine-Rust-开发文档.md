# Seraphine Rust Sidecar 设计与开发进度文档

- 文档日期：2026-11-25
- 项目状态：开发中，未达到默认切换与发布条件
- 当前默认后端：`python`
- 目标部署：`Seraphine.exe`（PyQt UI）+ `seraphine-sidecar.exe`（Rust）
- 适用范围：Windows 10 1809+ / Windows 11 x64，Python 3.8

---

## 1. 背景与目标

原版 Seraphine 使用 PyQt5/qasync 同时承担 UI、LCU/SGP 网络、WebSocket、数据解析、图片缓存和自动 B/P。在部分低 CPU 设备上，Python 进程占用不高，但游戏仍可能出现掉帧。

本项目的目标不是重做 UI，而是：

1. 完整保留现有 PyQt 界面、交互和视觉。
2. 将网络、WebSocket、解析、缓存、图片下载和自动化迁移到独立 Rust sidecar。
3. 通过请求合并、限流、缓存和游戏状态感知调度降低对游戏进程的干扰。
4. 保留 `python | shadow | sidecar` 三阶段开关和整包回滚能力。
5. 只有在真实游戏帧时间、内存、UI、打包和政策门槛全部通过后，才将 sidecar 切换为默认后端。

WinUI 3/Rust 全栈方案仅保留为技术 spike，不进入当前主路线。

---

## 2. 总体架构

```text
Seraphine.exe / PyQt5
  ├─ 现有界面、托盘、设置、更新
  ├─ app/backend/sidecar_client.py  子进程、帧收发、心跳、重连
  ├─ app/backend/compat.py          旧 connector/manager/signalBus 契约
  ├─ app/backend/catalog.py         静态目录兼容对象
  ├─ app/backend/image_pool.py      有界 QImage 后台解码
  └─ app/backend/supervisor.py      Job Object、优先级、路径解析
              │
              │ 匿名 stdin/stdout
              │ u32 little-endian length + UTF-8 JSON
              ▼
seraphine-sidecar.exe / Rust
  ├─ protocol             IPC 信封、错误、指标
  ├─ platform             Toolhelp、PEB、Windows API
  ├─ lcu                  LCU/SGP/External/Asset HTTP、WAMP WS、缓存
  ├─ domain               Catalog、OP.GG、ARAM、英雄别名
  ├─ automation           dry-run、写前守卫、写后确认、台账
  └─ resource_scheduler   P0–P4 游戏状态感知调度
```

### 为什么使用匿名 stdio

- sidecar 只由 Python 私有启动。
- 无需全局管道名、随机名称或 DACL。
- 不会被其他进程枚举连接。
- 启动和帧收发开销更低。
- `stderr` 仅用于受限诊断，业务协议禁止写 stdout。
- 子进程启动不使用 shell。

---

## 3. 关键技术规范

### 3.1 IPC 协议

- 帧格式：`u32_le length + UTF-8 JSON Message`。
- 当前实现为更严格限制：单帧 1 MiB、请求 256 KiB、响应/事件各 512 KiB。
- 全局在途字节预算 4 MiB。
- Python pending requests 最多 64，事件队列最多 128。
- `hello` 携带协议版本、能力集合、应用版本、PID 和 epoch。
- 请求携带 `id/epoch/op/args/deadline_ms`。
- sidecar 重启后通过 hello 继承上一代 epoch，再执行 LCU connect 时递增。
- 写操作取消仅表示停止等待，实际结果必须读回确认或返回 `OutcomeUnknown`。

### 3.2 进程与凭据

- Toolhelp 每 1.5 秒枚举 `LeagueClientUx.exe`。
- Python 与 Rust 均直接调用 Windows API，不再使用 `tasklist`、WMI、PowerShell 或 `cmd`。
- 从目标 PID 的 PEB/命令行读取端口、token 和服务器。
- token 只保存在 Rust `Zeroizing` 容器中，不 Debug、不回传、不落盘。
- 新 PID、token 轮换或 sidecar 重启时提升 epoch，旧结果失效。

### 3.3 HTTP 与缓存

- LCU、SGP、External、Asset 使用独立客户端。
- 仅对 `127.0.0.1` LCU 关闭证书校验。
- 外部请求必须校验证书并限制主机。
- LCU GET 支持 in-flight 合并和 TTL 缓存。
- match detail TTL 24 小时，match list/ranked 约 5 秒，volatile state 约 200 ms。
- OP.GG 支持请求合并、60 秒 TTL 和约 2 MiB 字节预算缓存。
- Catalog 生成紧凑快照，sidecar 只保留轻量索引。

### 3.4 WebSocket

- 单 WAMP actor，显式订阅 gameflow、champ-select、summoner、entitlement。
- 重连顺序：
  1. 建立连接并发送订阅。
  2. 并发缓冲 socket 事件。
  3. 使用无缓存 REST 拉取新鲜快照。
  4. 应用快照。
  5. 按可选 WAMP sequence 去重、排序并重放缓冲事件。
  6. 再次拉取快照消除竞态。
  7. 恢复正常事件处理。
- 断线退避 250 ms → 8 s。
- gameflow 事件同步更新资源调度器。

### 3.5 资源调度

- 游戏状态：NotRunning、ClientStarting、Lobby、ChampSelect、InGame、EndOfGame、Reconnect。
- 优先级：P0–P4。
- 游戏内暂停非必要预取、图片批处理和后台刷新。
- 交互请求保留最低配额。
- 不采用硬 CPU 限额、固定核心亲和性或强制 working-set 回收。

### 3.6 图片

- 继续使用原目录和文件名，兼容迁移到 `%LOCALAPPDATA%/Seraphine/Cache`。
- Python 使用有界 `QImageReader` 后台解码池。
- 限制队列、缓存字节、源文件大小和解码像素。
- sidecar 只传本地路径，不传图片字节。

### 3.7 自动 B/P 与写操作

- 默认真实写操作关闭。
- 仅在显式设置 `SERAPHINE_ENABLE_LCU_WRITES=1` 时开放技术能力。
- 写前重新获取 champ-select 会话并验证：
  - 本地 cell。
  - action 所属玩家。
  - action 类型。
  - `isInProgress`。
  - 未完成状态。
  - 阶段。
  - 目标英雄未被 ban/pick。
  - swap/trade 是否仍为 `RECEIVED`。
- 写后再次读取会话确认目标状态。
- 确认失败记为 `OutcomeUnknown`，不盲目重试。
- acceptTrade/acceptSwap 两步使用稳定 operation ID 分别记账。
- dry-run 计划和幂等台账已实现。
- 技术实现不等于 Riot 政策许可。

### 3.8 更新器

- 运行时更新器为 Rust 原生程序，不使用 PowerShell。
- 校验 `update-manifest.json` 中的路径、大小和 SHA256。
- 拒绝绝对路径、`..`、反斜杠路径和越界文件。
- 安装前等待 `Seraphine.exe` 与 sidecar 退出。
- 旧文件先备份，新文件原子替换。
- 失败时移除新文件并恢复备份。
- 支持 updater 自更新：旧进程暂存新 updater，一次性 finalizer 在临时目录等待旧进程退出后完成替换。
- 默认保留备份，便于人工回滚。

---

## 4. 目录说明

```text
app/
  backend/                 Python sidecar 兼容层
  common/                  配置、更新、工具、信号
  components/              UI 组件
  lol/                     原 LCU/SGP/OP.GG/ARAM/工具逻辑
  view/                    界面
rust/
  crates/
    automation/            dry-run、写守卫
    domain/                Catalog、OP.GG、ARAM、别名
    lcu/                   HTTP、WS、缓存
    platform/              Windows API
    protocol/              IPC 与指标
    resource_scheduler/    资源调度
  sidecar/
    src/main.rs            sidecar 主程序
    src/bin/updater.rs     原生更新器
    tests/stdio.rs         stdio 集成测试
spike/winui3-rust/         WinUI 3 可行性 spike，仅参考
tests/
  contract/                Python 契约、UI、Windows API、更新器测试
  bench/                   sidecar 与 PresentMon 关联基准
```

---

## 5. 开发铁律

1. **不得使用 PowerShell、WMI、`cmd`、`tasklist` 作为产品运行时旁路。**
2. 进程发现、句柄、优先级、缓存和更新优先使用 Windows API 或 Rust。
3. token、Authorization、accessToken、完整响应和 PUUID 不得落盘或回传 UI。
4. Python 仍是 `config.json` 的唯一写入者。
5. 保持现有设置键、方法签名和既有拼写，包括历史兼容字段。
6. 真实 LCU 写入默认关闭，政策审批前不得启用。
7. 不在 BP 进行中自动切换后端。
8. 不使用硬 CPU 限额、固定亲和性或强制 working-set 回收。
9. 外部请求必须校验证书并使用主机白名单。
10. 缓存和配置文件必须原子替换，不得先删后移。
11. 旧请求结果不得覆盖新目标，详情、生涯和搜索必须使用世代号或任务取消。
12. 不得吞掉 `asyncio.CancelledError`。
13. 不得用测试通过替代真实 UI 和真实游戏验收。
14. 未通过全部硬门槛前，默认后端必须保持 `python`。
15. 上传仓库前必须排除凭据、运行时数据、缓存、日志、构建产物和个人路径。

---

## 6. 进度安排与当前状态

| 阶段 | 内容 | 状态 |
|---|---|---|
| 1 | 契约、协议、后端开关、 Golden Diff | 已完成 |
| 2 | sidecar 骨架、stdio、心跳、Toolhelp、Job Object、指标 | 已完成 |
| 3 | LCU HTTP/WS、Catalog、图片、Career/Search/GameInfo/TFT | 已完成 |
| 4 | SGP、OP.GG、ARAM、英雄别名、按需分页 | 基本完成，ARAM 上游受阻 |
| 5 | 事件桥接、慢消费者、断线恢复 | 已完成 |
| 6 | dry-run、写前守卫、写后确认、台账 | 技术部分完成，政策门槛未通过 |
| 7 | 默认切换、打包、更新、灰度 | 未完成 |
| 8 | 清理旧 Python 后端 | 未开始 |

---

## 7. 已完成内容

### 7.1 Rust 后端

- 完整 Cargo workspace 与 release profile。
- 匿名 stdio IPC、能力协商、epoch 和心跳。
- 直接 Windows API 进程枚举与 PEB 命令行读取。
- LCU/SGP/External/Asset 四类隔离 HTTP 客户端。
- WAMP WebSocket、重连缓冲、REST 快照和序列去重。
- Catalog 紧凑快照与 Python manager 兼容。
- LCU TTL/in-flight 缓存和缓存指标。
- OP.GG 解析、图标落地、请求合并与字节预算缓存。
- ARAM/ChampionAlias 解析、原子缓存和旧缓存回退。
- 游戏状态感知 P0–P4 资源调度。
- 自动化 dry-run、写前守卫、写后确认和台账。
- 固定桶 p50/p95/p99、请求速率、错误率、缓存命中/合并指标。
- 原生 SHA256 更新器、备份、回滚和自更新 finalizer。

### 7.2 Python 兼容与 UI

- `backendMode=python|shadow|sidecar` 实际接入。
- `SidecarConnector` 兼容主要读方法和写方法。
- 历史记录统一为旧版 list 契约，TFT 和段位异常已修复。
- CatalogManager 的 `getPerkStyles()` 已构建完整 style/slot schema。
- OP.GG sidecar 模式切换到 `RustOpgg`。
- ARAM/别名在 sidecar 模式使用 Rust 数据。
- QImage 有界后台解码池。
- 搜索页首屏 20 局，后续按需每次 10 局。
- 搜索筛选每次最多额外预取两批。
- 搜索详情和生涯刷新加入世代号保护。
- 多客户端召唤师查询改为异步，不在 UI 线程同步请求。
- StartInterface 不再展示 LCU token。
- 运行时进程发现移除 tasklist/WMIC。
- 更新器启动移除 PowerShell。

### 7.3 测试与工具

- Rust 工作区测试。
- Python 3.8 契约测试。
- Qt offscreen 分页 UI 测试。
- Windows Toolhelp/PEB 测试。
- 更新器安装、失败回滚和自更新测试。
- 真实 LCU Golden Diff。
- sidecar 崩溃后自动重连和 epoch 递增验证。
- sidecar CPU/内存/延迟基准。
- PresentMon QPC 关联分析脚本。

---

## 8. 未完成内容与风险

### P0：发布阻断

1. **PyInstaller 整包实跑未完成。**
   - 两个 Rust exe 已能发布构建。
   - `make.ps1` 已修复并生成 SHA256 清单。
   - 仍需验证完整 `Seraphine.7z`、安装、更新和回滚。

2. **真实游戏帧时间验收未完成。**
   - 尚未在 Intel 250KF 上完成 PresentMon/WPR A/B/C。
   - 不能证明游戏内 P95/P99 帧时间和卡顿次数达标。

3. **完整 sidecar UI 灰度未完成。**
   - 默认仍为 Python。
   - 需要实际客户端中验证 Career、Search、GameInfo、OP.GG、TFT、ARAM 和多客户端切换。

### P1：功能与并发

1. sidecar 请求仍基本串行；deadline、取消、独立 writer 和完整有界请求调度仍需完善。
2. SGP 401 刷新一次和安全读取重试策略仍需完善。
3. 写台账键仍需按 PID/epoch/session/action/type/configRevision 完整作用域化。
4. HTTP 错误在读回前不应简单记为最终失败。
5. Asset 写入仍需全面改为原子替换，并补齐 in-flight 去重和 30 秒负缓存。
6. 图片 key 变化或加载失败时需要清理旧图标，剩余同步解码路径需要继续迁移。
7. ARAM 上游当前返回 503，HTTPS 证书也不可用；Rust 已能回退旧缓存，但真实源恢复前无法验收新数据。

### P2：性能与维护

1. 首次外部 TLS + 别名加载后 Working Set 约 10.65 MiB，超过 10 MiB 目标。
2. 10 分钟空闲基准曾因后台 sidecar 退出而无效；60 秒空闲复测稳定，但不能替代 10 分钟验收。
3. 旧 Python 网络和解析实现尚未删除。
4. 多服 fixture、429/5xx/超时/token 轮换等完整 mock 覆盖仍需补齐。
5. Authenticode 签名尚未配置。

---

## 9. 已验证数据

- Rust：37 项测试通过。
- Python：36 项通过，1 项真实 LCU 测试在未设置环境变量时跳过。
- Clippy：`-D warnings` 通过。
- Rust 格式检查通过。
- 真实 LCU Golden Diff 通过。
- sidecar 崩溃后自动重连通过，epoch 从 1 递增到 2。
- 60 秒空闲：CPU 平均约 0.026%，Working Set 约 6.43 MiB，Private 约 0.84 MiB。
- 20 req/s 短测无错误。
- 真实英雄别名源返回 173 个英雄。
- 更新器自替换 finalizer 通过发布版回归测试。

上述数据不等于最终发布验收。

---

## 10. 运行、构建与回滚

### 10.1 后端选择

```text
SERAPHINE_BACKEND_MODE=python   默认，Python 后端
SERAPHINE_BACKEND_MODE=shadow   Python 主用，Rust 影子对比
SERAPHINE_BACKEND_MODE=sidecar  Rust sidecar 主用，灰度使用
```

### 10.2 真实写入

默认关闭。仅在完成政策审批和完整写操作验收后，才可考虑：

```text
SERAPHINE_ENABLE_LCU_WRITES=1
```

### 10.3 Rust 构建

在已初始化 MSVC x64 开发环境的终端中：

```powershell
cargo fmt --manifest-path .\rust\Cargo.toml --all -- --check
cargo clippy --manifest-path .\rust\Cargo.toml --workspace --all-targets -- -D warnings
cargo test --manifest-path .\rust\Cargo.toml --workspace
cargo build --release --manifest-path .\rust\Cargo.toml -p seraphine-sidecar --bins
```

### 10.4 Python 测试

先构建 release sidecar/updater，再运行契约测试：

```powershell
cargo build --release --manifest-path .\rust\Cargo.toml -p seraphine-sidecar --bins
$env:QT_QPA_PLATFORM = "offscreen"
python -m unittest discover -s tests/contract -v
```

### 10.5 回滚

- 配置层：将 `BackendMode` 切回 `python` 或 `shadow`。
- 整包层：使用上一个发布版本。
- 更新层：保留 `UpdateBackup`，失败时由 updater 自动恢复或人工恢复。
- 协议层：主协议版本不匹配时拒绝写功能，不静默降级。

---

## 11. 上传与脱敏规范

上传内容应仅包含源码、静态资源、测试、文档和锁文件。

必须排除：

- `.git/`、`rust/target/`、`spike/winui3-rust/target/`。
- `build/`、`dist/`、`__pycache__/`、虚拟环境。
- `app/resource/game/` 运行时图片缓存。
- `config.json`、日志、`shadow_diff.jsonl`、运行时 Catalog/ARAM/Alias JSON。
- `.env`、私钥、证书、PFX/P12。
- 编译生成的 exe/dll/pyd/7z/zip。
- 本地用户名、绝对路径、PUUID、召唤师 ID 和真实 token。
- ARAM appsecret 等硬编码凭据；已改为通过 `SERAPHINE_ARAM_APPSECRET` 环境变量注入。

---

## 12. 下一步建议顺序

1. 完成 PyInstaller 整包安装、更新、回滚验证。
2. 完成 sidecar 模式完整 UI 灰度。
3. 完成 Intel 250KF PresentMon/WPR 真实游戏 A/B/C。
4. 将首次外部 TLS 场景 Working Set 降至 10 MiB 以下。
5. 完善 IPC 并发、取消、deadline 和独立 writer。
6. 完善 SGP 401 刷新、安全重试和完整写台账作用域。
7. 等待 ARAM 上游恢复并完成真实源 Golden Diff。
8. 完成政策审批后再评估真实写入。
9. 全部硬门槛通过后观察 7 天，再切换默认后端。
10. 最后清理旧 Python 网络/解析实现。

---

## 13. AI 接手操作清单

### 13.1 必读顺序

1. 本开发文档。
2. `app/backend/protocol.py` 与 `rust/crates/protocol/src/lib.rs`。
3. `app/backend/sidecar_client.py` 与 `rust/sidecar/src/main.rs`。
4. `app/backend/compat.py`、`app/backend/catalog.py`。
5. `rust/crates/lcu/src/websocket.rs`、`rust/crates/lcu/src/cache.rs`。
6. `tests/contract/` 与 `tests/bench/`。

### 13.2 开发环境

- Windows 10/11 x64。
- Python 3.8。
- Rust stable x86_64-pc-windows-msvc。
- Visual Studio Build Tools，包含 MSVC 和 Windows SDK。
- 不需要安装 WinUI 3 才能完成当前 sidecar 路线。
- spike 目录只用于历史技术验证，不应作为主程序依赖。

### 13.3 第一次验证

在仓库根目录执行：

```powershell
cargo fmt --manifest-path .\rust\Cargo.toml --all -- --check
cargo clippy --manifest-path .\rust\Cargo.toml --workspace --all-targets -- -D warnings
cargo test --manifest-path .\rust\Cargo.toml --workspace
cargo build --release --manifest-path .\rust\Cargo.toml -p seraphine-sidecar --bins
$env:QT_QPA_PLATFORM = "offscreen"
python -m unittest discover -s tests/contract -v
```

预期：Rust 37 项通过；Python 36 项通过、1 项真实 LCU 测试在未配置环境变量时跳过。

### 13.4 修改约束

- IPC 字段变更必须同步修改 Python 与 Rust 两侧。
- 新增外部端点必须加入主机白名单、超时和字节预算。
- 新增写操作必须默认关闭，并具备写前验证、写后确认和 `OutcomeUnknown` 处理。
- 新增 UI 异步任务必须处理取消、世代号和慢消费者。
- 不得把真实 token、PUUID、本地路径或运行时响应写入 fixture、日志或文档。
- 每完成一个阶段，更新本文档的“已完成”“未完成”和“已验证数据”。

### 13.5 当前最优先工作

1. 修复并验证 PyInstaller 整包。
2. 完善 sidecar IPC 并发、取消、deadline 和独立 writer。
3. 完善 SGP 401 刷新、安全重试与写台账完整作用域。
4. 完成 sidecar 模式完整 UI 灰度。
5. 完成真实游戏 PresentMon/WPR 验收和外部 TLS 内存优化。

在这些工作完成前，不要把默认后端从 `python` 切换为 `sidecar`，不要启用真实 LCU 写入。
