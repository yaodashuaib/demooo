# D50 设备配置

上位机把「已配置的 Agent」与声光偏好投影到 D50：图标槽、余额页、提示音/灯。本文件只定义语言；线格式以 `doc/D50与PC的应用层通信协议.md` 为准。

## Language

**Linked App（已配置应用）**：
`%AppData%\Roaming\Ulanzi\linked-apps.json` 里的一条应用记录；本上下文只关心 `category == "aiAgent"` 的条目。
_Avoid_：另建一份 D50 专用「绑定表」与之并行。

**Agent（已配置 Agent）**：
一条 `status == "configured"` 且 `id` 落在本 demo allowlist（`claude_code`、`kimi`、`chatgpt`、`workbuddy`）中的 `aiAgent` Linked App；以稳定 `id` 标识；同一 `id` 只占一个图标槽，不论桌面客户端还是 CLI；须带持久化 `agentIndex`。
_Avoid_：把 `error` / `unbound` 等非 configured 状态当成可投影；把客户端与 CLI 拆成两个 Agent；把 `chatgpt` 与 Codex 当成两个品牌——在本上下文中 `chatgpt` 即 Codex 线。

**AgentIndex（图标槽）**：
D50 上 Agent 图标的固定编号，取值 0–5；由 bridge 在 `config_bind_agent` 时分配最小空闲槽并写入 Linked App 的 `agentIndex` 字段后固化；解绑清除该字段且**不重排**其它 Agent 的槽位；经 `SET_AGENT_INDEX` 全量下发当前已绑定集合；审批会话只引用、不分配。
_Avoid_：用户手选槽位；按文件顺序临时算 `0..n-1`；把图标槽当成审批请求身份。

**Agent 图标（Agent Icon）**：
下发给某图标槽的 100×100 RGB/RGBA PNG（每通道 8bit，协议 11.9）：`claude_code`→`icon/cc.png`，`kimi`→`icon/kimi.png`，`chatgpt`→`icon/chatgpt.png`，`workbuddy`→`icon/workbuddy.png`。仅在绑定/解绑时全量下发；设备上线不推图标。
_Avoid_：把 `linked-apps.json` 的 `qrc:` 路径直接当设备载荷。

**余额投影（Balance Projection）**：
由当前可投影 Agent 与各 Agent 用量（采集对齐 CodexBar 各 provider 的指标）算出的 `SET_AGENT_BALANCE` 载荷；绑定只拉新 Agent 用量并与缓存合并下发；可配置间隔（默认 180s）与设备上线时全量刷新余额；可投影列表为空时下发清空；设备上的余额页不是真相源。
_Avoid_：把 D50 屏幕上的余额数字当作账户真实余额；为本 demo 另发明一套与 CodexBar 无关的指标；写死仅中文或仅英文标签。

**声光配置（Sound/Light Config）**：
上位机侧保存的 Agent 提示音量/开关与底部灯亮度/颜色/时长；本机文件 `%AppData%\Roaming\Ulanzi\D50\d50-sound-light.json` 为草稿与「上次意图」真相，经 bridge 用 `SET_SOUND_LIGHT_CONFIG` 下发；协议无回读。
_Avoid_：把设备当前灯效当成可查询的配置真相；与系统页 `SET_SYSTEM_CONFIG` 混称。
