# Seraphine 术语表

| 原术语 | 含义 | Rust/Tauri 名称 |
| --- | --- | --- |
| LCU | 英雄联盟客户端本地 REST/WebSocket 接口 | `seraphine_lcu::LcuClient` |
| SGP | 战绩服务接口 | `seraphine_lcu::SgpClient` |
| Catalog | 英雄/装备/符文/皮肤等静态目录 | `seraphine_domain::catalog::CatalogBuilder` |
| 会话 epoch | 每次 LCU 重连递增的隔离代号 | `SessionEpoch` |
| 写守卫 | 防止重复/未知结果写操作的持久化记录 | `seraphine_automation::write_guard` |
| OutcomeUnknown | 写操作结果未知，禁止盲目重试 | `WriteOutcome::Unknown` |
| 资源预算 | LCU/资产并发与字节上限 | `seraphine_resource_scheduler` |
| 职业页 | 召唤师主页与战绩概览 | Career view |
| 对局信息 | 选人/加载阶段的双方信息 | Game info view |
| BP | 禁用/选用流程 | Champion select |
| ARAM 加成 | 极地大乱斗英雄修正 | ARAM buffs |
| OP.GG 榜单 | 外部梯级数据 | Tier list |
| 辅助工具 | 改名、离线状态等工具 | Auxiliary tools |