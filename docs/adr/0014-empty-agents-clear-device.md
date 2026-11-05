# 可投影 Agent 为空时清空设备图标与余额

当 allowlist ∩ `configured` 为空时，bridge 仍下发 `SET_AGENT_INDEX`（`AgentCount=0`）与 `SET_AGENT_BALANCE`（`Agents: []`），避免设备残留上一轮投影。

## Status

accepted
