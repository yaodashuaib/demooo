//! 全链路：真实 hook 可执行文件 → 管道 → 服务 → 假设备 → stdout。
//!
//! 这条用例存在的意义是守住「hook 拿不到决策就回落到 `{}`、拿到决策才输出」
//! 这条纪律；单元测试只能验格式，验不了进程边界。

use agent_bridge_service::{Config, start};
use bridge_ipc::{Request, Response};
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

fn unique_pipe(name: &str) -> String {
    format!(r"\\.\pipe\d50-hook-e2e-{}-{}", name, uuid::Uuid::new_v4())
}

async fn wait_shown(pipe: &str) {
    for _ in 0..80 {
        if let Ok(Response::Device { device }) =
            bridge_ipc::call(pipe, &Request::DeviceStatus, Duration::from_millis(2_000)).await
        {
            if device.shown >= 1 {
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("hook 的请求没有出现在设备上");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hook_prints_allow_after_button_press() {
    let pipe = unique_pipe("allow");
    let bridge = start(Config {
        pipe: pipe.clone(),
        simulate: true,
        serial: None,
        device_index: 0,
        report_id_prefix: true,
    })
    .expect("bridge starts");

    let payload = serde_json::json!({
        "session_id": "s1",
        "tool_name": "Bash",
        "tool_input": { "command": "rm -rf build" },
        "cwd": "D:/work"
    })
    .to_string();

    let mut child = Command::new(env!("CARGO_BIN_EXE_d50-hook"))
        .args([
            "--pipe",
            &pipe,
            "--agent",
            "claude-code",
            "--timeout-ms",
            "10000",
            "--request-id",
            "req-hook-1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn hook");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(payload.as_bytes())
        .expect("write payload");

    wait_shown(&pipe).await;
    bridge_ipc::call(
        &pipe,
        &Request::DevicePress {
            request_id: "req-hook-1".into(),
            button_index: 1,
        },
        Duration::from_millis(2_000),
    )
    .await
    .expect("press");

    let output = child.wait_with_output().expect("hook exits");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "hook 必须 exit 0，stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains(r#""behavior":"allow""#),
        "unexpected stdout: {stdout}"
    );

    bridge.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hook_without_service_falls_back_to_empty_object() {
    // 服务没启动：hook 必须安静地输出 {} 并成功退出，让 Agent 自己问。
    let pipe = unique_pipe("noservice");
    let payload = serde_json::json!({ "tool_name": "Bash" }).to_string();
    let mut child = Command::new(env!("CARGO_BIN_EXE_d50-hook"))
        .args([
            "--pipe",
            &pipe,
            "--agent",
            "claude-code",
            "--timeout-ms",
            "500",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn hook");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(payload.as_bytes())
        .expect("write payload");

    let output = child.wait_with_output().expect("hook exits");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "{}");
}
