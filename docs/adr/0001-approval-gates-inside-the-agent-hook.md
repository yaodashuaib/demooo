# 审批门控阻塞在 Agent 的 hook 进程里

Agent 的 `PermissionRequest` hook 是唯一能在工具执行前改变其行为的同步入口，因此 demo 让 hook 进程一直等到设备给出决策再退出，而不是把设备只当通知屏。

## Status

accepted

## Considered Options

- **旁路通知**：设备只显示「有人等你审批」，人回终端按键。实现最快，但设备上没有真正的选择，价值接近于零。
- **阻塞 hook**（本决定）：hook 进程持有等待，最长到该 Agent 的 hook 超时（Codex 配置里已是 600 s，`ulanzi/cli/cli-client/runtime/installers/codex-config.js`），期间设备上按下的按钮直接决定 Agent 是否继续。

## Consequences

- 等待期间 Agent 进程被挂住，服务必须保证每个请求都有截止时刻，且 hook 进程的等待比 Agent 的超时短一个安全余量（默认 5 s）。
- 任何异常（设备掉线、服务崩溃、队列满）都只能回落到「不输出决策」，绝不能自动 `allow`；自动放行会让这条链路变成安全隐患而不是审批。
- 「总是允许」不在这一层实现：hook 只能对本次请求给答案，永久放行属于各 Agent 自己的设置，服务不假装支持。
