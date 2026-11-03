# 设备只裁决这一次，其余回到 App：open app 与待回复提示

设备上一共四个键位，扣掉裁决按钮后剩下的那颗叫 **open app**：它不产生任何决定，只把 deeplink
交给系统外壳，打开 Agent 自己的会话页。Agent 需要人在自己界面里回答的请求（选择题、沙箱扩权）
不再假装能裁决，而是发一条「有 N 个问题待您回复…」的提示，同样只有一个 open app。

## Status

accepted

## 决定

1. **权限裁决**：Codex 两个按钮（Agree / Ignore）+ open app；Claude 在有
   `permission_suggestions` 时三个（Agree / Ignore / Always）+ open app。
2. **对话类请求**（`AskUserQuestion`、`request_user_input`、`request_permissions`）：只发提示，
   hook 立即返回 `{}`，不阻塞 Agent。Kimi 的全部 `PermissionRequest`（含普通工具权限）
   因 fireAndForget 也走这条「只提醒」路径（[ADR-0005](0005-kimi-permission-request-is-read-only.md)）。
3. **deeplink 由 hook 生成**，服务只负责打开：

   | Agent | deeplink |
   | --- | --- |
   | Claude Code | `claude://code/continue?session=local_{session_id}` |
   | Codex | `codex://threads/{session_id}` |
   | WorkBuddy | `workbuddy://chat/{session_id}` |
   | Kimi | 无官方 scheme 时退回 `focus://window/…` 或 `focus://pid/…` |

## 为什么

- `request_permissions` 与选择题在钩子层没有「批准」通道（[ADR-0004](0004-dialogs-stay-in-the-terminal.md)、
  [ADR-0006](0006-request-permissions-stays-in-the-desktop.md)）。设备上给同意/拒绝只会制造
  「按了没反应」，所以只保留「跳回去自己处理」这一个真实动作。
- open app 作为**显式的反馈按钮**下发，不依赖固件的「全局打开」动作（`ButtonIndex=0`）：
  后者在有两个按钮时不占键位，实测按不到。

## CLI 没有 App 可开时怎么办

`d50-hook` 按**进程树**区分宿主，open app 行为不同：

| 宿主 | 判定 | open app |
| --- | --- | --- |
| **桌面客户端** | 祖先为 `WorkBuddy.exe` / 商店版 `Claude.exe`（`WindowsApps\Claude_*`）或 `ChatGPT.exe` | 已注册则 `workbuddy://…` / `claude://…` / `codex://…`；否则 `focus://exe/…` 置前客户端 |
| **CLI 终端** | 祖先含 Windows Terminal / conhost / cmd… | 优先 `focus://window/{hwnd}`；否则 `focus://pid/…?title=…` |
| **无 UI daemon**（Codex `app-server-daemon`） | 祖先无终端宿主 | 发现活着的非 daemon CLI，再按上列 CLI 策略 |

不要对桌面壳发 `focus://pid/…` 再 `AttachConsole`：会匹配错窗，甚至卡住服务线程。
**服务进程**也禁止 `AttachConsole`/`FreeConsole`（会拆掉自身控制台）；按键置前走 `d50-focus`。

```powershell
reg query HKEY_CLASSES_ROOT\claude     # 可能不存在（仅 CLI）
reg query HKEY_CLASSES_ROOT\codex      # 常存在（装了桌面客户端）
```

`focus://` 路径：

- **Windows（有控制台的 hook）**：`GetConsoleWindow` 提升到 WT owner；不 `AttachConsole`。
- **Windows（无控制台 / daemon hook）**：可 `AttachConsole` 到活着的 CLI（跳过
  `app-server-daemon`），再提升到框架窗；失败才发 `focus://pid/{cli…}?title=`。
- **Windows（Windows Terminal / ConPTY）**：同一 `WindowsTerminal` 可有多顶层窗。
  消歧顺序：本品牌标题 → `Action Required` → **中性自定义标题**（如 `aa66621321`）；
  **拒绝** Explorer / 路径标题 / 外品牌（Claude↔Codex）。多中性窗时跟前台，否则失败，
  不乱点任意窗。置前用文档化 Win32（`ShowWindow` / `AttachThreadInput` /
  `SetForegroundWindow` / `SendInput`）。
- **macOS**：`TERM_PROGRAM` 推出终端 App，`open -a` 激活。

`focus://` 与 deeplink 都拿不到时，按钮才退化成 `dismiss`，内容带「（终端）」。
`--no-open-app` 可以强制关掉。

## Consequences

- 4 个键位是硬上限：Claude 三选项时正好占满 KEY2–KEY5（Agree / Ignore / Always / open app），
  再加选项就放不下了。
- 按下任一反馈键，固件会清掉 `NeedsAttention`、按钮随即不可操作。open app 之后服务要
  `touch()` 重新点亮一次，否则剩下两颗裁决键就按不动了。这条已写进服务与测试。
- 提示会占一个会话槽（最多 6 个）；槽位满时直接丢弃，不阻塞任何等待中的审批。
- deeplink 里带着会话 ID：日志里出现链接就等于出现会话 ID，按内部信息处理。
- 设备上看不到规则全文时（>32 字节的按钮名），规则写在内容行；仍然放不下就截断加省略号，
  这是「按钮只说 Always，内容说清楚」这条取舍的代价。