# 余额采集源（对齐 CodexBar）

| Agent id | 凭据 | HTTP |
| --- | --- | --- |
| `chatgpt` | `%USERPROFILE%\.codex\auth.json`（或 `CODEX_HOME`） | `GET https://chatgpt.com/backend-api/wham/usage` |
| `claude_code` | `%USERPROFILE%\.claude\.credentials.json` | `GET https://api.anthropic.com/api/oauth/usage?cedar_ember=1` |
| `kimi` | `KIMI_CODE_API_KEY` → 否则 `~\.kimi-code\credentials\kimi-code.json`（不用 `KIMI_AUTH_TOKEN` / 网页用量路径） | `GET https://api.kimi.com/coding/v1/usages` |
| `workbuddy` | 见下 | `POST https://www.workbuddy.cn/billing/meter/get-user-resource-summary` |

超时默认 8s；失败时该 Agent 显示「暂无数据」，仍占图标槽。

## WorkBuddy 为什么要 Cookie

WorkBuddy / CodeBuddy 的额度在 **网站计费 API** 上，会话绑定浏览器 Cookie + Chrome User-Agent。  
CodexBar 同样：**不读、不解密** `~\.workbuddy\` 里的桌面端 token。  
因此本 demo 也无法用「安装 WorkBuddy 客户端即自动有余额」——只能复用已登录网站的会话。

## 推荐交互（配置客户端）

在 `d50-config-client` 的 **WorkBuddy 会话** 区粘贴 Cookie → **保存会话**。  
写入：`%AppData%\Roaming\Ulanzi\D50\workbuddy-session.json`  
IPC：`config_set_workbuddy_session`（状态里只回 `workbuddy_session_configured`，不回传 Cookie）。

操作步骤：

1. 浏览器打开并登录 [www.workbuddy.cn](https://www.workbuddy.cn)（建议 Chrome）
2. F12 → Network → 任意对 `workbuddy.cn` 的请求 → 复制 Request Headers 里的 `Cookie`
3. 粘贴到配置客户端并保存

## Cookie 与 Chrome 版本

WorkBuddy 会校验 Cookie 与 **Chrome User-Agent 主版本** 是否匹配；版本不对会 HTTP 401。  
bridge 会自动读取本机 `Google\Chrome\Application\` 下的版本目录（例如 156.x），并依次尝试 `major` / `major-1`。

可选覆盖：

- `D50_WORKBUDDY_COOKIE` / `WORKBUDDY_COOKIE`：调试用，优先于文件
- `D50_CHROME_MAJOR`：强制 UA 主版本
- 会话 JSON 里的 `chrome_major` 字段

失败原因会出现在 `config_status.errors`（例如 UA/Cookie 401 提示）。

以后若要做「一键从 Chrome 导入」，需要 DPAPI 解 Cookie DB，可作为单独切片。
