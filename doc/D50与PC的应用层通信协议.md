# D50与PC的应用层通信协议

# D50与PC的应用层通信协议

## 文档变更记录

|版本|日期|状态|说明|
|---|---|---|---|
|V0\.1|2026\-10\-22|初稿||

## 1\. 概述

本文定义 D50 与 PC 应用之间的统一应用层通信协议，包括应用帧格式、单帧与多帧传输、逐帧确认、超时重传、CRC 校验以及业务命令格式。

应用协议独立于底层 Transport。USB HID Report、BLE Packet、2\.4G Packet 等仅负责承载连续的应用帧字节流，其 MTU、分包与组包不属于应用层分帧。

## 2\. 协议分层

### 2\.1 Application Frame

Application Frame 是本协议的基本传输和校验单位。每个 Application Frame 具有独立帧头、Payload 和 CRC16。

一个 Application Frame 可以由一个或多个 Transport Packet 承载；一个 Transport Packet 也可以包含一个应用帧的部分字节、一个完整应用帧或连续应用帧的字节。

### 2\.2 Transport Packet

Transport 层负责：

- 保持同一连接内字节顺序；

- 按自身 MTU 拆包发送；

- 将收到的 Packet 字节按顺序交给应用帧解析器。

Transport 层的 Packet 边界不改变 `FrameType`、`FrameSeq`、`FrameLength` 或 CRC16。应用帧长度不要求是 MTU 的整数倍，MTU 也不得写入应用帧头。

## 3\. 基础约定

### 3\.1 字节序

除单字节字段外，本文定义的所有整数均使用小端序（Little\-endian）。

### 3\.2 字符串与 JSON

- JSON Payload 必须使用 UTF\-8 编码。

- 字符串长度限制均按 UTF\-8 编码后的字节数计算。

- 除命令明确允许的字段外，不应发送未定义字段。

### 3\.3 通信方向

|缩写|方向|
|---|---|
|H2D|PC → D50|
|D2H|D50 → PC|

应用帧中不设置方向字段。数据帧、Response 与 ACK 的语义由通信方向、`CommandID`、Payload 长度及当前命令交互上下文共同确定。

## 4\. Application Frame

### 4\.1 帧结构

```Plaintext
SyncFrame(2) | CommandID(1) | FlagsSeq(1) | FrameLength(2) | Payload(N) | CRC16(2)
```

|偏移|长度|字段|定义|
|---|---|---|---|
|0|2|SyncFrame|固定为 `0x55 0xD5`|
|2|1|CommandID|业务命令编号，完整 `uint8`|
|3|1|FlagsSeq|Bit7:6 为 FrameType；Bit5:0 为 FrameSeq|
|4|2|FrameLength|当前完整 Application Frame 的总字节数，`uint16 LE`|
|6|N|Payload|业务数据或 Response 数据；ACK 不包含 Payload|
|6 \+ N|2|CRC16|`CRC-16/CCITT-FALSE`，低字节在前|

固定帧头长度为 6 字节，CRC16 长度为 2 字节。因此：

```Plaintext
FrameLength = 6 + PayloadLength + 2
PayloadLength = FrameLength - 8
```

### 4\.2 FrameLength

`FrameLength` 表示当前完整 Application Frame 的总长度，包含 `SyncFrame`、帧头、Payload 和 CRC16。

|项目|数值|
|---|---|
|最小 FrameLength|8 字节|
|最大 FrameLength|65535 字节|
|最大 PayloadLength|65527 字节|

发送方使用的帧长不得超过接收方支持的上限。

### 4\.3 FlagsSeq

```Plaintext
Bit:  7 6 | 5 4 3 2 1 0
      Type|   FrameSeq
```

编码公式：

```Plaintext
FlagsSeq = (FrameType << 6) | (FrameSeq & 0x3F)
FrameType = (FlagsSeq >> 6) & 0x03
FrameSeq  = FlagsSeq & 0x3F
```

|Bit7:6|FrameType|含义|
|---|---|---|
|`00`|COMPLETE|单个应用帧承载完整业务 Payload|
|`01`|FIRST|Multi\-Frame Transfer 第一帧|
|`10`|MIDDLE|Multi\-Frame Transfer 中间帧|
|`11`|END|Multi\-Frame Transfer 最后一帧|

`FrameSeq` 为 6\-bit 无符号序号，范围为 0～63。

### 4\.4 CRC16

每个 Application Frame 必须独立计算 CRC16，算法为 `CRC-16/CCITT-FALSE`：

|参数|值|
|---|---|
|Width|16|
|Poly|`0x1021`|
|Init|`0xFFFF`|
|RefIn|false|
|RefOut|false|
|XorOut|`0x0000`|

计算范围为：

```Plaintext
SyncFrame | CommandID | FlagsSeq | FrameLength | Payload
```

CRC16 从 `SyncFrame` 的第一个字节开始，按线上字节顺序计算至 Payload 的最后一个字节，不包含 CRC16 字段本身。参与计算的 `FrameLength` 字节顺序与线上传输一致，即低字节在前。计算结果在线上传输时低字节在前。

## 5\. 单帧传输

当完整业务 Payload 可以放入一个 Application Frame 时，发送方使用 `COMPLETE`：

```Plaintext
FrameType = COMPLETE
FrameSeq  = 0
```

Transport 对该帧的 MTU 拆分不改变其 `COMPLETE` 属性。

## 6\. Multi\-Frame Transfer

### 6\.1 使用条件

当业务 Payload 数据包过大，如发送图片以及OTA时，无法或不适合放入一个 Application Frame 时，发送方使用 `FIRST`、零个或多个 `MIDDLE`、`END` 组成一次 Multi\-Frame Transfer。

同一 `CommandID` 的业务可根据本次 Payload 长度选择 COMPLETE 或 Multi\-Frame，不得将某个业务命令永久绑定为其中一种方式，命令另有明确规定的除外。

### 6\.2 分帧规则

```Plaintext
FIRST   Seq=0
MIDDLE  Seq=1
MIDDLE  Seq=2
...
MIDDLE  Seq=63
MIDDLE  Seq=0
...
END     Seq=(上一帧 Seq + 1) mod 64
```

具体规则如下：

1. `FIRST` 的 `FrameSeq` 必须为 0。

2. 后续 `MIDDLE` 或 `END` 的 `FrameSeq` 必须在上一帧基础上加 1。

3. `FrameSeq` 从 63 递增后回到 0。

4. 同一次 Transfer 的所有数据帧必须使用相同 `CommandID`。

5. 接收方按顺序拼接各帧 Payload：`FIRST.Payload || MIDDLE.Payload... || END.Payload`。

6. 单方向同一时刻只允许存在一个未完成的 Multi\-Frame Transfer；完成或放弃前不得在同方向交叉发送另一组业务数据帧。

7. 两个方向的接收与发送状态相互独立，反方向 ACK 不属于业务数据交叉。

### 6\.3 处理边界

接收方在收到 FIRST 和 MIDDLE 后，应在该帧校验通过、序号正确且 Payload 已安全接纳后返回 ACK。收到 END 后，应在完整业务 Payload 拼接、业务校验及处理成功后返回 ACK。

若发送方决定从 `FIRST / Seq=0` 重新开始，接收方应放弃同方向尚未完成的旧 Transfer，并以新 FIRST 建立新的 Transfer。

## 7\. ACK、超时与重传

### 7\.1 ACK 帧格式

ACK 使用标准 Application Frame，不包含 Payload：

```Plaintext
CommandID  = 原帧 CommandID
FrameType  = 原帧 FrameType
FrameSeq   = 原帧 FrameSeq
Payload    = NULL
FrameLength= 8
```

即 ACK 原样返回原帧的 `CommandID` 和完整 `FlagsSeq`。发送方使用 `CommandID + FrameType + FrameSeq` 匹配当前等待确认的帧。

ACK 的 `PayloadLength` 为 0，线上不传输任何 Payload 字节。收到匹配的 ACK 即表示原帧已成功接收、校验并按当前阶段完成处理。本协议不定义错误 ACK 或 NACK。ACK 帧自身不要求 ACK。

### 7\.2 ACK 策略

命令表为每个业务方向规定返回类型：

|返回类型|含义|
|---|---|
|NONE|不返回 ACK 或 Response|
|ACK|成功处理后返回 ACK；若使用 Multi\-Frame，则每个应用帧分别返回 ACK|
|RESPONSE|返回同一 CommandID 的非空业务 Payload；Response 作为带业务数据的确认，不再另发 ACK|

需要 ACK 的命令采用 Stop\-and\-Wait：发送方收到当前帧的匹配 ACK 后，才能发送下一数据帧。因此同一方向任意时刻最多存在一个未确认数据帧。

### 7\.3 静默丢弃

接收方仅对完整接收、字段合法、CRC 校验通过、序号正确且成功处理的有效帧返回 ACK；其他情况均静默丢弃。

业务命令的 ACK 表示整条命令已校验并生效；未返回 ACK 的命令不产生任何修改，设备保持原状态。

### 7\.4 超时与重传

发送方未在超时时间内收到匹配 ACK 时，应重传当前 Application Frame。ACK 超时时间与最大重试次数由具体产品和 Transport 实现确定，不在本协议中固定。

当前帧多次重传仍未收到 ACK 时，发送方可以：

- 终止本次 Transfer；或

- 放弃当前进度，从 `FIRST / Seq=0` 重新发送完整 Multi\-Frame Transfer。

## 8\. Response 规则

Response 是带有非空业务 Payload 的特殊 ACK，使用与请求相同的 `CommandID`，不设置单独的 Response CommandID，也不再另发 ACK。Response 仍遵循 COMPLETE 或 Multi\-Frame 规则；Response 与普通 ACK 由通信方向、Payload 长度及当前命令上下文区分。

## 9\. USB Vendor HID 承载

D50 的 USB 复合设备可同时提供标准键盘 HID、标准鼠标 HID 和 Vendor HID。Vendor HID 协议有效区为 64 字节，不使用 Report ID。

应用帧按连续字节流映射到 HID Report：

- 一个应用帧可以跨越多个 64 字节 Report；

- Report 分包不增加应用层分包头；

- 应用帧不要求从 Report 边界开始或结束；

- 固定长度 Report 中未承载有效流数据的区域填 `0x00`；

- 接收方始终以 `SyncFrame` 和 `FrameLength` 确定应用帧边界，不得把 HID Report 边界当作应用帧边界。

BLE、2\.4G 等 Transport 采用相同原则，仅 MTU 和底层投递方式不同。

## 10\. 命令总表

CommandID 按高 4 位分段，段内预留空位供后续扩展：

|范围|分类|
|---|---|
|`0x00～0x0F`|系统：版本、设备信息、心跳、时间|
|`0x10～0x1F`|持久化配置：设备完整校验并保存后返回 ACK|
|`0x20～0x3F`|内容推送：Agent 图标、会话、余额、日程|
|`0x40～0x4F`|设备上报：D2H 事件|
|`0xE0～0xEF`|维护：OTA、恢复出厂|
|`0xF0～0xFF`|产测|

|ID|命令|请求/<br>数据方向|Payload|返回类型|说明|
|---|---|---|---|---|---|
|`0x00`|GET\_PROTOCOL\_VERSION|H2D|空|RESPONSE|查询应用协议版本|
|`0x01`|GET\_DEVICE\_INFO|H2D|空|RESPONSE|查询设备型号、序列号及软硬件版本|
|`0x02`|HEARTBEAT|H2D|1 字节二进制|NONE|携带 HostType；其他命令空闲超过 1 秒时补发，作为连接确认补充|
|`0x03`|TIME\_CONFIG|H2D|7 字节二进制|NONE|同步 PC 本地时间|
|`0x10`|SET\_PAGE\_CONFIG|H2D|JSON|ACK|配置一级页面、显示名称及顺序，成功后持久化|
|`0x11`|SET\_CONTROL\_CONFIG|H2D|JSON|ACK|配置五键及摇杆的名称、模式与输出方式，成功后持久化|
|`0x12`|SET\_SOUND\_LIGHT\_CONFIG|H2D|JSON|ACK|声音与灯光配置，以及 Agent 提示在线预览|
|`0x13`|SET\_SYSTEM\_CONFIG|H2D / D2H|JSON|H2D：ACK；D2H：NONE|PC 部分更新小时制与时钟格式；<br>设备在首次在线及本机修改时上报完整设置|
|`0x20`|SET\_AGENT\_INDEX|H2D|索引、长度与 PNG|ACK|全量替换最多六个 Agent PNG 图标|
|`0x21`|SET\_CONVERSATION\_LIST|H2D|JSON|ACK|全量替换最多六个 Agent 会话|
|`0x22`|UPSERT\_CONVERSATION|H2D|JSON|ACK|新会话插入首位；已有会话原位更新|
|`0x23`|SET\_AGENT\_BALANCE|H2D|JSON|ACK|全量配置 Agent 余额卡片的类型、进度条及文本|
|`0x24`|SET\_SCHEDULE|H2D|JSON|ACK|全量配置最多三组日程，每组最多七天|
|`0x40`|CONTROL\_EVENT|D2H|3 字节二进制|NONE|上报自定义按键或摇杆事件|
|`0x41`|CONVERSATION\_ACTION|D2H|JSON|NONE|上报会话 ID 与全局或反馈按钮动作|
|`0xE0`|OTA\_FIRMWARE|H2D|BIN|ACK|传输 OTA 固件镜像|
|`0xE1`|FACTORY\_RESET|H2D|JSON|ACK|清除用户配置与同步数据后重启，保留序列号|
|`0xF1`|SET\_FACTORY\_SERIAL\_NUMBER|H2D|JSON|ACK|产测配置设备序列号|

## 11\. 命令详细定义

### 11\.1 GET\_PROTOCOL\_VERSION \(`0x00`\)

H2D 使用 COMPLETE 帧和空 Payload 查询协议版本，D2H 使用相同 `CommandID` 返回 COMPLETE Response。Response Payload 固定为 2 字节：

```Plaintext
Byte0 Major | Byte1 Minor
```

|偏移|长度|类型|字段|说明|
|---|---|---|---|---|
|0|1|uint8|Major|主版本号|
|1|1|uint8|Minor|次版本号|

V0\.1 的 Response Payload 为：

```Plaintext
00 01
```

请求与 Response 均使用 `FrameType=COMPLETE`、`FrameSeq=0`。

### 11\.2 GET\_DEVICE\_INFO \(`0x01`\)

PC 使用 COMPLETE 帧发送空 Payload 查询，设备使用相同 `CommandID` 返回 COMPLETE Response：

```JSON
{
  "SerialNumber": "D0D36I024U3670001",
  "Dversion": "1.0.0",
  "DeviceType": "D50",
  "HardwareVersion": "1.1"
}
```

|字段|约束|说明|
|---|---|---|
|SerialNumber|≤32 ASCII 字节|设备唯一序列号|
|Dversion|≤32 ASCII 字节|固件应用版本|
|DeviceType|固定为 `D50`|设备型号|
|HardwareVersion|≤32 ASCII 字节|硬件版本|

查询请求 Payload 必须为空；成功 Response Payload 必须为非空 JSON，且不超过 256 字节。请求非法或查询失败时不返回数据。

### 11\.3 HEARTBEAT \(`0x02`\)

H2D 保活心跳，Payload 固定为 1 字节，固定使用 COMPLETE、`FrameSeq=0`，设备不返回 ACK：

```Plaintext
Byte0 HostType
```

|`HostType`|上位机|
|---|---|
|`0x00`|studio|
|`0x01`|server|
|`0x02～0xFF`|保留|

- PC 下发的任意有效帧（含 Multi\-Frame 中的每一帧及 OTA 数据帧）均作为连接确认；

- PC 超过 1 秒未发送任何帧时，补发 HEARTBEAT；连接建立后应立即发送一次 HEARTBEAT 告知 HostType；

- 设备连续 5 秒未收到 PC 的任何有效帧即判定上位机离线；

- 同一时间只允许一种上位机连接设备；保留的 HostType 设备丢弃。

设备仅在在线时发送 D2H 业务消息（`CONTROL_EVENT`、`CONVERSATION_ACTION` 及 `SET_SYSTEM_CONFIG` 上报）。

### 11\.4 TIME\_CONFIG \(`0x03`\)

H2D 发送 PC 当前本地年月日时分秒，不携带时区。

PC 应在连接建立后立即发送一次，此后在每分钟或 PC 系统时间、时区变化时再次发送；无需每秒发送。

|偏移|长度|类型|字段|范围|
|---|---|---|---|---|
|0|2|uint16 LE|Year|2000～2099|
|2|1|uint8|Month|1～12|
|3|1|uint8|Day|有效日期|
|4|1|uint8|Hour|0～23|
|5|1|uint8|Minute|0～59|
|6|1|uint8|Second|0～59|

示例：

```Plaintext
2026-08-11 15:30:45 -> EA 07 08 0B 0F 1E 2D
```

Payload 固定为 7 字节；本命令固定使用 COMPLETE、`FrameSeq=0`。

### 11\.5 SET\_PAGE\_CONFIG \(`0x10`\)

PC 全量配置一级页面，`Pages` 数组顺序即显示顺序。Payload 最大 2048 字节，设备掉电保存。未配置时默认值如下：

```JSON
{
  "Pages": [
    { "PageID": "tasks", "Name": "会话队列" },
    { "PageID": "balance", "Name": "余额" },
    { "PageID": "clock", "Name": "时钟" },
    { "PageID": "schedule_1", "Name": "日程" },
    { "PageID": "game", "Name": "游戏" }
  ]
}
```

|字段|约束|说明|
|---|---|---|
|Pages|1～8 项，必须存在|一级页面列表|
|PageID|下表固定值，不得重复|页面标识，不随名称或顺序变化|
|Name|1～32 UTF\-8 字节|页面显示名称|

|PageID|默认名称|
|---|---|
|`tasks`|会话队列|
|`balance`|余额|
|`clock`|时钟|
|`schedule_1`|日程|
|`schedule_2`|日程2|
|`schedule_3`|日程3|
|`game`|游戏|
|`test`|硬件测试|

仅支持表内页面；未列入数组的页面不显示。

### 11\.6 SET\_CONTROL\_CONFIG \(`0x11`\)

PC 使用 JSON 一次性配置 KEY1～KEY5 和摇杆。Payload 最大 4096 字节，设备掉电保存。未配置时五键均为 `CUSTOM`、摇杆为 `XY`。

```JSON
{
  "Keys": [
    {
      "Index": 0,
      "Name": "发送",
      "Output": {
        "OutputType": 1,
        "Modifier": 0,
        "Usage": 40
      }
    },
    {
      "Index": 1,
      "Name": "取消",
      "Output": {
        "OutputType": 0
      }
    },
    {
      "Index": 2,
      "Name": "复制",
      "Output": {
        "OutputType": 1,
        "Modifier": 1,
        "Usage": 6
      }
    },
    {
      "Index": 3,
      "Name": "鼠标左键",
      "Output": {
        "OutputType": 2,
        "MouseAction": 0
      }
    },
    {
      "Index": 4,
      "Name": "功能键",
      "Output": {
        "OutputType": 1,
        "Modifier": 0,
        "Usage": 41
      }
    }
  ],
  "Joystick": {
    "Mode": 0,
    "Name": "摇杆"
  }
}
```

#### 11\.6\.1 顶层字段

|字段|类型|约束|
|---|---|---|
|`Keys`|array|必须包含 `Index=0～4`，不得重复或缺失|
|`Joystick`|object|必须存在|

#### 11\.6\.2 按键配置

`Keys` 用于配置 5 个独立按键。每个数组元素表示一个按键，其中 `Index` 唯一标识按键，`Name` 用于显示，`Output` 定义该按键的输出行为。

|字段|类型|约束|
|---|---|---|
|`Index`|uint8|按键索引，取值 `0～4`|
|`Name`|string|按键显示名称，最长 32 个 UTF\-8 字节|
|`Output`|object|按键输出配置，必须存在，结构见 Output 字段|

`Keys` 必须完整包含 `Index=0～4` 的 5 个按键，且 `Index` 不重复。

#### 11\.6\.3 Output 字段

`OutputType` 定义输出方式：

|值|名称|说明|
|---|---|---|
|`0`|`CUSTOM`|通过 `CONTROL_EVENT` 上报|
|`1`|`KEYBOARD`|通过标准 USB HID 键盘上报|
|`2`|`MOUSE`|通过标准 USB HID 鼠标上报|

`CUSTOM`：

```JSON
{
  "OutputType": 0
}
```

`KEYBOARD`：

```JSON
{
  "OutputType": 1,
  "Modifier": 0,
  "Usage": 40
}
```

|字段|类型|约束|
|---|---|---|
|`Modifier`|uint8|USB HID Keyboard Modifier 位掩码，可按位组合|
|`Usage`|uint8|USB HID Keyboard/Keypad Usage Page `0x07` 的 Usage；`0` 表示仅发送 `Modifier`|

`MOUSE`：

```JSON
{
  "OutputType": 2,
  "MouseAction": 0
}
```

|`MouseAction`|名称|说明|
|---|---|---|
|`0`|`LEFT_BUTTON`|鼠标左键|
|`1`|`RIGHT_BUTTON`|鼠标右键|
|`2`|`MIDDLE_BUTTON`|鼠标中键|
|`3`|`WHEEL_UP`|向上滚动|
|`4`|`WHEEL_DOWN`|向下滚动|

`CUSTOM` 不得携带 `Modifier`、`Usage` 或 `MouseAction`；`KEYBOARD` 不得携带 `MouseAction`；`MOUSE` 不得携带 `Modifier` 或 `Usage`。

`KEYBOARD` 和 `MOUSE` 经标准 USB HID 输出，与上位机在线状态无关；`CUSTOM` 仅在线时发送 `CONTROL_EVENT`。

#### 11\.6\.4 摇杆配置

`Joystick` 用于配置摇杆的工作方式。摇杆支持 `XY` 和 `FOUR_WAY` 两种模式，由 `Mode` 指定。

|`Mode`|名称|说明|
|---|---|---|
|`0`|`XY`|将摇杆作为连续二维输入，通过 `CONTROL_EVENT` 上报 `X/Y` 坐标|
|`1`|`FOUR_WAY`|将摇杆的上、下、左、右视为 4 个独立方向输入，每个方向分别配置 `Name` 和 `Output`|

##### XY 模式

`Mode=0` 时，摇杆整体作为一个连续控制项，仅配置摇杆名称；设备根据摇杆位置通过 `CONTROL_EVENT` 上报 `X/Y` 坐标。

```JSON
{
  "Joystick": {
    "Mode": 0,
    "Name": "摇杆"
  }
}
```

|字段|类型|约束|
|---|---|---|
|`Mode`|uint8|固定为 `0`|
|`Name`|string|摇杆显示名称，最长 32 个 UTF\-8 字节|

`Mode=0` 时必须包含 `Name`，不得携带 `Directions`。

##### 四向模式

`Mode=1` 时，摇杆被划分为上、下、左、右 4 个独立方向。每个方向均作为一个独立控制项，其 `Name` 和 `Output` 的含义与普通按键一致。

```JSON
{
  "Joystick": {
    "Mode": 1,
    "Directions": [
      {
        "Direction": 0,
        "Name": "向上",
        "Output": {
          "OutputType": 1,
          "Modifier": 0,
          "Usage": 82
        }
      },
      {
        "Direction": 1,
        "Name": "向下",
        "Output": {
          "OutputType": 2,
          "MouseAction": 4
        }
      },
      {
        "Direction": 2,
        "Name": "向左",
        "Output": {
          "OutputType": 1,
          "Modifier": 0,
          "Usage": 80
        }
      },
      {
        "Direction": 3,
        "Name": "向右",
        "Output": {
          "OutputType": 0
        }
      }
    ]
  }
}
```

每个方向项包含：

|字段|类型|约束|
|---|---|---|
|`Direction`|uint8|方向标识：`0=上`，`1=下`，`2=左`，`3=右`|
|`Name`|string|方向显示名称，最长 32 个 UTF\-8 字节|
|`Output`|object|该方向的输出配置，结构与普通按键的 `Output` 相同|

`Directions` 必须完整包含 `Direction=0～3` 的 4 个方向，且不得重复。每个方向必须包含 `Name` 和 `Output`。

XY 模式的坐标仅在线时发送；四向模式各方向的 `Output` 规则与按键相同。

### 11\.7 SET\_SOUND\_LIGHT\_CONFIG \(`0x12`\)

PC 全量配置声音和灯光，或预览 Agent 提示效果。Payload 最大 512 字节，设备掉电保存。默认配置如下：

```JSON
{
  "Sound": {
    "Volume": 80,
    "AgentPrompt": {
      "Enabled": true,
      "Preview": false
    }
  },
  "Light": {
    "Brightness": 80,
    "AgentPrompt": {
      "Enabled": true,
      "Color": {
        "R": 255,
        "G": 128,
        "B": 0
      },
      "DurationMs": 6000,
      "Preview": false
    }
  }
}
```

|字段|约束|说明|
|---|---|---|
|Sound\.Volume|整数 `0～100`|Agent 提示音和游戏总音量，0 为静音|
|Sound\.AgentPrompt\.Enabled|boolean|Agent 会话提示音开关|
|Sound\.AgentPrompt\.Preview|boolean|`true` 预览一次提示音|
|Light\.Brightness|整数 `0～100`|实体审批按键灯和底部提示灯亮度，0 为熄灭|
|Light\.AgentPrompt\.Enabled|boolean|Agent 底部提示灯开关|
|Light\.AgentPrompt\.Color|R/G/B 各为整数 `0～255`|底部提示灯 RGB 颜色|
|Light\.AgentPrompt\.DurationMs|整数 `0～60000 ms`（0～60 秒）|总持续时间，0 不启动灯光|
|Light\.AgentPrompt\.Preview|boolean|`true` 预览一次底部灯呼吸效果|

`Preview` 可省略。两个 `Preview` 均为 `false` 或省略时保存配置；任一为 `true` 时仅按本次参数预览对应效果，不保存配置。

### 11\.8 SET\_SYSTEM\_CONFIG \(`0x13`\)

系统设置，同一 CommandID 双向使用。Payload 最大 256 字节，设备掉电保存。

|字段|约束|说明|
|---|---|---|
|TimeFormat|整数 `12` 或 `24`|时钟页小时制，默认 `24`|
|ClockStyle|整数 `1～3`|时钟页显示格式，默认 `1`|

|`ClockStyle`|显示内容|
|---|---|
|`1`|仅时间|
|`2`|时间 \+ 日期|
|`3`|时间 \+ 日期 \+ 星期|

#### 11\.8\.1 PC 下发（H2D，返回 ACK）

PC 可只携带需要修改的字段，至少一个，不得携带未定义字段或重复字段；未携带的字段保持设备当前值：

```JSON
{
  "TimeFormat": 12
}
```

PC 下发的修改以 ACK 确认，设备不回发上报。

#### 11\.8\.2 设备上报（D2H，无 ACK）

设备以相同 CommandID 发送 COMPLETE 帧，Payload 固定携带全部字段：

```JSON
{
  "TimeFormat": 24,
  "ClockStyle": 2
}
```

设备在以下时机各上报一次，PC 收到后无需 ACK，以设备上报为准：

- 离线后首次在线（包括开机后首次收到 PC 的有效帧）；

- 在线期间用户在设备上修改设置。

离线期间的本机修改随下次首次在线上报同步。上报帧带非空 Payload，可与空 Payload 的 ACK 区分。

### 11\.9 SET\_AGENT\_INDEX \(`0x20`\)

PC 发送 Agent 可选列表以及图标的全量配置。业务 Payload 由多个图标条目连续拼接，不使用 JSON、Base64、文件名或路径。Payload 可由 COMPLETE 或 Multi\-Frame 承载。

#### 11\.9\.1 业务 Payload

```Plaintext
AgentCount(1) |
    AgentIndex(1) | ImageLength(2) | Image(ImageLength) |
    AgentIndex(1) | ImageLength(2) | Image(ImageLength) | ...
```

所有长度字段使用 uint16 小端序。

|字段|类型|约束|说明|
|---|---|---|---|
|AgentCount|uint8|0～6|Agent数量；0 表示清空|
|AgentIndex|uint8|0～5，不得重复|Agent 索引|
|ImageLength|uint16 LE|1～65535|后续单张 PNG 的字节数|
|Image|binary|完整 PNG 文件|按条目顺序连续排列|

完整业务 Payload 长度必须满足：

```Plaintext
PayloadLength = 1 + sum(3 + ImageLength[i])
```

#### 11\.9\.2 PNG 规则

|项目|要求|
|---|---|
|尺寸|固定 100 × 100 像素|
|编码|标准 PNG；每通道 8 bit|
|颜色|支持 RGB 和 RGBA；允许透明背景|

### 11\.10 SET\_CONVERSATION\_LIST \(`0x21`\)

PC 在初始化或需要完整替换时下发最多六个 Agent 会话。数组按会话创建顺序从新到旧排列，索引 0 为最新会话；设备按数组顺序显示。

```JSON
{
  "Conversations": [
    {
      "ID": "conv-001",
      "Title": "D50 协议实现",
      "Path": "D:/Code/D50",
      "Content": "协议解析已完成，请审阅变更。",
      "AgentIndex": 0,
      "NeedsAttention": 1,
      "Buttons": [
        {"Index": 1, "Name": "同意"},
        {"Index": 2, "Name": "拒绝"}
      ]
    }
  ]
}
```

|字段|约束|说明|
|---|---|---|
|Conversations|0～6 项|`[]` 表示清空；从新到旧|
|ID|唯一；1～64 UTF\-8 字节|会话唯一标识，不得为空|
|Title|0～64 UTF\-8 字节|会话标题，可为空|
|Path|0～64 UTF\-8 字节|工作路径，可为空|
|Content|0～64 UTF\-8 字节|会话或功能内容，可为空|
|AgentIndex|0～5|引用已传输的 Agent 图标|
|NeedsAttention|必填；数值 0 或 1|1：未读/未审批；0：已处理|
|Buttons|0～4 项|反馈按钮数组|
|Buttons\[\]\.Index|1～4，不得重复|反馈按钮索引；0 保留为全局动作|
|Buttons\[\]\.Name|0～32 UTF\-8 字节|反馈按钮名称，可为空|

`NeedsAttention` 为 0 时会话的反馈按钮不可操作。

### 11\.11 UPSERT\_CONVERSATION \(`0x22`\)

PC 下发一个完整会话对象，字段结构与 `Conversations` 数组中的单项相同，不使用 JSON Patch。

```JSON
{
  "ID": "conv-001",
  "Title": "修复登录异常",
  "Path": "D:/Code/project",
  "Content": "任务已完成，请审阅输出。",
  "AgentIndex": 0,
  "NeedsAttention": 1,
  "Buttons": []
}
```

|条件|处理|
|---|---|
|新 ID，当前少于 6 个|插入索引 0，其余会话依次右移|
|新 ID，当前已有 6 个|淘汰尾部索引 5 的最旧会话，其余右移，新会话插入索引 0|
|已有 ID|在原位置替换完整会话内容，不改变顺序或数量|

已有 ID 的更新以及设备上的会话操作不改变会话顺序；`NeedsAttention` 以本次下发值为准。

### 11\.12 SET\_AGENT\_BALANCE \(`0x23`\)

PC 低频全量更新 Agent 余额页，支持双进度条、单进度条、纯文本及数值卡片，可混合显示。Payload 最大 16384 字节，支持 COMPLETE 或 Multi\-Frame。

```JSON
{
  "Agents": [
    {
      "Name": "Codex",
      "AgentIndex": 0,
      "Type": "DOUBLE_BAR",
      "Bars": [
        {"LeftLabel": "5小时剩余", "RightLabel": "76%", "Value": 76},
        {"LeftLabel": "周剩余", "RightLabel": "42% · 周五重置", "Value": 42}
      ]
    },
    {
      "Name": "API账户",
      "AgentIndex": 1,
      "Type": "SINGLE_BAR",
      "Bars": [
        {"LeftLabel": "可用余额", "RightLabel": "$18.50", "Value": 62}
      ]
    },
    {
      "Name": "订阅账户",
      "AgentIndex": 2,
      "Type": "TEXT",
      "Text": "当前套餐：Pro\n本月已使用：128次\n续费日期：2026-11-01"
    },
    {
      "Name": "余额账户",
      "AgentIndex": 3,
      "Type": "VALUE",
      "Label": "Balance",
      "ValueText": "2400"
    }
  ]
}
```

|字段|约束|说明|
|---|---|---|
|Agents|0～6 项，数组|数组顺序为显示顺序；`[]` 表示清空|
|Name|≤32 UTF\-8 字节，字符串|Agent 名称|
|AgentIndex|整数 0～5，不得重复|引用已传输图标|
|Type|`DOUBLE_BAR`、`SINGLE_BAR`、`TEXT` 或 `VALUE`|双进度条、单进度条、纯文本或数值|
|Bars|`DOUBLE_BAR` 为 2 项，`SINGLE_BAR` 为 1 项|仅进度条类型携带，按数组顺序显示|
|Bars\[\]\.LeftLabel|≤32 UTF\-8 字节，字符串，可为空，不允许换行|进度条上方左侧文字|
|Bars\[\]\.RightLabel|≤32 UTF\-8 字节，字符串，可为空，不允许换行|进度条上方右侧文字|
|Bars\[\]\.Value|整数 0～100|剩余或可用比例，独立决定填充百分比|
|Text|≤256 UTF\-8 字节，字符串，可为空|仅 TEXT 类型携带|
|Label|≤32 UTF\-8 字节，字符串，可为空，不允许换行|仅 VALUE 类型携带；数值标题|
|ValueText|1～16 个字符的字符串，仅允许 ASCII 数字 `0`～`9`|仅 VALUE 类型携带；原样显示的数值|

各类型携带的字段互斥：`DOUBLE_BAR`/`SINGLE_BAR` 仅携带 `Bars`，`TEXT` 仅携带 `Text`，`VALUE` 仅携带 `Label` 与 `ValueText`；缺少或多出字段时整帧非法。`ValueText` 不支持小数点、千分位、正负号及货币符号，单位等说明文字请放入 `Label`。

### 11\.13 SET\_SCHEDULE \(`0x24`\)

PC 全量下发最多三组日程，每组对应一个独立日程页，可包含多个日期及多条事项。Payload 最大 16384 字节，支持 COMPLETE 或 Multi\-Frame。

```JSON
{
  "Schedules": [
    {
      "PageID": "schedule_1",
      "Days": [
        {
          "Date": "2026-10-09",
          "Events": [
            {
              "StartTime": "09:30",
              "EndTime": "10:00",
              "Title": "项目启动会"
            },
            {
              "StartTime": "14:00",
              "EndTime": "15:00",
              "Title": "方案评审"
            }
          ]
        }
      ]
    },
    {
      "PageID": "schedule_2",
      "Days": [
        {
          "Date": "2026-10-09",
          "Events": [
            {
              "StartTime": "19:00",
              "EndTime": "20:00",
              "Title": "学习计划"
            }
          ]
        }
      ]
    },
    {
      "PageID": "schedule_3",
      "Days": []
    }
  ]
}
```

|字段|约束|说明|
|---|---|---|
|Schedules|0～3 项，必须存在|全量替换所有日程组；\[\] 表示清空全部日程|
|PageID|`schedule_1`、`schedule_2` 或 `schedule_3`，不得重复|对应一级日程页的固定标识|
|Days|每组 0～7 项，必须存在|\[\] 表示该组无日程|
|Date|合法的 `YYYY-MM-DD`，同组不得重复|与 TIME\_CONFIG 使用相同的本地日期语义|
|Events|每天 0～16 项，必须存在|\[\] 表示当天无事项|
|StartTime|HH:mm，24 小时制|日程开始时间|
|EndTime|HH:mm，24 小时制，晚于 StartTime|同一天内的日程结束时间|
|Title|≤64 UTF\-8 字节|日程标题|

数量与 Payload 长度限制必须同时满足。未下发的日程组清空旧数据；设备按日期、开始时间排序显示，同一开始时间保持下发顺序。允许向未加入页面列表的日程组下发数据。

日程不掉电保存，设备重启后需 PC 重新下发。

### 11\.14 CONTROL\_EVENT \(`0x40`\)

D2H 上报自定义按键和摇杆事件。Payload 固定为 3 字节：

```Plaintext
Byte0 ControlType | Byte1 Value1 | Byte2 Value2
```

|`ControlType`|`Value1`|`Value2`|定义|
|---|---|---|---|
|`0`|保留|保留|保留，不使用|
|`1`|`KeyIndex`|`Action`|普通按键事件|
|`2`|`X`|`Y`|XY 模式摇杆坐标|
|`3`|`Direction`|`Action`|四向模式摇杆事件|

#### 11\.14\.1 普通按键事件

|字段|类型|取值|
|---|---|---|
|`KeyIndex`|uint8|`0～4`|
|`Action`|uint8|`0=按下`，`1=释放`|

#### 11\.14\.2 XY 摇杆坐标

|字段|类型|取值|
|---|---|---|
|`X`|int8|`-100～100`，左负右正|
|`Y`|int8|`-100～100`，上负下正|

摇杆离开死区或坐标变化时上报；回到中心时上报一次 `X=0、Y=0`。

#### 11\.14\.3 四向摇杆事件

|字段|类型|取值|
|---|---|---|
|`Direction`|uint8|`0=上`，`1=下`，`2=左`，`3=右`|
|`Action`|uint8|`0=按下`，`1=释放`|

方向切换时，必须先上报原方向释放，再上报新方向按下；回到中心时释放当前方向。

### 11\.15 CONVERSATION\_ACTION \(`0x41`\)

设备以 JSON 上报会话全局动作或反馈按钮操作，不使用会话槽位：

```JSON
{
  "ID": "conv-001",
  "ButtonIndex": 1
}
```

|字段|约束|说明|
|---|---|---|
|ID|≤64 UTF\-8 字节|当前会话 ID|
|ButtonIndex|0～4|0：全局打开会话；1～4：Buttons 中实际存在的按钮索引|

每条会话默认拥有全局动作 0，无需在 `Buttons` 中下发；动作 0 表示打开会话，不代表选择审批选项。

设备发送后按以下规则将该会话 `NeedsAttention` 置 0，PC 应按相同规则同步，后续下发携带最新状态：

- 有实际按钮时，仅 `ButtonIndex=1～4` 清除标识，动作 0 不清除；

- 无实际按钮时，动作 0 清除标识。

### 11\.16 OTA\_FIRMWARE \(`0xE0`\)

H2D 发送固件端提供的完整 OTA BIN 文件，业务 Payload 为该文件的原始字节流：

```Plaintext
BinData
```

- OTA BIN 由固件构建时打包，文件内已包含设备校验所需的文件头与 CRC；PC 不解析、不修改，按原样从首字节发送到末字节；

- 必须使用 Multi\-Frame 传输，不支持 COMPLETE；FIRST 帧 Payload 不少于 16 字节；

- OTA 传输期间只允许插入 `HEARTBEAT`，发送其他命令会中止本次 OTA；

- 设备在 END 帧完成整包校验后返回 ACK，随后重启。PC 可重新读取设备信息，按版本号确认升级结果。

### 11\.17 FACTORY\_RESET \(`0xE1`\)

PC 请求恢复出厂设置。Payload 最大 64 字节，必须且只能包含固定确认字：

```JSON
{
  "Confirm": "FACTORY_RESET"
}
```

|字段|约束|说明|
|---|---|---|
|Confirm|固定为 `FACTORY_RESET`|防止误发触发清除|

设备清除全部用户配置及 PC 下发的数据（序列号保留），返回 ACK 后重启。设备端不二次确认，PC 应在发送前向用户确认。

### 11\.18 SET\_FACTORY\_SERIAL\_NUMBER \(`0xF1`\)

本命令仅用于产测。PC 发送新的设备序列号：

```JSON
{
  "SerialNumber": "D0D36I024U3670001"
}
```

|字段|约束|说明|
|---|---|---|
|SerialNumber|1～32 个可打印 ASCII 字节|新设备唯一序列号|

Payload 只能包含 `SerialNumber` 字段，总长度不得超过 256 字节。

