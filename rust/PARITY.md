# Python → Rust 功能对照

状态：done / partial / blocked / todo。证据以真机截图、测试或代码路径为准。

| 功能 | Python 来源 | Rust 状态 | 证据 / 差距 |
| --- | --- | --- | --- |
| 启动、无边框窗口、托盘、单实例 | main.py / main_window.py | done | 真机启动；托盘显示/退出 |
| 窗口尺寸记忆 | main_window.py | done | 1000×700 保存并重启恢复 |
| 配置迁移与设置页 | config.py / setting_interface.py | partial | 常规/网络/功能/OP.GG/外观/名单/客户端/自动技能/队列过滤已接入；语言切换按“中文优先”暂缓 |
| LCU 发现/连接/重连/事件 | connector.py / sidecar | done | 真机 PID/区服、WS 事件、自动重连 |
| 职业页（召唤师/段位/战绩/头像） | career_interface.py | done | 真机 20 场数据 |
| 搜索（名称/PUUID/分页/筛选/详情） | search_interface.py | done | 真机搜索与双方详情 |
| 对局信息（己方/敌方/近况/上局标记） | game_info_interface.py / tools.py | partial | SGP 优先 + LCU 回退；选人/对局真机字段待核对 |
| 英雄选择只读状态（计时/槽位/禁用） | main_window.py | done | 页面与命令已接入；待选人实测 |
| 自动接受对局 | tools.py | partial | 事件驱动 + 3 次重试（400ms，独立操作 id）；真实 ReadyCheck 均返回 500 且重查已 resolved，缺“无人手动点击”的通过证据 |
| 英雄选择写操作（禁用/选用/交换/自动） | tools.py / sidecar | done | 2026-11-27 真机征召：技能 4/14、ban 安妮、pick 艾希并锁定；ban 需 BAN_PICK 首秒两步 PATCH（PLANNING 写入被客户端丢弃）；每局 nonce 防跨局守卫碰撞 |
| 辅助工具（在线状态/背景/段位/代币/观战等） | auxiliary_interface.py | done | 在线状态/可用性/头像/背景/段位展示/观战/重连/重启/代币/徽章/锁定配置（可逆）已接入；窗口修复 2026-11-27 真机：1920×1080 → 1280×720 并居中 |
| ARAM 加成展示 | aram.py / game_info | done | 本地数据 + 对局信息展示；真机大乱斗待验 |
| ARAM 远端更新 | aram_data.py | partial | 校验/节流/原子替换与手动按钮已实现；CI 生成支持 Wiki→jsdelivr 回退并发布到 GitHub 固定标签 `aram-data-latest`；仍需推送触发首次 CI 发布后再做端到端热更新验证 |
| OP.GG 榜单/出装/符文/图标 | opgg.py / opgg_window.py | done | 真机韩服榜单与出装、符文、竞技场强化 |
| TFT 战绩 | career_interface.py / tools.py | done | 真机 20 场、前四率/平均排名/单位 |
| TFT 信息条 | tft_game_infobar.py | done | TFT 战绩卡覆盖排名/等级/单位/强化，等价展示 |
| 游戏内实时信息 | game_infobar_widget.py | partial | Live Client Data 实时卡；窗口修复工具未做 |
| 自动更新 | update.py / make.ps1 | done | Tauri updater 已端到端验证：2.0.0 检查/下载/验签/安装 2.0.1/重启；真实 GitHub 发布待上传 latest.json |
| 多客户端切换 | connector.py | done | 设置页列出 PID/区服并切换 |
| 图片池/资源缓存 | image_pool.py | done | Tauri asset 缓存 + 客户端目录重建 |
| 运行日志 | app/common/logger.py | done | 文件日志 + 轮转 + panic 记录，已实测启动/连接行 |
| 国际化 zh/en/pt | i18n/*.ts | todo | 当前仅中文 |
| 安装包与卸载 | seraphine.iss | done | NSIS 静默安装/启动/卸载 smoke；数据目录保留 |
| 性能基线 | PresentMon 工具 | partial | 启动/CPU/内存已测；目标机 PresentMon/WPR 待做 |

## 下一步优先级

1. 真机验证选人与大乱斗，修正 W2-1/W2-3 字段。
2. 政策审批后接入写操作与辅助工具。
3. 更新器签名与发布清单（需私钥保管方案）。
4. i18n 与剩余设置项。
5. 目标机性能门槛与七天观察。