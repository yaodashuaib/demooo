# config_status 只读缓存；绑定/解绑驱动 HID

`config_status` 返回内存中的设备与 Agent 视图，**不**触发用量拉取或 HID。上位机在绑定、解绑或用户点「刷新」时调用。图标与余额由 `config_bind_agent` / `config_unbind_agent`、余额定时器与上线事件驱动。

## Status

accepted（supersedes「轮询 config_status 触达 HID」）
