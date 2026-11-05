//! 设备工作线程：唯一碰 HID 的地方。
//!
//! 上层只提交「会话投影 / 图标 / 余额 / 声光」，不关心帧、ACK、心跳或重传。
//! 设备事件通过 tokio 通道回到服务，跨线程不需要 `async` 设备接口。
//!
//! 真机断链后按 `reopen` 回调自动重开 HID；配置类下发失败只打日志，不主动判死链。

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
    Sync {
        revision: u64,
        slots: Vec<SlotView>,
    },
    SetAgentIcons {
        entries: Vec<(u8, Vec<u8>)>,
    },
    SetAgentBalance {
        payload: Vec<u8>,
    },
    SetSoundLight {
        payload: Vec<u8>,
    },
}

pub type ReopenFn = Box<dyn FnMut() -> Result<Box<dyn Link>, String> + Send>;

#[derive(Clone)]
pub struct DeviceWorker {
    commands: Sender<WorkerCmd>,
}

impl DeviceWorker {
    pub fn spawn(
        link: Box<dyn Link>,
        events: UnboundedSender<SessionEvent>,
        reopen: Option<ReopenFn>,
    ) -> Self {
        let (commands, inbox) = channel();
        thread::spawn(move || run(link, reopen, inbox, events));
        Self { commands }
    }

    pub fn sync(&self, revision: u64, slots: Vec<SlotView>) {
        let _ = self.commands.send(WorkerCmd::Sync { revision, slots });
    }

    pub fn set_agent_icons(&self, entries: Vec<(u8, Vec<u8>)>) {
        let _ = self.commands.send(WorkerCmd::SetAgentIcons { entries });
    }

    pub fn set_agent_balance(&self, payload: Vec<u8>) {
        let _ = self.commands.send(WorkerCmd::SetAgentBalance { payload });
    }

    pub fn set_sound_light(&self, payload: Vec<u8>) {
        let _ = self.commands.send(WorkerCmd::SetSoundLight { payload });
    }
}

fn run(
    initial: Box<dyn Link>,
    mut reopen: Option<ReopenFn>,
    inbox: std::sync::mpsc::Receiver<WorkerCmd>,
    events: UnboundedSender<SessionEvent>,
) {
    let mut link = Some(initial);
    loop {
        let next = match link.take() {
            Some(l) => l,
            None => {
                let Some(reopen) = reopen.as_mut() else {
                    return;
                };
                loop {
                    match reopen() {
                        Ok(l) => break l,
                        Err(err) => {
                            eprintln!("device: reopen failed: {err}");
                            thread::sleep(Duration::from_secs(2));
                        }
                    }
                }
            }
        };

        let device = Device::spawn(next);
        prepare(&device);
        if !session_loop(&device, &inbox, &events) {
            return; // inbox disconnected
        }
        eprintln!("device: session ended; will reopen if possible");
        link = None;
        thread::sleep(Duration::from_millis(500));
        if reopen.is_none() {
            return;
        }
    }
}

/// 返回 `false` 表示命令通道已断开，进程应退出。
fn session_loop(
    device: &Device,
    inbox: &std::sync::mpsc::Receiver<WorkerCmd>,
    events: &UnboundedSender<SessionEvent>,
) -> bool {
    let mut last_revision = 0u64;
    loop {
        while let Some(event) = device.try_event() {
            let lost = matches!(event, SessionEvent::LinkLost(_));
            if events.send(event).is_err() {
                return false;
            }
            if lost {
                return true; // 结束本会话，外层重开
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
                    eprintln!("device: SET_CONVERSATION_LIST failed (session may be dead)");
                }
            }
            Ok(WorkerCmd::SetAgentIcons { entries }) => {
                let refs: Vec<(u8, &[u8])> = entries
                    .iter()
                    .map(|(i, png)| (*i, png.as_slice()))
                    .collect();
                let payload = match commands::set_agent_index(&refs) {
                    Ok(payload) => payload,
                    Err(err) => {
                        eprintln!("device: encode SET_AGENT_INDEX: {err}");
                        continue;
                    }
                };
                eprintln!(
                    "device: SET_AGENT_INDEX payload={} bytes ({} agents)",
                    payload.len(),
                    entries.len()
                );
                // 真机偶发慢 ACK：超时后重试一次。
                let mut ok = false;
                for attempt in 0..2 {
                    match device.request(command::SET_AGENT_INDEX, payload.clone()) {
                        Ok(_) => {
                            eprintln!("device: SET_AGENT_INDEX ok (attempt {})", attempt + 1);
                            ok = true;
                            break;
                        }
                        Err(err) => {
                            eprintln!(
                                "device: SET_AGENT_INDEX failed (attempt {}): {err}",
                                attempt + 1
                            );
                            thread::sleep(Duration::from_millis(300));
                        }
                    }
                }
                if !ok {
                    eprintln!("device: SET_AGENT_INDEX gave up; device may keep stale icons");
                }
            }
            Ok(WorkerCmd::SetAgentBalance { payload }) => {
                if let Err(err) = device.request(command::SET_AGENT_BALANCE, payload) {
                    eprintln!("device: SET_AGENT_BALANCE failed: {err}");
                }
            }
            Ok(WorkerCmd::SetSoundLight { payload }) => {
                if let Err(err) = device.request(command::SET_SOUND_LIGHT_CONFIG, payload) {
                    eprintln!("device: SET_SOUND_LIGHT_CONFIG failed: {err}");
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return false,
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
