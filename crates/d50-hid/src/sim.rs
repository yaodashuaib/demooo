//! 内存版 D50：按协议回 ACK / Response，并允许测试注入按键与掉线。
//!
//! 它不是"假数据"，而是同一份协议的另一端实现：组帧、校验、多帧重组、
//! 每帧 ACK 都走 `d50-protocol`，因此上层代码无法靠特判绕过。

use crate::{Link, LinkError, REPORT_LEN};
use d50_protocol::{
    Conversation, ConversationList, Frame, FrameType, Reassembler, Scanner, command, encode,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

pub fn pair() -> (SimLink, SimHandle) {
    let (to_device, device_rx) = channel();
    let (host_tx, from_device) = channel();
    let handle_tx = host_tx.clone();
    let state = Arc::new(Mutex::new(Vec::new()));
    let offline = Arc::new(AtomicBool::new(false));

    let device_state = Arc::clone(&state);
    let device_offline = Arc::clone(&offline);
    thread::spawn(move || device_loop(device_rx, host_tx, device_state, device_offline));

    (
        SimLink {
            to_device,
            from_device,
            offline: Arc::clone(&offline),
        },
        SimHandle {
            to_host: handle_tx,
            state,
            offline,
        },
    )
}

pub struct SimLink {
    to_device: Sender<[u8; REPORT_LEN]>,
    from_device: Receiver<[u8; REPORT_LEN]>,
    offline: Arc<AtomicBool>,
}

impl Link for SimLink {
    fn write_report(&mut self, report: &[u8; REPORT_LEN]) -> Result<(), LinkError> {
        if self.offline.load(Ordering::SeqCst) {
            return Err(LinkError::Closed);
        }
        self.to_device.send(*report).map_err(|_| LinkError::Closed)
    }

    fn read_report(&mut self, timeout_ms: i32) -> Result<Option<[u8; REPORT_LEN]>, LinkError> {
        if self.offline.load(Ordering::SeqCst) {
            return Err(LinkError::Closed);
        }
        let wait = Duration::from_millis(timeout_ms.max(1) as u64);
        match self.from_device.recv_timeout(wait) {
            Ok(report) => Ok(Some(report)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(LinkError::Closed),
        }
    }
}

#[derive(Clone)]
pub struct SimHandle {
    to_host: Sender<[u8; REPORT_LEN]>,
    state: Arc<Mutex<Vec<Conversation>>>,
    offline: Arc<AtomicBool>,
}

impl SimHandle {
    /// 模拟用户按下某个会话上的反馈按钮；`button = 0` 是「打开会话」。
    pub fn press(&self, id: &str, button: u8) {
        let payload = serde_json::json!({ "ID": id, "ButtonIndex": button }).to_string();
        let frame = encode(
            command::CONVERSATION_ACTION,
            FrameType::Complete,
            0,
            payload.as_bytes(),
        )
        .expect("valid action frame");
        write_reports(&self.to_host, &frame);
    }

    pub fn conversations(&self) -> Vec<Conversation> {
        self.state.lock().unwrap().clone()
    }

    pub fn offline(&self) {
        self.offline.store(true, Ordering::SeqCst);
    }
}

fn device_loop(
    reports: Receiver<[u8; REPORT_LEN]>,
    to_host: Sender<[u8; REPORT_LEN]>,
    state: Arc<Mutex<Vec<Conversation>>>,
    offline: Arc<AtomicBool>,
) {
    let mut scanner = Scanner::new();
    let mut reassembler = Reassembler::new();
    loop {
        if offline.load(Ordering::SeqCst) {
            break;
        }
        match reports.recv_timeout(Duration::from_millis(50)) {
            Ok(report) => {
                scanner.feed(&report);
                while let Some(frame) = scanner.next_frame() {
                    handle(&to_host, &state, &mut reassembler, frame);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

/// 协议第 6.3 节：FIRST/MIDDLE 在 Payload 被接纳后立刻 ACK，END 在整段处理成功后 ACK。
fn handle(
    to_host: &Sender<[u8; REPORT_LEN]>,
    state: &Arc<Mutex<Vec<Conversation>>>,
    reassembler: &mut Reassembler,
    frame: Frame,
) {
    let (id, frame_type, seq) = (frame.command, frame.frame_type, frame.seq);
    match frame_type {
        FrameType::Complete => {
            if process(to_host, state, id, &frame.payload) {
                ack(to_host, id, frame_type, seq);
            }
        }
        FrameType::First | FrameType::Middle => {
            if reassembler.push(frame).is_ok() {
                ack(to_host, id, frame_type, seq);
            }
        }
        FrameType::End => {
            let done = reassembler.push(frame);
            if let Ok(Some((command_id, payload))) = done {
                if process(to_host, state, command_id, &payload) {
                    ack(to_host, id, frame_type, seq);
                }
            }
        }
    }
}

/// 返回 true 表示已成功处理、应当回 ACK；false 表示按协议静默丢弃。
fn process(
    to_host: &Sender<[u8; REPORT_LEN]>,
    state: &Arc<Mutex<Vec<Conversation>>>,
    id: u8,
    payload: &[u8],
) -> bool {
    match id {
        command::HEARTBEAT => true,
        command::GET_PROTOCOL_VERSION => {
            respond(to_host, id, &[0x00, 0x01]);
            true
        }
        command::GET_DEVICE_INFO => {
            respond(
                to_host,
                id,
                br#"{"SerialNumber":"D50SIM0001","Dversion":"1.0.0","DeviceType":"D50","HardwareVersion":"1.1"}"#,
            );
            true
        }
        command::SET_CONVERSATION_LIST => match serde_json::from_slice::<ConversationList>(payload)
        {
            Ok(list) => {
                *state.lock().unwrap() = list.conversations;
                true
            }
            Err(_) => false,
        },
        command::UPSERT_CONVERSATION => match serde_json::from_slice::<Conversation>(payload) {
            Ok(value) => {
                let mut guard = state.lock().unwrap();
                match guard.iter().position(|item| item.id == value.id) {
                    Some(index) => guard[index] = value,
                    None => {
                        guard.insert(0, value);
                        guard.truncate(6);
                    }
                }
                true
            }
            Err(_) => false,
        },
        _ => true,
    }
}

fn respond(to_host: &Sender<[u8; REPORT_LEN]>, id: u8, payload: &[u8]) {
    let bytes = encode(id, FrameType::Complete, 0, payload).expect("response frame");
    write_reports(to_host, &bytes);
}

fn ack(to_host: &Sender<[u8; REPORT_LEN]>, id: u8, frame_type: FrameType, seq: u8) {
    let bytes = encode(id, frame_type, seq, &[]).expect("ack frame");
    write_reports(to_host, &bytes);
}

fn write_reports(to_host: &Sender<[u8; REPORT_LEN]>, bytes: &[u8]) {
    for chunk in bytes.chunks(REPORT_LEN) {
        let mut report = [0u8; REPORT_LEN];
        report[..chunk.len()].copy_from_slice(chunk);
        if to_host.send(report).is_err() {
            return;
        }
    }
}
