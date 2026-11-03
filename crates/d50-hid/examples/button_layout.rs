//! 一次性对照实验：只改 `Buttons` 的内容，看设备点亮哪几颗实体键。
//!
//! 用法（会独占 HID，先把 agent-bridge-service 停掉）：
//!
//! ```text
//! cargo run -p d50-hid --features hid --example button_layout -- 0 12
//! ```
//!
//! 参数：设备索引、每一档停留的秒数。

use d50_hid::{Device, hid};
use d50_protocol::{Button, Conversation, command, commands};
use std::time::{Duration, Instant};

fn conversation(id: &str, buttons: &[(u8, &str)]) -> Conversation {
    Conversation {
        id: id.to_string(),
        title: "按钮布局实验".to_string(),
        path: "D:/demo".to_string(),
        content: "看哪几颗键亮".to_string(),
        agent_index: 0,
        needs_attention: 1,
        buttons: buttons
            .iter()
            .map(|(index, name)| Button {
                index: *index,
                name: (*name).to_string(),
            })
            .collect(),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let index: usize = args.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
    let hold: u64 = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(20);
    let lead: u64 = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(10);

    let link = match hid::open_index(index, true) {
        Ok(link) => link,
        Err(error) => {
            eprintln!("打开设备失败：{error}");
            std::process::exit(1);
        }
    };
    let device = Device::spawn(link);

    let states: [(&str, &[(u8, &str)]); 4] = [
        ("A 无按钮", &[]),
        ("B 两个按钮 Index 1/2", &[(1, "同意"), (2, "拒绝")]),
        ("C 两个按钮 Index 3/4", &[(3, "C3"), (4, "C4")]),
        ("D 四个按钮 Index 1..4", &[(1, "D1"), (2, "D2"), (3, "D3"), (4, "D4")]),
    ];

    println!("==== 对照实验：每档 {hold} 秒，请盯着设备按键灯 ====");
    for (name, _) in states {
        println!("  {name}");
    }
    println!("==== {lead} 秒后开始，现在看设备 ====");
    for left in (1..=lead).rev() {
        print!("\r等待 {left:>2}s ");
        use std::io::Write;
        let _ = std::io::stdout().flush();
        std::thread::sleep(Duration::from_secs(1));
    }
    println!();

    let started = Instant::now();
    for (state_index, (name, buttons)) in states.iter().enumerate() {
        let id = format!("probe-{}", state_index + 1);
        let payload = match commands::set_conversation_list(&[conversation(&id, buttons)]) {
            Ok(payload) => payload,
            Err(error) => {
                eprintln!("构造 payload 失败：{error:?}");
                return;
            }
        };
        if let Err(error) = device.request(command::SET_CONVERSATION_LIST, payload) {
            eprintln!("下发失败：{error:?}");
            return;
        }
        let at = started.elapsed().as_secs();
        println!("\n>>> [{at:>3}s] 状态 {name}：现在看设备，记住亮的是哪几颗键");

        let deadline = Instant::now() + Duration::from_secs(hold);
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_secs(1));
            let left = (deadline - Instant::now()).as_secs();
            if left > 0 && left % 3 == 0 {
                print!("…{left}s ");
                use std::io::Write;
                let _ = std::io::stdout().flush();
            }
        }
    }

    println!("\n\n清理：下发空会话列表");
    if let Ok(payload) = commands::set_conversation_list(&[]) {
        let _ = device.request(command::SET_CONVERSATION_LIST, payload);
    }
    println!("完成。可以重启 agent-bridge-service 了。");
}