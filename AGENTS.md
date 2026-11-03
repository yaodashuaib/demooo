# AGENTS.md — Ulanzi Studio 工作区规则（DSH）

本文件是 DSH 的 `dsh-agent-instructions` 在本仓库加载的项目级指令。
Cursor 的规则保留在 `.cursor/rules/*.mdc`；DSH 不按 glob 自动激活规则，所以这里给的是**索引 + 必守不变量**：动手前先读对应 `.mdc` 原文，冲突时以原文为准。

## 一、动手前先读什么

| 要改的文件 | 先读 | 一句话 |
| --- | --- | --- |
| `**/*.qml` | `.cursor/rules/qml-layout.mdc`、`qml-lists.mdc`、`qml-mvvm.mdc` | 布局用 Layout/anchors；重复行用 model + Repeater；View 不写业务 |
| `**/*.{h,hpp,cpp}` | `.cursor/rules/qt-typed-outcome.mdc`；QML 相关再读 `qml-mvvm.mdc` | 内部传 struct，边界才 toJson；禁 `_error` 魔串信封 |
| 协议 / 领域术语 / 数据归属 | `CONTEXT.md` | 沿用其中的 Language 定义，不自造同义词 |
| C++ 与 Rust 的契约字段 | `service-protocol/docs/`、`services-rust/apps/*/README.md` | 文档即规格，三处同步 |

## 二、不可违反的硬约束

1. **QML 布局**：普通 UI 禁止绝对 `x` / `y`，用 `RowLayout` / `ColumnLayout` / `GridLayout` 或 `anchors`；从 `x`/`y` 重构到布局时视觉结果必须不变。只有设计锁定的画布层（如设备预览对齐 Figma 像素）或布局确实无法表达时才例外。
2. **QML 重复行**：同构的行 / 菜单 / 标签 / chip 行不许复制粘贴近似块；用 `property var`、`ListModel` 或 VM 的 `QVariantList` 配一个 `Repeater` 委托。同一行 chrome 只发一个带 action id 的信号（如 `menuActionClicked(string actionId)`），不要 N 个平行信号。浮层超出卡片时抬升宿主 `z`，并避开父 `Flickable.clip`。
3. **Qt C++ typed outcome**：模块内部用 struct 传数据（字段具名、`toJson()` 仅供出站），只有 HTTP/WS/文件边界才序列化 JSON。可失败方法返回 `MethodOutcome` / `RpcResponse`，用 `isOk()` / `isError()` / `isDeferredPoll()` 分支；**禁止** `_error` / `_message` 信封，也禁止 `response.contains("error")` 这类字符串探测。参考 `ulanzi/core/remote/remoteoutcome.h`，线格式用 `RpcResponse::fromOutcome(id, outcome)`。
4. **QML MVVM 分层**：View 只做布局与绑定；C++ `QObject` ViewModel 用 `Q_PROPERTY` / `Q_INVOKABLE` / 信号持有 loading、error、列表与用户动作；数据经既有 `HttpClient`（`component/uclockdevicemanage`）解析进模型，先 mock 再换真实 URL。可复用视觉放 `component/qmlcommon`，不逐页复制。
5. **Q_PROPERTY**：纯存储字段用 `MEMBER` + `NOTIFY`（QML 绑定必须靠 NOTIFY，否则只读一次）；C++ 直接改成员**不会**自动发信号，改完自己判不等再 emit。只有 setter 有校验/副作用、属性是计算值、或 C++ 调用方需要完整类型化访问时才写 `READ`/`WRITE`。本仓库刻意不用 Qt 6 的 `BINDABLE`。

## 三、构建与验证

- Qt 6.5.3 / MSVC 2019 x64 Debug；构建目录 `build/Desktop_Qt_6_5_3_MSVC2019_64bit_Debug`。
- Rust 服务在 `services-rust/`（cargo workspace：`apps/device-service`、`apps/integration-service` 与 `crates/*`）。
- 改契约时同步：C++ 结构体与 JSON 序列化、Rust 侧 serde 模型、`service-protocol/docs` 与对应 README 的说明。

## 四、技能

Cursor 技能库已通过 junction 接入 DSH 的技能发现根，不需要复制：

- 用户级：`C:\Users\aa\.agents\skills` → `C:\Users\aa\.cursor\skills`（单一来源，在 Cursor 里改，DSH 看同一份）
- 本仓库：`.agents\skills\remote-control-api` → `.cursor\skills\remote-control-api`（项目级优先于用户级）

模型可直接调用的：`code-review`、`codebase-design`、`diagnosing-bugs`、`domain-modeling`、`prototype`、`research`、`resolving-merge-conflicts`、`tdd`、`test-case-generator`、`wizard`、`remote-control-api`。

另 9 个在 frontmatter 里标了 `disable-model-invocation: true`，只在你显式点名时使用：`ask-matt`、`grill-with-docs`、`implement`、`improve-codebase-architecture`、`setup-matt-pocock-skills`、`to-spec`、`to-tickets`、`triage`、`wayfinder`。你说「按 triage 流程走」时，我按名字读对应 `SKILL.md` 再执行。

## 五、与 CONTEXT.md 的关系

`CONTEXT.md` 拥有领域语言与数据归属（Source / Account / Provider / Reminder / 设备受理 / 固件包 等），并与 `services-rust/apps/integration-service/README.md` 的文件布局说明配套。写代码、写文档、写评审结论时沿用其中的术语，不要引入同义词。
