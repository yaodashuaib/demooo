# D50 审批门控 demo

Agent 在工具执行前停下来，D50 屏幕上出现一条带按钮的会话，人按下「同意」或「拒绝」，答案回到 Agent 继续执行。

计划与决策见 [实现计划](docs/d50-approval-plan.md)、[术语表](CONTEXT.md)、[ADR](docs/adr/)。

## 形态

```text
Claude Code / Codex
      │  PermissionRequest hook（stdin: JSON）
      ▼
  d50-hook  ──命名管道──►  agent-bridge-service  ──HID 64B──►  D50
      │                          │    ▲                        │
      │                     审批状态机  └── 0x41 按键上报 ──────┘
      └──── stdout: 该 Agent 的 decision JSON ────► Agent 继续 / 中止
```

服务只回答「人做了什么」，不认识任何 Agent 的 JSON；Agent 的方言由 hook 翻译。

| crate | 职责 | 不做什么 |
| --- | --- | --- |
| `crates/d50-protocol` | 帧、CRC16、多帧、命令 Payload | 不碰 HID/时间/进程 |
| `crates/d50-hid` | 一个线程独占设备：心跳、ACK/重传、组帧 | 不认识审批 |
| `crates/approval-core` | 请求表、6 个会话槽、投影、截止 | 不碰 IO |
| `crates/bridge-ipc` | 命名管道上的 NDJSON 契约 | 不做业务判断 |
| `apps/agent-bridge-service` | 组合根 + 设备工作线程 + 配置投影 | |
| `apps/d50-hook` | 被 Agent 调用，翻译方言 | 不做决策 |
| `crates/config-core` | Linked App → 图标槽 / 余额卡片 / 声光草稿 | 不碰 HID |
| `apps/d50-config-client` | Qt 配置上位机（IPC 客户端） | 不打开 HID |

## 没有设备时跑一遍

```powershell
# 终端 A：内存版 D50
cargo run --bin agent-bridge-service -- --simulate

# 终端 B：扮演 Agent 的 hook（会阻塞，等按键）
'{"session_id":"s1","tool_name":"Bash","tool_input":{"command":"rm -rf build"},"cwd":"D:/work"}' |
  cargo run --bin d50-hook -- --agent claude-code --request-id demo-1 --timeout-ms 60000

# 终端 C：代替人按下「同意」
cargo run --bin d50ctl -- press --id demo-1 --button 1
```

`d50ctl` 是服务的小客户端，三个子命令：`status`（在线/显示/排队）、`press`（仅 `--simulate`）、
`self-test`（推一条不阻塞任何人的演示会话）。

终端 B 会打印：

```json
{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}
```

按 `--button 2` 出 `deny`；不按，等到截止时刻，终端 B 打印 `{}` 并 exit 0——Agent 回落到自己的终端询问。

## 接真机

### 1. 先看设备，不用猜序列号

```powershell
cargo run --features hid --bin agent-bridge-service -- --list-devices
```

```text
idx vid:pid    serial              iface  product
0   303A:4005  E0D38I026U3670001   0      D50
1   303A:8001  -                   0      Ulanzi Deck

idx 0 → D50 序列号 E0D38I026U3670001 固件 1.0.3.0（Report 前缀 0x00）
idx 1 → 不是 D50：握手失败（handshake failed）：这大概率不是 D50，或 Report 前缀要换一下
```

这条命令做三件事：列出所有 Vendor HID（usage page `0xFF00`）、逐个握手确认是不是 D50、
并且**两个 Report 前缀方向都试一遍**，直接告诉你哪个能用。

序列号是设备在握手时自报的，不是从设备管理器或包装盒上查来的；很多 HID 设备干脆不上报序列号，
那一列的 `-` 就是正常现象。

### 2. 启动服务

```powershell
cargo run --features hid --bin agent-bridge-service
```

- **不用传序列号**：默认打开第 0 个候选，再用握手自报的 `DeviceType == "D50"` 确认身份。
- 只有在同一台机器上有多台候选、又需要指定其中一台时，才用 `--serial E0D38…`；
  如果那台设备不上报序列号，改用 `--device-index 1`（序号来自 `--list-devices`）。
- 如果 `--list-devices` 说「Report 前缀 不需要」，启动时加 `--no-report-id-prefix`。

### 3. 冒烟

```powershell
# 推一条不阻塞任何人的演示会话：设备上应出现「同意 / 拒绝」
cargo run --bin d50ctl -- self-test

# 看服务是否认为设备在线，以及是否还有未处理的请求
cargo run --bin d50ctl -- status
```

真机上如果设备完全不回帧，优先跑 `d50-probe --sweep` 看原始收发。

> **2026-10-29 实测**：手头这台 D50（`303A:83D8`，SN `E8F60AE7A976`）的厂商集合
> 在**两个方向上都没有响应**——18 种帧摆法全无回音，按键 80 秒内不产生输入报告，
> 连「预览提示灯」都不亮。报告 ID 与报告长度已经用 Windows HID API 核对过，与协议一致，
> 不是上位机的问题。证据与给固件的问题清单见
> [D50 真机联调记录](docs/d50-hardware-findings.md)。

## 接 Agent

示例配置见 [hooks 配置](docs/hooks.md)。要点只有一个：hook 的 `timeout` 必须比
`--timeout-ms` 大几秒，给回落留余量。

## 验证

```powershell
cargo test --workspace
```

| 层 | 覆盖 |
| --- | --- |
| 协议 35 项 | CRC 标准向量、帧往返、任意切分点组帧、坏帧重同步、多帧序号回绕、UTF-8 截断 |
| 状态机 18 项 | 第 7 个排队、决策只记一次、超时不算拒绝、掉线只影响显示中、幂等 ID、open app 不裁决、提示占槽与过期 |
| HID 会话 6 项 | 握手按协议认设备、会话下发、按键事件、多帧、拔线 |
| 服务 e2e 7 项 | 同意/拒绝/超时/排队/非法按钮/提示/打开 App，全部走真实命名管道 |
| hook e2e 2 项 | 真实 hook 进程输出 allow；服务不在时输出 `{}` 并 exit 0 |

## 已知边界

- **真机已验证**（2026-10-29，SN `D0D36I024U3670001`，固件 1.0.0）：会话下发、屏幕显示、
  按键上报、决策回填、hook 输出 allow，整条链路在真实设备上跑通。Claude Code 的
  `PermissionRequest` 与 Codex 的 hook 均已接上；`NeedsAttention` 的界面过渡尚未细看。
  排查记录见 [D50 真机联调记录](docs/d50-hardware-findings.md)。
- **设备只裁决这一次**：四个键位里扣掉裁决键，剩下的一颗是 `open app`（deeplink 打开
  Agent 自己的会话页）。Codex 两个按钮，Claude 在有建议规则时三个（Agree / Ignore / Always），
  见 [ADR-0007](docs/adr/0007-open-app-and-notices.md)。
- **对话类请求只提醒、不裁决**：`AskUserQuestion`、`request_user_input`、`request_permissions`
  在设备上只显示「有 N 个问题待您回复…」+ 一颗 open app，hook 立即返回（见
  [ADR-0004](docs/adr/0004-dialogs-stay-in-the-terminal.md)、[ADR-0006](docs/adr/0006-request-permissions-stays-in-the-desktop.md)）。
- **Kimi Code 仅通知**：`PermissionRequest` 是 `fireAndForget`，不读 hook 决定；D50 只推
  open app 提醒，裁决仍在 Kimi 界面（见 [ADR-0005](docs/adr/0005-kimi-permission-request-is-read-only.md)）。
- **WorkBuddy 可门控**：官方 PermissionRequest 支持决策回读时，与 Claude 同形回填；
  `request_permissions` 等仍只提醒（见 [hooks.md](docs/hooks.md)）。
- **「总是允许」只在 Claude 上做**：靠 `permission_suggestions` → `updatedPermissions`。
  Codex 该字段会让整条决定作废，所以永远只有两个按钮；我们也不自己造规则（见 ADR-0006）。
- **不做多设备**：一个服务实例只持有一台 D50。
- **掉线不自动重连**：设备拔掉后显示中的请求全部转 `unavailable`，需要重启服务重新握手。
- **决定后直接从会话列表移除**：计划里的「先置 NeedsAttention=0 再撤下」被简化成一次全量
  `SET_CONVERSATION_LIST`；真机上如果觉得消失得太突兀，再补两段式。
- **威胁模型**：本机同用户进程都能连管道冒充 hook。demo 接受，正式接入要随服务一起设计。
