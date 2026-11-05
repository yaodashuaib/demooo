# 配置客户端只经 bridge IPC；bridge 拥有图标/余额推送循环

Qt（及任何其它上位机）不打开 D50 HID，只对本机 `agent-bridge-service` 发 IPC。读 `linked-apps`、拉余额、组 `SET_AGENT_INDEX` / `SET_AGENT_BALANCE`、声光下发，以及等 ACK 的命令队列，都由 bridge 内的配置投影循环拥有，避免多上位机抢推送，并延续 ADR-0003 的单 owner 约束。

## Status

accepted

## Considered Options

- **Qt 自己轮询余额并经 IPC 逐条「裸发命令」**：解耦看起来更强，但两个上位机可同时推送，且图/余额竞争要在每个客户端各做一遍队列。
- **bridge 拥有投影循环**（本决定）：设备侧时序集中一处；代价是 bridge 从「纯审批」扩成配置+审批的组合根。

## Consequences

- 审批与设备配置共用同一 HID 写队列（协议本就 Stop-and-Wait）。
- 领域上拆成两个上下文（见 `CONTEXT-MAP.md`），进程上仍是一个 bridge。
