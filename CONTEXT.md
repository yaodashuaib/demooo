# D50 审批门控

Agent 在工具执行前停下来等人回答，人按 D50 上的按钮给出答案，答案回到 Agent。本文件只定义这个上下文的语言；协议字段以 `doc/D50与PC的应用层通信协议.md` 为准。多上下文关系见根目录 `CONTEXT-MAP.md`；`AgentIndex`（图标槽）由「D50 设备配置」上下文分配，本上下文只引用。

## Language

**审批请求（Approval Request）**：
Agent 在执行某个工具前发起的一次待人工裁决，具有稳定的 `request_id`、选项集合和截止时刻。
_Avoid_：把 hook 进程的一次调用当成审批请求——同一个请求可能被重试多次。

**决策（Decision）**：
人对一次审批请求给出的结论，只有 `allow` 和 `deny` 两个值。
_Avoid_：把超时、掉线、取消说成「拒绝」——它们没有产生决策，Agent 必须自行回落到终端询问。

**会话槽（Conversation Slot）**：
D50 上显示一条会话的固定位置，最多 6 个，按数组顺序显示。
_Avoid_：把会话槽当成审批的身份；槽位会被复用，`request_id` 才是身份。

**设备投影（Projection）**：
把当前待决审批映射成会话槽内容的计算结果。它随时可以由审批请求表重建，不持有独立事实。
_Avoid_：把设备上显示的内容当作状态真相。

**回填（Report-back）**：
hook 进程把决策翻译成该 Agent 自己的 JSON 并写到 stdout 的动作。翻译规则属于 CLI，不属于服务。
_Avoid_：与设备受理（Accepted）混用——受理是设备对命令的确认，不是人对审批的回答。

**门控（Gate）**：
Agent 在拿到决策前不得继续执行的状态。本上下文只对 `PermissionRequest` 承诺门控，其它确认最多做镜像。
_Avoid_：宣称所有「对话确认」都能门控。

**权限裁决（Permission Decision）**：
Agent 执行某个工具动作前向人索要的允许或拒绝；结论只有 allow 与 deny。D50 只承接这一类。
_Avoid_：把对话确认当成权限裁决。

**对话确认（Dialog Confirmation）**：
Agent 请人回答一个问题或从几个选项里挑一个，不改变「是否执行」。它同样会触发 `PermissionRequest`，但 D50 不接管，一律留在终端。
_Avoid_：把它的「同意」理解成批准了一个动作。

**在线（Online）**：
D50 在连续 5 秒内收到过上位机任一有效帧的状态，服务侧以心跳维持。
_Avoid_：把 HID 枚举成功当作在线。
