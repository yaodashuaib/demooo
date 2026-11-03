# 把 d50-hook 接进 Agent

三条规矩：

1. hook 的 `timeout` 必须比 `--timeout-ms` 大几秒，否则 Agent 先杀掉 hook，回落都来不及。
2. 只挂 `PermissionRequest`。其它事件挂上来没有任何意义——本 demo 不解析它们。
3. 命令里写绝对路径，Agent 的工作目录不可预期。

> `d50-hook` 会跳过对话类工具（`AskUserQuestion`、`ExitPlanMode`），直接输出 `{}` 交回终端。
> 它们同样触发 `PermissionRequest`，但设备上按「同意」只是允许提问，随后仍要人在终端里选。
> 见 [ADR-0004](adr/0004-dialogs-stay-in-the-terminal.md)。

## Claude Code

`~/.claude/settings.json`（或项目内 `.claude/settings.json`）：

```json
{
  "hooks": {
    "PermissionRequest": [
      {
        "matcher": "*",
        "hooks": [
          {
            "type": "command",
            "command": "D:/demo/target/debug/d50-hook.exe",
            "args": ["--agent", "claude-code", "--timeout-ms", "55000"],
            "timeout": 60
          }
        ]
      }
    ]
  }
}
```

`timeout` 以秒计，60 秒对应 `--timeout-ms 55000`。

### PreToolUse：只发提示，不阻塞

对话类请求（`AskUserQuestion`、`request_user_input`、`request_permissions`）没有可以回填的裁决，
让 hook 挂上去只是提醒人回 App 处理。用一个窄 matcher 挂同一条命令：

```json
{
  "name": "d50-notice",
  "matcher": "AskUserQuestion|request_permissions|request_user_input",
  "hooks": [
    {
      "type": "command",
      "command": "D:/demo/target/debug/d50-hook.exe",
      "args": ["--agent", "claude-code"],
      "timeout": 15
    }
  ]
}
```

设备上会出现一条「有 N 个问题待您回复…」，只有一颗 `open app` 键，按下去用
`claude://code/continue?session=local_{session_id}` 打开会话页。细节见
[ADR-0007](adr/0007-open-app-and-notices.md)。

> **Claude 会双事件**：同一 `AskUserQuestion` 先 `PreToolUse`、再 `PermissionRequest`。
> `d50-hook` 只在 `PreToolUse`（`--event PreToolUse`）推送提示；`PermissionRequest` 上遇到
> 问题类工具直接 `{}` 放行，避免 D50 连响两次。PermissionRequest 的 hook 也请带上
> `--event PermissionRequest`。

> `claude://` 只有装了**桌面客户端**才注册。CLI 环境里 hook 查不到这个 scheme 就不给
> open app：提示照发，按钮变成 `dismiss`，内容带「（终端）」。审批请求同理，不会多出一颗
> 按了没反应的键。`--no-open-app` 可强制关闭。

> **Windows 上必须用 exec form**（`command` 只写程序路径，参数放 `args`）。
> 没装 Git Bash 时，不带 `shell` 的 hook 会交给 PowerShell 执行，而
> `"command": "\"C:/path/d50-hook.exe\" --agent ..."` 这种「引号开头的整串命令」
> PowerShell 会**原样打印**而不是运行——hook 静默失效，D50 上什么都不会出现。
> 错误信息是 `PowerShell started no program`。

> **同一事件别挂第二个会抢答的 hook**。`ulanzi-studio` 的 CLI hook 在
> `PermissionRequest` 上立即返回 `{}`（无决策）；和阻塞式 hook 并存时，
> 谁的决定生效取决于 Claude Code 的聚合顺序。要门控就只留 `d50-hook` 一条。

## Codex

Hook 写在 `~/.codex/hooks.json`（`ulanzi/cli/cli-client/runtime/installers/install.js:55`），
并且要在 Codex 里批准一次 hook 命令才能生效。Codex 给 `PermissionRequest` 的超时是 600 秒
（`codex-config.js:155`）：

```json
{
  "PermissionRequest": [
    {
      "name": "d50-approval",
      "matcher": "*",
      "hooks": [
        {
          "type": "command",
          "command": "cmd /d /s /c \"\"D:/demo/target/debug/d50-hook.exe\" --agent codex --timeout-ms 590000\"",
          "timeout": 600
        }
      ]
    }
  ]
}
```

> **每条要有自己的 entry**。把 `d50-hook` 塞进既有的 `ulanzi-studio` entry 里当第二条命令时，
> Codex 的 `config.toml` 只会给第一条写 `enabled = true`——第二条是
> `[hooks.state.'…:permission_request:0:1']`，只有 `trusted_hash`，没有 `enabled`，
> 于是它**从来不会运行**，表现就是「批准了 hook，但 D50 没有反应」。
> 单独成 entry 后重新批准一次（或 `/hooks` 里启用）即可。

> **`codex` 与 `claude` 方言输出同一种结构**：`hookSpecificOutput.decision.behavior`，
> 取值 `allow` / `deny`（取自 Codex 内置 schema 的 `PermissionRequestDecisionWire` 与
> `PermissionRequestBehaviorWire`）。Codex 明确规定 `updatedInput`、`updatedPermissions`、
> `interrupt: true` 会让 hook **整条决定作废**（`decision = None`，回落它自己的审批），
> 所以 Codex 永远只发 `behavior`，也永远只有两个按钮。

### PreToolUse：question 类请求只发提示

`request_permissions` / `request_user_input` / `AskUserQuestion` 在 Codex 里是**工具调用**，不是
`PermissionRequest`；`PreToolUse` 只能 `deny`、不能批准。所以不要在这条路上等设备裁决，挂一个
窄 matcher 的提示 hook 即可（非阻塞，按 `codex://threads/{session_id}` 跳回会话页）：

```json
{
  "name": "d50-notice",
  "matcher": "request_permissions|request_user_input|AskUserQuestion",
  "hooks": [
    {
      "type": "command",
      "command": "cmd /d /s /c \"\"D:/demo/target/debug/d50-hook.exe\" --agent codex\"",
      "timeout": 15
    }
  ]
}
```

细节见 [ADR-0006](adr/0006-request-permissions-stays-in-the-desktop.md) 与
[ADR-0007](adr/0007-open-app-and-notices.md)。

## Kimi Code CLI

**仅通知，不门控**（[ADR-0005](adr/0005-kimi-permission-request-is-read-only.md)）。
`PermissionRequest` 是 `fireAndForget`：通知 hook，但不读返回值；权限仍在 Kimi 界面裁决。
`d50-hook` 收到后只推一条带 open app 的提示并立刻 `{}`，**不会**阻塞等 Agree/Ignore。
`request_permissions` / 选择题等同理。

```toml
# ~/.kimi-code/config.toml
[[hooks]]
event = "PermissionRequest"
matcher = ".*"
command = "D:/demo/target/debug/d50-hook.exe --agent kimi-cli --event PermissionRequest --timeout-ms 15000"
timeout = 15
```

也可为 `request_permissions` 等工具名收窄 matcher。不要指望按键能批准——那是 Kimi UI 的事。

## WorkBuddy

WorkBuddy（腾讯云 CodeBuddy 系）用户级 hooks：`~/.workbuddy/settings.json`。
官方 Hooks 文档含 `PermissionRequest`，决策形态与 Claude 对齐
（`hookSpecificOutput.decision.behavior`）。`--agent workbuddy` 时：

- **PermissionRequest（动作类工具）**：阻塞门控，Agree / Ignore 回填（方言 `claude`）。
- **`request_permissions` / `AskUserQuestion` / `request_user_input`**：只提醒 + open app
  （`workbuddy://chat/{session_id}` 或 CLI focus），与 Claude/Codex 相同。
- **PreToolUse**：不阻塞（立刻 `{}`），避免误挂。

> **WorkBuddy 桌面实测（2026-11）**：
> 1. `HookExecutor` 日志只写 `cmd=…d50-hook.exe`，**不传 settings 的 `args` 数组**。
>    参数必须写进 `command` 一整串，否则会落到默认 `--agent claude-code`（标题变
>    「Claude Code」、deeplink 也错）。
> 2. `AskUserQuestion` 在 `waiting_for_permission` 用客户端原生选题 UI，**此阶段不跑**
>    用户 `PermissionRequest` hook；人选完后才 `TOOL_STARTED` → `PreToolUse`。
>    弹层一出来就亮 D50 **做不到**（WorkBuddy 不回调）；`PreToolUse` 仍会推
>    「WorkBuddy · 待回复」，多半已答完，仅用来验 open app / 品牌。
> 3. 无害 `Bash`（如 `node -e …`）常走 **sandbox skip 8-Phase**，根本不进
>    `PermissionRequest`，D50 不会出现同意/拒绝——要测门控需关掉沙箱或换会弹权限的命令。
> 4. 宿主识别含 `WorkBuddy.exe`：桌面客户端走 `workbuddy://`（已注册）或
>    `focus://exe/WorkBuddy.exe`，不应再落到 `focus://window/…` 的 CLI 路径。

```json
{
  "PermissionRequest": [{
    "matcher": "*",
    "hooks": [{
      "type": "command",
      "command": "D:/workplace/demo/target/debug/d50-hook.exe --agent workbuddy --event PermissionRequest --timeout-ms 55000"
    }]
  }],
  "PreToolUse": [{
    "matcher": "request_permissions|request_user_input|AskUserQuestion",
    "hooks": [{
      "type": "command",
      "command": "D:/workplace/demo/target/debug/d50-hook.exe --agent workbuddy --event PreToolUse"
    }]
  }]
}
```

若某版本 PermissionRequest 又不读 hook 决定了，会退化成「按了没用」——那时应改成与 Kimi
相同的仅通知策略；判断方法是看官方是否仍文档化同步决策回读。

## 参数

| 参数 | 作用 |
| --- | --- |
| `--agent` | 决定方言与默认超时：`claude-code` / `codex` / `workbuddy` / `kimi-cli` / 其它 |
| `--dialect` | 显式覆盖方言：`claude` / `codex` / `simple`（workbuddy 默认 `claude`） |
| `--timeout-ms` | 本次最多等多久；省略时按方言取默认（claude/simple/workbuddy 55 s，codex 590 s） |
| `--request-id` | Agent 重试同一次请求时复用，服务返回同一个答案 |
| `--pipe` | 服务不在默认管道上时指定 |
| `--dry-run` | 只打印将要发送的请求，便于确认字段 |

## 方言对照

| 方言 | allow | deny | 拿不到决策 |
| --- | --- | --- | --- |
| `claude` | `hookSpecificOutput.decision.behavior = "allow"` | `"deny"` | `{}` |
| `codex` | 同上 | 同上 | `{}` |
| `simple` | `{"decision":"allow"}` | `{"decision":"deny"}` | `{}` |

`{}` 的意思是「我没意见」：Agent 会走它自己的询问流程。任何异常路径都不会自动放行。
