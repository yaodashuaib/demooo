//! hook 与审批服务之间的本机信道：`\\.\pipe\` 上的 NDJSON。
//!
//! 只对本机当前用户开放，不监听 TCP。一次连接可以发多条请求，但 `approval_request`
//! 本身是阻塞的，所以 hook 通常只发一条。
//!
//! 本 crate 仅在 Windows 上编译；其他地方没有 D50，也没有这条管道。

pub mod focus;

use approval_core::{Decision, Outcome};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions};

pub const PROTOCOL: u32 = 1;
pub const DEFAULT_PIPE: &str = r"\\.\pipe\ulanzi-d50-bridge.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestOption {
    pub index: u8,
    pub label: String,
    pub decision: Decision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum Request {
    Hello {
        protocol: u32,
        client: String,
    },
    /// 阻塞到有结果或超时；`timeout_ms` 是相对时长，截止时刻由服务自己算。
    ApprovalRequest {
        request_id: String,
        agent_id: String,
        session_id: String,
        tool_name: String,
        summary: String,
        cwd: String,
        options: Vec<RequestOption>,
        timeout_ms: u64,
        /// 会话页 deeplink；服务会在选项之后追加一颗「open app」键。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        open_app: Option<String>,
    },
    /// 非阻塞提示：Agent 需要人回到自己的界面里回答。设备只显示一个「open app」。
    Notice {
        notice_id: String,
        agent_id: String,
        session_id: String,
        title: String,
        content: String,
        link: String,
        ttl_ms: u64,
    },
    ApprovalStatus {
        request_id: String,
    },
    ApprovalCancel {
        request_id: String,
    },
    DeviceStatus,
    /// 下发一条演示会话，用来验证「设备能显示、能按键、能收回」。
    DeviceSelfTest,
    /// 仅 `--simulate` 模式可用：代替人按设备按钮。
    DevicePress {
        request_id: String,
        button_index: u8,
    },
    /// 配置投影状态：在线、Agent 槽、余额摘要、声光草稿（只读缓存，不 kick HID）。
    ConfigStatus,
    /// 绑定 allowlist Agent：分配并持久化 `agentIndex`，在线时下发 INDEX + 合并余额。
    ConfigBindAgent {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    /// 解绑 Agent：释放槽位（不重排），在线时全量 INDEX + 缓存组 BALANCE。
    ConfigUnbindAgent {
        id: String,
    },
    /// 设置余额刷新间隔（秒）；钳制 30–3600，写入 d50-bridge.json 并重置 timer。
    ConfigSetBalanceInterval {
        secs: u64,
    },
    /// 下发或预览声光；`preview` 为 true 时不写本机草稿文件。
    ConfigSetSoundLight {
        /// 与协议 `SET_SOUND_LIGHT_CONFIG` 同形的 JSON 对象。
        config: serde_json::Value,
        #[serde(default)]
        preview: bool,
    },
    /// 保存或清除 WorkBuddy 网站会话 Cookie（写入本机 AppData，不经 HID）。
    ConfigSetWorkbuddySession {
        /// 完整 Cookie 头；空字符串表示清除已保存会话。
        cookie: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        chrome_major: Option<u32>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceView {
    pub online: bool,
    pub serial: Option<String>,
    pub firmware: Option<String>,
    pub shown: usize,
    pub queued: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigAgentView {
    pub id: String,
    pub name: String,
    pub agent_index: u8,
    pub balance_ok: bool,
    pub balance_summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigView {
    pub device: DeviceView,
    pub agents: Vec<ConfigAgentView>,
    pub last_projection_at: Option<u64>,
    pub sound_light: serde_json::Value,
    /// 是否已配置 WorkBuddy 会话（不回传 Cookie 明文）。
    #[serde(default)]
    pub workbuddy_session_configured: bool,
    /// 余额全量刷新间隔（秒）。
    #[serde(default = "default_balance_refresh_secs")]
    pub balance_refresh_secs: u64,
    pub errors: Vec<String>,
}

fn default_balance_refresh_secs() -> u64 {
    180
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Response {
    Hello { protocol: u32, device: DeviceView },
    Approval { outcome: Outcome },
    Status { outcome: Option<Outcome> },
    Device { device: DeviceView },
    /// 提示是否真的显示出来了（槽位满时会被丢掉）。
    Notice { shown: bool },
    Config { config: ConfigView },
    Ok,
    Error { code: String, message: String },
}

impl Response {
    pub fn error(code: &str, message: impl Into<String>) -> Self {
        Self::Error {
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Error)]
pub enum IpcError {
    #[error("IPC_IO: {0}")]
    Io(String),
    #[error("IPC_TIMEOUT")]
    Timeout,
    #[error("IPC_PROTOCOL")]
    Protocol,
    #[error("SERVICE_UNAVAILABLE")]
    Unavailable,
}

/// 服务端：一个实例同时只接一条连接，连接建立后立刻准备下一个实例。
pub struct Listener {
    name: String,
    pending: NamedPipeServer,
}

impl Listener {
    pub fn bind(name: &str) -> Result<Self, IpcError> {
        let pending = ServerOptions::new()
            .first_pipe_instance(true)
            .create(name)
            .map_err(|error| IpcError::Io(error.to_string()))?;
        Ok(Self {
            name: name.to_owned(),
            pending,
        })
    }

    pub async fn accept(&mut self) -> Result<Connection, IpcError> {
        self.pending
            .connect()
            .await
            .map_err(|error| IpcError::Io(error.to_string()))?;
        let next = ServerOptions::new()
            .create(&self.name)
            .map_err(|error| IpcError::Io(error.to_string()))?;
        let connected = std::mem::replace(&mut self.pending, next);
        Ok(Connection {
            reader: BufReader::new(connected),
        })
    }
}

pub struct Connection {
    reader: BufReader<NamedPipeServer>,
}

impl Connection {
    /// 读一行请求；对端关闭返回 `Ok(None)`。
    pub async fn next_request(&mut self) -> Result<Option<Request>, IpcError> {
        let mut line = String::new();
        let read = self
            .reader
            .read_line(&mut line)
            .await
            .map_err(|error| IpcError::Io(error.to_string()))?;
        if read == 0 {
            return Ok(None);
        }
        let request = serde_json::from_str(line.trim_end()).map_err(|_| IpcError::Protocol)?;
        Ok(Some(request))
    }

    pub async fn send(&mut self, response: &Response) -> Result<(), IpcError> {
        let mut bytes = serde_json::to_vec(response).map_err(|_| IpcError::Protocol)?;
        bytes.push(b'\n');
        self.reader
            .get_mut()
            .write_all(&bytes)
            .await
            .map_err(|error| IpcError::Io(error.to_string()))?;
        self.reader
            .get_mut()
            .flush()
            .await
            .map_err(|error| IpcError::Io(error.to_string()))
    }
}

/// 客户端：连接、发一条请求、读一条响应，整体超时由调用方控制。
pub async fn call(name: &str, request: &Request, timeout: Duration) -> Result<Response, IpcError> {
    let mut last = IpcError::Unavailable;
    let mut client = None;
    for _ in 0..20 {
        match ClientOptions::new().open(name) {
            Ok(value) => {
                client = Some(value);
                break;
            }
            Err(error) => {
                last = IpcError::Io(error.to_string());
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        }
    }
    let client = client.ok_or(last)?;
    let mut reader = BufReader::new(client);
    let mut bytes = serde_json::to_vec(request).map_err(|_| IpcError::Protocol)?;
    bytes.push(b'\n');
    let exchange = async {
        reader
            .get_mut()
            .write_all(&bytes)
            .await
            .map_err(|error| IpcError::Io(error.to_string()))?;
        reader
            .get_mut()
            .flush()
            .await
            .map_err(|error| IpcError::Io(error.to_string()))?;
        let mut line = String::new();
        let read = reader
            .read_line(&mut line)
            .await
            .map_err(|error| IpcError::Io(error.to_string()))?;
        if read == 0 {
            return Err(IpcError::Protocol);
        }
        serde_json::from_str(line.trim_end()).map_err(|_| IpcError::Protocol)
    };
    tokio::time::timeout(timeout, exchange)
        .await
        .map_err(|_| IpcError::Timeout)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trips_through_json() {
        let request = Request::ApprovalRequest {
            request_id: "r1".into(),
            agent_id: "claude-code".into(),
            session_id: "s1".into(),
            tool_name: "Bash".into(),
            summary: "rm -rf build".into(),
            cwd: "D:/work".into(),
            options: vec![RequestOption {
                index: 1,
                label: "同意".into(),
                decision: Decision::Allow,
            }],
            timeout_ms: 55_000,
            open_app: Some("claude://code/continue?session=local_s1".into()),
        };
        let text = serde_json::to_string(&request).unwrap();
        let decoded: Request = serde_json::from_str(&text).unwrap();
        assert_eq!(decoded, request);
        assert!(text.contains(r#""method":"approval_request""#));
    }

    #[test]
    fn notice_round_trips_through_json() {
        let request = Request::Notice {
            notice_id: "n1".into(),
            agent_id: "codex".into(),
            session_id: "s1".into(),
            title: "codex · 待回复".into(),
            content: "有 3 个问题待您回复…".into(),
            link: "codex://threads/s1".into(),
            ttl_ms: 120_000,
        };
        let text = serde_json::to_string(&request).unwrap();
        let decoded: Request = serde_json::from_str(&text).unwrap();
        assert_eq!(decoded, request);
        assert!(text.contains(r#""method":"notice""#));
    }

    #[test]
    fn approval_without_open_app_stays_compatible() {
        let text = r#"{"method":"approval_request","request_id":"r1","agent_id":"codex","session_id":"s1","tool_name":"Bash","summary":"x","cwd":".","options":[{"index":1,"label":"同意","decision":"allow"}],"timeout_ms":1000}"#;
        let decoded: Request = serde_json::from_str(text).unwrap();
        match decoded {
            Request::ApprovalRequest { open_app, .. } => assert!(open_app.is_none()),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn response_tags_are_stable() {
        let response = Response::Approval {
            outcome: Outcome::Expired,
        };
        let text = serde_json::to_string(&response).unwrap();
        assert_eq!(
            text,
            r#"{"result":"approval","outcome":{"outcome":"expired"}}"#
        );
    }

    #[test]
    fn error_response_is_explicit() {
        let text = serde_json::to_string(&Response::error("QUEUE_FULL", "6 shown")).unwrap();
        assert!(text.contains(r#""result":"error""#));
        assert!(text.contains("QUEUE_FULL"));
    }

    #[test]
    fn config_bind_unbind_interval_round_trip() {
        let bind = Request::ConfigBindAgent {
            id: "chatgpt".into(),
            name: Some("ChatGPT".into()),
        };
        let text = serde_json::to_string(&bind).unwrap();
        assert!(text.contains(r#""method":"config_bind_agent""#));
        let decoded: Request = serde_json::from_str(&text).unwrap();
        assert_eq!(decoded, bind);

        let unbind = Request::ConfigUnbindAgent {
            id: "kimi".into(),
        };
        let text = serde_json::to_string(&unbind).unwrap();
        assert!(text.contains(r#""method":"config_unbind_agent""#));

        let interval = Request::ConfigSetBalanceInterval { secs: 180 };
        let text = serde_json::to_string(&interval).unwrap();
        assert!(text.contains(r#""method":"config_set_balance_interval""#));
        assert!(text.contains("180"));
    }
}
