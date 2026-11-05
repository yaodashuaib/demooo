# D50 真机联调记录

日期：2026-10-29　设备：303A:83D8，USB SN `E8F60AE7A976` / `E8F60AE7A97F`（两台样机）

## 2026-10-29 更正：之前的「固件未实现」结论作废

重新插拔后，同一台设备（`E8F60AE7A97F`）立刻按协议应答：

```text
写 55 D5 00 00 08 00 99 A0（GET_PROTOCOL_VERSION）
读 64 字节: 55 D5 00 00 0A 00 00 01 A7 F2 00 00 ...
          ↑ 长度 10、Payload 00 01（协议 V0.1）、CRC A7 F2
```

同一次扫描里，只有「协议原文」和「Report ID=0x55 + 完整帧」两种摆法有回音——后者其实是
hidapi 把报告 ID 吃掉后、设备仍从 `55 D5` 开始解析，属于同一帧。

所以：**帧格式从一开始就是对的**，第一轮的沉默是设备处于未就绪状态（插拔前它连
`SET_SOUND_LIGHT_CONFIG` 的预览灯都不亮，等于应用层没有运行），不是固件缺功能。

## 结论（已更新）

- 上位机侧：帧、CRC、Report ID、报告长度全部与设备一致，无需改动。
- 设备侧：正常上电后按协议工作。遇到「写成功、读 0 字节」时，先重新插拔，
  再核对 `d50-probe --sweep` 是否只有 `doc` 摆法有回音。

## 完整链路已验证（2026-10-29，设备 `D0D36I024U3670001` / 固件 1.0.0）

恢复通讯后按顺序验证，服务侧日志：

```text
device: self-test-efb2570a-… button 1 -> Allow
device: real-1 button 1 -> Allow
```

1. `d50ctl status` → `online true / serial D0D36I024U3670001 / firmware 1.0.0`。
2. `d50ctl self-test` → `shown 1`，设备屏幕出现一条带「同意 / 拒绝」的会话。
3. 人按「同意」→ `shown 0`，服务记到 `button 1 -> Allow`。
4. `d50-hook --agent claude-code --request-id real-1` 阻塞等待；设备显示
   「claude-code · Bash / rm -rf build && cmake --build build」；按「同意」后 hook 输出
   `{"hookSpecificOutput":{"decision":{"behavior":"allow"},"hookEventName":"PermissionRequest"}}`。

尚未验证：真实 Claude Code / Codex 的 hook 接入；决定后设备上 `NeedsAttention` 的视觉过渡。

### 两个序列号不是一回事

| 来源 | 值 | 用途 |
| --- | --- | --- |
| USB 描述符 `iSerialNumber` | `E8F60AE7A97F` | `--serial` 匹配的是它（`--list-devices` 的 `usb-serial` 列） |
| `GET_DEVICE_INFO.SerialNumber` | `D0D36I024U3670001` | 设备自报身份，`d50ctl status` 与日志里显示 |

两者完全不同，选设备时别拿固件 SN 去配对。

## 排查过程（保留作为证据）

### 1. 设备与集合

```text
[4] 303A:83D8 usage 0001:0006 iface 2  D50 Keyboard & Mouse   E8F60AE7A976
[5] 303A:83D8 usage 0001:0002 iface 2  D50 Keyboard & Mouse   E8F60AE7A976
[6] 303A:83D8 usage FF00:0001 iface 3  D50 Vendor Protocol    E8F60AE7A976
```

键盘/鼠标集合的读取被 Windows 拒绝（`ReadFile 0x5 拒绝访问`），属正常安全策略。

### 2. 报告格式（`HidP_GetCaps` / `HidP_GetValueCaps`）

| 项 | 值 |
| --- | --- |
| InputReportByteLength | 65 |
| OutputReportByteLength | 65 |
| FeatureReportByteLength | 0 |
| Input value cap | usagePage `FF00`、reportId `0x00`、8 bit × 64 = 64 字节 |
| Output value cap | 同上 |

即「1 字节 Report ID（0）+ 64 字节数据」，与协议文档「有效区 64 字节、不使用 Report ID」一致。
注意 Windows 对该设备**不校验 Report ID**（连 `0x7F` 都写成功），所以「写成功」不能当作「设备收到了」。

### 3. 故障现象（已解释）

- 8 种帧摆法 + 10 种 Report ID，全部无回音。
- 只读 45 秒、40 秒，人工按键与摇杆，0 字节。
- §11.7 预览灯命令连发 3 次，底部提示灯毫无变化——说明当时应用层没在跑。

### 4. 恢复后的复测

```powershell
cargo run -p d50-hid --features hid --bin d50-probe -- --sweep
cargo run --features hid --bin agent-bridge-service -- --list-devices
cargo run --bin d50ctl -- self-test
```

`--sweep` 出现 `doc` 行回音，`--list-devices` 打出设备自报的 SN 与固件版本，
`self-test` 后设备屏幕出现「同意 / 拒绝」，即链路正常。

## 仍然待确认

1. 五个键当前是 `CUSTOM` 还是 `KEYBOARD`/`MOUSE` 输出——只有 `CUSTOM` 才会产生 `CONVERSATION_ACTION`。
2. 设备被拔插前的「半枚举」状态是否可复现，以及上位机能否用 `GET_DEVICE_INFO` 超时提前判定。
3. 同一台设备重插后 USB 序列号是否稳定（两台样机 SN 不同，固件 SN 也不同）。

## 2026-10-30 对照实验：反馈按钮亮哪几颗键

方法：停掉 `agent-bridge-service` 释放 HID，用
`cargo run -p d50-hid --features hid --example button_layout -- 0 20 10`
逐档下发会话，每档 20 秒，只改 `Buttons`，其余字段（`NeedsAttention: 1`）不变。

| 档 | `Buttons` | 实测亮灯 |
| --- | --- | --- |
| A | `[]` | KEY5 |
| B | `Index 1「同意」`、`Index 2「拒绝」` | KEY4、KEY5 |
| C | `Index 3`、`Index 4` | KEY4、KEY5（与 B 完全相同） |
| D | `Index 1..4` | KEY2、KEY3、KEY4、KEY5 |

结论：

- 亮哪几颗键由**按钮数量**决定，且右对齐占最右 N 颗（N = `Buttons` 项数）；N = 0 时也亮 KEY5 一颗，
  说明无按钮时最右键承担「有会话/打开」的提示角色。
- `Buttons[].Index` 只决定 `CONVERSATION_ACTION` 回报哪个号，**不影响键位**（C 档与 B 档完全一致）。
- 审批区是 KEY2～KEY5，KEY1 未参与。协议没有「指定键位」的字段；PC 侧能改的只有按钮数量、
  `Name`、`NeedsAttention` 和 `0x12 Light.Brightness`（整组亮度）。
- 因此「同意/拒绝 落在 KEY4/KEY5」是 2 个按钮这一事实的必然结果，不是我们选的。

遗留：无。数组顺序与左右顺序的对应见下一节。

## 2026-10-30 设备上的按钮布局（实测 + 产品定稿）

| 场景 | `Buttons` | 亮灯 | 按下回报 |
| --- | --- | --- | --- |
| 提示（有 N 个问题待回复） | `1 open app` | KEY5 | 1 → 服务开 deeplink 并撤掉提示 |
| Codex 审批 | `1 Agree` `2 Ignore` `3 open app` | KEY3 / KEY4 / KEY5 | 1 allow，2 deny，3 开 App |
| Claude 审批（有建议规则） | `1 Agree` `2 Ignore` `3 Always` `4 open app` | KEY2–KEY5 | 3 → `updatedPermissions` |

两个约束：

- **4 个键位是硬上限**：三个裁决 + open app 正好占满 KEY2–KEY5，再加一项就放不下。
- **按下任意反馈键会清 `NeedsAttention`**，按钮立刻不可操作。按的是 open app 时请求还挂着，
  服务必须 `touch()` 重新点亮一次，否则剩下两颗裁决键按不动。

## 2026-10-30 三个选项：数组顺序 = 从左到右

方法：`cargo run -p d50-hid --features hid --example button_order -- 0 180`，下发
`Buttons` 为 `1「同意」/ 2「拒绝」/ 3「总是允许」`（亮 KEY3、KEY4、KEY5），人按从左到右各按一次，
程序记录每次回报的 `ButtonIndex`，并每收到一次就把同一份会话重新下发（按下会清 `NeedsAttention`，
按钮随即不可操作，不重置就按不了第二颗）。

实测（连按两轮，结果一致）：

```
从左数第 1 颗（KEY3） → ButtonIndex=1
从左数第 2 颗（KEY4） → ButtonIndex=2
从左数第 3 颗（KEY5） → ButtonIndex=3
```

结论：

- **`Buttons` 数组顺序就是键位从左到右的顺序**，`ButtonIndex` 就是数组序号，没有反序、没有偏移。
- 3 个选项（同意 / 拒绝 / 总是允许）在键盘面可用：占 KEY3～KEY5，左中右语义直观。
- 按下任一反馈按钮会清掉该会话的 `NeedsAttention`，同一会话不会让你按第二次——这对审批是好事
  （一次请求只能被裁决一次），但也意味着**要重复实验必须重新下发会话**。

注意：实验设备 USB 序列号为 `E8F60AE7A976`，固件 SN `D0D36I024U3670001`，与 2026-10-29 记录的
`E8F60AE7A97F` 不同一位，原因未查明（换口/换样机/固件 SN 与 USB SN 混记都可能）。

## SET_AGENT_INDEX（2026-11-04，对照硬件发图日志）

- 协议要求 **100×100、每通道 8bit 的 RGB/RGBA PNG**（`icon/` 已按此准备）。
- 硬件工具：业务 Payload 按 **1024** 切应用帧（First/Middle/End），每帧再拆成 64B HID report；**逐帧等 ACK**。
- **END 帧 ACK 偏慢**（日志约 1.5s，设备解码整段 PNG），bridge `ACK_TIMEOUT=5s`。
- 传输中设备可能插播 `SET_SYSTEM_CONFIG` 上报，应派发、不中断当前 0x20 等待。
- 勿同时开 `D50HidTool` 抢 HID。

