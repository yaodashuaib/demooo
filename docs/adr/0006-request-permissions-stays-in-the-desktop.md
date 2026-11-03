# `request_permissions` 留在桌面，D50 不接

Codex 0.16x 的 `request_permissions` 是一个**工具调用**，不是 `PermissionRequest` 事件：模型在主
动申请沙箱权限（`network` / `file_system.read` / `file_system.write`，`scope: "turn"`），弹窗是这个
工具自己的界面，用户的选择直接写进工具返回值。钩子侧只能看到 `PreToolUse` / `PostToolUse`，而
`PreToolUse` 没有「批准授权」的语义，只有 `deny`。

## Status

accepted

## 证据（2026-10-30，codex-cli 0.165.0-alpha.1.2）

用独立 `CODEX_HOME` 跑 `codex exec --json --enable request_permissions_tool`，三个事件都挂了
`matcher: "*"` 的日志钩子，模型只调用了一次 `request_permissions`：

```
PreToolUse   tool_name=request_permissions tool_input={"permissions":{"network":{"enabled":true}},"reason":"probe"}
PostToolUse  tool_name=request_permissions tool_response={"permissions":{"network":null,...},"scope":"turn"}
```

**`PermissionRequest` 一次都没有触发**。同日用户交互会话的 transcript 也印证了这条路径：工具返回值
里带 `{"permissions":{"network":{"enabled":true}},"scope":"turn"}`，而 Codex 的日志里是独立的
`operation="request_permissions_response"`，走的是和审批（approval）不同的 op。

钩子能对 `PreToolUse` 做的决定：

| `permissionDecision` | Codex 行为 |
| --- | --- |
| `deny` | 拦下这次工具调用（模型收到被拒） |
| `allow` | 只在配合 `updatedInput` 改写入参时有效，**不是**授权 |
| `ask` | 解析但未实现（[openai/codex#28437](https://github.com/openai/codex/issues/28437) 仍开着） |

## Consequences

- `d50-hook` 继续只挂 `PermissionRequest`；不要挂 `PreToolUse`。
- `request_permissions` 批的是**整个 turn 的网络 / 文件权限**，不是一条命令。设备上只有一行摘要和
  四个键，看不到 scope、路径、域名——这种「扩权」必须留在能看到完整上下文的桌面上。
- 不要做「只能拒绝、不能同意」的半门控（同 [ADR-0004](0004-dialogs-stay-in-the-terminal.md) /
  [ADR-0005](0005-kimi-permission-request-is-read-only.md)）。
- 若把 `d50-hook` 误挂到 `PreToolUse`：它不认事件名，会阻塞到 590 s 等设备按键，然后把
  `hookSpecificOutput.hookEventName = "PermissionRequest"` 的决定写到 `PreToolUse` 上，Codex 不认。
- 将来能接的信号：Codex 支持 `PreToolUse: ask`（把调用路由进原生审批），或 `request_permissions`
  改走 `PermissionRequest`。判断方法是看当次二进制的错误串里还有没有
  `PreToolUse hook returned unsupported permissionDecision:ask`。