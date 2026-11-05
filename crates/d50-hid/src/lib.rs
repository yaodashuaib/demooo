//! D50 的 HID 传输与协议会话。
//!
//! 一条纪律：同一时刻只有一个线程碰 HID。写入、读回、ACK 等待、心跳全部在
//! 这个线程里串行完成，因此不存在「ACK 迟到被当成新响应」的窗口。
//!
//! 本 crate 不认识审批，只认识帧和 D2H 事件。

pub mod sim;

#[cfg(feature = "hid")]
pub mod hid;

use d50_protocol::{
    ControlEvent, ConversationAction, DeviceInfo, Frame, FrameType, HostType, ReturnType, Scanner,
    command, commands, encode, return_type, split_payload,
};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use thiserror::Error;

/// Vendor HID 有效区固定 64 字节，不使用 Report ID。
pub const REPORT_LEN: usize = 64;
/// 单个应用帧最多带多少 Payload；超过就用多帧。
///
/// 与硬件侧发图工具一致：应用帧 Payload=1024，走 First/Middle/End；HID 再切 64B report。
pub const MAX_FRAME_PAYLOAD: usize = 1024;
/// FIRST/MIDDLE 几乎立刻 ACK；END 要等设备解码整段 PNG（硬件日志约 1.5s）。
pub const ACK_TIMEOUT: Duration = Duration::from_secs(5);
pub const MAX_RETRIES: u32 = 2;
/// PC 空闲超过 1 秒必须补心跳，否则设备 5 秒后判离线。
pub const HEARTBEAT_IDLE: Duration = Duration::from_millis(1000);
const POLL_MS: i32 = 20;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LinkError {
    #[error("HID_IO: {0}")]
    Io(String),
    #[error("HID_CLOSED")]
    Closed,
}

/// 一个 64 字节 report 的收发通道。实现方不需要知道帧边界。
pub trait Link: Send + 'static {
    fn write_report(&mut self, report: &[u8; REPORT_LEN]) -> Result<(), LinkError>;
    /// 阻塞至多 `timeout_ms`；没有数据返回 `Ok(None)`，链路断了返回 `Err`。
    fn read_report(&mut self, timeout_ms: i32) -> Result<Option<[u8; REPORT_LEN]>, LinkError>;
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SessionError {
    #[error("DEVICE_TIMEOUT")]
    Timeout,
    #[error("DEVICE_OFFLINE")]
    Offline,
    #[error("PROTOCOL_ERROR")]
    Protocol,
    #[error("INVALID_ARGUMENT")]
    Invalid,
}

impl From<d50_protocol::ProtocolError> for SessionError {
    fn from(_: d50_protocol::ProtocolError) -> Self {
        SessionError::Protocol
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    Action(ConversationAction),
    Control(ControlEvent),
    ProtocolVersion {
        major: u8,
        minor: u8,
    },
    DeviceInfo(DeviceInfo),
    LinkLost(String),
    /// 收到了本层不处理的 D2H 命令；保留命令号便于诊断。
    Ignored {
        command: u8,
    },
}

enum Command {
    Send {
        id: u8,
        payload: Vec<u8>,
        reply: Sender<Result<Vec<u8>, SessionError>>,
    },
}

/// 会话句柄。线程在后台跑，`request` 同步阻塞到有结果或超时。
pub struct Device {
    commands: Sender<Command>,
    events: Receiver<SessionEvent>,
    thread: Option<JoinHandle<()>>,
}

impl Device {
    pub fn spawn(link: Box<dyn Link>) -> Self {
        let (command_tx, command_rx) = channel();
        let (event_tx, event_rx) = channel();
        let thread = thread::spawn(move || run(link, command_rx, event_tx));
        Self {
            commands: command_tx,
            events: event_rx,
            thread: Some(thread),
        }
    }

    pub fn request(&self, id: u8, payload: Vec<u8>) -> Result<Vec<u8>, SessionError> {
        let (tx, rx) = channel();
        // 大 payload（如 SET_AGENT_INDEX 多图）按帧数放宽等待，避免外层先超时、会话还在传。
        let frame_count = payload.len().div_ceil(MAX_FRAME_PAYLOAD).max(1) as u32;
        self.commands
            .send(Command::Send {
                id,
                payload,
                reply: tx,
            })
            .map_err(|_| SessionError::Offline)?;
        let budget = ACK_TIMEOUT * (MAX_RETRIES + 1) * frame_count
            + Duration::from_secs(2);
        rx.recv_timeout(budget).map_err(|_| SessionError::Timeout)?
    }

    pub fn events(&self) -> &Receiver<SessionEvent> {
        &self.events
    }

    /// 不阻塞地取一条事件。
    pub fn try_event(&self) -> Option<SessionEvent> {
        self.events.try_recv().ok()
    }

    pub fn recv_event_timeout(&self, timeout: Duration) -> Option<SessionEvent> {
        self.events.recv_timeout(timeout).ok()
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        // 丢掉 sender，后台线程会在下一次 try_recv 看到 Disconnected 并退出。
        let (dead, _) = channel::<Command>();
        self.commands = dead;
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

fn run(mut link: Box<dyn Link>, commands: Receiver<Command>, events: Sender<SessionEvent>) {
    let mut scanner = Scanner::new();

    if !handshake(&mut link, &mut scanner, &events) {
        let _ = events.send(SessionEvent::LinkLost("handshake failed".into()));
        return;
    }
    let mut last_tx = Instant::now();

    loop {
        match commands.try_recv() {
            Ok(Command::Send { id, payload, reply }) => {
                let result = transact(&mut link, &mut scanner, &events, &mut last_tx, id, payload);
                let _ = reply.send(result);
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => break,
        }

        match poll(&mut link, &mut scanner, &events) {
            Ok(()) => {}
            Err(SessionError::Offline) => {
                let _ = events.send(SessionEvent::LinkLost("read failed".into()));
                break;
            }
            Err(_) => {}
        }

        if last_tx.elapsed() >= HEARTBEAT_IDLE {
            let payload = commands::heartbeat(HostType::Studio);
            if write_frame(
                &mut link,
                command::HEARTBEAT,
                FrameType::Complete,
                0,
                &payload,
            )
            .is_err()
            {
                let _ = events.send(SessionEvent::LinkLost("heartbeat failed".into()));
                break;
            }
            last_tx = Instant::now();
        }
    }
}

fn handshake(
    link: &mut Box<dyn Link>,
    scanner: &mut Scanner,
    events: &Sender<SessionEvent>,
) -> bool {
    let mut last_tx = Instant::now();
    let version = transact(
        link,
        scanner,
        events,
        &mut last_tx,
        command::GET_PROTOCOL_VERSION,
        Vec::new(),
    );
    match version {
        Ok(payload) if payload.len() >= 2 => {
            let _ = events.send(SessionEvent::ProtocolVersion {
                major: payload[0],
                minor: payload[1],
            });
        }
        _ => return false,
    }

    let info = transact(
        link,
        scanner,
        events,
        &mut last_tx,
        command::GET_DEVICE_INFO,
        Vec::new(),
    );
    match info {
        Ok(payload) => match serde_json::from_slice::<DeviceInfo>(&payload) {
            // 用协议自报的 DeviceType 认设备，而不是猜 VID/PID。
            Ok(info) if info.device_type == "D50" => {
                let _ = events.send(SessionEvent::DeviceInfo(info));
            }
            _ => return false,
        },
        Err(_) => return false,
    }
    true
}

fn transact(
    link: &mut Box<dyn Link>,
    scanner: &mut Scanner,
    events: &Sender<SessionEvent>,
    last_tx: &mut Instant,
    id: u8,
    payload: Vec<u8>,
) -> Result<Vec<u8>, SessionError> {
    let kind = return_type(id);
    // 与硬件日志一致：按 1024 切 First/Middle/End，逐帧等 ACK。
    let frames = split_payload(&payload, MAX_FRAME_PAYLOAD)?;
    let mut response = Vec::new();
    for (frame_type, seq, chunk) in frames {
        let mut attempt = 0;
        loop {
            if write_frame(link, id, frame_type, seq, chunk).is_err() {
                return Err(SessionError::Offline);
            }
            *last_tx = Instant::now();
            // 心跳这类命令设备不回任何东西，写出去就算完成。
            if kind == ReturnType::None {
                break;
            }
            match wait_frame(link, scanner, events, id, frame_type, seq, kind) {
                Ok(Some(body)) => {
                    response = body;
                    break;
                }
                Ok(None) if kind == ReturnType::Ack => break,
                Ok(None) => {}
                Err(SessionError::Timeout) if attempt < MAX_RETRIES => {
                    attempt += 1;
                    continue;
                }
                Err(other) => return Err(other),
            }
        }
    }
    Ok(response)
}

/// 等一帧「匹配的确认」：ACK 看空 Payload，Response 看非空 Payload。
/// 期间收到的其它帧照常派发，不会被吞掉。
fn wait_frame(
    link: &mut Box<dyn Link>,
    scanner: &mut Scanner,
    events: &Sender<SessionEvent>,
    command_id: u8,
    frame_type: FrameType,
    seq: u8,
    kind: ReturnType,
) -> Result<Option<Vec<u8>>, SessionError> {
    let deadline = Instant::now() + ACK_TIMEOUT;
    loop {
        if let Some(frame) = scanner.next_frame() {
            let matched =
                frame.command == command_id && frame.frame_type == frame_type && frame.seq == seq;
            match (matched, kind) {
                (true, ReturnType::Ack) if frame.is_ack() => return Ok(None),
                (true, ReturnType::Response) if frame.is_response() => {
                    return Ok(Some(frame.payload));
                }
                (true, _) => {}
                (false, _) => dispatch(&frame, events),
            }
            continue;
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(SessionError::Timeout);
        }
        let remaining = deadline - now;
        let wait = remaining.as_millis().min(POLL_MS as u128) as i32;
        match link.read_report(wait.max(1)) {
            Ok(Some(report)) => scanner.feed(&report),
            Ok(None) => {}
            Err(_) => return Err(SessionError::Offline),
        }
    }
}

fn poll(
    link: &mut Box<dyn Link>,
    scanner: &mut Scanner,
    events: &Sender<SessionEvent>,
) -> Result<(), SessionError> {
    match link.read_report(POLL_MS) {
        Ok(Some(report)) => {
            scanner.feed(&report);
            while let Some(frame) = scanner.next_frame() {
                dispatch(&frame, events);
            }
            Ok(())
        }
        Ok(None) => Ok(()),
        Err(_) => Err(SessionError::Offline),
    }
}

fn dispatch(frame: &Frame, events: &Sender<SessionEvent>) {
    let event = match frame.command {
        command::CONVERSATION_ACTION => match ConversationAction::decode(&frame.payload) {
            Ok(action) => SessionEvent::Action(action),
            // 坏帧不能被当成一次按键：宁可丢掉。
            Err(_) => SessionEvent::Ignored {
                command: frame.command,
            },
        },
        command::CONTROL_EVENT => match ControlEvent::decode(&frame.payload) {
            Ok(control) => SessionEvent::Control(control),
            Err(_) => SessionEvent::Ignored {
                command: frame.command,
            },
        },
        _ => SessionEvent::Ignored {
            command: frame.command,
        },
    };
    let _ = events.send(event);
}

fn write_frame(
    link: &mut Box<dyn Link>,
    id: u8,
    frame_type: FrameType,
    seq: u8,
    payload: &[u8],
) -> Result<(), SessionError> {
    let bytes = encode(id, frame_type, seq, payload)?;
    for chunk in bytes.chunks(REPORT_LEN) {
        let mut report = [0u8; REPORT_LEN];
        report[..chunk.len()].copy_from_slice(chunk);
        link.write_report(&report)
            .map_err(|_| SessionError::Offline)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::sim;
    use super::*;
    use d50_protocol::Conversation;

    fn conversation(id: &str) -> Conversation {
        Conversation {
            id: id.into(),
            title: "Claude Code · Bash".into(),
            path: "D:/work".into(),
            content: "rm -rf build".into(),
            agent_index: 0,
            needs_attention: 1,
            buttons: vec![
                d50_protocol::Button {
                    index: 1,
                    name: "同意".into(),
                },
                d50_protocol::Button {
                    index: 2,
                    name: "拒绝".into(),
                },
            ],
        }
    }

    #[test]
    fn handshake_identifies_device_by_protocol_not_vid() {
        let (link, _handle) = sim::pair();
        let device = Device::spawn(Box::new(link));
        let mut info = None;
        let mut version = None;
        for _ in 0..12 {
            match device.recv_event_timeout(Duration::from_millis(200)) {
                Some(SessionEvent::DeviceInfo(value)) => info = Some(value),
                Some(SessionEvent::ProtocolVersion { major, minor }) => {
                    version = Some((major, minor))
                }
                _ => {}
            }
            if info.is_some() && version.is_some() {
                break;
            }
        }
        assert_eq!(version, Some((0, 1)));
        assert_eq!(info.unwrap().device_type, "D50");
    }

    #[test]
    fn conversation_push_is_acked_and_lands_on_device() {
        let (link, handle) = sim::pair();
        let device = Device::spawn(Box::new(link));
        let payload =
            d50_protocol::commands::set_conversation_list(&[conversation("conv-1")]).unwrap();
        device
            .request(command::SET_CONVERSATION_LIST, payload)
            .expect("ack");
        assert_eq!(handle.conversations().len(), 1);
        assert_eq!(handle.conversations()[0].id, "conv-1");
    }

    #[test]
    fn button_press_arrives_as_action_event() {
        let (link, handle) = sim::pair();
        let device = Device::spawn(Box::new(link));
        let payload =
            d50_protocol::commands::set_conversation_list(&[conversation("conv-1")]).unwrap();
        device
            .request(command::SET_CONVERSATION_LIST, payload)
            .unwrap();
        handle.press("conv-1", 2);
        let mut action = None;
        for _ in 0..20 {
            match device.recv_event_timeout(Duration::from_millis(200)) {
                Some(SessionEvent::Action(value)) => {
                    action = Some(value);
                    break;
                }
                Some(_) => continue,
                None => break,
            }
        }
        let action = action.expect("action event");
        assert_eq!(action.id, "conv-1");
        assert_eq!(action.button_index, 2);
    }

    #[test]
    fn global_open_action_is_reported_but_not_an_answer() {
        let (link, handle) = sim::pair();
        let device = Device::spawn(Box::new(link));
        handle.press("conv-1", 0);
        let mut action = None;
        for _ in 0..20 {
            match device.recv_event_timeout(Duration::from_millis(200)) {
                Some(SessionEvent::Action(value)) => {
                    action = Some(value);
                    break;
                }
                Some(_) => continue,
                None => break,
            }
        }
        assert_eq!(action.expect("action event").button_index, 0);
    }

    #[test]
    fn unplugged_link_reports_lost() {
        let (link, handle) = sim::pair();
        let device = Device::spawn(Box::new(link));
        // 先等握手完成，否则 LinkLost 可能抢在 DeviceInfo 前面。
        let mut ready = false;
        for _ in 0..20 {
            if matches!(
                device.recv_event_timeout(Duration::from_millis(100)),
                Some(SessionEvent::DeviceInfo(_))
            ) {
                ready = true;
                break;
            }
        }
        assert!(ready, "handshake must finish first");
        handle.offline();
        let mut lost = false;
        for _ in 0..20 {
            if matches!(
                device.recv_event_timeout(Duration::from_millis(100)),
                Some(SessionEvent::LinkLost(_))
            ) {
                lost = true;
                break;
            }
        }
        assert!(lost, "link loss must be reported");
    }

    #[test]
    fn multi_frame_payload_reassembles_on_device() {
        let (link, handle) = sim::pair();
        let device = Device::spawn(Box::new(link));
        let many: Vec<Conversation> = (0..6)
            .map(|i| {
                let mut value = conversation(&format!("conv-{i}"));
                // 撑到 1024 字节以上，强制走 FIRST/MIDDLE/END。
                value.content = "x".repeat(64);
                value.title = "y".repeat(64);
                value
            })
            .collect();
        // 用超大图标 payload 强制多帧（sim 对 SET_AGENT_INDEX 只 ACK，不解析 PNG）。
        let png = vec![0u8; MAX_FRAME_PAYLOAD + 128];
        let payload = d50_protocol::commands::set_agent_index(&[(0, png.as_slice())]).unwrap();
        assert!(
            payload.len() > MAX_FRAME_PAYLOAD,
            "must exercise multi-frame"
        );
        device
            .request(command::SET_AGENT_INDEX, payload)
            .expect("multi-frame ack");
        let _ = (many, handle);
    }
}
