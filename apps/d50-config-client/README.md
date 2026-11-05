# d50-config-client

Qt 6.5 配置上位机 demo：D50 在线状态、绑定/解绑 Agent、余额刷新间隔、WorkBuddy Cookie 与声光设置。经命名管道调用 `agent-bridge-service`（见 `docs/device-config/ipc.md`）。无自动轮询；点「刷新」或绑定/解绑后读 `config_status`。

## 构建

```powershell
cmake -S apps/d50-config-client -B build/d50-config-client -DCMAKE_PREFIX_PATH="C:/Qt/6.5.3/msvc2019_64"
cmake --build build/d50-config-client --config Debug
```

先启动：

```powershell
cargo run --bin agent-bridge-service -- --simulate
```
