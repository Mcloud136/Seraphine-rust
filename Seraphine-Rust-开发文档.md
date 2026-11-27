# Seraphine Rust Sidecar 设计与开发进度文档

- 文档日期：2026-11-26
- 修订：2026-11-26 补齐 §14 IPC 契约、§15 验收标准、§16 阶段 8 清理与多客户端规范；修正 hello 字段说明、1.5 s 轮询归属与首次验证命令的构建口径
- 项目状态：开发中，未达到默认切换与发布条件
- 当前默认后端：`python`
- 目标部署：`Seraphine.exe`（PyQt UI）+ `seraphine-sidecar.exe`（Rust）
- 适用范围：Windows 10 1809+ / Windows 11 x64，Python 3.13

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
- `hello` 携带协议版本、能力集合、应用版本和 epoch；sidecar 自身 PID 由 Python 以子进程句柄直接获得，不放在 hello 帧内。
- 请求携带 `id/epoch/op/args/deadline_ms`。
- sidecar 重启后通过 hello 继承上一代 epoch，再执行 LCU connect 时递增。
- 写操作取消仅表示停止等待，实际结果必须读回确认或返回 `OutcomeUnknown`。

### 3.2 进程与凭据

- Python 侧 `LolProcessExistenceListener` 每 1.5 秒用本机 Toolhelp（ctypes 实现，函数名保留 `getTasklistPath` / `getLolClientPids` 以兼容旧调用）枚举 `LeagueClientUx.exe`；Rust 侧仅在收到 `lcu.connect` / `process.command_line` 时按需读取目标 PID 的 PEB 命令行，不自行轮询。
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

- 发行版真实写操作直接开启，开箱即用，不设环境变量或产品开关。
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
| 4 | SGP、OP.GG、ARAM、英雄别名、按需分页 | 已完成（ARAM 改为 Wiki 快照管线） |
| 5 | 事件桥接、慢消费者、断线恢复 | 已完成 |
| 6 | dry-run、写前守卫、写后确认、台账 | 技术部分完成，政策门槛未通过 |
| 7 | 默认切换、打包、更新、灰度 | 未完成 |
| 8 | 清理旧 Python 后端 | 未开始 |

---

## 7. 已完成内容

### 7.1 Rust 后端

- 完整 Cargo workspace 与 release profile。
- 匿名 stdio IPC、能力协商、epoch 和心跳。
- 独立 stdout writer，128 条消息与 4 MiB 在途字节双重有界队列，保持帧顺序并提供慢消费者背压。
- 最多 64 个请求任务、8 个只读并发槽和单控制槽；支持 cancel、deadline、请求代次与 WS 连接代次隔离，写开始后不可中止。Python 超时/外部取消会有界补发 CANCEL；Rust 读请求在获取 client 快照前校验 epoch，Python pending 同时拒绝跨代响应。
- stdin 使用持久帧缓冲，`select!` 取消轮询时不会丢失半帧或串帧。
- 直接 Windows API 进程枚举与 PEB 命令行读取。
- LCU/SGP/External/Asset 四类隔离 HTTP 客户端；SGP 401 互斥刷新 entitlement token 一次，单次 RPC 对 429/5xx/网络错误最多安全重试一次。LCU 200 空体与 204 均返回 `null`，不再误报 JSON 失败。
- WAMP WebSocket、重连缓冲、REST 快照和序列去重；去重水位按连接重置，缓冲事件重放时推进水位，避免重连重复投递或 seq 归零丢事件。
- Catalog 紧凑快照与 Python manager 兼容，`questSkinInfo.tiers` 已展开为独立皮肤并保留 Augments；Catalog 与社区缓存均改为原子替换。
- LCU TTL/in-flight 缓存和缓存指标；缓存领头请求使用 RAII `CacheFlight`，取消/abort 时自动清理并唤醒等待者，不再永久毒化 in-flight。
- Asset 原子写入、按目标/来源/字节预算合并、30 秒负缓存和缓存指标；OP.GG 图标复用同一路径。
- OP.GG 与 Asset 控制请求改为 `NetworkSnapshot`：网络期间不持全局 `State` 锁，懒初始化客户端只短暂回写；OP.GG 缓存增加取消安全的 `CacheFlight`。
- Catalog、英雄别名刷新与 SGP 同样改为快照处理；SGP 401 token 刷新期间不持全局锁，切换 epoch 后丢弃旧刷新结果。Catalog 写入与恢复统一使用 `%LOCALAPPDATA%/Seraphine/catalog.json`。
- `lcu.write` 使用一次性 `State` 工作副本执行写前校验、HTTP 写入、读回确认与守卫持久化，网络期间不持全局锁；结束后仅在 epoch 未变时回写台账/守卫，连接切换返回 `OutcomeUnknown`，不盲目重放。
- OP.GG 解析、图标落地、请求合并与字节预算缓存。
- ARAM/ChampionAlias 解析、原子缓存和旧缓存回退。
- 游戏状态感知 P0–P4 资源调度。
- 自动化 dry-run、写前守卫、写后确认和台账；真实写台账按 PID/epoch/session/action/type/configRevision/operationId 作用域化，并使用请求指纹拒绝冲突复用。
- HTTP 写失败后先读回确认；确认成功记 Completed，否则记 OutcomeUnknown；未知结果重放不会再次发写。
- 写守卫以 PID+operationId 和请求 SHA256 哈希脱敏持久化到 `%APPDATA%/Seraphine/write-guards.json`；文件锁内原子 claim，多实例合并且不覆盖未知状态，重启后仍保持封锁。Completed 记录保留 30 天，过期自动清理；OutcomeUnknown 与旧版无时间戳记录永不淘汰。
- 固定桶 p50/p95/p99、请求速率、错误率、缓存命中/合并指标。
- 原生 SHA256 更新器、备份、回滚和自更新 finalizer。

### 7.2 Python 兼容与 UI

- `backendMode=python|shadow|sidecar` 实际接入。
- `SidecarConnector` 兼容主要读方法和写方法。
- 历史记录统一为旧版 list 契约，TFT 和段位异常已修复。
- CatalogManager 的 `getPerkStyles()` 已构建完整 style/slot schema。
- OP.GG sidecar 模式切换到 `RustOpgg`。
- ARAM/别名在 sidecar 模式使用 Rust 数据；ARAM 只读本地快照，Wiki 生成与可选 Gitee 热更新由 Python 兼容层负责。
- QImage 有界后台解码池，所有运行时图片路径统一异步解码；key 变化、失败、文本切换和清空时移除旧图并拒绝过期结果。
- 搜索页首屏 20 局，后续按需每次 10 局。
- 搜索筛选每次最多额外预取两批。
- 搜索详情和生涯刷新加入世代号保护。
- 多客户端召唤师查询改为异步，不在 UI 线程同步请求。
- StartInterface 不再展示 LCU token。
- 运行时进程发现移除 tasklist/WMIC。
- 更新器启动移除 PowerShell。
- `SidecarConnector.start()` 二次调用先取消旧事件消费者，转发时按 epoch 过滤；写路径统一去除尾斜杠，兼容 Rust 守卫。
- shadow 对照支持递归易变字段白名单（当前 `time_eliminated`），避免进行中 TFT 对局误报差异。
- `SidecarConnector.call/start` 已对齐旧端五次重试、`callStack`、`SummonerNotFound` 不重试与 `lcuApiExceptionRaised`；启动连续失败包装为 `RetryMaximumAttempts`，主窗口可正确等待下一次客户端事件。

### 7.3 测试与工具

- Rust 工作区测试，含 Asset 原子写、合并、负缓存和真实 stdio 回归。
- Python 3.13 契约测试。
- Qt offscreen 分页 UI 测试。
- Qt offscreen 图片回归测试：异步解码、DPR、缓存复用、key 切换清理、失败清理和过期结果拒绝。
- Windows Toolhelp/PEB 测试。
- 更新器安装、失败回滚和自更新测试。
- 真实 LCU Golden Diff。
- 本地 HTTPS 模拟 LCU 写安全回归：HTTP 500 后读回确认、operationId 冲突、跨会话封锁、OutcomeUnknown 重放和非法路径拒绝。
- sidecar 崩溃后自动重连和 epoch 递增验证。
- sidecar CPU/内存/延迟基准。
- PresentMon QPC 关联分析脚本。
- `tests/bench/frame_time_ab.py` 默认 60 秒热身、300 秒采样、3 轮聚合；支持基线 JSON、P99 回退阈值与卡顿增量阈值，不达标返回退出码 2。
- `tests/bench/package_smoke.py`：manifest 路径/大小/SHA256、`--version`、包内 sidecar 握手，以及真实 Windows UI 必须写入 `Seraphine initialized` 启动完成标记。
- 新增取消传播、跨 epoch 响应、缓存领头取消、WS 水位重置、200/204 空体、quest 皮肤层、事件消费者重启、shadow 易变字段白名单等回归测试。
- CI 发布门禁改为固定 PyInstaller 6.22.3 + Rust release 测试 + 完整 `package_smoke`（不跳 UI）；内存基准测试独立成集成测试进程，避免库内并发测试污染 Working Set。

---

## 8. 未完成内容与风险

### P0：发布阻断

1. **真实游戏帧时间验收未完成。**
   - 尚未在 Intel 250KF 上完成 PresentMon/WPR A/B/C。
   - 不能证明游戏内 P95/P99 帧时间和卡顿次数达标（判定口径见 §15.1）。

2. **完整 sidecar UI 灰度未完成。**
   - 默认仍为 Python。
   - 默认 Python 后端已在真实客户端验证 Career、Search、GameInfo、设置、辅助功能和 OP.GG；sidecar 已通过 LCU Golden Diff、生涯/搜索及 OP.GG 兼容层实测。本轮已修复取消不补发、缓存领头取消毒化、旧 epoch 读跨客户端、WS 重连去重与事件消费者重复问题；高并发对局详情期间的控制锁/背压仍需真实 UI 灰度复测。
   - 仍需实际客户端中完成 sidecar 的 TFT、ARAM、多客户端切换和完整英雄选择/对局流程稳定性验收（验收矩阵见 §15.2）。

### P1：功能与并发

1. **ARAM 上游事件已关闭（2026-11-26）**：`jddld.com` 与 `www.jddld.com` 的 DNS 均已 CNAME 到域名停放/跳转服务 `url.gname.net`（公共 DNS 8.8.8.8 结果相同，非本地 DNS 劫持），HTTPS 握手失败。直连探测（绕过代理、aiohttp 与 Chrome UA、根路径与 `/index.php` API、带/不带 appsecret）全部返回 `503` + 152 字节 `Loading in progress` 的 JS 中间页（`location.href=""`，`Server: Tengine`），未返回任何 JSON。**结论：该域名已无法获取大乱斗数据**，社区反馈浏览器访问会被中间页跳转至不良站点——按"域名被接管"处理。Rust `ExternalClient` 不跟随跳转（`Policy::none`）且失败时回退旧缓存，但明文 HTTP 会把 `appsecret` 发给停放服务，视为已泄露。替换可信源前无法验收新数据。

   处置（已实施，2026-11-26）：
   - **数据生产**：`scripts/update_aram_data.py` 从 Wiki 模块生成 `app/resource/data/AramBuff.json` + `aram-manifest.json`（含 SHA256/大小/版本校验）；`.github/workflows/update_aram_data.yaml` 每周三/四/五自动运行并提交，`scripts/publish_gitee_assets.py` 将两个文件发布到 Gitee Release 固定标签 `aram-data-latest`。
   - **数据消费**：`app/backend/aram_data.py` 实现本地优先加载（缓存 > 内置快照，缓存缺失时自动播种）、"24 小时或客户端版本不一致"节流检查、HTTPS 主机白名单、SHA256/schema 校验、原子替换；失败静默回退，不阻塞启动。
   - **交互**：设置页新增"自动更新 ARAM 数据"开关与"立即更新"按钮（配置项 `AramDataAutoUpdate` / `AramDataFeedUrl`，环境变量 `SERAPHINE_ARAM_DATA_URL` / `SERAPHINE_ARAM_AUTO_UPDATE`）。
   - **sidecar**：`aram_buff.refresh` 改为纯读本地缓存，删除 jddld 地址、appsecret 与上游解析代码。
   - **凭据**：`SERAPHINE_ARAM_APPSECRET` 已移除，视为泄露并作废。

   换源评估（2026-11-26 实测）：
   - **LoL Wiki `Module:ChampionData/data`（推荐）**：覆盖 ARAM 全部 10 项修正（`dmg_dealt`/`dmg_taken`/`healing`/`shielding`/`ability_haste`/`mana_regen`/`energy_regen`/`attack_speed`/`movement_speed`/`tenacity`），是社区插件 `nomi-san/balance-buff-viewer`（MIT）的数据来源。参考实现（已核实源码）：GitHub Actions 每周三/四/五用普通 `fetch`（UA `curl/8.13.0`）抓取模块 → `luaparse` 解析为 `{id,name,title,stats:{aram:{...}}}` → 生成 `dist/balance.json` 并内联进 npm 构建（约 49 KB JS / 100 KB 包）→ 运行时 stub 仅 `import('https://esm.sh/balance-buff-viewer-plugin@latest')`，**客户端不直连 wiki**；版本检测使用 **Data Dragon** `ddragon.leagueoflegends.com/api/versions.json` 的 `versions[0]`，按 SHA1 判数据变化。本机境内 `curl`/`aiohttp` 直连 403（Cloudflare 人机挑战），浏览器与 GitHub Actions 均可访问——挑战按客户端/IP 定向，非地域封锁。许可为 CC BY-SA（需核实版本并署名）。
   - **OP.GG `api/{region}/champions/aram*`**：实测响应无 balance/buff 字段，不能替代；继续用于 ARAM 梯队/出装。
   - **`czgreat/aramhex-data`**：国服海克斯大乱斗胜率/海克斯统计（CC BY-NC 4.0），非 buff 数值且禁商用，不宜并入 MIT 发行物。
   - **CommunityDragon 角色 bin JSON**：哈希键结构，需解哈希与编写抽取器，成本高，暂不推荐。
   - **WeGame / 腾讯官方**：WeGame 客户端不展示大乱斗 buff；腾讯 101 数据库仅有胜率类数据；官方无公开 buff API。
   - **全站镜像 wiki.leagueoflegends.com**：不建议。内容为 CC BY-SA 3.0（可再分发，需署名并相同方式共享），但图片等为 Riot 版权素材、不在 CC 覆盖范围；`api.php`/`Special:Export`/`robots.txt` 均受 Cloudflare 人机挑战（本机 403 实测），批量同步只能在 CI/浏览器环境做并受速率限制；复刻全站还需 Scribunto/Cargo/DynamicPageList4 等扩展栈。对 Seraphine 而言，只需"数据层快照"（单个 Lua 模块 → JSON），不必镜像站点。

2. **高并发真实 UI 复测仍待完成。** 读侧长控制操作与真实写均已快照化，本地 mock 已验证慢写期间 health 不被阻塞；仍需真实客户端完整 BP/对局流程确认 pending 背压与断线恢复表现。

### P2：性能与维护

1. 旧 Python 网络和解析实现尚未删除。
2. 多服 fixture、429/5xx/超时/token 轮换等完整 mock 覆盖仍需补齐。
3. Authenticode 签名尚未配置。
4. 持久化写守卫上限为 512 条；过期 Completed 已实现 30 天自动清理，OutcomeUnknown 不淘汰。旧版无时间戳 Completed 仍永久保留，需后续迁移策略。
5. `ExternalClient` 依赖 `error_for_status`，3xx 不算错误；跳转已被 `Policy::none` 拦住不会跟随，但 3xx 响应体仍会被读取到上限字节。建议社区端点读体前显式要求 `status().is_success()`，并禁止非 HTTPS 的社区数据源地址。

---

## 9. 已验证数据

- Rust：76 项测试通过（2026-11-26 Visual Studio 2026 18.12.1、MSVC 19.52.37029 / 工具集 14.52、`x86_64-pc-windows-msvc` release 实跑；内存基准独立进程）。
- Python：独立虚拟环境（Python 3.12 + 锁定依赖 + Qt offscreen）完整契约套件 94 项：93 项通过、1 项真实 LCU 条件跳过；另重试契约隔离场景 4 项通过。
- Clippy：`-D warnings` 通过（Visual Studio 2026 MSVC 工具链）。
- Rust 格式检查通过。
- 真实 LCU Golden Diff 通过。
- sidecar 崩溃后自动重连通过，epoch 从 1 递增到 2。
- MSVC release 首次外部 TLS + 别名加载：基线 Working Set 4.98 MiB，别名刷新后 9.64 MiB，外网 Asset 下载后 9.85 MiB，达到 10 MiB 目标。
- MSVC release 10 分钟空闲：零错误、进程未退出，CPU 平均约 0.008%，Working Set 平均/最大约 4.99/5.06 MiB，Private 平均约 0.85 MiB。
- MSVC release 60 秒空闲：CPU 平均约 0.026%，Working Set 约 4.98 MiB，Private 约 0.89 MiB。
- MSVC release 20 req/s 短测：400 次请求零错误，延迟 P50/P95/P99 约 0.253/0.310/0.352 ms，Working Set 约 5.04 MiB。
- 真实英雄别名源返回 173 个英雄。
- 更新器自替换 finalizer 通过发布版回归测试。
- 2026-11-25 本机 Qt offscreen 图片回归 6/6 通过，8 个真实图片控件加载/切换/失败清理冒烟通过。
- 2026-11-25 本机完整 Python 契约 50 项全部通过（1 项真实 LCU 按条件跳过）。
- 2026-11-25 本地 HTTPS 模拟 LCU 写安全回归 2/2 通过，真实 release sidecar 参与；覆盖跨重启完成重放、未知写封锁、文件锁、多实例合并和单 operation 原子 claim。
- 2026-11-25 真实 release sidecar 外网 Asset 下载 102,339 字节并命中磁盘缓存，失败负缓存 stdio 回归通过。
- 2026-11-25 SGP 本地 mock 回归通过：503 重试一次，400/401/JSON 错误不普通重试，401 token 刷新与旧事件防护通过。
- 2026-11-25 独立 IPC writer 回归通过：帧顺序、4 MiB 字节背压、真实 release 子进程握手/心跳/关闭均通过。
- 2026-11-25 IPC 请求并发回归通过：slow/fast 并发、8 读槽有界、64 pending 上限、cancel、deadline、写开始后不可中止及请求/WS 代次隔离。
- 2026-11-25 stdin 持久帧缓冲回归通过：轮询取消后半帧仍可正确解析，连续帧不串包。
- 2026-11-25 Python 3.13/GNU 预备整包通过：PyInstaller 6.22.3、501 文件 manifest、7z 归档、包内 sidecar 握手和真实 Windows UI 存活；真实 updater 恢复与损坏源拒绝通过。
- 2026-11-26 Visual Studio 2026/MSVC 最终整包通过：PyInstaller 6.22.3、503 个归档条目、501 文件 manifest、37 MiB 7z；包内 `--version`、sidecar 握手和真实 Windows UI 启动完成标记通过。
- 2026-11-26 修复首屏卡死：托盘菜单父参数兼容 PyQt-Fluent-Widgets 1.11.3，信号先于监听线程连接，启动异常时关闭首屏；隔离环境整包启动回归通过。
- 2026-11-26 真实客户端默认 Python 后端 UI 验证通过：大厅/游戏中标题、Career 20 局统计与历史、Search 实时十人对局详情、Game Information 友敌分区、辅助功能、设置和 OP.GG 16.23 极地大乱斗梯队数据。
- 2026-11-26 适配真实 LCU 新版对局历史结构，同时兼容旧版列表；Python 与 sidecar 契约测试覆盖两种结构。
- 2026-11-26 更新、关于、反馈、README 和 issue 模板统一指向 `https://github.com/Mcloud136/Seraphine-rust`，更新检查改用 HTTPS GitHub API。
- 2026-11-26 修复设置与辅助卡片正文标签未继承 Fluent 14 px 字体的问题；真实窗口诊断中 LCU 并发标签宽度由 21 px 恢复为 98 px，自动交换、延迟、默认英雄/技能等标签均恢复正常，头部冗余状态文本不再显示。
- 2026-11-26 新增“游戏中自动静默到托盘”开关并启用当前配置；真实 `InProgress` 状态下 Seraphine 主窗口与 OP.GG 窗口均为不可见，托盘图标仍可恢复。
- 2026-11-26 sidecar OP.GG 梯队列表改为紧凑占位符并在 Python 侧解析，单个反 Counter 限制为 UI 可见的 3 个；实测 ARAM 响应约 45 KiB、全位置排位约 149 KiB。
- 2026-11-26 sidecar pending 请求达到 64 时改为有界等待；写禁用和瞬时协议错误不再触发整程序退出。
- 2026-11-26 TFT 真实对局历史接口返回 1 局，Python 与 sidecar 结构一致，仅进行中对局的 `time_eliminated` 动态字段不同；shadow 对照已将该字段加入递归白名单，ARAM 数据已切换 Wiki 快照。
- 2026-11-26 修复自动语言检测：`Language.AUTO` 现在按系统语言选择中文、英文或葡萄牙文；`Ranked TFT` 补充为“云顶之弈排位”，`Howling Abyss` 在中文系统下显示为“嚎哭深渊”，包内 `.qm` 翻译已验证。
- 2026-11-26 ARAM 数据源切换：内置 Wiki 快照 `app/resource/data/AramBuff.json`（132 个有实际 ARAM 修正的英雄；按社区惯例，无修正英雄不收录、UI 不显示面板；manifest SHA256 校验）；`tests/contract/test_aram_data.py` 9 项纯逻辑契约测试通过（本机 Python 3.12；覆盖缓存优先/播种、清单与数据校验、篡改检测、节流规则与主机白名单）。
- 2026-11-26 sidecar 韧性修复：Python 超时/外部取消补发 CANCEL、pending 拒绝跨 epoch 响应；Rust 缓存领头取消 RAII 清理、WS 每连接去重水位、200/204 空体、quest 皮肤层展开；真实 release sidecar 客户端回归 13/13，Rust release 72/72。
- 2026-11-26 同机 A/B 实跑（HEAD 基线 vs 当前未提交版本，MSVC release，60 s / 20 req/s，各 1200 请求零错误）：P50 0.359→0.384 ms、P95 0.767→0.778 ms、P99 0.896→0.869 ms、CPU 平均 0.893%→0.972%、Working Set 平均 9.25→9.24 MiB；未观察到显著性能回退。文档早期 4.98 MiB / 0.026% 等绝对值属于不同机器/工具链/测量口径，不作横向回归结论。
- 2026-11-26 控制锁重构实跑（真实 OP.GG ARAM 请求进行中并发 32 个 health）：HEAD 基线 P50/P95/最大约 2395/2395/2395 ms，快照版本约 0.958/0.983/0.993 ms；消除长控制操作对读请求的全局锁阻塞。
- 2026-11-26 英雄别名刷新并发 A/B（真实 gtimg 请求进行中并发 32 个 health）：HEAD 基线约 198.9/199.0/199.0 ms，快照版本约 0.818/0.892/0.909 ms；Catalog/SGP 无本机 LCU，未做真实网络验收。
- 2026-11-26 写守卫保留策略回归：过期 Completed 清理、未知/旧版记录保留、512 满容量自动释放，Rust 新增 2 项测试；本地 HTTPS 模拟 LCU 的真实 release sidecar 写安全回归 8/8 通过。
- 2026-11-26 本机整包复验：PyInstaller 6.22.3 + MSVC release sidecar/updater，`package_smoke` 校验 490 文件 manifest、版本 1.3.0、包内 sidecar 握手，并启动真实 Windows UI 到达主界面（`Seraphine initialized`）。本机缺少 7z CLI，7z 归档步骤未执行，不影响已生成 `dist/Seraphine` 的整包验证。
- 2026-11-26 修复 OP.GG 启动期 `ByteBudget`：图标下载可短暂占满全局 2 MiB 预算，OP.GG 原先立即拒绝；资源调度器新增异步 `acquire`，容量释放后唤醒等待者，OP.GG 请求改用该路径。Rust 工作区 release 76/76、Clippy `-D warnings`、Python 契约 93/94（1 项真实 LCU 条件跳过）通过；最新整包 `package_smoke` 校验 548 文件 manifest、版本 1.3.0、sidecar 握手和 UI 主界面。真实客户端启动日志不再出现 `ByteBudget`，OP.GG 16.23 榜单及英雄构建、装备、符文详情均加载成功。
- 2026-11-26 写路径快照化回归：本地 HTTPS 模拟 LCU 的 9 项写安全/并发测试通过；同一慢写场景 HEAD 基线 health 在 0.5 秒超时内无法返回，当前版本正常返回，消除写操作对读请求的全局锁阻塞。
- 上一版 MSVC 包 SHA256（本轮代码改动后需重新打包）：`0d12207abc30f2eb115e37eaf7c6cd0c98cd3e3309e01752febd0287f2c6dde5`。
- 2026-11-25 `cargo audit` 扫描 155 个 Rust 依赖无已知漏洞；Python 3.13.17 环境下 `pip-audit` 无已知漏洞，依赖版本固定在 `requirements.txt`。
- MSVC 编译与整包结果不替代真实游戏、完整 sidecar UI 灰度和政策验收。

上述数据不等于最终发布验收。

---

## 10. 运行、构建与回滚

### 10.1 后端选择

```text
SERAPHINE_BACKEND_MODE=python   默认，Python 后端
SERAPHINE_BACKEND_MODE=shadow   Python 主用，Rust 影子对比
SERAPHINE_BACKEND_MODE=sidecar  Rust sidecar 主用，灰度使用
SERAPHINE_ARAM_DATA_URL=<https://…/aram-manifest.json>   覆盖 ARAM 数据 feed 清单（默认空 = 只用内置快照）
SERAPHINE_ARAM_AUTO_UPDATE=0|1                           覆盖“自动更新 ARAM 数据”设置
```

### 10.2 真实写入

发行版直接启用，无需环境变量或产品开关。写操作仍受 Allowlist、阶段校验、读回确认与守卫记录约束。

### 10.3 Rust 构建

在 Visual Studio 2026 Build Tools 的 x64 Native Tools 命令提示符中，或已加载对应 `vcvars64.bat` 的终端内：

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

### 10.5 整包验证

生成最终包并解压到独立目录后，在仓库根目录执行：

```powershell
python -m tests.bench.package_smoke .\dist\Seraphine
```

该命令会校验 manifest、版本、包内 sidecar 握手，并启动真实 Windows UI 等待 `Seraphine initialized` 标记；卡首屏或启动异常会判定失败。无桌面环境时可用 `--skip-ui`，但这不能替代实机 UI 验收。

### 10.6 回滚

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
- `config.json`、日志、`shadow_diff.jsonl`、运行时 Catalog/ARAM/Alias JSON、`write-guards.json`/`write-guards.lock`。
- `.env`、私钥、证书、PFX/P12。
- 编译生成的 exe/dll/pyd/7z/zip。
- 本地用户名、绝对路径、PUUID、召唤师 ID 和真实 token。
- 已移除 ARAM appsecret 机制（jddld 上游停用，改为 Wiki 快照管线，见 §8 P1-1）。

---

## 12. 下一步建议顺序

1. 完成 sidecar 模式完整 UI 灰度。
2. 完成 Intel 250KF PresentMon/WPR 真实游戏 A/B/C。
3. 观察 Wiki 快照 CI/Gitee 数据更新与国服客户端版本对齐。
4. 完成政策审批后再评估真实写入。
5. 全部硬门槛通过后观察 7 天，再切换默认后端。
6. 最后清理旧 Python 网络/解析实现。

---

## 13. AI 接手操作清单

### 13.1 必读顺序

1. 本开发文档。
2. `app/backend/protocol.py` 与 `rust/crates/protocol/src/lib.rs`。
3. `app/backend/sidecar_client.py` 与 `rust/sidecar/src/main.rs`。
4. `app/backend/compat.py`、`app/backend/catalog.py`。
5. `rust/crates/lcu/src/websocket.rs`、`rust/crates/lcu/src/cache.rs`。
6. `tests/contract/` 与 `tests/bench/`。
7. 本文 §14 IPC 契约、§15 验收标准、§16 阶段 8 清单（契约以本章为准，代码为最终事实）。

### 13.2 开发环境

- Windows 10/11 x64。
- Python 3.13（当前实机 3.13.17）；运行依赖固定在 `requirements.txt`，打包使用 PyInstaller 6.22.3。
- Rust stable x86_64-pc-windows-msvc。
- Visual Studio 2026 Build Tools，包含 C++ Build Tools core、最新 MSVC x64/x86 与当前 Windows 11 SDK；无需安装 IDE、CMake 或 vcpkg 推荐项。
- 不需要安装 WinUI 3 才能完成当前 sidecar 路线。
- spike 目录只用于历史技术验证，不应作为主程序依赖。

### 13.3 第一次验证

在仓库根目录执行：

```powershell
cargo fmt --manifest-path .\rust\Cargo.toml --all -- --check
cargo clippy --manifest-path .\rust\Cargo.toml --workspace --all-targets -- -D warnings
cargo test --release --manifest-path .\rust\Cargo.toml --workspace
cargo build --release --manifest-path .\rust\Cargo.toml -p seraphine-sidecar --bins
$env:QT_QPA_PLATFORM = "offscreen"
python -m unittest discover -s tests/contract -v
```

预期：Rust release 75 项通过（其中 2 个内存预算断言按 release 口径设定，默认 debug 构建下这两项预计失败，属预期行为）；Python 92 项通过（含新增 ARAM 数据 13 项、重试契约 4 项、sidecar 路由 1 项与帧时间分析 3 项）、1 项真实 LCU 测试在未配置环境变量时跳过。

### 13.4 修改约束

- IPC 字段变更必须同步修改 Python 与 Rust 两侧，并更新本文 §14 契约表。
- 新增外部端点必须加入主机白名单、超时和字节预算。
- 新增写操作必须默认关闭，并具备写前验证、写后确认和 `OutcomeUnknown` 处理。
- 新增 UI 异步任务必须处理取消、世代号和慢消费者。
- 不得把真实 token、PUUID、本地路径或运行时响应写入 fixture、日志或文档。
- 每完成一个阶段，更新本文档的“已完成”“未完成”和“已验证数据”。

### 13.5 当前最优先工作

1. 修复并验证 PyInstaller 整包。
2. 完成 sidecar 模式完整 UI 灰度。
3. 完成真实游戏 PresentMon/WPR 验收和外部 TLS 内存优化。

在这些工作完成前，不要把默认后端从 `python` 切换为 `sidecar`，不要启用真实 LCU 写入。

---

## 14. IPC 契约（操作、错误码与版本演进）

> 本章是 Python 与 Rust 两侧共同遵守的契约源。实现事实以 `rust/sidecar/src/main.rs`（控制请求 `handle_request`、并发只读分支）与 `app/backend/sidecar_client.py` 为准；任何字段变更必须同 PR 更新两侧代码、`tests/contract/test_protocol.py` 与本章。

### 14.1 消息类型与路由

- 消息类型：`hello | request | response | event | cancel | ping | pong | shutdown`；请求必带 `id/epoch/op/args/deadline_ms`，响应原样带回 `id/epoch`，`cancel` 按 `id` 取消。
- 限额（两侧常量一致）：单帧 1 MiB、请求 256 KiB、响应/事件各 512 KiB、全局在途 4 MiB、pending 64、事件队列 128 条且 1 MiB。
- 并发只读槽（8 个）：`health`、`process.list`、`process.command_line`、`lcu.summoner_by_pid`、`lcu.get`、`external.get`、`aram_buff.get`、`champion_alias.search`、`echo`。
- 单控制槽（1 个，串行）：其余所有操作（`lcu.connect`、`lcu.catalog`、`lcu.write`、`sgp.get`、`asset.get`、`automation.*`、`opgg.*`、`*.refresh`、`diagnostics` 等）。写操作与自动化因此天然串行。
- 事件：`event.topic` 集合为 `gameflow.changed`、`champselect.changed`（`data` 为 `{uri,eventType,data}` 信封，与其余 topic 直接携带原始 data 不同）、`summoner.changed`、`entitlements.changed`、`lcu.changed`（兜底）；自动化 dry-run 额外产生合成事件 `automation.planned`（`{plans}`）。Python `compat.py` 目前只把前三种转发到 `signalBus`；`entitlements.changed` 还会触发内部 SGP token 刷新。
- `stdout` 只承载协议帧，业务代码禁止写入；诊断文本只写 `stderr`（Python 侧保留最后 64 行环形缓冲）。

### 14.2 操作表（22 项能力 + `echo`）

| op | 参数 | 成功响应 `data` | 主要错误码 | 调度 |
|---|---|---|---|---|
| `health` | `{}` | `{pid,epoch,protocol,uptimeMs,memory,connection}` | – | 无预算 |
| `process.list` | `{}` | `{processes:[{pid,name}],leagueClientPids}` | `PlatformError`(retryable) | 无预算 |
| `process.command_line` | `{pid}` | `{pid,port,server,hasToken}`，**永不返回 token** | `InvalidParams`,`PlatformError` | 无预算 |
| `lcu.summoner_by_pid` | `{pid}` | 召唤师 JSON | `InvalidParams`,`PlatformError`,`ConnectionError`,`LcuError`,`ResourceDeferred` | P2 / 64 KiB |
| `scheduler.set_game_state` | `{state}` | 调度快照 | `InvalidParams` | 无预算 |
| `lcu.connect` | `{pid,proxy?,cacheRoot?}` | `{pid,port,server,epoch}` | `InvalidParams`,`PlatformError`,`ConnectionError` | 控制槽 |
| `lcu.get` | `{path}` | LCU JSON | `InvalidParams`,`NotConnected`,`LcuError`,`ResourceDeferred` | P1 / 64 KiB |
| `lcu.write` | `{method,path,operationId,data?}` | 写后读回确认结果 | `InvalidParams`,`NotConnected`,`InvalidState`,`ResourceDeferred`,`OutcomeUnknown`,`PersistenceError` | P0 / 256 KiB |
| `lcu.catalog` | `{force?}` | `{path,bytes,epoch}`（快照写 `%LOCALAPPDATA%/Seraphine/catalog.json`） | `NotConnected`,`LcuError`,`SerializeError`,`IoError`,`ResourceDeferred` | P4（force→P1）/ 512 KiB×8 |
| `sgp.get` | `{path}` | SGP JSON | `InvalidParams`,`NotConnected`,`SgpError`,`ConnectionError`,`ResourceDeferred` | P2 / 128 KiB |
| `asset.get` | `{folder,name,path\|url,maxBytes?,force?}` | `{path}`（仅本地路径） | `InvalidParams`,`AssetDeferred`,`AssetError`,`NotConnected`,`IoError` | P3（force→P2） |
| `external.get` | `{url}` | JSON | `InvalidParams`（含主机非白名单）,`ConnectionError`,`ExternalError`(retryable),`ResourceDeferred` | P2 / 512 KiB |
| `automation.configure` | `{sessionId,config,queueId?}` | `{configured:true}` | `InvalidParams`（含 `stale automation epoch`） | 控制槽 |
| `automation.dry_run` | champ-select 会话事件 | `{plans,snapshot}` | `InvalidParams` | 控制槽 |
| `opgg.positions` | `{region,mode,tier,position,championId?}` | 位置名数组 | `InvalidParams`,`ConnectionError`,`ExternalError`,`ResourceDeferred` | P2 / 2 MiB |
| `opgg.tier_list` | `{region,mode,tier,position}` | `{data,version}`（紧凑占位符格式） | 同上 | P2 / 2 MiB |
| `opgg.champion_build` | `{region,mode,tier,position,championId?}` | `{data,version,mode}`（含图标落地） | 同上 | P2 / 2 MiB |
| `champion_alias.refresh` | `{force?,gameVersion?}` | `{available,updated,data,warning}` | `ResourceDeferred`,`ExternalError`；上游失败降级为旧缓存 + `warning` | P3 / 1 MiB |
| `aram_buff.refresh` | `{force?,gameVersion?}`（兼容保留，不再联网） | `{available,updated:false,data,warning}`，读本地缓存/内置快照 | 未安装时 `available:false` + warning | 控制槽 / 无预算 |
| `champion_alias.search` | `{query}` | `[championId,…]`（升序） | `NotFound` | 只读槽 |
| `aram_buff.get` | `{championId}` | `{available,version,info}` | `InvalidParams` | 只读槽 |
| `diagnostics` | `{}` | metrics/缓存/调度/写台账/WS/内存/能力集 | – | 控制槽 |
| `echo` | 任意 | 原样返回 | – | 只读槽 |

约定与副作用：

- `lcu.websocket` 是能力标记而非请求入口：WS 在 `lcu.connect` 成功后自动建立（gameflow / champ-select / summoner / entitlement 四订阅），客户端只消费 `event` 帧。
- `lcu.connect` 副作用：读取 PEB 凭据并尝试拉取 entitlement token → `epoch` 递增（wrapping+1）→ 清空写台账 → 重置 SGP/External/Asset 客户端（下次使用时懒加载重建）→ 重启 WS → 恢复 catalog 索引 → 调度器置为 `ClientStarting`。旧 epoch 的响应/事件/写台账随之失效。
- 路径类参数必须以 `/` 开头且不含 `://`；`lcu.write` 额外禁止 `/lol-champ-select/v1/session`、`.`/`..` 段、`%2e`/`%2f`/`%5c`、`?`、`#`、CR/LF，长度 ≤2048，body ≤256 KiB，`operationId` 1–128 字符。
- `asset.get` 的 `folder/name` 经 `safe_cache_path` 规范化，不得逃逸缓存根；负缓存 30 秒。
- `external.get` 主机白名单为 `op.gg` 与 `communitydragon` 及其子域；仅 `127.0.0.1` 的 LCU 关闭证书校验。
- **ARAM 数据链路（2026-11-26 起）**：`aram_buff.refresh` 不再联网，只读 `%APPDATA%/Seraphine/AramBuff.json`（Python 侧会用内置快照播种缓存）；数据由 CI 从 Wiki 生成（§8 P1-1），客户端经 `app/backend/aram_data.py` 按"24 小时或游戏版本不一致"节流检查可配置 feed（默认空 = 仅内置快照；配置 Gitee Release 清单后启用热更新），校验 SHA256/schema 后原子替换；jddld 与 appsecret 已移除。
- 社区数据缓存位于 `%APPDATA%/Seraphine/ChampionAlias.json` 与 `AramBuff.json`；写守卫位于 `%APPDATA%/Seraphine/write-guards.json`。
- 写台账/持久化守卫上限 512 条，达到后新写返回 `ResourceDeferred`（`write ledger is full`），不自动淘汰（见 §8 P2-4）。

### 14.3 错误码分类

| 类别 | 错误码 | retryable / 处置语义 |
|---|---|---|
| 参数与协议 | `InvalidParams`、`ProtocolError`、`MethodNotFound` | 修正后重试；`duplicate request id` 也以 `InvalidParams` 返回 |
| 平台 | `PlatformError` | 多数 retryable=true |
| 连接 | `ConnectionError`、`NotConnected` | 等待客户端/网络恢复后重试 |
| 上游 | `LcuError`、`SgpError`、`ExternalError`、`AssetError` | retryable=true，受退避与负缓存约束 |
| 资源 | `ResourceDeferred`（调度拒绝、在途上限、并发超限、台账满） | retryable=true，退避后重试 |
| 缓存 | `AssetDeferred`（30 s 负缓存命中） | TTL 到期后重试 |
| 写安全 | `InvalidState`（守卫拒绝）、`OutcomeUnknown`（写后无法确认，**禁止重放**）、`PersistenceError`（守卫持久化失败） | `OutcomeUnknown` 必须先读回确认或人工处置 |
| 生命周期 | `DeadlineExceeded`（未开始执行可安全重试）、`Canceled`（已开始执行的写会转为 `OutcomeUnknown`） | 见 §3.1 |
| 序列化与 IO | `SerializeError`、`IoError` | 修复环境后重试 |
| 数据缺失 | `NotFound`（英雄别名未就绪） | 先调用 `champion_alias.refresh` |

### 14.4 协议版本演进

- `PROTOCOL_VERSION = 1`；`hello.version` 不匹配时 Python 拒绝启动并提示更新，**不静默降级**。
- 兼容变更（不 bump 版本）：新增可选 `args` 字段、新增 `op`、新增能力位。
- 破坏性变更（必须 bump）：删除/改名 `op`、改变必填字段或语义、改变 epoch/seq/deadline 语义、改变限额。
- bump 流程：同一 PR 修改 `PROTOCOL_VERSION`、两侧代码、`tests/contract/test_protocol.py` 与本章 §14.2/§14.3；发布包内 Python 与 sidecar 版本锁定，不允许新旧混跑。
- 能力协商：Python 只能调用 `hello.capabilities` 中存在的 op；`lcu.write` 缺失时自动 B/P 真实写入口必须禁用（当前默认即此状态）。

### 14.5 调度优先级与配额（§3.5 的操作级映射）

| 优先级 | 承载操作 | 并发上限 | 游戏态规则 |
|---|---|---|---|
| P0 | `lcu.write` | 8 | 始终允许 |
| P1 | `lcu.get`、写前读回、写后确认、`lcu.catalog`(force) | 8 | 始终允许 |
| P2 | `lcu.summoner_by_pid`、`sgp.get`、`external.get`、OP.GG、`asset.get`(force) | 8（InGame 限 2、ChampSelect 限 4） | 始终允许 |
| P3 | `asset.get`（常规）、社区数据刷新、catalog 常规 | 2 | InGame / ChampSelect 禁止；Lobby / Reconnect / 空闲态允许 |
| P4 | 后台预取（catalog 非 force） | 1 | 仅 NotRunning / ClientStarting / EndOfGame |

全局上限：32 个活跃任务、2 MiB 估算在途字节。调度器只做准入与延迟，不做硬 CPU 限额、固定亲和性或强制工作集回收（§5 铁律 8）。

---

## 15. 验收标准

> 本章把 §8 的"硬门槛"转换为可执行判定。未全部满足前：默认后端保持 `python`，`lcu.write` 保持关闭。阈值中标注"建议默认"的数值在设备基线确定后由项目 owner 书面固化。

### 15.1 真实游戏帧时间验收（P0 门槛）

- 工具：`tests/bench/frame_time_ab.py`（默认 `--warmup 60 --duration 300 --rounds 3`；PresentMon CSV `MsBetweenPresents`/`CPUStartQPCTime` + 20 req/s `health` 压测；500 ms 采样 sidecar CPU/RSS；帧与请求按 QPC 最近邻关联）。传入 `--baseline` 后按下方阈值判定，不达标退出码为 2。
- 场景（同一台 Intel 250KF、同一自定义/训练模式对局）：
  - A：`python` 后端，不运行 sidecar（基线）；
  - B：`python` 后端 + sidecar 20 req/s 空跑（旁路开销）；
  - C：`sidecar` 后端 + 20 req/s（目标形态）。
- 采样：每场景 ≥300 s，前 60 s 热身不计入；A/B/C 各至少 3 轮。
- 判定（建议默认）：
  1. C 相对 A：`frameTimeP99Ms` 回退 ≤5%，`stallsOver50Ms` 增量 ≤2 次/分钟；
  2. B 相对 A：`frameTimeP99Ms` 回退 ≤2%；
  3. sidecar 进程 Working Set ≤10 MiB；20 req/s 下平均 CPU ≤0.5%（历史实测 P99 延迟约 0.35 ms、CPU 约 0.026%，余量充足）。
- 证据归档：`benchmark-output` 下的 JSON（`frames`、`sidecar`、`correlation` 三段）随发布记录保留。

### 15.2 sidecar UI 灰度矩阵（P0 门槛）

每个格子记录：功能可用；数据与 python 后端 golden 一致（白名单外 0 差异）；无未处理异常日志；无 >200 ms 的 UI 冻结。

| 功能 \ 状态 | NotRunning | Lobby | ChampSelect | InGame | EndOfGame | Reconnect |
|---|---|---|---|---|---|---|
| 启动/首屏/托盘 | 必测 | – | – | – | – | – |
| Career（20 局+历史） | 必测 | 必测 | 必测 | 必测 | 必测 | 必测 |
| Search（首屏 20+翻页+筛选预取） | – | 必测 | 必测 | 必测 | 必测 | – |
| GameInfo 友敌分区 | – | – | 必测 | 必测 | – | – |
| OP.GG（梯度/出装/一键符文） | 必测 | 必测 | 必测 | 必测 | – | – |
| ARAM Buff | – | – | 必测 | 必测 | – | – |
| 设置/辅助开关 | 必测 | – | – | – | – | – |
| 自动接受/选禁/换人（dry-run） | – | 必测 | 必测 | – | – | – |
| TFT 对局历史 | 必测 | 必测 | – | – | 必测 | – |
| 多客户端切换 | 按 §16.2 执行 | | | | | |
| 客户端修复（重连/结算加载） | – | – | – | – | 必测 | 必测 |

- 灰度推进：内部白名单 → 群内小范围（≥20 人、≥7 天）→ 默认切换；期间 `BackendMode` 必须可随时切回 `python`，出现 P0/P1 问题立即回退并冻结晋级。
- 本机整包灰度启动（PowerShell，仓库根目录）：
  ```powershell
  $env:SERAPHINE_BACKEND_MODE = "sidecar"
  .\dist\Seraphine\Seraphine.exe
  ```
  日志位于 `%APPDATA%\Seraphine\log`；回滚时关闭程序，将 `SERAPHINE_BACKEND_MODE` 改为 `python` 或删除该环境变量后重启。
- 切换默认后再观察 7 天无 P0/P1，方可进入阶段 8。

### 15.3 shadow 对照验收

- `shadow` 模式生成 `shadow_diff.jsonl`，对比两侧读结果：对局详情/生涯/搜索/OP.GG 字段一致率 100%，易变字段白名单（如进行中对局的 `time_eliminated`、计时器类字段）除外。
- diff 文件不进发布包、不入库（§11）。

### 15.4 打包与更新验收（每次发布）

- `python -m tests.bench.package_smoke .\dist\Seraphine`：manifest 路径/大小/SHA256、包内 `--version`、sidecar 握手、真实 Windows UI `Seraphine initialized` 标记；`--skip-ui` 不替代实机 UI 验收。
- updater 回归：正常更新、损坏源拒绝、`UpdateBackup` 人工恢复、自更新 finalizer（对应 `tests/contract/test_update.py`、`test_updater_self_update.py`）。

### 15.5 真实写操作政策记录

2026-11-27 项目 owner 决定：发行版直接启用真实写操作，不设环境变量或产品开关；写操作仍受 Allowlist、阶段校验、读回确认与守卫记录约束。以下事项作为后续审计与告知记录维护，不再作为发布阻塞：

- [ ] 功能范围与 Riot / 腾讯政策逐条对照结论（自动接受、自动选禁、自动换人分别评估）
- [ ] 用户风险告知与免责声明更新（README"套盾"同步）
- [ ] 写操作范围告知与用户文案
- [ ] 事故预案（误 ban/pick、`OutcomeUnknown` 处置、回滚开关与台账审计路径）
- [ ] 灰度人群与观察期（≥7 天，日志脱敏审查）
- [ ] 技术验收：§15.2 的 BP 全流程全绿 + 写安全回归全通过

### 15.6 内存/CPU 测量口径

- 对象：`seraphine-sidecar.exe` 进程（不是测试进程）；构建：MSVC release（`lto=fat`、`panic=abort`、`strip`）。
- 口径：10 分钟空闲 + 60 秒 20 req/s；记录 Working Set / Private / CPU 平均值；目标 Working Set ≤10 MiB、空闲 CPU ≤0.1%。
- 注：`cargo test` 默认 debug 构建下有 2 个内存预算断言会失败（阈值按 release 口径设定）；完整 75 项须用 `cargo test --release` 运行。

---

## 16. 阶段 8 清理清单与多客户端行为

### 16.1 阶段 8 删除清单（默认切换并观察 7 天后执行）

删除前先确认目标模块在 `python` 回退路径上不再被引用；`app/backend/compat.py`、`catalog.py`、`image_pool.py`、`supervisor.py`、`sidecar_client.py` 保留。

| 模块/文件 | 处理 | 前置条件 |
|---|---|---|
| `app/lol/connector.py` 中 LCU/SGP/OP.GG/ARAM HTTP 实现 | 删除（保留 compat 依赖的方法外壳） | sidecar 默认 7 天无回退 |
| `app/lol/` 内旧对局/生涯/搜索/TFT 解析器 | 删除 | Golden Diff 连续通过 |
| `app/common/util.py` 中旧凭据读取（psutil 命令行方案） | 删除（Toolhelp 枚举保留） | `lcu.connect` PEB 读取稳定 |
| `app/backend/shadow.py` 两侧 diff 采样 | 保留开关，默认关闭 | 观察期结束 |
| `app/resource/game/` 运行时图片缓存 | 打包排除已生效，无需迁移删除 | §11 已覆盖 |

### 16.2 多客户端行为规范

1. Python `LolProcessExistenceListener` 每 1.5 s 枚举 `LeagueClientUx.exe`；当前连接 PID 消失时切换到 `pids[0]` 并触发 `lolClientChanged`。
2. 切换时 Python 调用 `lcu.connect`；Rust 提升 epoch（wrapping+1），旧 epoch 的响应、事件、写台账全部失效；`automation.configure` 必须在新 epoch 下重新配置，否则返回 `stale automation epoch`。
3. BP（ChampSelect）进行中禁止切换后端（§5 铁律 7）；因多客户端切换导致的会话丢失，通过"事件重放 + 快照重建"恢复，不得沿用旧结果。
4. 验收判定：双开客户端时 Career/Search 归属当前 PID；切换后 catalog 重新加载；旧请求结果不得覆盖新目标（铁律 11）。
