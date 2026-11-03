//! 设备工作线程：唯一碰 HID 的地方。
//!
//! 上层只提交「现在应该显示哪几条会话」，不关心帧、ACK、心跳或重传。
//! 设备事件通过 tokio 通道回到服务，跨线程不需要 `async` 设备接口。

use crate::state::to_conversation;
use approval_core::SlotView;
use d50_hid::{Device, Link, SessionEvent};
use d50_protocol::{command, commands};
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::thread;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;

enum WorkerCmd {
    /// 用 `revision` 丢弃迟到的旧投影，避免两个处理器交错把设备写回旧内容。
    Sync { revision: u64, slots: Vec<SlotView> },
}

#[derive(Clone)]
pub struct DeviceWorker {
    commands: Sender<WorkerCmd>,
}

impl DeviceWorker {
    pub fn spawn(link: Box<dyn Link>, events: UnboundedSender<SessionEvent>) -> Self {
        let (commands, inbox) = channel();
        thread::spawn(move || run(link, inbox, events));
        Self { commands }
    }

    pub fn sync(&self, revision: u64, slots: Vec<SlotView>) {
        let _ = self.commands.send(WorkerCmd::Sync { revision, slots });
    }
}

fn run(
    link: Box<dyn Link>,
    inbox: std::sync::mpsc::Receiver<WorkerCmd>,
    events: UnboundedSender<SessionEvent>,
) {
    let device = Device::spawn(link);
    prepare(&device);

    let mut last_revision = 0;
    loop {
        while let Some(event) = device.try_event() {
            if events.send(event).is_err() {
                return;
            }
        }
        match inbox.recv_timeout(Duration::from_millis(20)) {
            Ok(WorkerCmd::Sync { revision, slots }) => {
                if revision <= last_revision {
                    continue;
                }
                last_revision = revision;
                let conversations: Vec<_> = slots.iter().map(to_conversation).collect();
                let payload = match commands::set_conversation_list(&conversations) {
                    Ok(payload) => payload,
                    Err(_) => continue,
                };
                if device
                    .request(command::SET_CONVERSATION_LIST, payload)
                    .is_err()
                {
                    let _ = events.send(SessionEvent::LinkLost("conversation sync failed".into()));
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// 连接后的基础设施，失败不影响会话推送——真机上先让审批能跑。
fn prepare(device: &Device) {
    let _ = device.request(
        command::SET_PAGE_CONFIG,
        commands::set_page_config(&["tasks", "balance", "clock", "schedule_1", "game"])
            .unwrap_or_default(),
    );
    let now = chrono::Local::now();
    use chrono::{Datelike, Timelike};
    let _ = device.request(
        command::TIME_CONFIG,
        commands::time_config(
            now.year() as u16,
            now.month() as u8,
            now.day() as u8,
            now.hour() as u8,
            now.minute() as u8,
            now.second() as u8,
        ),
    );
}
