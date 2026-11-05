# 配置类 IPC（PROTOCOL=1 增量）

管道：`\\.\pipe\ulanzi-d50-bridge.v1`（`bridge_ipc::DEFAULT_PIPE`）。NDJSON 请求/响应，与审批方法共用连接模型。

新上位机只需实现下列方法；**不要**假设存在裸 `CommandID` 灌帧接口。

## `config_status`

请求：

```json
{"method":"config_status"}
```

响应：

```json
{
  "result": "config",
  "config": {
    "device": { "online": true, "serial": "...", "firmware": "...", "shown": 0, "queued": 0 },
    "agents": [
      {
        "id": "kimi",
        "name": "Kimi Code CLI",
        "agent_index": 0,
        "balance_ok": true,
        "balance_summary": "5小时剩余 80% | 周剩余 42%"
      }
    ],
    "last_projection_at": 1730000000000,
    "sound_light": { "Sound": { "...": "..." }, "Light": { "...": "..." } },
    "workbuddy_session_configured": false,
    "balance_refresh_secs": 180,
    "errors": []
  }
}
```

**只读缓存**：不拉用量、不入队 HID。上位机在绑定/解绑后、或用户点「刷新」时调用。图标与余额由 `config_bind_agent` / `config_unbind_agent`、余额定时器与上线事件驱动。

## `config_bind_agent`

```json
{"method":"config_bind_agent","id":"chatgpt","name":"ChatGPT"}
```

- `id` 须在 allowlist：`claude_code` / `kimi` / `chatgpt` / `workbuddy`
- `name` 可选；缺省用内置显示名
- 写入 `%AppData%\Roaming\Ulanzi\linked-apps.json`：`status=configured`，分配最小空闲 `agentIndex`（0–5，不重排已有槽）
- 若在线：全量 `SET_AGENT_INDEX`；只 `fetch_usage(新 id)`，与内存余额缓存合并后 `SET_AGENT_BALANCE`
- 成功：`{"result":"config","config":{...}}`

## `config_unbind_agent`

```json
{"method":"config_unbind_agent","id":"kimi"}
```

- 将对应 `aiAgent` 的 `status` 改为 `unbound` 并清除 `agentIndex`（不重排其它槽）
- 若在线：全量 `SET_AGENT_INDEX`；用缓存组 `SET_AGENT_BALANCE`（不重拉其它用量）
- 成功：`{"result":"config","config":{...}}`

## `config_set_balance_interval`

```json
{"method":"config_set_balance_interval","secs":180}
```

- `secs` 钳制到 30–3600；写入 `%AppData%\Roaming\Ulanzi\D50\d50-bridge.json` 的 `balance_refresh_secs` 并重置余额 timer
- 成功：`{"result":"config","config":{...}}`

## `config_set_sound_light`

请求：

```json
{
  "method": "config_set_sound_light",
  "config": {
    "Sound": { "Volume": 80, "AgentPrompt": { "Enabled": true } },
    "Light": {
      "Brightness": 80,
      "AgentPrompt": {
        "Enabled": true,
        "Color": { "R": 255, "G": 128, "B": 0 },
        "DurationMs": 6000
      }
    }
  },
  "preview": false
}
```

- `preview: false`：下发并写入 `%AppData%\Roaming\Ulanzi\D50\d50-sound-light.json`
- `preview: true`：仅预览（协议 Preview），不改本机草稿

成功：`{"result":"ok"}`。

## `config_set_workbuddy_session`

保存或清除 WorkBuddy 网站 Cookie（本机 AppData，不经 HID）：

```json
{"method":"config_set_workbuddy_session","cookie":"a=1; b=2","chrome_major":131}
```

- `cookie` 为空：删除已保存会话  
- `config_status` 仅返回 `workbuddy_session_configured: true/false`，**不**回传 Cookie 明文  
- 保存成功后后台全量刷新余额（不下发图标）
