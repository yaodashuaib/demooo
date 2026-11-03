//! 服务的小客户端：看状态、按按钮、发一条自检会话。
//!
//! `press` 只在 `--simulate` 模式下有意义——真按键只能从 HID 上来。

use clap::{Parser, Subcommand};
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(name = "d50ctl", about = "D50 审批桥接服务的命令行客户端")]
struct Cli {
    #[arg(long, default_value = bridge_ipc::DEFAULT_PIPE, global = true)]
    pipe: String,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 设备是否在线、显示了几条、排了几条。
    Status,
    /// 代替人按下某个会话的反馈按钮（仅 --simulate）。
    Press {
        /// 会话 ID，也就是 approval 请求的 request_id。
        #[arg(long)]
        id: String,
        /// 1–4 是反馈按钮；0 是「打开会话」的全局动作。
        #[arg(long, default_value_t = 1)]
        button: u8,
    },
    /// 推一条不阻塞任何人的演示会话，用来确认设备能显示、能按键。
    SelfTest,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let cli = Cli::parse();
    let request = match cli.command {
        Command::Status => bridge_ipc::Request::DeviceStatus,
        Command::Press { id, button } => bridge_ipc::Request::DevicePress {
            request_id: id,
            button_index: button,
        },
        Command::SelfTest => bridge_ipc::Request::DeviceSelfTest,
    };

    match bridge_ipc::call(&cli.pipe, &request, Duration::from_secs(8)).await {
        Ok(bridge_ipc::Response::Device { device }) => {
            println!("online   {}", device.online);
            println!("serial   {}", device.serial.unwrap_or_else(|| "-".into()));
            println!("firmware {}", device.firmware.unwrap_or_else(|| "-".into()));
            println!("shown    {}", device.shown);
            println!("queued   {}", device.queued);
        }
        Ok(bridge_ipc::Response::Ok) => println!("ok"),
        Ok(bridge_ipc::Response::Error { code, message }) => {
            eprintln!("{code}: {message}");
            std::process::exit(1);
        }
        Ok(other) => {
            eprintln!("unexpected response: {other:?}");
            std::process::exit(1);
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
