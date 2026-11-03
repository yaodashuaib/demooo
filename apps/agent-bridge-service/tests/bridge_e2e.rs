//! 端到端：管道 → 服务 → 假设备 → 按键 → 回填。
//!
//! 这些用例不需要真机，也不需要 hook 进程，验证的是「服务 + 状态机 + 设备会话」
//! 三者在真实 IPC 上的配合。

use agent_bridge_service::{Config, start};
use approval_core::{Decision, Outcome};
use bridge_ipc::{Request, RequestOption, Response};
use std::time::Duration;

fn unique_pipe(name: &str) -> String {
    format!(r"\\.\pipe\d50-e2e-{}-{}", name, uuid::Uuid::new_v4())
}

fn approval_request(id: &str, timeout_ms: u64) -> Request {
    Request::ApprovalRequest {
        request_id: id.into(),
        agent_id: "claude-code".into(),
        session_id: "s1".into(),
        tool_name: "Bash".into(),
        summary: "rm -rf build".into(),
        cwd: "D:/work".into(),
        options: vec![
            RequestOption {
                index: 1,
                label: "同意".into(),
                decision: Decision::Allow,
            },
            RequestOption {
                index: 2,
                label: "拒绝".into(),
                decision: Decision::Deny,
            },
        ],
        timeout_ms,
        open_app: None,
    }
}

async fn send(pipe: &str, request: Request, timeout_ms: u64) -> Response {
    bridge_ipc::call(pipe, &request, Duration::from_millis(timeout_ms))
        .await
        .expect("ipc call")
}

/// 等到设备投影里出现 `shown` 条，再按下按钮，避免与服务注册请求抢跑。
async fn wait_shown(pipe: &str, shown: usize) {
    for _ in 0..80 {
        if let Response::Device { device } = send(pipe, Request::DeviceStatus, 2_000).await {
            if device.shown >= shown {
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("device projection never reached {shown} shown items");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pressing_allow_returns_allow_and_is_idempotent() {
    let pipe = unique_pipe("allow");
    let bridge = start(Config {
        pipe: pipe.clone(),
        simulate: true,
        serial: None,
        device_index: 0,
        report_id_prefix: true,
    })
    .expect("bridge starts");

    let waiting = {
        let pipe = pipe.clone();
        tokio::spawn(async move { send(&pipe, approval_request("req-allow", 8_000), 15_000).await })
    };
    wait_shown(&pipe, 1).await;

    assert!(matches!(
        send(
            &pipe,
            Request::DevicePress {
                request_id: "req-allow".into(),
                button_index: 1
            },
            2_000
        )
        .await,
        Response::Ok
    ));

    match waiting.await.expect("client task") {
        Response::Approval { outcome } => match outcome {
            Outcome::Decided {
                decision,
                option_index,
                ..
            } => {
                assert_eq!(decision, Decision::Allow);
                assert_eq!(option_index, 1);
            }
            other => panic!("expected allow, got {other:?}"),
        },
        other => panic!("unexpected response {other:?}"),
    }

    // 同一个 request_id 重试必须拿到同一个答案，而不是再等一次。
    match send(&pipe, approval_request("req-allow", 8_000), 3_000).await {
        Response::Approval { outcome } => assert!(matches!(
            outcome,
            Outcome::Decided {
                decision: Decision::Allow,
                ..
            }
        )),
        other => panic!("duplicate should replay the decision, got {other:?}"),
    }

    bridge.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pressing_deny_returns_deny() {
    let pipe = unique_pipe("deny");
    let bridge = start(Config {
        pipe: pipe.clone(),
        simulate: true,
        serial: None,
        device_index: 0,
        report_id_prefix: true,
    })
    .expect("bridge starts");

    let waiting = {
        let pipe = pipe.clone();
        tokio::spawn(async move { send(&pipe, approval_request("req-deny", 8_000), 15_000).await })
    };
    wait_shown(&pipe, 1).await;
    send(
        &pipe,
        Request::DevicePress {
            request_id: "req-deny".into(),
            button_index: 2,
        },
        2_000,
    )
    .await;

    match waiting.await.expect("client task") {
        Response::Approval {
            outcome:
                Outcome::Decided {
                    decision,
                    option_index,
                    ..
                },
        } => {
            assert_eq!(decision, Decision::Deny);
            assert_eq!(option_index, 2);
        }
        other => panic!("expected deny, got {other:?}"),
    }
    bridge.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nobody_presses_means_expired_not_denied() {
    let pipe = unique_pipe("expire");
    let bridge = start(Config {
        pipe: pipe.clone(),
        simulate: true,
        serial: None,
        device_index: 0,
        report_id_prefix: true,
    })
    .expect("bridge starts");

    let response = send(&pipe, approval_request("req-expire", 300), 6_000).await;
    assert_eq!(
        response,
        Response::Approval {
            outcome: Outcome::Expired
        },
        "超时不能变成拒绝"
    );

    // 已经到期的请求不应再占设备槽位。
    if let Response::Device { device } = send(&pipe, Request::DeviceStatus, 2_000).await {
        assert_eq!(device.shown, 0);
    }
    bridge.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn seventh_request_queues_instead_of_evicting() {
    let pipe = unique_pipe("queue");
    let bridge = start(Config {
        pipe: pipe.clone(),
        simulate: true,
        serial: None,
        device_index: 0,
        report_id_prefix: true,
    })
    .expect("bridge starts");

    let mut tasks = Vec::new();
    for index in 0..7 {
        let pipe = pipe.clone();
        tasks.push(tokio::spawn(async move {
            let id = format!("req-{index}");
            send(&pipe, approval_request(&id, 20_000), 30_000).await
        }));
    }

    let mut seen = None;
    for _ in 0..80 {
        if let Response::Device { device } = send(&pipe, Request::DeviceStatus, 2_000).await {
            if device.shown == 6 && device.queued == 1 {
                seen = Some(device);
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let device = seen.expect("six shown, one queued");
    assert_eq!(device.shown + device.queued, 7);

    for task in tasks {
        task.abort();
    }
    bridge.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unknown_button_does_not_decide() {
    let pipe = unique_pipe("badbutton");
    let bridge = start(Config {
        pipe: pipe.clone(),
        simulate: true,
        serial: None,
        device_index: 0,
        report_id_prefix: true,
    })
    .expect("bridge starts");

    let waiting = {
        let pipe = pipe.clone();
        tokio::spawn(async move { send(&pipe, approval_request("req-bad", 1_200), 8_000).await })
    };
    wait_shown(&pipe, 1).await;
    // 3 号按钮不在选项里：服务应当忽略，而不是把请求判掉。
    send(
        &pipe,
        Request::DevicePress {
            request_id: "req-bad".into(),
            button_index: 3,
        },
        2_000,
    )
    .await;

    match waiting.await.expect("client task") {
        Response::Approval { outcome } => assert_eq!(outcome, Outcome::Expired),
        other => panic!("unexpected response {other:?}"),
    }
    bridge.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn notice_shows_one_button_and_disappears_after_press() {
    let pipe = unique_pipe("notice");
    let bridge = start(Config {
        pipe: pipe.clone(),
        simulate: true,
        serial: None,
        device_index: 0,
        report_id_prefix: true,
    })
    .expect("bridge starts");

    match send(
        &pipe,
        Request::Notice {
            notice_id: "notice-1".into(),
            agent_id: "claude-code".into(),
            session_id: "s1".into(),
            title: "Claude Code · 待回复".into(),
            content: "有 3 个问题待您回复…".into(),
            link: "claude://code/continue?session=local_s1".into(),
            ttl_ms: 60_000,
        },
        2_000,
    )
    .await
    {
        Response::Notice { shown } => assert!(shown),
        other => panic!("notice should be shown, got {other:?}"),
    }
    wait_shown(&pipe, 1).await;

    // 唯一的键就是「open app」，按下去只关掉提示，不产生任何裁决。
    send(
        &pipe,
        Request::DevicePress {
            request_id: "notice-1".into(),
            button_index: 1,
        },
        2_000,
    )
    .await;

    for _ in 0..40 {
        if let Response::Device { device } = send(&pipe, Request::DeviceStatus, 2_000).await {
            if device.shown == 0 {
                bridge.shutdown().await;
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("notice was never dismissed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn open_app_button_does_not_decide() {
    let pipe = unique_pipe("openapp");
    let bridge = start(Config {
        pipe: pipe.clone(),
        simulate: true,
        serial: None,
        device_index: 0,
        report_id_prefix: true,
    })
    .expect("bridge starts");

    let mut request = approval_request("req-open", 3_000);
    if let Request::ApprovalRequest { open_app, .. } = &mut request {
        *open_app = Some("codex://threads/s1".into());
    }
    let waiting = {
        let pipe = pipe.clone();
        tokio::spawn(async move { send(&pipe, request, 10_000).await })
    };
    wait_shown(&pipe, 1).await;

    // 2 个选项 → open app 是 3 号。按它不能裁决。
    send(
        &pipe,
        Request::DevicePress {
            request_id: "req-open".into(),
            button_index: 3,
        },
        2_000,
    )
    .await;
    match send(
        &pipe,
        Request::ApprovalStatus {
            request_id: "req-open".into(),
        },
        2_000,
    )
    .await
    {
        // 打开 App 之后请求仍在等：还能继续裁决。
        Response::Status { outcome: None } => {}
        other => panic!("open app must not decide, got {other:?}"),
    }

    send(
        &pipe,
        Request::DevicePress {
            request_id: "req-open".into(),
            button_index: 2,
        },
        2_000,
    )
    .await;
    match waiting.await.expect("client task") {
        Response::Approval { outcome } => assert!(matches!(
            outcome,
            Outcome::Decided {
                decision: Decision::Deny,
                ..
            }
        )),
        other => panic!("unexpected response {other:?}"),
    }
    bridge.shutdown().await;
}
