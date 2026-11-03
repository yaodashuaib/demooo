# 审批桥接是独立进程，独占 D50 的 HID

demo 做成 `apps/agent-bridge-service` 这样一个独立进程：命名管道对 hook，HID 对设备。不塞进 `integration-service`，也不复用 `device-service` 的 `BuddyHostSdk`。

## Status

accepted

## Considered Options

- **并进 `integration-service`**：能直接复用 hook 路由、outbox、诊断日志，但那条链路是「观察 → 入队 → 通知」，没有回程通道，硬加阻塞语义会把一个成熟服务改出两套时序。
- **并进 `device-service`**：HID 独占问题自然解决（它已经是 device 的唯一 owner），但 device-service 的 `DeviceSdk` 抽象是给 Buddy/AU05 的「命令 + 事件」，D50 的多帧 ACK 与心跳塞进去会撑破该接口。
- **独立进程**（本决定）：边界最干净，demo 可整体搬走；代价是同一台 D50 出现第二个 owner。

## Consequences

- 必须遵守「一个 D50 只有一个 owner」：demo 用与 Buddy 相同形态的全局命名互斥（`Global\Ulanzi.D50.<serial>`），第二个持有者直接失败，绝不抢占。
- 搬进 `ulanzistudio-desktop` 时 owner 必须换成 `device-service`（见实现计划 §11），否则会和未来的正式适配器抢设备；这是这次独立进程最明确的已知代价。
- 命名管道只对本机当前用户开放，且只在 `\\.\pipe\` 上，不监听 TCP。任何本机同用户进程都能冒充 hook 批准请求，这个威胁模型在 demo 阶段接受，正式接入时需要随服务一起设计。
