# 审批复用 D50 的会话槽，不改固件

D50 协议里的 `SET_CONVERSATION_LIST`(0x21) / `UPSERT_CONVERSATION`(0x22) 已经带 `NeedsAttention` 和最多 4 个 `Buttons`，设备按键用 `CONVERSATION_ACTION`(0x41) 上报 `{ID, ButtonIndex}`。这条往返正好就是「显示问题 + 收回答案」，所以审批直接投影成会话，不新增固件命令。

## Status

accepted

## Considered Options

- **新增 `SET_APPROVAL` / `APPROVAL_ACTION` 命令**：语义更贴切，但需要固件先发版，demo 无法在现有机器上跑通。
- **复用会话槽**（本决定）：零固件改动，今天就能端到端验证；代价见下。

## Consequences

- 硬上限来自会话而不是审批：最多 6 条同时在线、每条 4 个按钮、`Title`/`Path`/`Content` 各 64 个 UTF-8 字节。第 7 个请求必须排队而不是下发，否则 `UPSERT` 会静默挤掉最老的那条未决请求。
- 协议没有「删除单条会话」的命令。撤下一条只能用 `SET_CONVERSATION_LIST` 全量重写，所以服务必须是会话槽的唯一写入者，且每次决策后按自己的投影全量同步一次。
- 设备按下按钮后会自行把 `NeedsAttention` 置 0，PC 必须按同样规则同步，否则设备与服务的状态会各说各话。
- 「会话」这个词在界面上就是「审批」，短期可以接受；长期如果审批变多，再考虑固件级命令。
