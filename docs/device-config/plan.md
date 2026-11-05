# D50 设备配置客户端 — 已决议计划

依据 `doc/D50 功能实现1.md`，经 grilling 钉死。术语见 [CONTEXT.md](./CONTEXT.md)；总图见根目录 [CONTEXT-MAP.md](../../CONTEXT-MAP.md)。

## 形态

```text
apps/d50-config-client (Qt)          其它上位机
        │                                    │
        └──────── named pipe (PROTOCOL=1) ───┘
                         │
              agent-bridge-service
                 │    配置投影循环（上线立即 / 每 180s）
                 │    与审批共用 HID 写队列（Stop-and-Wait）
                 ▼
                D50
```

## 关键决定

| 主题 | 决定 | ADR / 文档 |
| --- | --- | --- |
| HID / 进程 | 上位机不碰 HID；bridge 独占 | 0003, 0008 |
| 领域 | 审批与设备配置分上下文；共享 AgentIndex | CONTEXT-MAP |
| 真相源 | `linked-apps.json` 的 aiAgent；本 Qt 只读不写绑定 | CONTEXT |
| 可投影集合 | allowlist ∩ `status==configured`；最多 6；顺序→AgentIndex | CONTEXT |
| chatgpt | 即 Codex 品牌；图标 `icon/chatgpt.png` | 0009 |
| 余额 | 指标对齐 CodexBar；Type 跟数据形态；标签随系统语言 | 0010 |
| 声光 | 本机 `%AppData%\Roaming\Ulanzi\D50\d50-sound-light.json`；无设备回读 | CONTEXT |
| IPC | `config_status` / `config_set_sound_light`；不开放裸命令；宿主轮询 | 0011, 0013 |
| 节奏 | 180s 一轮；上线强制全量；空列表清空设备 | 0012, 0014 |

## 切片

1. ~~bridge：队列上挂图标 / 余额 / 声光；上线与 180s 投影骨架（余额可先 mock）~~
2. ~~`bridge-ipc`：增量方法 + 字段文档（方便新上位机）~~ → `docs/device-config/ipc.md`
3. ~~用量适配器 `config_core::fetch_usage`（CodexBar 同源：Codex `wham/usage`、Claude OAuth usage、Kimi Code usages、WorkBuddy billing；缺凭据则「暂无数据」）~~
4. ~~`apps/d50-config-client`：在线状态、只读 Agent 列表、声光页（Figma 精修可后续）~~
