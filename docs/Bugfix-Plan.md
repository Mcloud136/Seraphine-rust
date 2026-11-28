# Seraphine Rust 缺陷修复设计

> 状态：设计稿（未改动源码）
> 基线：`main` / Rust-only，2.0.2 发布后
> 依据：bug-review 报告与对应源码、测试、UI 实机检查

## 0. 目标与约束

目标：在不回退现有功能、不重新引入隐藏开关、不改变更新签名与配置兼容性的前提下，修复已确认的逻辑、效率、功能接线和 UI 缺陷。

硬约束：

- 不重新引入 `SERAPHINE_ENABLE_LCU_WRITES` 或任何隐藏写开关。
- 不把产品功能实现改成 PowerShell/external script；需要系统能力时使用 Rust/Win32/Tauri API。
- 配置 schema、Gitee/GitHub 更新地址、minisign 签名、安装目录升级路径必须保持可迁移。
- 每个修复必须有可重复验证；涉及写操作/自动化的修复必须先补测试。
- UI 继续遵守 368px 卡片、国服术语、全中文、友好错误提示。

## 1. 缺陷分批

| 批次 | 内容 | 缺陷 |
| --- | --- | --- |
| A | 自动化与 LCU 协议正确性 | D1、D2、D3、D4、自动状态重置、ban 延迟阻塞 |
| B | 写守卫、配置健壮性、缓存 | 写守卫 pending/落盘、config 损坏、Notify 丢唤醒 |
| C | 性能 | 战绩页 5 秒串行拉取、缓存、前端 bundle |
| D | 前端状态与 UI | 对局详情竞态、页面状态、断线残留、保存按钮、主题文案、窗口尺寸、数字输入、侧栏文案 |
| E | 发布与安装器 | Inno 版本、安装目录、ARAM 数据回退、Gitee 原子发布、README、许可证 |
| F | 设置功能接线 | 主题色/字体/DPI/OP.GG 默认值、自动重连、保留对局信息、显示段位、OP.GG 窗口、随机皮肤 |

## 2. A 批次：自动化与 LCU

### A1. “选人超时自动锁定”立即锁定（D1）

**根因**

`auto_complete` 在找到进行中的 pick 后立即写 `completed:true`，没有读取 `timer.adjustedTimeLeftInPhase`。它被 1 秒轮询/WS 事件重复调用，所以实际上是“hover 后马上锁定”。

**设计**

- 在 `auto_complete` 前增加纯函数 `completion_ready(session) -> bool`：
  - 读取 `timer.adjustedTimeLeftInPhase`（毫秒）与 `timer.internalNowInEpochMs`；
  - `isInfinite=true` 或字段缺失时不锁定；
  - 计算本地 deadline；即使 ARAM/FINALIZATION 末尾 timer 短暂冻结，也按本地时间继续判断；
  - 只有剩余 ≤ `LOCK_LEAD_MS`（建议 4000）才允许锁定。
- 不在锁内 sleep；依靠已有 1 秒轮询在临近结束时触发。
- 写入前重新获取一份 fresh champ-select session，重新确认本地 pick action、`isInProgress`、`completed`、当前 championId；不要直接复用首次传入的旧快照。
- 汇总双方已完成 pick 与 bans；当前英雄已被选/ban 时，按配置名单选择下一个可用候选；无候选则不锁。
- 写入仍走 `Writes::execute` 的阶段校验/读回确认，防止动作已经完成时误写。

```rust
const LOCK_LEAD_MS: i64 = 4_000;

fn completion_ready(session: &Value, now_ms: i64) -> bool {
    let Some(timer) = session.get("timer") else {
        return false;
    };
    if timer.get("isInfinite").and_then(Value::as_bool) == Some(true) {
        return false;
    }
    let (Some(left), Some(internal_now)) = (
        timer.get("adjustedTimeLeftInPhase").and_then(Value::as_i64),
        timer.get("internalNowInEpochMs").and_then(Value::as_i64),
    ) else {
        return false;
    };
    let deadline = internal_now.saturating_add(left);
    deadline.saturating_sub(now_ms) <= LOCK_LEAD_MS
}
```

**测试**

- `time_left=10000` → 不写；
- `time_left=3999` → 写；
- 无 timer → 不写；
- action 已完成 → `find_in_progress_pick` 为空。

**风险/回退**

- 轮询间隔 1 秒，锁定可能比 Python 版晚约 0–1 秒；如果用户要求严格 4 秒，可把 lead 调为 4500。
- 不影响手动点“锁定”。

### A2. WebSocket Ping/Pong 触发重连（D2）

**根因**

稳态循环只接受 `Message::Text`，其余帧直接 `break`；Ping/Pong/Binary 被当作断开，触发重连、订阅和快照，快照里的 ChampSelect 又会重置自动化状态。

**设计**

- 只把 `Text` 作为事件；`Close`/`None`/`Err` 结束；其余帧 `continue`。
- tungstenite 会自动回 Pong（已核对实现），因此不必在业务循环里手动重复发送；`Ping`/`Pong` 只忽略即可。
- 将帧分发抽成 `handle_message` 纯函数/小函数，便于测试。

```rust
match message {
    Some(Ok(Message::Text(text))) => { /* parse + emit */ }
    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
    _ => continue,
}
```

**测试**

- 本地 WS 服务依次发送 Ping、Text；断言 `reconnects` 不增加、Text 事件到达。
- 发送 Close 或断开 TCP；断言进入一次重连。

**风险**

- 忽略 Binary 是安全的：LCU 事件为 JSON Text；若未来有二进制协议，另行实现。

### A3. 交易/换位开关无效（D3）

**根因**

`auto_trades` 无条件处理 `trades` 和 `pickOrderSwaps`，没有读取 `auto_accept_champ_trade` / `auto_accept_ceil_swap`。

**设计**

- 将 `FunctionConfig` 传入 `auto_trades`。
- 增加纯函数 `trade_enabled(config, collection)`，映射：
  - `trades` → `auto_accept_champ_trade`
  - `pickOrderSwaps` → `auto_accept_ceil_swap`
- 默认值仍为 false（与配置/UI 一致）。

**测试**

- 两个开关全 false：不发出任何 accept；
- 仅 trade true：只 accept `trades`；
- 仅 swap true：只 accept `pickOrderSwaps`。

**风险**

- 行为从“总是接受”改为“按 UI”，这是修复而非回退；需在更新说明中标注。

### A4. OutcomeUnknown 仍重试（D4）

**根因**

`terminal()` 只匹配 `OperationConflict`。写守卫在确认失败时返回 `OutcomeUnknown`，自动化下一轮用带 `attempt` 的新 operation id 重试，绕过守卫。

**设计**

```rust
fn terminal(error: &CoreError) -> bool {
    matches!(
        error,
        CoreError::OperationConflict(_) | CoreError::OutcomeUnknown(_)
    )
}
```

- 保留 `NotConnected`/`Lcu` 等可重试错误。
- 增加单测；自动化日志明确“outcome unknown, stop”。

**风险**

- 结果未知时不再自动重试，可能需要用户手动处理；这是安全语义要求。

### A5. 自动状态在丢失 ChampSelect 事件后无法重置

**根因**

`begin` 在 gameflow `ChampSelect` 事件里重置 `AutomationState`；如果该事件丢失，1 秒轮询调用 `on_champ_select` 时只会把 `active` 设为 true，旧的 done 标志/token 仍在，导致本局自动化静默跳过。

**设计（推荐）**

- 在 `on_champ_select` 中，如果 `!automation.active`，重置整个 `AutomationState`（token、done、attempt），再置 active。
- 增强：给 `AutomationState` 增加 `draft_id`。优先从 `/lol-gameflow/v1/session` 的 `gameId` 获取；不要假设 champ-select session 一定含 gameId。没有稳定 id 时，用 epoch + phase 代次/会话指纹，并在进入 ChampSelect 时重置。
- `begin` 仍保留为事件快路径；轮询作为兜底。

**测试**

- 模拟先调用 `reset` 再直接 `on_champ_select`（不调用 begin），断言 ban/pick 会执行而不是被 done 挡住。
- 模拟 gameId 变化，断言 token 更新。

**风险**

- 同一局内多次 session 更新不能误重置；draft_id 必须稳定。

### A6. ban 延迟阻塞 pick/complete（D5）

**根因**

`auto_ban` 在 `on_champ_select` 的全局 `automation_lock` 内 `sleep(auto_ban_delay)`，后续 pick/complete/trade 全部等待；失败重试时还会重复 sleep。

**设计（推荐）**

- 用“到期时间”代替 sleep：`AutomationState` 增加 `ban_ready_at: Option<Instant>`。
- 第一次计算延迟后记录到期时间并返回；后续轮询在 `now >= ready_at` 时才执行 ban。
- 不在锁内 sleep，pick/complete 不被阻塞。
- 每局重置 `ban_ready_at`。

**备选**

- 把 ban 拆成独立 spawned task；需要更细的状态同步，风险更高，不作为首选。

**测试**

- delay=5s，轮询第 1 秒不写，5 秒后写；期间 spells/pick 不被阻塞。
- 写入失败不会重新计时（除非显式重置）。

## 3. B 批次：写守卫、配置、缓存

### B1. 写守卫缺少 pending 占位（D20）

**根因**

`execute` 只在成功后 `store()`，两个相同 operation id 的并发请求都能通过检查；`store()` 的磁盘错误被 `let _ =` 丢弃。

**设计**

- 引入 `GuardState { Pending, Completed, Unknown }`（或 `in_flight` + `completed`）。
- 检查阶段：
  - 已有 Completed → 返回 replay；
  - 已有 Pending → 返回 `Deferred("write is in progress")`（调用方可稍后重试，不执行并发写）；
  - 已有 Unknown → 返回 `OutcomeUnknown`；
  - hash 不同 → `OperationConflict`。
- 检查通过后先插入 Pending 并持久化；持久化失败则移除内存记录并返回 `Io`，**不执行 HTTP**。
- HTTP + 确认后更新为 Completed/Unknown 并持久化。
- 非 guarded 写入保持现有语义：HTTP 明确失败时清理 Pending 并允许后续重试；guarded 写入确认失败必须落 Unknown，不能清理。
- 持久化失败：保留内存状态；若落盘的是 Pending，重启后仍会阻止重试（安全）；日志告警。
- `store()` 返回 `Result`，调用方记录；不要静默吞错。

**测试**

- 两个并发相同 id：一个执行、一个 `Deferred`；
- 模拟磁盘不可写：不执行 HTTP，返回 Io；
- 确认失败落盘为 Unknown，重启加载后返回 OutcomeUnknown。

### B2. 配置损坏静默清空（D19）

**设计**

- `schema_version` 加 `#[serde(default)]`；其他结构已有 default。
- 解析失败时：
  1. 将原文件重命名为 `config.json.corrupt-<unix>`；
  2. `log::warn!` 记录路径；
  3. 再走 legacy/default 流程，并 `save_current()`。
- 如果备份重命名失败：不要覆盖原文件；使用内存默认值并在 UI/日志提示“配置损坏且无法备份”，等待用户处理。
- 读取旧配置时同样记录错误，不静默。
- feed 迁移需同时识别旧的 GitHub release URL 与 Gitee release URL，统一迁到新的 raw URL；用户自定义 URL 保持不变。updater 端点变更由新版本编译生效，旧版本端点继续双写。

**测试**

- 非法 JSON、字段类型错误（`logLevel: "oops"`）：生成备份，加载默认值，原文件不丢；
- 正常配置不受影响。

### B3. 缓存 Notify 丢唤醒（D22）

**设计**

- 将 `Notify` 换成带版本号的 `watch` 或每 key `Mutex`：
  - `Start::Wait` 返回版本号 + receiver；
  - 进入等待前先比较版本；leader 完成后版本递增；
  - 等待 `changed()` 或被超时唤醒；超时后重新检查缓存。
- 必须处理 leader 失败/任务取消：无论成功、失败还是 Drop，都要清理 inflight 并唤醒等待者；否则一个被取消的 leader 会永久挂住所有同 key 请求。
- 或使用 `tokio::sync::Mutex` per key，leader 持锁，等待者获取锁后直接读缓存；实现更直观，但每个 key 多一个 Mutex。
- 禁止只用 `notify_waiters()` 作为唯一同步。

**测试**

- leader 在 waiter 注册前完成：waiter 仍能拿到缓存结果；
- 超时后重试不挂死。

### B4. ARAM 时间戳导致空提交（D25 低）

**设计**

- 生成器在内容未变（version/champions/source 相同）时复用上次 `generatedAt`，或工作流传入上次 manifest 的 `generatedAt`。
- 保持 `--generated-at` 可覆盖，便于测试。
- 这样 `git diff --quiet` 才能真正生效，减少空提交与无意义发布。

## 4. C 批次：性能

### C1. 战绩页每 5 秒串行拉 20 局（D5）

**根因**

`App` 的 5 秒状态刷新无条件调用 `career_overview`；`summarize_history` 串行 `match_detail`，缓存 2 MB 任意淘汰。

**设计**

前端：

- 拆分 `refreshStatus()`（5 秒）与 `refreshCareer()`。
- `refreshCareer()` 仅在切到 `career` 页、连接建立、对局结束事件或手动刷新时执行；其它页面不再加载 20 局详情。
- 注意 App 还从 `career` 派生 `ownQuery`/头像；改为连接时只取一次轻量当前召唤师（或保留上一份 career 快照），不能因为跳过 career 页导致 TFT 跳转/头像失效。
- 可选 30–60 秒缓存，避免每次切页重复。

后端：

- `summarize_history` 使用有界并发（建议 4–6，`buffer_unordered` 或分块 `join_all` 后恢复原顺序）；不要一次并发 20 个详情请求，避免 LCU 高延迟/429 时进一步恶化。
- `JsonCache` 改为明确 LRU（`VecDeque` + map）或扩大至 8 MB；禁止 HashMap 任意淘汰刚插入项。
- 对局详情显示失败时保留列表条目，仅缺详情字段；429/5xx 使用退避重试。

**测试/指标**

- 模拟 20 局：详情请求并发（最大在途数 >1），总耗时下降；
- 切换非职业页 60 秒：`match_detail` 调用为 0；
- 缓存命中率/内存不超预算。

### C2. 前端 bundle（D26 低）

- 用 `React.lazy`/`Suspense` 拆分 OP.GG、TFT、GameInfo、Settings/Auxiliary；
- 或 Vite `manualChunks` 拆出 Fluent/图表；
- 目标：主入口 chunk < 500 KB；不改变页面行为。

## 5. D 批次：前端状态与 UI

### D1. 对局详情竞态/TFT 叠加（D11）

**设计**

- `openGame` 使用自增 `requestSeq`：
  - 请求前 `setSelected(null); setTftSelected(null); setError("")`；
  - 请求返回仅在 seq 仍为最新时写入；
  - 失败仅在最新请求时显示错误；不保留旧详情。
- `focusPlayer`/participant 跳转也使用同一 token，清空旧详情。
- `runSearch` 使用独立 `searchSeq`，不要因 `loading` 直接丢弃新搜索；最新请求覆盖旧请求，旧响应只忽略不写状态。
- `openSearch` 的 nonce 改为单调计数器/ref，避免 `Date.now()` 同毫秒碰撞。
- 关闭详情时清空对应 token。

**测试**

- 快速点击 A/B：只显示 B；
- 从 TFT 详情点击普通对局：只显示 GameDetail；
- 请求失败：不显示旧对局。

### D2. 页面卸载导致搜索/草稿丢失（D12）

**设计（推荐）**

- `SearchPage`、`SettingsPage`、`GameFunctionsPage` 保持挂载，用 `hidden`/`display:none` + `aria-hidden`/`inert` 切换；显式传入 `active`，隐藏页暂停所有副作用（Settings 自动更新检查、Search 自动请求、GameFunctions 定时器如有）。
- Search 的 `searchRequest` 在消费后通过 `onRequestHandled` 置空，避免重新挂载时重放旧请求；请求到达时 App 同时切到搜索页。
- Settings/GameFunctions 的 draft 在组件生命周期内保留；仍然只有“保存设置”落盘；可增加未保存提示（可选）。
- 其他无状态页（Home/TFT/OP.GG/GameInfo/Auxiliary）继续条件渲染，避免无意义轮询。

**备选**

- 将搜索/草稿状态提升到 App/Context；改动更大，但页面可自由卸载。

### D3. 断线后对局信息残留（D13）

- `!connected` 时始终清空 `live`、`champSelect`、`buffs`、`aramMeta`、`error`；
- `ally/enemy` 仅在 `enableReserveGameinfo=true` 时保留最后一局快照（这是该设置的真实语义），否则一并清空；
- JSX 额外用 `connected && live`/`connected && champSelect` 防御。

### D4. 辅助工具无连接态/错误泛化（D14）

- App 向 `AuxiliaryPage` 传 `connected`。
- 断线时禁用写入按钮，显示“请先连接客户端”的提示。
- `friendlyError` 增加场景映射：
  - `NotConnected` → 请先连接客户端；
  - `SummonerNotFound` → 未找到该玩家；
  - 观战 404/409 → 玩家当前不在可观战对局；
  - 其余显示通用文案，详情进日志。

### D5. 设置/游戏功能保存按钮滚出视口（D10）

- `header` 改为 sticky（`position: sticky; top: 0; z-index; background: tokens.colorNeutralBackground1`），或在页面底部增加固定操作栏。
- 滚动后仍可保存，成功/失败 MessageBar 放在 header 下。
- 验证 1000×700 与 1112×754 两种窗口。

### D6. 主题下拉显示 Light/Dark（D15）

- Fluent Dropdown 的 `value` 是显示文本；增加映射：
```ts
const THEME_LABELS = { Light: "浅色", Dark: "深色" } as const;
value={THEME_LABELS[draft.theme.themeMode] ?? draft.theme.themeMode}
```
- DPI 的 “Auto” 同步映射为“自动”。

### D7. 窗口尺寸字段被覆盖（D16）

- 先修正现有口径：Tauri `WindowMessage::SetSize` 最终调用 `set_inner_size`（已核对 runtime 源码），而当前代码持久化 `outer_size`（含标题栏/边框）。应统一持久化 `inner_size` 并按比例转为逻辑尺寸，否则每次重启可能增长/漂移。
- 短期：改标签“窗口尺寸（重启生效）”，并在保存时调用 Rust `set_window_size`，立即应用；关闭时仍保存实际 inner size。
- 或改为只读展示当前尺寸，移除可编辑输入，避免误导。
- 若保留可编辑，保存时校验 `>=640×480`，失败给字段级错误；验证 100%/125%/150% DPI 下重启尺寸一致。

### D8. 数字输入清空即重置（D17）

- draft 内保存字符串原值；`onBlur`/保存时再 `Number()` + clamp。
- `careerGamesNumber` UI 限制 1–20，与后端 clamp 一致。
- 空串允许编辑中，不立即回写 fallback。

### D9. 侧边栏文案换行（D18）

- 使用 `white-space: nowrap; overflow: hidden; text-overflow: ellipsis`，或缩短为“等待客户端”，并加 `title`。
- 在 64px 收起模式已隐藏文本，不改布局。

### D10. 多个设置“假开关”（D9）

建议按语义分三类处理：

**立即实现**

- 主题色：从配置生成 Fluent `BrandVariants`（或设置品牌 token/CSS 变量），保存后即时应用。
- 字体：FluentProvider/根容器设置 `fontFamily`；保存后即时应用。
- DPI 缩放：用 Tauri `getCurrentWebview().setZoom(scale)`；`Auto` 使用系统缩放；保存后即时应用。需要在 `src-tauri/capabilities/default.json` 增加 WebView zoom 权限（按 Tauri 版本实际名称校验）。
- OP.GG 默认值：OpggPage 挂载时读取 config（region/tier/position），不再硬编码。
- 自动重连：把 `enable_auto_reconnect` 接入 session monitor/重连循环；默认 false。
- 保留对局信息：GameInfo 在游戏结束/断开时根据配置保留最后数据。
- 显示段位：GameInfo 拉取/展示段位（需要数据源/缓存设计，避免每 2 秒为 10 人请求）。

**需要新窗口/较大改动**

- 英雄选择时显示 OP.GG：在 ChampSelect 事件创建/显示第二个 Tauri WebviewWindow（opgg 路由），可选 always-on-top；不复用主导航。需要补充 `core:webview`/`core:window` 的窗口创建与置顶权限。
- OP.GG 窗口置顶：与上一条同一窗口的 `alwaysOnTop` 设置。

**建议移除/隐藏**

- 自动随机皮肤：legacy 中无消费者，语义不明确；隐藏开关并标注“暂未支持”，避免假功能。
- 若暂不实现“显示段位”，也应先隐藏，而不是留着无效开关。

每一项实现都要有 UI 手动验收；避免一次 PR 改动多个页面状态。

### D11. 错误展示降级（低）

- GameInfo/OP.GG/TFT 的主要失败用 `MessageBar intent="error"`，提供“重试”；
- 单侧失败也提示对应一侧，而不是只在双方都失败时提示；
- Catalog 失败提示“客户端数据未就绪，图标稍后加载”，不静默显示灰块。

## 6. E 批次：发布与安装器

### E1. Inno 版本硬编码（D6）

- `.iss` 不要使用 `/D` 作为唯一注入方式：已实测 Inno Setup 7 不再接受 `/D`（会把它当成第二个脚本），而 Inno 6 使用 `/D`。改为版本无关的预处理器变量：
```text
#define AppVersion GetEnv("SERAPHINE_INSTALLER_VERSION")
#if AppVersion == ""
#error SERAPHINE_INSTALLER_VERSION is required
#endif
```
- CI：
```bash
export SERAPHINE_INSTALLER_VERSION="$version"
"$iscc" installer/seraphine-rust.iss
```
- 本地手动构建同理设置该环境变量；不要在 CI 里依赖 Inno 版本选择 `/D` 或 `-d`。
- 增加校验：输出文件名必须包含 `$version`；否则失败。
- 测试：临时改 tauri 版本 2.0.3，dry-run 生成 2.0.3 中文包；在本机 Inno 7 与 CI Inno 6 各验证一次。

### E2. Inno 与 NSIS 安装目录不一致（D7）

- 已核对 Tauri NSIS 源码：currentUser 安装到 `$LOCALAPPDATA\${PRODUCTNAME}`，即 `%LOCALAPPDATA%\Seraphine Rust`；Inno 当前为 `%LOCALAPPDATA%\Programs\Seraphine Rust`。
- Inno `DefaultDirName={localappdata}\{#AppName}` 对齐 NSIS。
- 注意 AppId 不同会导致两个卸载项；仅对齐目录不能消除重复卸载记录。
- 迁移：
  - 新安装检测旧 `%LOCALAPPDATA%\Programs\Seraphine Rust` 卸载项；
  - 提示用户先卸载旧中文版，或提供“卸载旧版后继续”；
  - 不自动删除用户数据。
- 更新说明明确：2.0.2 中文版用户升级前先卸载旧版一次。

### E3. Release 重复发布 ARAM 数据（D8）

- 从 `release_rust.yaml` 删除 `Publish ARAM data to Gitee` 步骤；
- ARAM 固定数据只由 `update_aram_data.yaml` 维护；
- 应用 release 仍可携带当次数据快照作为版本资产，但不覆盖 `aram-data-latest`。

### E4. Gitee 先删后传（D21）

**已核实**：Gitee 的 `update_asset` 实现本身也是“查旧 asset id → 删除 → 重新上传”，不是服务端原子替换；action-gitee-release 只是封装了这个流程。因此原计划中的“atomic update_asset”不成立。

推荐方案（二选一）：

1. **清单改走 git raw（推荐）**
   - `latest.json`、`aram-manifest.json` 放在固定路径（如 `updater/latest.json`、`aram/aram-manifest.json`），通过 Gitee/GitHub Contents API 或 git commit 原子更新；
   - 应用端点改为 Gitee raw 优先、GitHub raw 回退；
   - 数据包仍放 release（版本化 tag），清单里的下载 URL 指向 release asset；
   - 这样不再删除 release 附件，不存在清单空窗。
   - **不能只靠 mirror 工作流**：GitHub `GITHUB_TOKEN` 推送不会触发新的 workflow。推荐流程为：Release/ARAM 工作流先用 Contents API 更新 GitHub main 上的清单，再调用 `repository_dispatch(sync-gitee)`；`mirror_gitee.yaml` 监听该事件并同步 main/tags 到 Gitee。这样只有一条写入链，不会互相覆盖。
   - Gitee 公开 raw 文件会重定向到 `raw.giteeusercontent.com`，并有 60–300 秒缓存；需在更新流程与验收中考虑这段延迟，验证重定向与 `Cache-Control`，GitHub raw 作为即时回退。
   - 迁移期双写：旧 release 端点继续更新，新增 raw 端点同时发布；旧客户端不受影响，新客户端切到 raw。至少维持一次版本迁移周期后再考虑退役旧端点。
2. **保留 release 附件（短期）**
   - 先上传 `*.next` 并做可读性/内容校验；
   - 再删除旧附件、上传最终名；
   - 任一步失败立即让 job 失败并保留临时文件/告警；
   - 明确接受短暂清单空窗，不作为长期方案。

- 无论哪种方案，失败时明确让 job 失败，不要“看起来成功”。

### E5. README/许可证/软链接（低）

- `readme.md` `cd ../..` 改为 `cd ..`（从 `rust/app` 到 `rust`）。
- Inno `[Files]` 增加 `ChineseSimplified-LICENSE.txt` 到 `{app}\ThirdPartyLicenses`。
- softprops action 升 v3（Node 24，已确认 action.yml），确认 `make_latest`/`overwrite_files` 参数兼容。

## 7. F 批次：设置接入矩阵

| 设置 | 语义 | 决策 |
| --- | --- | --- |
| themeColor | 主题品牌色 | 实现：Fluent brand variants/CSS token |
| fontFamily | UI 字体 | 实现：FluentProvider/style |
| dpiScale | WebView 缩放 | 实现：setZoom |
| OP.GG region/tier/position | 默认筛选 | 实现：OpggPage 初始化 |
| autoReconnect | 游戏内掉线重连（legacy 的 `Reconnect` phase，不是 LCU 自动连接） | 实现：监听 gameflow `Reconnect`，按 0.3s 节流调用 `/lol-gameflow/v1/reconnect`，直到离开 Reconnect |
| reserveGameinfo | 游戏结束保留信息 | 实现：GameInfo 条件清理 |
| showTierInGameInfo | 对局信息显示段位 | 实现：先做数据缓存与 UI，避免 2 秒轮询 10 人 |
| autoShowOpgg | 选人自动打开 OP.GG | 实现：第二窗口/路由 |
| enableOpggOnTop | OP.GG 窗口置顶 | 实现：第二窗口 alwaysOnTop |
| enableRandomSkin | 随机背景皮肤 | 暂不实现：隐藏并标注；legacy 也无消费者 |
| 未保存草稿 | 页面切换 | 保持挂载 + 可选未保存提示 |

实现细节：

- **主题色/字体**：用 16 级 `BrandVariants` + `createLightTheme/createDarkTheme` 生成浅/深主题；字体通过 `tokens.fontFamilyBase` 注入，不能只改某个容器 `style`。保存后 FluentProvider 立即换 theme，无需重启。
- **DPI**：主窗口与 OP.GG 第二窗口都通过 `getCurrentWebview().setZoom()` 应用；补 `core:webview:allow-set-webview-zoom`。Auto 表示系统缩放，数值表示额外 webview zoom；保存后即时应用并做布局回归。
- **OP.GG 第二窗口**：Rust 端用固定 label 创建/复用，URL 带 opgg 路由/查询； ChampSelect 开始显示、离开时隐藏或关闭；always-on-top 由 Rust builder 或带权限的 `setAlwaysOnTop` 控制。补 `core:webview:allow-create-webview-window` 与窗口 label capability。
- **autoReconnect**：监听 gameflow `Reconnect`，按 0.3 秒节流调用 `/lol-gameflow/v1/reconnect`，离开 Reconnect 或用户手动操作后停止；不要把它误接成 LCU `auto_connect`。
- **showTierInGameInfo**：先确认 SGP/LCU 是否已有段位字段；若需要逐人请求 ranked stats，必须复用缓存并有界并发，否则先隐藏开关，不能留下 2 秒轮询 10 人的假功能。

## 8. 实施顺序与验收

### PR 1：自动化/LCU（A）

- A1–A4 + A5/A6 设计实现；
- 新增纯函数测试和 WS 模拟测试；
- `cargo test --release --workspace`、clippy 全绿；
- 手动选人：hover 不立即锁、临近结束锁定；开关关闭不交易；断线重连不重置自动化。

### PR 2：写守卫/配置/缓存（B）

- 写守卫状态机、config 备份、watch/锁；
- 并发/损坏配置/缓存测试；
- 手动写一次辅助工具，确认守卫文件落盘。

### PR 3：性能（C）

- 拆分刷新、有界并发（4–6）、LRU/预算、bundle 拆分；
- 记录 20 局请求数/耗时与 5 分钟空闲流量；
- 页面切换不丢状态。

### PR 4：UI 状态（D）

- 详情 token、页面保持、断线清理、保存 sticky、主题文案、窗口尺寸、数字输入、错误展示；
- 实机截图 1000×700/1112×754/368px 卡片；
- 手动清单逐项通过。

### PR 5：发布（E）

- Inno 版本/目录/许可证、ARAM republish 删除、Gitee raw 清单与失败阻断、README；
- 本地 dry-run 2.0.3 + CI tag 测试；
- Gitee/GitHub 双源资产校验。

### PR 6：设置接线（F）

- 按矩阵逐项；每项单独验收，不与其他功能混在一起。

### 全局验收

```bash
cd rust
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --release --workspace

cd app/src-tauri && cargo test
cd app && npm ci && npm run build
```

前端测试建议补最小 Vitest + Testing Library 覆盖：搜索/详情请求 token、主题 label 映射、数字输入解析、active 隐藏页副作用门控；不要只依赖手工点击。

发布演练：

- 版本 +0.0.1 的临时 tag（不发布）验证 Inno 文件名、latest.json、签名；
- Gitee 临时分支/文件验证 raw 清单更新、60–300 秒缓存、失败回滚与旧 release 端点双写；
- ARAM 数据版本只升不降；
- 故障注入：Gitee 上传失败、GitHub Contents API 409、mirror dispatch 失败、Inno 6/7 编译、安装目录已存在旧版。

回滚策略：

- 清单走 git/Contents API，可 revert 到上一提交；旧 release 资产保留至少一个版本周期。
- 配置损坏有 `.corrupt-*` 备份；写守卫 pending/Unknown 落盘，重启不会重复写。
- 新功能均有独立配置开关或可隐藏项；出现 P0/P1 时先关闭对应功能，再发修复版本。Tauri updater 不做降级，不能依赖“装回旧版”作为唯一回滚。

## 9. 需要确认的决策

1. **D9 设置范围**：是否接受“随机皮肤”暂时隐藏；“显示段位/OP.GG 第二窗口”按上述方案实现？
2. **D7 安装目录迁移**：是否接受 2.0.2 中文版用户先卸载旧版一次，以统一到 NSIS 目录？
3. **D12 页面状态**：保持挂载（推荐）还是提升到全局 store？
4. **D21 Gitee 更新方式**：已确认 `update_asset` 是删+传；是否采用 git-raw 清单（推荐，无空窗）或短期保留 release 附件并接受空窗？
5. **A1 锁定提前量**：4 秒（Python 版基准）是否符合你的预期？

在确认以上决策后，按 PR 1 → PR 6 顺序落地；每个 PR 先补测试，再改实现，最后做真机/发布回归。
## 10. 五轮联网审查记录

### Round 1：LCU 计时器与 WebSocket

- 确认 `GET /lol-champ-select/v1/session` 的 `timer.adjustedTimeLeftInPhase` 是毫秒，示例中 `BAN_PICK=36183`、`FINALIZATION=59371`；字段可用。
- 发现 ARAM/FINALIZATION 末尾 timer 可能短暂冻结，单看 `adjustedTimeLeftInPhase` 会漏锁；计划改为 `internalNowInEpochMs + adjustedTimeLeftInPhase` 的本地 deadline，并跳过 `isInfinite`。
- 确认 tungstenite 会自动回 Pong（`hyper-tungstenite` 文档明确无需手动回复）；计划从“手发 Pong”改为“Ping/Pong 继续循环，不手发”。
- 结论：A1/A2 方案成立，但需上述两处修订。

### Round 2：异步同步、窗口尺寸与 Gitee API

- 核对 Tokio Notify：`notify_waiters()` 只唤醒调用前已注册的 `Notified`，存在丢失唤醒风险；计划改用 `watch`/per-key 锁，并验证 leader 先完成、waiter 后注册的场景。
- 核对 Tauri runtime：`WindowMessage::SetSize` 最终调用 `set_inner_size`；当前代码持久化 `outer_size`，会与恢复口径不一致。计划改为统一 `inner_size`，并做 DPI 往返测试。
- 核对 Gitee action 源码：`update_asset` 实际是“查 id → delete_asset → upload_asset”，不是原子 API；原计划“原子 update_asset”作废。改为 git-raw 清单（推荐）或临时文件 + 失败阻断。
- 结论：E4 与 D16 需要实质修订，B3 方案不变但需测试。

### Round 3：Tauri 前端、多窗口与 React 状态

- `setZoom` 需要 `core:webview:allow-set-webview-zoom`，不在 `core:default` 中；DPI 设置需补 capability。
- 多窗口需要 `core:webview:allow-create-webview-window`，置顶还涉及 `core:window:allow-set-always-on-top`；OP.GG 第二窗口方案需同时补权限与窗口 label 管理。
- Fluent Dropdown 的 `value` 是显示文本，主题 `Light/Dark` 必须映射为中文；原计划正确。
- 保持挂载会保留状态但 `useEffect` 继续运行；因此 Search/Settings/GameFunctions 必须增加 active 门控，不能只 `display:none`。
- 结论：D9/D10/D12 方案可行，但必须补 capability 与副作用门控。

### Round 4：Inno、NSIS 与发布资产

- 实测 Inno Setup 7 不接受 `/D`（被当成脚本文件名），Inno 6 使用 `/D`；原 AI 方案会破坏本机 Inno 7。改为 `GetEnv("SERAPHINE_INSTALLER_VERSION")`，并在本机 Inno 7 临时脚本实测成功生成 `SeraphineRust-9.9.7-setup-zh.exe`。
- 核对 Tauri NSIS 源码：currentUser 路径为 `$LOCALAPPDATA\${PRODUCTNAME}`；Inno 当前为 `Programs`，对齐方案成立，但 AppId 不同仍会留下两个卸载项，需要迁移提示。
- 核对 Tauri updater：Inno 不支持 NSIS 的静默参数 `/S`；自动更新仍应使用 NSIS。中文字体包只能作为手动安装/离线包，或单独设计 updater 配置。
- 核对 softprops v3：使用 Node 24，`make_latest`/`overwrite_files` 输入仍存在；升级可行。
- 结论：E1/E2 需要修订，E3/E5 不变。

### Round 5：跨项一致性与本地验证

- 交叉核对发现：跳过 career 轮询会丢失 `ownQuery`/头像来源；计划补充“连接时轻量当前召唤师或保留 career 快照”，TFT payload 已自带 puuid，可优先用 puuid。
- `enableReserveGameinfo` 与断线清理冲突；明确为“始终清 live/BP，仅保留 ally/enemy 最后一局快照”。
- 本地执行 `cargo fmt --check`、`cargo clippy -D warnings`、`cargo test --release --workspace`、app 单测、`npm run build` 全部通过；启动 release 应用检查了 Home/游戏功能/设置（展开/收起/滚动）UI，确认保存按钮滚出视口、主题显示 `Light`、侧栏文案换行。
- Inno `/D` 与 `GetEnv` 方案均在临时脚本验证；Gitee update_asset 非原子已由源码确认。
- 结论：计划修订后可作为实施基线，剩余决策见第 9 节。

### 联网来源

- LCU session 计时器字段样例：https://gist.github.com/xadamxk/8cb5d21d24bb78d63c5241e97087bb23
- ARAM timer 冻结说明：https://github.com/hzSouai/ARAM-Navigator
- tungstenite 自动 Pong：https://github.com/de-vri-es/hyper-tungstenite-rs
- Tokio Notify 文档：https://docs.rs/tokio/latest/tokio/sync/struct.Notify.html
- Tauri NSIS 源码：https://github.com/tauri-apps/tauri/blob/dev/crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi
- Tauri webview setZoom：https://v2.tauri.app/reference/javascript/api/namespacewebview/
- Tauri 多窗口权限：https://tauri.app/learn/mobile-multiwindow/
- Gitee update_asset 实现：https://github.com/nicennnnnnnlee/action-gitee-release
- softprops v3 action.yml：https://github.com/softprops/action-gh-release
- Inno 版本注入：https://stackoverflow.com/questions/36449026/passing-in-version-number-to-inno-setup-compiler

## 11. 第二轮五轮联网审查记录

### Round 1：自动化语义与 LCU 边界

- 核对 legacy 实现：`autoComplete` 在 sleep 后会重新拉取 champ-select session，检查双方已完成 pick、bans、当前 champion intent，并在目标不可选时从配置名单挑下一个候选。原计划只做 timer gating 不够，已补“fresh session + cantSelect + 候选回退”。
- champ-select session 样例没有稳定保证 `gameId`；draft_id 优先从 gameflow session 获取， champ-select 只作 fallback，避免错误依赖。
- `pickOrderSwaps`/champion trades 在最后 5 秒会被客户端取消；自动接受/锁定必须尊重阶段末尾，不能与 swap 并发抢占。
- LCU 无官方公开并发限额，但高延迟场景下 20 个详情并发会放大压力；career 详情改为 4–6 有界并发，429/5xx 退避。

### Round 2：Rust 并发、配置与缓存

- 写守卫 pending 状态机方向正确；补充非 guarded 写入的失败清理语义，避免改变现有辅助工具重试行为。
- 缓存替换不能只解决丢唤醒：leader 失败/任务取消也必须清理 inflight 并唤醒等待者，否则同 key 请求永久挂起。
- 配置迁移需同时识别 GitHub release 与 Gitee release 两个旧 feed URL；自定义 URL 不动。updater 端点迁移依靠新版本编译，旧端点继续双写。
- `autoComplete` 的 fresh session 与 candidate fallback 必须纳入 PR1 测试，不能只测 timer 纯函数。

### Round 3：Tauri/Fluent 前端

- Fluent v9 品牌主题需要 16 级 `BrandVariants` + `createLightTheme/createDarkTheme`；字体应改 `tokens.fontFamilyBase`，不能只设容器 style。
- `setZoom` 权限已确认；主窗口和 OP.GG 第二窗口都要应用，Auto/数值语义与布局回归要覆盖 100%/125%/150%。
- 保持挂载在 React 18 不会暂停 effects；Search/Settings/GameFunctions 必须显式 `active` 门控，并用 `hidden`/`aria-hidden`/`inert` 处理可访问性。
- 搜索竞态不止 openGame：`runSearch` 也要独立 token，不能因 `loading` 丢弃最新请求；nonce 用单调计数器替代 `Date.now()`。

### Round 4：发布、清单与安装器

- 在 Inno 6.4.3 源码中确认 ISPP 注册了 `GetEnv`，因此环境变量注入方案在 6/7 均可用；Inno 7 已实测。
- Gitee raw 公开文件会重定向到 `raw.giteeusercontent.com`，缓存 60–300 秒；raw 清单方案必须把这段延迟与重定向纳入验收。
- `GITHUB_TOKEN` 推送不会触发 mirror workflow；推荐 GitHub Contents API 写清单 → `repository_dispatch(sync-gitee)` → mirror 同步，避免直接写 Gitee 与 mirror 互相覆盖。
- 旧客户端仍使用 release 端点，迁移期必须双写；至少保留一个版本周期后再退役旧端点。
- Inno 与 NSIS AppId/卸载项不同，目录对齐之外仍需旧版迁移提示；自动更新继续使用 NSIS。

### Round 5：跨项一致性与可验证性

- `enableAutoReconnect` 的 legacy 语义是 gameflow `Reconnect` 阶段调用 `/lol-gameflow/v1/reconnect`，不是 LCU `auto_connect`；计划已纠正，避免接错开关。
- `reserveGameinfo` 只保留 ally/enemy 信息快照；live/BP 必须清理，二者不再冲突。
- career 轮询拆分需保留轻量当前召唤师/ownQuery；TFT payload 已自带 puuid，可优先使用。
- 增加前端最小测试矩阵与故障注入；回滚不能依赖 updater 降级，需 git/配置/功能开关三层回退。
- 本地 Rust fmt/clippy/test、app 单测、前端 build 均通过；Inno GetEnv 临时构建通过。本轮未修改产品源码，仅修订计划。

### 第二轮来源

- legacy autoComplete 实现：`legacy-python:app/lol/tools.py`
- LCU session timer 样例：https://gist.github.com/xadamxk/8cb5d21d24bb78d63c5241e97087bb23
- Team drafting / swap 末尾规则：https://wiki.leagueoflegends.com/en-us/Pre-made_party
- Fluent UI v9 brand theme：https://dev.to/paulgildea/quick-branded-light-and-dark-modes-with-fluent-ui-react-v9-1pe6
- Tauri setZoom：https://v2.tauri.app/reference/javascript/api/namespacewebview/
- Tauri 多窗口权限：https://tauri.app/zh-cn/learn/mobile-multiwindow/
- Gitee raw 缓存与重定向：https://help.gitee.com/repository/file-operate/raw
- GitHub GITHUB_TOKEN 不触发 workflow：https://docs.github.com/zh/actions/concepts/security/github_token
- Inno 6 ISPP GetEnv 源码：https://github.com/jrsoftware/issrc/blob/is-6_4_3/Projects/Src/ISPP.Funcs.pas
