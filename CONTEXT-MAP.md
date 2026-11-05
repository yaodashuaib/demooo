# Context Map

## Contexts

- [D50 审批门控](./CONTEXT.md)：Agent 工具执行前的人工裁决，经会话槽投影到设备并回填决策
- [D50 设备配置](./docs/device-config/CONTEXT.md)：已配置 Agent 的图标槽、余额投影与声光参数，经 bridge 独占下发到 D50

## Relationships

- **设备配置 → 审批门控**：配置上下文拥有 `AgentIndex`（图标槽）的分配与图标下发；审批投影会话时只引用已分配的 `AgentIndex`，不自行发明图标
- **两者 → agent-bridge-service**：同一 HID owner；上位机（含 `apps/d50-config-client` 与后续宿主）只经本机 IPC 讲话，不直接打开 D50；配置状态由上位机轮询 `config_status`
