//! 三个选项的键位对照：下发 3 个按钮，然后记录每一次按键回报的 `ButtonIndex`。
//!
//! 设备在有实际按钮时，按一次就会清掉 `NeedsAttention`（按钮随即不可操作），
//! 所以这里每收到一次动作就立刻把同一份会话重新下发一遍，恢复可按状态。
//!
//! 用法（先把 agent-bridge-service 停掉）：
//!
//! ```text
//! cargo run -p d50-hid --features hid --example button_order -- 0 90
//! ```
//!
//! 参数：设备索引、监听秒数。

use d50_hid::{Device, SessionEvent, hid};
use d50_protocol::{Button, Conversation, command, commands};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

const LABELS: [&str; 3] = ["同意", "拒绝", "总是允许"];

fn conversation(id: &str) -> Conversation {
    Conversation {
        id: id.to_string(),
        title: "三个选项".to_string(),
        path: "D:/demo".to_string(),
        content: "请从左到右按一遍".to_string(),
        agent_index: 0,
        needs_attention: 1,
        buttons: LABELS
            .iter()
            .enumerate()
            .map(|(offset, name)| Button {
                index: offset as u8 + 1,
                name: (*name).to_string(),
            })
            .collect(),
    }
}

fn push(device: &Device, id: &str) -> bool {
    let Ok(payload) = commands::set_conversation_list(&[conversation(id)]) else {
        eprintln!("构造 payload 失败");
        return false;
    };
    match device.request(command::SET_CONVERSATION_LIST, payload) {
        Ok(_) => true,
        Err(error) => {
            eprintln!("下发失败：{error:?}");
            false
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let index: usize = args.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
    let seconds: u64 = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(90);

    let link = match hid::open_index(index, true) {
        Ok(link) => link,
        Err(error) => {
            eprintln!("打开设备失败：{error}");
            std::process::exit(1);
        }
    };
    let device = Device::spawn(link);

    let id = "order-probe";
    if !push(&device, id) {
        return;
    }

    println!("==== 三个选项的键位对照 ====");
    println!("设备上应亮三颗键（KEY3 / KEY4 / KEY5），从左到右依次是：");
    for (offset, name) in LABELS.iter().enumerate() {
        println!("  第 {} 颗：{}（Index {}）", offset + 1, name, offset + 1);
    }
    println!("请【从左到右】各按一次，每次按完等 2 秒；本条监视 {seconds} 秒。");
    println!("每次按键后会自动重置，可以继续按。");
    println!("============================");

    let mut seen: BTreeMap<u8, usize> = BTreeMap::new();
    let mut total = 0usize;
    let deadline = Instant::now() + Duration::from_secs(seconds);

    while Instant::now() < deadline {
        match device.recv_event_timeout(Duration::from_millis(200)) {
            Some(SessionEvent::Action(action)) => {
                total += 1;
                let position = seen.len();
                seen.entry(action.button_index).or_insert(position);
                let at = Instant::now();
                let left = deadline.saturating_duration_since(at).as_secs();
                let label = LABELS
                    .get(usize::from(action.button_index).saturating_sub(1))
                    .copied()
                    .unwrap_or("全局动作");
                println!(
                    "[剩余 {left:>3}s] 收到 ButtonIndex={} → {}",
                    action.button_index, label
                );
                // 按过一次就把 attention 标志清掉了，重新下发恢复可按。
                if !push(&device, id) {
                    return;
                }
            }
            Some(SessionEvent::LinkLost(reason)) => {
                eprintln!("链路断开：{reason}");
                break;
            }
            Some(_) => {}
            None => {}
        }
    }

    println!("\n==== 汇总 ====");
    if seen.is_empty() {
        println!("没有收到任何按键。");
    } else {
        let mut order: Vec<(u8, usize)> = seen.iter().map(|(k, v)| (*k, *v)).collect();
        order.sort_by_key(|(_, first)| *first);
        println!("按下的先后 → 回报的 ButtonIndex（共 {total} 次）：");
        for (button, first) in &order {
            println!("  第 {} 次按下 → ButtonIndex={button}", first + 1);
        }
    }

    println!("\n清理：下发空会话列表");
    if let Ok(payload) = commands::set_conversation_list(&[]) {
        let _ = device.request(command::SET_CONVERSATION_LIST, payload);
    }
}