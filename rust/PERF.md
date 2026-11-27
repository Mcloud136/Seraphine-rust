# 性能证据（本机）

日期：2026-11-26
机器：i9-14900HX（当前开发机；目标 250KF 的 PresentMon/WPR 验收仍待执行）
对比对象：`dist/Seraphine/Seraphine.exe`（Python 版，默认 python 后端）与 `rust/app` release 二进制。

## 结果

| 指标 | Python 版 | Rust/Tauri 版 | 变化 |
| --- | --- | --- | --- |
| 启动到窗口（5 次中位数） | 750 ms | 62 ms | 12.1× 更快 |
| 启动到窗口（10 次中位数 / 首次冷启动） | — | 63 ms / 107 ms（热启动 60–64 ms） | — |
| 启动到窗口（首次/冷启动） | 4,479 ms | 289–305 ms | 约 15× 更快 |
| 主进程 Working Set（启动后 8 s） | 262.0 MB | 107.2 MB | 2.4× 更低 |
| 进程树 Working Set（启动后 8 s） | 262.0 MB（1 进程） | 494.1 MB（7 进程，含 WebView2） | 约 1.9× 更高 |
| 空闲 CPU（20 s，单核百分比） | 2.73% | 1.25% | 2.2× 更低 |

> 2026-11-27 复测：Rust 主进程占用较早期 48.9 MB 上升（updater/write/automation 与 WebView2 预热后），以本节 5 次启动与 8 s 稳定的数据为准。

## 解读

- 启动、主进程占用与空闲 CPU 均明显改善。
- 总驻留内存上升来自 WebView2 多进程（浏览器/渲染/GPU/工具进程）。这是 Tauri 方案的固有成本，需在“内存”与“开发效率/视觉还原”之间取舍。
- 进程树统计已排除搜狗输入法注入进程（`SOGOUSmartAssistant.exe`），否则会虚增约 64 MB。
- Private 内存 Python 版偏高与 PyQt/Python 运行时提交策略有关；跨版本比较以 Working Set 为主，Private 仅作参考。

## 游戏内帧时间 A/B（2026-11-27，本机 i9-14900HX）

方法：PresentMon 2.6 Console，`--output_stdout --v1_metrics --qpc_time_ms`，目标 `League of Legends.exe`，每段 60 s；同一局匹配对局内做两对相邻窗口的“应用运行 / 停止应用”切换。采集前清理残留 ETW 会话（`logman stop <name> -ets`），避免 error 1450。

| 采样 | 应用 | 帧数 | 平均 FPS | p50 | p95 | p99 | 最大 | >50ms |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 第 1 对 | 运行 | 13,220 | 235.12 | 4.19 | 5.09 | 6.16 | 9.86 | 0 |
| 第 1 对 | 停止 | 13,918 | 232.04 | 4.21 | 5.26 | 6.36 | 19.36 | 0 |
| 第 2 对 | 运行 | 13,935 | 232.26 | 4.21 | 5.34 | 6.42 | 16.31 | 0 |
| 第 2 对 | 停止 | 14,045 | 234.18 | 4.19 | 5.19 | 6.34 | 15.30 | 0 |

结论：伴随应用运行与否的 p50/p95/p99 差异在 0.1–0.2 ms 内（约 1–3%），两对方向不一致，属对局阶段噪声；两对均无 >50 ms 停顿。本机帧时间质量不因 Seraphine Rust 常驻而下降。

## 复测命令

原始 PowerShell/Python 测量与截图脚本已归档到 `legacy-python` 分支；`main` 不再保留这些工具。复测可使用 Windows 性能计数器、任务管理器或 PresentMon 官方 CLI，按本文件口径记录启动、Working Set、CPU 与帧时间。

## 待补

- 目标 Intel 250KF 上的复测（当前证据为本机 i9-14900HX）。
- 选人/对局中事件延迟与 OP.GG 加载时延（P50/P95/P99）。

## 回归与帧时间状态（2026-11-27）

- Rust workspace release：全部通过（含 `release-tool` 2 项；1 个 OP.GG 联网测试忽略）。
- Tauri app 单元测试：2 通过（自动化动作选择/候选顺序）。
- Python/Qt 契约测试为迁移前历史证据，已随旧版归档到 `legacy-python` 分支，不在 main 运行。
- PresentMon 2.6 Console 已安装；帧时间 A/B 的验收对象是“游戏运行时的 League of Legends.exe”，本次会话没有进行中的对局，故未采集。UI 自身的 Qt 与 WebView2 呈现路径不同，不能替代该门禁。
- 采集工具（PowerShell/Python）已归档到 `legacy-python` 分支；PresentMon 的 ETW 残留与 UTF-16 CSV 处理经验保留在本文件，main 不再保留这些脚本。
- sidecar 内存断言按 release 口径：release `working_set=9,666,560`（9.22 MiB）/ `private=6,504,448` 通过；debug 构建 11.3 MB 超 10 MiB 为构建差异，CI 使用 `cargo test --release --workspace`。
- 安装版（含 updater/write 插件）已再次完成静默安装、启动读取真实战绩、卸载且旧数据保留的 smoke。
