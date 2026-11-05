# 配置类 IPC 只提供状态与声光，不开放裸 CommandID

上位机（本 Qt demo 与后续其它宿主）经命名管道调用稳定方法：查询配置/设备投影状态、下发或预览声光。图标与余额由 bridge 投影循环拥有，客户端不能绕过队列直接灌帧。契约放在 `bridge-ipc`，便于新宿主只依赖文档化方法面，而不复制 HID/ACK 细节。

## Status

accepted

## Consequences

- 新宿主接入成本是实现 IPC 客户端，而不是实现 D50 传输。
- 若将来需要「调试灌帧」，应另开显式 debug 方法或工具，不塞进正式 config API。
