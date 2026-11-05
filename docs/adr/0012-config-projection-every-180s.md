# 余额按可配置间隔全量刷新；上线只推余额

bridge 按 `%AppData%\Roaming\Ulanzi\D50\d50-bridge.json` 的 `balance_refresh_secs`（默认 180，钳制 30–3600）定时全量拉用量并 `SET_AGENT_BALANCE`。设备从离线到在线时立即跑一轮**仅余额**（不下发图标）。图标仅在 `config_bind_agent` / `config_unbind_agent` 时全量 `SET_AGENT_INDEX`。

## Status

accepted（supersedes 旧「列表变化即重发图标 + 上线全量」表述）
