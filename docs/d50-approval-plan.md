# D50 审批门控 demo：实现计划

Agent 在工具执行前停下来，D50 屏幕上出现一条带按钮的会话，人按下「同意」或「拒绝」，答案回到 Agent 继续执行。目标是在 `services-rust` 的工程形态下先做出可跑通的 demo，之后能整体搬进 `ulanzistudio-desktop`。

> **实现状态（2026-10-29）**：M0–M4 的服务侧与 hook 侧已落地，`cargo test --workspace` 72 项通过，
> `--simulate` 全链路与真实进程边界的 e2e 均已验证。真机（SN `D0D36I024U3670001`，固件 1.0.0）
> 已验证会话下发、按键上报、决策回填与 hook 输出 allow；真实 Claude Code / Codex 的 hook 接入
> 与 `NeedsAttention` 的视觉过渡仍待验收，见 [README](../README.md) 的「已知边界」。

## 0. 输入与边界

- 设备协议：`doc/D50与PC的应用层通信协议.md`（V0.1，2026-10-22），下文所有命令号与字段都出自这里。
- 现有工程：`D:\workplace\ulanzistudio-desktop\services-rust`（device-service / integration-service / service-contracts / device-sdk-*）与 `ulanzi\cli` 的 hook 运行时。
- 飞书需求文档打不开（跳登录），本计划以以上两份材料为准。若有出入，以补充信息修正。

非目标：不改 D50 固件；不做多设备并发；不做「总是允许」的持久规则；不接 CLI 里那条 HTTP `device-direct` 链路（未在本项目使用）；不做 GUI。

## 1. 这件事难在哪

三个条件必须同时成立，缺一条就不是审批：

1. **Agent 侧能停住。** Codex 的 hook 配置已经给了 `PermissionRequest` 600 秒（`runtime/installers/codex-config.js:156`），Claude Code 的 `PermissionRequest` hook 也是同步阻塞点。窗口是有的，只是现在没人用：`runtime/hooks/mappings.js:142` 的 `stdoutForHook` 对所有事件都直接返回 `{}`。
2. **设备侧能显示和收回答案。** D50 已有会话 + 反馈按钮 + `CONVERSATION_ACTION` 上报，零固件改动。
3. **中间有个可信的等待者。** 谁持有 HID、等到什么时候、掉线怎么办。

现有链路只解决了第 2 条的一半：`agent.hook.report` → outbox → `notification.deliver` → 灯效/弹跳，终态是「设备受理」，没有回程，也不等人。所以这次不是给现有链路加个字段，而是补一条反向的、有阻塞语义的通道。

## 2. 三个决定

| ADR | 决定 | 一句话代价 |
| --- | --- | --- |
| [0001](adr/0001-approval-gates-inside-the-agent-hook.md) | 门控阻塞在 Agent 的 hook 进程里 | 等待期间 Agent 挂住；任何异常只能回落，绝不自动放行 |
| [0002](adr/0002-approval-reuses-d50-conversation-slot.md) | 审批投影成 D50 会话槽，不加固件命令 | 6 槽、4 键、每字段 64 字节；没有「删单条会话」命令 |
| [0003](adr/0003-bridge-owns-d50-over-hid.md) | 独立桥接进程独占 HID | 与未来的 device-service 抢设备，搬入时必须换 owner |

## 3. 形态

### 3.1 数据流

```text
Claude Code / Codex
      │  PermissionRequest hook（stdin: JSON）
      ▼
 d50-hook.exe ──命名管道──►  agent-bridge-service  ──HID 64B──►  D50
      │                            │   ▲                          │
      │                       审批状态机  └──── 0x41 按键上报 ──────┘
      │                            │
      └──── stdout: 该 Agent 的 decision JSON ────► Agent 继续 / 中止
```

服务只回答「人做了什么」，不认识任何 Agent 的 JSON；Agent 的方言由 hook 进程翻译。这条分界是后面能接第二、第三个 Agent 的前提。

### 3.2 crate 布局（镜像 services-rust）

```text
D:\demo\
├── Cargo.toml                      # workspace
├── crates\
│   ├── d50-protocol\               # 纯字节层：帧、CRC16、多帧、各命令 payload
│   ├── d50-hid\                    # 传输层：hidapi、64B report、心跳、ACK/重传、收到即解析
│   ├── approval-core\              # 纯状态机：请求表、槽位分配、投影 diff、截止
│   └── bridge-ipc\                 # 管道上的信封、方法、错误码
├── apps\
│   ├── agent-bridge-service\       # 组合根：管道 + 状态机 + d50-hid
│   └── d50-hook\                   # 被 Agent 调用的短命进程
└── docs\
```

与 `services-rust` 的对应关系：`d50-protocol` ↔ `service-contracts`（只放类型和校验），`d50-hid` ↔ `device-sdk-buddy`（一个 owner 一个线程），`approval-core` ↔ `integration-service` 里的 `agent_feedback.rs`（纯决策、无 IO），`bridge-ipc` ↔ `service-contracts::envelope`。

### 3.3 线程模型

- **HID 线程（1 个）**：独占句柄，循环读 report、按 `SyncFrame + FrameLength` 组帧、把 D2H 事件丢进 channel；写侧从命令队列取帧，按 64 字节切分写出，多帧时逐帧等 ACK。
- **tokio 运行时**：管道服务器 + 审批状态机（单 `Mutex<Approval>`，等价于仓库里 `self.mutation.lock()` 的用法）。
- **定时器**：心跳（空闲 1 s 必发）、请求截止、队列提升。

`d50-hid` 不 `async`，也不认识审批；`approval-core` 不碰 IO。这条边界保证协议和状态机能各测各的。

## 4. 契约

### 4.1 管道方法

`hello` 之后按 `\n` 分隔 JSON，请求/响应都带 `request_id`，错误用 `{code, message}`（复用 `service-contracts` 的错误码风格，不发明 `_error` 信封）。

```json
// approval.request（阻塞，直到有结果或到 deadline）
{
  "request_id": "8f1c…",
  "agent_id": "claude-code",
  "session_id": "s-42",
  "tool_name": "Bash",
  "summary": "rm -rf build && cmake --build build",
  "cwd": "D:/workplace/proj",
  "options": [
    {"index": 1, "label": "同意", "decision": "allow"},
    {"index": 2, "label": "拒绝", "decision": "deny"}
  ],
  "deadline_ms": 55000
}
```

```json
// 有决策
{"request_id":"8f1c…","outcome":"decided","decision":"allow","option_index":1,"decided_at":"2026-10-28T11:02:13.204Z"}
// 其它终态，都没有 decision 字段
{"request_id":"8f1c…","outcome":"expired"}
{"request_id":"8f1c…","outcome":"unavailable","reason":"device_offline"}
{"request_id":"8f1c…","outcome":"cancelled"}
```

`outcome` 只有五种：`decided` / `expired` / `unavailable` / `cancelled` / `rejected`（参数或队列满）。服务**不**输出 `deny` 来表达超时——这是 ADR-0001 的直接后果，也是防止「自动拒绝把 Agent 卡死」的关键。

其它方法：`approval.status`（按 `request_id` 读终态，供 hook 重连用）、`approval.cancel`、`device.status`、`device.self_test`（下发一条测试会话）、`shutdown`。

### 4.2 D50 命令映射

| 时机 | 命令 | 说明 |
| --- | --- | --- |
| 连上设备 | `0x01 GET_DEVICE_INFO` | 以 `DeviceType == "D50"` 认设备，**不**写死 VID/PID（协议未定义） |
| 连上设备 | `0x02 HEARTBEAT` | 之后空闲 1 s 必发；5 s 没帧设备判离线 |
| 连上设备 | `0x03 TIME_CONFIG` | 时钟页需要；每分钟或系统时间变化再发 |
| 连上设备 | `0x10 SET_PAGE_CONFIG` | 确保 `tasks` 在页面列表里，否则会话页不显示 |
| 连上设备 | `0x20 SET_AGENT_INDEX` | 每个 Agent 一张 100×100 PNG；M1 可先只发一张占位图 |
| 连上设备 | `0x21 SET_CONVERSATION_LIST` | 全量同步当前投影，重连与每次决策后都用它 |
| 新增/更新一条 | `0x22 UPSERT_CONVERSATION` | 新 ID 插首位；已有 ID 原位替换 |
| 撤下/重排 | `0x21 SET_CONVERSATION_LIST` | 协议没有删单条的命令，只能全量重写 |
| 收到按键 | `0x41 CONVERSATION_ACTION` | `{ID, ButtonIndex}`，`ButtonIndex=0` 是「打开会话」，不是选择 |

### 4.3 请求投影成会话

| 会话字段 | 取值 | 约束 |
| --- | --- | --- |
| `ID` | `request_id` | 1–64 字节，UUID 直接可用 |
| `Title` | `<Agent> · <ToolName>` | ≤64 UTF-8 字节 |
| `Path` | `cwd` | ≤64 UTF-8 字节 |
| `Content` | `summary` 第一行 | ≤64 UTF-8 字节 |
| `AgentIndex` | 该 Agent 的图标索引 | 0–5 |
| `NeedsAttention` | `1` | 为 0 时按钮不可操作 |
| `Buttons` | `[{1,"同意"},{2,"拒绝"}]` | Index 1–4，≤4 项 |

截断规则照抄 `integration-service/src/hooks.rs` 的 `clipped()`：按 UTF-8 字符边界回退，不把中文截成半个字节。超长时在末尾留 `…`（3 字节）再算上限。

## 5. 状态机

```
received ──► queued ──► shown ──► decided ──► reported
   │            │         │          │
   │            └─────────┴──────────┴──► expired / cancelled / unavailable
```

- **请求表是唯一真相**，投影可重建。设备上显示什么永远由「当前 shown 集合」算出来，不从设备回读。
- **槽位分配**：最多 6 条 `shown`，按到达顺序先进先出。第 7 条进 `queued`，**不下发**——若直接 `UPSERT`，设备会挤掉索引 5 的会话，那条未决请求就无声消失了。
- **决策只产生一次**：`decided` 是终态，后续同名按键、迟到 ACK、重复 `approval.status` 都返回同一个结果。这正是仓库里「不把迟到 ACK 当新响应」的同一条纪律。
- **撤下一刻**：先 `UPSERT` 该条 `NeedsAttention=0`（灯灭、按钮失效），再 `SET_CONVERSATION_LIST` 重写剩余集合。两步都在同一次投影提交里算好，避免设备与服务的状态短暂分叉。

## 6. 失败矩阵

| 情况 | 设备上 | 服务 | hook 输出 | 为什么这样 |
| --- | --- | --- | --- | --- |
| 设备掉线 / 5 s 无帧 | 无 | 全部 `shown` 转 `unavailable` | 无 decision，exit 0 | 没有设备就不能假装有人批准；Agent 回落到自己的终端询问 |
| 到达 deadline | 撤下该条 | `expired` | 无 decision | 同上 |
| 第 7 个请求 | 不下发 | 排队；到期仍未显示则 `expired` | 无 decision | 不能静默挤掉别人 |
| 同一按钮连按 | 设备已置 `NeedsAttention=0` | 只认第一次 | 同一次决策 | 幂等 |
| hook 进程被杀 | 会话照旧 | 请求保留到 deadline，结果仍缓存 | — | Agent 可能重试同一 `request_id`，重试要拿到同一个答案 |
| 服务崩溃 | 会话残留在设备上 | 下次启动全量 `SET_CONVERSATION_LIST` 覆盖 | 无 decision | 协议没有「清空会话」以外的恢复手段 |
| 同名 `request_id` 不同参数 | — | `rejected` + `IDEMPOTENCY_CONFLICT` | 无 decision | 防止两个 Agent 撞 ID |
| 设备被别的进程占用 | — | `BUDDY_ALREADY_OWNED` 风格的 `DEVICE_BUSY`，不抢占 | 无 decision | 与 Buddy 的互斥纪律一致 |

## 7. 能力边界（先讲清楚，别在验收时才发现）

- **能门控**：`PermissionRequest`，且该事件必须**同步等待** hook 的决定。目前确认可用的有 Claude Code、Codex（2026-10-29 实测）与 WorkBuddy（官方 Hooks 文档含同步决策）。`kimi-cli` 的 `PermissionRequest` 是 `fireAndForget`，不读决定——D50 **只发 open app 提醒**，不做 Agree/Ignore（见 [ADR-0005](adr/0005-kimi-permission-request-is-read-only.md)）。选项只有「同意 / 拒绝」，与协议按钮一一对应。
- **不接管**：`AskUserQuestion` 这类「对话确认」。它也会触发 `PermissionRequest`（调用工具本身要许可），但产物是一个问题而不是一个动作——设备上按「同意」只是允许提问，随后屏幕上还会留一个设备答不了的选择菜单（2026-10-29 实测）。`d50-hook` 在工具名层面直接放行回终端，不建会话。见 [ADR-0004](adr/0004-dialogs-stay-in-the-terminal.md)。
- **不做**：永久放行 / 「不再询问」。hook 只能回答本次请求，写规则属于 Agent 自己的设置，服务不假装支持。
- **不做**：多个 Agent 同时审批时的优先级仲裁。先进先出，同一时刻只有 6 条可见。

## 8. 里程碑

每个里程碑都以「可观察的结果」收尾，不以「代码写完」收尾。

### M0 协议层（1–2 天）

交付：`d50-protocol` + `d50-hid` 的最小握手；单测覆盖帧编解码、CRC、多帧。

判据：
1. 对真机发 `0x00` 收到 `00 01`；发 `0x01` 收到 `DeviceType = "D50"` 的 JSON。
2. 连续 60 s 只发心跳，设备不判离线。
3. 拔出重插能重新枚举；第二个打开者被互斥拒绝。
4. 单测里一个帧在 64 字节边界上每一种切分都能拼回原帧。

必须先做的一个实验：**hidapi 在「无 Report ID」设备上是否要补一个 `0x00` 前缀**。`buddy_report` 里 Report ID 与帧头共用 `0x55`，D50 是另一套规则，写错了表现为设备完全不回帧。这个结论要写进 `d50-hid` 的注释，别让下一个人再试一遍。

### M1 会话投影与按键（2 天）

交付：能往设备推一条会话并收到按键。

判据：
1. 推一条 `NeedsAttention=1` 的会话，设备上出现标题、内容、两个按钮。
2. 按「同意」，服务收到 `0x41 {ID, ButtonIndex:1}`；按「拒绝」收到 2。
3. 按完之后设备上该会话的 `NeedsAttention` 变 0、按钮不可再按。
4. 全量 `SET_CONVERSATION_LIST` 能把「已处理」的会话从设备上撤掉。

### M2 门控服务（2–3 天）

交付：`approval-core` + `bridge-ipc` + `agent-bridge-service`，可用测试客户端完整驱动。

判据：
1. 测试客户端发 `approval.request`，按同意 → 返回 `decided/allow`；按拒绝 → `decided/deny`。
2. 不发请求时手按设备按钮，服务忽略而不是崩。
3. 截止到期 → `expired`，设备上会话被撤下。
4. 等待中拔线 → `unavailable`，恢复连接后能重新服务新请求。
5. 连发 7 个请求：第 7 个排队，前 6 个按顺序显示；`device.status` 能看到 `queued=1`。
6. 杀服务再起：设备上的残留会话被全量同步覆盖，不出现幽灵按钮。

### M3 接进 Agent（1–2 天）

交付：`d50-hook` + Claude Code / Codex 的 hook 配置样例。

判据：
1. 在 Claude Code 里触发一次需要授权的 Bash，设备亮起「同意 / 拒绝」。
2. 按同意 → Agent 继续执行；按拒绝 → Agent 中止该工具且不报错。
3. 不按，等到 hook 超时 → Agent 自己的终端询问照常出现（回落可用）。
4. Codex 同一条路径复验一次（它给的是 600 s，余量足够）。
5. `stdoutForHook` 改成按结果输出后，其它事件（`Stop`、`Notification` 等）的行为不变。

### M4 硬化与搬入准备（2 天）

交付：队列与重连的边界测试、诊断日志（沿用 `services-rust` 的 JSONL 风格）、`--simulate` 假设备模式、README，以及 §11 的改动清单。

判据：`cargo test --workspace` 全绿；`--simulate` 下无需真机跑完 M2 的全部判据。

## 9. 测试策略

| 层 | 方式 | 覆盖 |
| --- | --- | --- |
| `d50-protocol` | 纯单测 + 属性测试 | 帧往返、CRC 向量、多帧序号回绕 63→0、非法帧静默丢弃、长度边界 8 / 65535 |
| `d50-hid` | 假 Transport 实现 `D50Link` | 短写、读超时、ACK 丢包重传、迟到 ACK 不算新响应 |
| `approval-core` | 纯状态机测试，无 IO | 槽位分配、投影 diff、幂等、截止与队列提升 |
| 端到端 | `d50-hook` ↔ 服务 ↔ 假设备 | 上表失败矩阵逐行一条用例 |
| 真机 | 手工清单 | M0–M3 判据；沿用 BUDDY-M1 的做法，写入 evidence 目录 |

真机测试与工作区测试分开记录，`accepted` 只代表设备回执，物理按键必须现场确认——这和仓库里 Buddy 的既有纪律一致。

## 10. 风险与待验证

1. **HID 报告前缀**（M0 实验）：错一字节就是完全无响应，最容易浪费半天。
2. **多帧 + ACK 的时序**：写和读必须在同一线程，否则 ACK 交错；这是 `buddy.rs` 已经踩过的坑。
3. **Agent hook 的方言**：Claude Code 的决策 JSON 外壳与 Codex 不同，且各版本可能变。hook 层要按 Agent 分派，并且**默认不输出**，只在拿到 `decided` 时才输出。
4. **超时预算**：Codex 给 600 s，Claude Code 的 hook 超时要小得多（默认值以 M3 实测为准）。`deadline_ms` 由 hook 按「该 Agent 的超时 − 5 s」算出，服务不自己猜。
5. **设备在线判定**：5 s 无帧即离线，Windows 休眠/唤醒会瞬间触发。重新上线必须先全量同步再接受新请求。
6. **安全**：本机同用户进程都能连管道并冒充 hook。demo 接受，正式接入要连同服务一起设计（见 ADR-0003）。

## 11. 搬进 ulanzistudio-desktop 的改动清单

目标不是「复制代码」，而是让 owner 和契约各归其位。

1. **HID owner 换人**：`d50-hid` 改成 `crates/device-sdk-d50`，实现 `device-sdk-api` 的 `DeviceSdk`，在 `apps/device-service` 注册。桥接进程不再直接开 HID，改为调 device-service。互斥问题随之消失。
2. **会话命令进契约**：`service-contracts` 增加 `conversation.upsert` / `conversation.list.set`，`studio_to_device_service` 对应方法；`0x41` 上报映射成既有的 `input_event`（`control_id: "d50.conv.<slot>.button<n>"`）。
3. **审批进 integration-service**：`approval-core` 作为 `crates/approval-core` 被 integration-service 引用，新增 `agent.approval.request`（阻塞）与既有 `agent.hook.report` 并存；`cli_to_integration_service` 加方法，`integration_service_to_cli` 加结果。
4. **CLI 改回填**：`ulanzi/cli` 的 `stdoutForHook` 从同步函数变成 `await approval`，按 Agent 输出各自方言；`codex-config.js` 的 600 s 与 Claude Code 的超时在这里对齐。
5. **三处同步**：C++ 结构体与 JSON 序列化、Rust serde 模型、`service-protocol/docs` 与 `apps/*/README.md`，按 AGENTS.md 的规矩一起改。
6. **删除**：demo 自己的命名管道与 owner 互斥，搬入后不再保留第二条通路。

## 附录：帧速查

```text
SyncFrame(2)=0x55 0xD5 | CommandID(1) | FlagsSeq(1) | FrameLength(2, LE) | Payload(N) | CRC16(2, LE)

FlagsSeq = (FrameType << 6) | FrameSeq      FrameType: 00 COMPLETE / 01 FIRST / 10 MIDDLE / 11 END
FrameLength = 6 + PayloadLength + 2         CRC-16/CCITT-FALSE (poly 0x1021, init 0xFFFF, no reflect)
```

- ACK 是空 Payload 的同 `CommandID` + 同 `FlagsSeq` 帧，`FrameLength = 8`；只对通过校验且已处理的帧回 ACK，其余静默丢弃。
- Vendor HID 有效区 64 字节、无 Report ID；应用帧可跨 report，接收方只认 `SyncFrame + FrameLength`。
- `0x02 HEARTBEAT` Payload 1 字节，`0x00 = studio`；连接建立后立即发一次。
