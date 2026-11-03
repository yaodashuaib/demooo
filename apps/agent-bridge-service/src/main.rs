use clap::Parser;

/// D50 审批门控桥接服务：命名管道进，设备 HID 出。
#[derive(Debug, Parser)]
#[command(name = "agent-bridge-service", version, about)]
struct Args {
    /// 命名管道名；hook 用同一个名字连接。
    #[arg(long, default_value = bridge_ipc::DEFAULT_PIPE)]
    pipe: String,
    /// 用内存版设备跑通全链路，不需要真机。
    #[arg(long)]
    simulate: bool,
    /// 只枚举 Vendor HID 并逐个握手，告诉你哪台是 D50、要不要补 Report 前缀。
    #[arg(long)]
    list_devices: bool,
    /// 多台设备时按序列号选择；序列号由设备在握手时自报，见 --list-devices。
    #[arg(long)]
    serial: Option<String>,
    /// 多台设备且都没报序列号时，按 --list-devices 的序号选择。
    #[arg(long, default_value_t = 0)]
    device_index: usize,
    /// 设备无 Report ID 时通常要补 0x00 前缀；--list-devices 会告诉你哪个能用。
    #[arg(long)]
    no_report_id_prefix: bool,
}

#[cfg(feature = "hid")]
fn list_devices() {
    let devices = match d50_hid::hid::candidates() {
        Ok(devices) => devices,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };
    if devices.is_empty() {
        println!("没有找到 Vendor HID（usage page 0xFF00）。");
        println!("确认：设备已插上；驱动是 HID 而不是串口；没有被别的程序独占。");
        return;
    }
    println!(
        "{:<3} {:<10} {:<18} {:<6} {}",
        "idx", "vid:pid", "usb-serial", "iface", "product"
    );
    for device in &devices {
        println!(
            "{:<3} {:04X}:{:04X}  {:<18} {:<6} {}",
            device.index,
            device.vendor_id,
            device.product_id,
            device.serial.clone().unwrap_or_else(|| "-".into()),
            device.interface,
            device.label()
        );
    }
    println!();
    println!("usb-serial 是 USB 描述符里的序列号，`--serial` 匹配的是它；");
    println!("下面一行是设备固件自报的身份，可能完全不同。");
    println!();
    for device in &devices {
        match probe(device.index) {
            Ok((probed, prefix)) => println!(
                "idx {} → {} 设备序列号 {} 固件 {}（Report 前缀 {}）",
                device.index,
                probed.device_type,
                probed.serial,
                probed.firmware,
                if prefix { "0x00" } else { "不需要" }
            ),
            Err(error) => println!(
                "idx {} → 握手未完成：{error}\n      设备没按协议回应，跑 `d50-probe --sweep` 看原始收发",
                device.index
            ),
        }
    }
}

#[cfg(feature = "hid")]
fn probe(index: usize) -> Result<(d50_hid::hid::Probed, bool), String> {
    // 两个方向都试一次：这正是 M0 要确认的那个字节。
    match d50_hid::hid::probe_index(index, true) {
        Ok(probed) => Ok((probed, true)),
        Err(first) => d50_hid::hid::probe_index(index, false)
            .map(|probed| (probed, false))
            .map_err(|_| first),
    }
}

#[cfg(not(feature = "hid"))]
fn list_devices() {
    eprintln!(
        "枚举 HID 需要重新构建：cargo run --features hid --bin agent-bridge-service -- --list-devices"
    );
    std::process::exit(2);
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    if args.list_devices {
        list_devices();
        return;
    }
    let config = agent_bridge_service::Config {
        pipe: args.pipe,
        simulate: args.simulate,
        serial: args.serial,
        device_index: args.device_index,
        report_id_prefix: !args.no_report_id_prefix,
    };
    let bridge = match agent_bridge_service::start(config) {
        Ok(bridge) => bridge,
        Err(error) => {
            eprintln!("agent-bridge-service: {error}");
            std::process::exit(2);
        }
    };
    eprintln!(
        "agent-bridge-service: pipe={} simulate={}",
        bridge.pipe, args.simulate
    );

    tokio::signal::ctrl_c().await.ok();
    bridge.shutdown().await;
}
