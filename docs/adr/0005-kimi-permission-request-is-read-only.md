# Kimi Code 的 PermissionRequest 不能门控，D50 只做提醒

Kimi Code CLI 的 `PermissionRequest` 事件是用 `fireAndForget` 触发的：它把请求通知给 hook，但**不读取 hook 的返回值**，权限裁决仍然发生在 Kimi 自己的界面里。因此 D50 **不能**做 Agree/Ignore 门控；可以挂 hook，但设备上只发一条「open app」提示，请人回 Kimi 自己处理。

`request_permissions`、选择题等同样没有可回填的裁决通道，走同一条「仅通知」路径。

## Status

accepted

## 证据（2026-10-30，kimi.exe 2026-10-29 构建，161 MB）

```js
this.eventBus.subscribe(PermissionApprovalRequested, (e) => {
  this.fireAndForget("PermissionRequest", inputData, e.toolName);
});
```

官方 Hooks 文档也将 `PermissionRequest` 标为观察型（触发后即发即忘，返回值不影响主流程）。
`fireAndForget` 与 `runPromptSubmitHook` 那种 `await` 的调用不同：前者不消费结果。
Kimi 里唯一能左右执行的钩子入口是 `PreToolUse`，它解析 `hookSpecificOutput.permissionDecision`，
但只把 `"deny"` 变成 `{ action: "block" }`，没有等价的「批准」语义。

## Consequences

- `d50-hook --agent kimi-cli`：**PermissionRequest / 对话类工具 → Notice（open app）+ 立刻 `{}`**，
  绝不 `ApprovalRequest` 阻塞等按键（否则会白等超时，且按了同意也无效）。
- 配置写在 `~/.kimi-code/config.toml` 的 `[[hooks]]`；超时宜短（例如 15s），因为本来就不等人。
- 不做「只能拒绝、不能同意」的半门控：按了同意却没反应，比只提醒更糟（同
  [ADR-0004](0004-dialogs-stay-in-the-terminal.md)）。
- 如果将来 Kimi 把 PermissionRequest 改成同步等待（或 `PreToolUse` 支持
  `permissionDecision: "allow"`），再升级为门控；判断方法就是看当次构建是否还是 `fireAndForget`。
